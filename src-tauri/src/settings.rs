use anyhow::{bail, Context, Result};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Clone, Copy, Deserialize, Serialize, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum AccountAuth {
    Login,
    Manual,
}

impl AccountAuth {
    fn as_str(self) -> &'static str {
        match self {
            Self::Login => "login",
            Self::Manual => "manual",
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Profile {
    pub platform: String,
    pub user_agent: String,
    pub proxy: Option<String>,
    #[serde(default)]
    pub bypass_proxy: bool,
    pub cookie_configured: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub login_configured: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_auth: Option<AccountAuth>,
    #[serde(default)]
    pub http_chunk_size: Option<u64>,
    #[serde(skip_serializing)]
    pub credential_ref: Option<String>,
    #[serde(skip_serializing)]
    cookie: String,
    #[serde(default, skip_serializing)]
    login_cookie: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Settings {
    pub download_directory: PathBuf,
    pub filename_template: String,
    pub max_downloads: u8,
    pub profiles: Vec<Profile>,
}

#[derive(Deserialize)]
pub struct ProfileUpdate {
    pub platform: String,
    pub user_agent: String,
    // None keeps saved data; empty string explicitly clears it.
    pub cookie: Option<String>,
    pub proxy: Option<String>,
    pub bypass_proxy: Option<bool>,
    // Missing keeps the setting; explicit null inherits; 0 disables chunking.
    #[serde(default, deserialize_with = "deserialize_chunk_setting")]
    pub http_chunk_size: Option<Option<u64>>,
}

fn deserialize_chunk_setting<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Option<Option<u64>>, D::Error> {
    Option::<u64>::deserialize(deserializer).map(Some)
}

fn valid_chunk_size(value: u64) -> bool {
    value == 0 || (1_048_576..=1_073_741_824).contains(&value) && value.is_multiple_of(1_048_576)
}

fn entry(reference: &str) -> Result<keyring::Entry> {
    // This persistent credential namespace predates the bundle identifier change.
    // Keep it stable so existing authenticated proxy secrets remain accessible.
    keyring::Entry::new("com.yingqu.desktop", reference).context("CREDENTIAL_ERROR")
}

pub(crate) fn init(db: &rusqlite::Transaction<'_>) -> Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS app_settings (id INTEGER PRIMARY KEY CHECK(id=1), directory TEXT NOT NULL);
         CREATE TABLE IF NOT EXISTS platform_settings (
           platform TEXT PRIMARY KEY, user_agent TEXT NOT NULL, proxy TEXT,
           cookie TEXT NOT NULL, credential_ref TEXT);",
    ).context("DATABASE_ERROR")?;
    let platform_columns = db
        .prepare("PRAGMA table_info(platform_settings)")?
        .query_map([], |r| r.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if !platform_columns.iter().any(|name| name == "http_chunk_size") {
        db.execute_batch("ALTER TABLE platform_settings ADD COLUMN http_chunk_size INTEGER")
            .context("DATABASE_ERROR")?;
    }
    if !platform_columns.iter().any(|name| name == "bypass_proxy") {
        db.execute_batch("ALTER TABLE platform_settings ADD COLUMN bypass_proxy INTEGER NOT NULL DEFAULT 0 CHECK(bypass_proxy IN (0,1))")
            .context("DATABASE_ERROR")?;
    }
    if !platform_columns.iter().any(|name| name == "login_cookie") {
        let legacy_session = platform_columns.iter().any(|name| name == "yuanbao_cookie");
        if legacy_session {
            db.execute_batch("ALTER TABLE platform_settings RENAME COLUMN yuanbao_cookie TO login_cookie")
                .context("DATABASE_ERROR")?;
        } else {
            db.execute_batch("ALTER TABLE platform_settings ADD COLUMN login_cookie TEXT NOT NULL DEFAULT ''")
                .context("DATABASE_ERROR")?;
        }
    }
    if !platform_columns.iter().any(|name| name == "account_auth") {
        db.execute_batch("ALTER TABLE platform_settings ADD COLUMN account_auth TEXT
            CHECK(account_auth IN ('login','manual'))").context("DATABASE_ERROR")?;
        if platform_columns.iter().any(|name| name == "weixin_auth") {
            db.execute_batch(
                "UPDATE platform_settings SET account_auth = CASE weixin_auth
                    WHEN 'yuanbao' THEN 'login' WHEN 'manual' THEN 'manual'
                    WHEN 'browser' THEN NULL
                    ELSE CASE WHEN login_cookie <> '' THEN 'login' WHEN cookie <> '' THEN 'manual' END
                 END WHERE platform <> 'telegram';
                 ALTER TABLE platform_settings DROP COLUMN weixin_auth;",
            ).context("DATABASE_ERROR")?;
        } else {
            db.execute_batch(
                "UPDATE platform_settings SET account_auth = CASE
                    WHEN login_cookie <> '' THEN 'login' WHEN cookie <> '' THEN 'manual' END
                 WHERE platform <> 'telegram';",
            ).context("DATABASE_ERROR")?;
        }
    }
    // Rotate an opaque scope in the same transaction as an effective profile edit.
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS douyin_author_scope(
            id INTEGER PRIMARY KEY CHECK(id=1), scope TEXT NOT NULL);
         INSERT OR IGNORE INTO douyin_author_scope VALUES(1,lower(hex(randomblob(16))));
         CREATE TRIGGER IF NOT EXISTS douyin_author_profile_insert AFTER INSERT ON platform_settings
           WHEN NEW.platform='douyin' BEGIN
             UPDATE douyin_author_scope SET scope=lower(hex(randomblob(16))) WHERE id=1;
           END;
         CREATE TRIGGER IF NOT EXISTS douyin_author_profile_update AFTER UPDATE ON platform_settings
           WHEN NEW.platform='douyin' AND (
             OLD.cookie IS NOT NEW.cookie OR OLD.user_agent IS NOT NEW.user_agent
             OR OLD.proxy IS NOT NEW.proxy OR OLD.credential_ref IS NOT NEW.credential_ref
             OR OLD.bypass_proxy IS NOT NEW.bypass_proxy) BEGIN
             UPDATE douyin_author_scope SET scope=lower(hex(randomblob(16))) WHERE id=1;
           END;",
    )
    .context("DATABASE_ERROR")?;
    let has_limit = db
        .prepare("PRAGMA table_info(app_settings)")?
        .query_map([], |r| r.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?
        .iter()
        .any(|name| name == "max_downloads");
    if !has_limit {
        db.execute_batch("ALTER TABLE app_settings ADD COLUMN max_downloads INTEGER NOT NULL DEFAULT 5 CHECK(max_downloads BETWEEN 1 AND 10);")
            .context("DATABASE_ERROR")?;
    }
    let has_template = db
        .prepare("PRAGMA table_info(app_settings)")?
        .query_map([], |r| r.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?
        .iter()
        .any(|name| name == "filename_template");
    if !has_template {
        db.execute_batch("ALTER TABLE app_settings ADD COLUMN filename_template TEXT NOT NULL DEFAULT '{platform}/{user_name}-{content}'")
            .context("DATABASE_ERROR")?;
    }
    // Older installations use DEFAULT 2 / 3 with a 1..=3 / 1..=5 CHECK.
    // SQLite requires rebuilding the table to widen that constraint; retain
    // the user's directory, naming rule and chosen limit in one transaction.
    let limit_default: String = db
        .query_row(
            "SELECT dflt_value FROM pragma_table_info('app_settings') WHERE name='max_downloads'",
            [],
            |row| row.get(0),
        )
        .context("DATABASE_ERROR")?;
    if limit_default != "5" {
        db.execute_batch(
            "CREATE TABLE app_settings_expanded (
               id INTEGER PRIMARY KEY CHECK(id=1), directory TEXT NOT NULL,
               max_downloads INTEGER NOT NULL DEFAULT 5 CHECK(max_downloads BETWEEN 1 AND 10),
               filename_template TEXT NOT NULL DEFAULT '{platform}/{user_name}-{content}');
             INSERT INTO app_settings_expanded(id,directory,max_downloads,filename_template)
               SELECT id,directory,max_downloads,filename_template FROM app_settings;
             DROP TABLE app_settings;
             ALTER TABLE app_settings_expanded RENAME TO app_settings;",
        )
        .context("DATABASE_ERROR")?;
    }
    db.execute_batch(
        "CREATE TRIGGER IF NOT EXISTS douyin_author_login_update AFTER UPDATE ON platform_settings
           WHEN NEW.platform='douyin' AND (
             OLD.login_cookie IS NOT NEW.login_cookie OR OLD.account_auth IS NOT NEW.account_auth) BEGIN
             UPDATE douyin_author_scope SET scope=lower(hex(randomblob(16))) WHERE id=1;
           END;",
    ).context("DATABASE_ERROR")?;
    Ok(())
}

pub fn load(db: &Connection, default_directory: PathBuf) -> Result<Settings> {
    let tx = db.unchecked_transaction().context("DATABASE_ERROR")?;
    init(&tx)?;
    tx.commit().context("DATABASE_ERROR")?;
    let filename_template = match db.query_row(
        "SELECT filename_template FROM app_settings WHERE id=1",
        [],
        |r| r.get::<_, String>(0),
    ) {
        Ok(value) => {
            crate::naming::validate(&value).context("DATABASE_ERROR")?;
            value
        }
        Err(rusqlite::Error::QueryReturnedNoRows) => crate::naming::DEFAULT_TEMPLATE.into(),
        Err(_) => bail!("DATABASE_ERROR"),
    };
    let max_downloads = match db.query_row(
        "SELECT max_downloads FROM app_settings WHERE id=1",
        [],
        |r| r.get::<_, u8>(0),
    ) {
        Ok(value @ 1..=10) => value,
        Err(rusqlite::Error::QueryReturnedNoRows) => 5,
        _ => bail!("DATABASE_ERROR"),
    };
    let directory = db.query_row("SELECT directory FROM app_settings WHERE id=1", [], |r| {
        r.get::<_, String>(0)
    });
    let download_directory = match directory {
        Ok(path) => PathBuf::from(path),
        Err(rusqlite::Error::QueryReturnedNoRows) => default_directory,
        Err(_) => bail!("DATABASE_ERROR"),
    };
    let profiles = [
        "douyin",
        "xiaohongshu",
        "tiktok",
        "youtube",
        "telegram",
        "weixin",
    ]
    .into_iter()
    .map(|platform| read_profile(db, platform))
    .collect::<Result<Vec<_>>>()?;
    if profiles.iter().any(|p| {
        p.http_chunk_size
            .is_some_and(|n| p.platform != "youtube" || !valid_chunk_size(n))
    }) {
        bail!("DATABASE_ERROR");
    }
    Ok(Settings {
        download_directory,
        filename_template,
        max_downloads,
        profiles,
    })
}

pub(crate) fn read_profile(db: &Connection, platform: &str) -> Result<Profile> {
    let row = db.query_row(
        "SELECT user_agent,proxy,cookie,credential_ref,http_chunk_size,bypass_proxy,login_cookie,account_auth FROM platform_settings WHERE platform=?",
        [platform],
        |r| {
            let cookie: String = r.get(2)?;
            let login_cookie: String = r.get(6)?;
            let account_auth = match r.get::<_, Option<String>>(7)?.as_deref() {
                Some("login") => Some(AccountAuth::Login),
                Some("manual") => Some(AccountAuth::Manual),
                None => None,
                Some(_) => return Err(rusqlite::Error::InvalidQuery),
            };
            Ok(Profile {
                platform: platform.into(),
                user_agent: r.get(0)?,
                proxy: r.get(1)?,
                bypass_proxy: r.get(5)?,
                cookie_configured: !cookie.is_empty(),
                login_configured: !login_cookie.is_empty(),
                account_auth,
                cookie,
                login_cookie,
                credential_ref: r.get(3)?,
                http_chunk_size: r.get::<_, Option<u32>>(4)?.map(u64::from),
            })
        },
    );
    Ok(match row {
        Ok(profile) => profile,
        Err(rusqlite::Error::QueryReturnedNoRows) => Profile {
            platform: platform.into(),
            user_agent: "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/150.0.0.0 Safari/537.36 Edg/150.0.0.0".into(),
            proxy: None, bypass_proxy: false, cookie_configured: false, cookie: String::new(), login_configured: false, login_cookie: String::new(), account_auth: None, credential_ref: None, http_chunk_size: None,
        },
        Err(_) => bail!("DATABASE_ERROR"),
    })
}

// Explicit settings-editor read; normal settings and task events omit cookies.
pub fn cookie(settings: &Settings, platform: &str) -> Result<String> {
    Ok(settings
        .profiles
        .iter()
        .find(|p| p.platform == platform)
        .context("CONFIG_INVALID")?
        .cookie
        .clone())
}

// Explicit settings-editor read; ordinary settings only contain the public address.
pub fn proxy(settings: &Settings, platform: &str) -> Result<Option<String>> {
    let profile = settings
        .profiles
        .iter()
        .find(|p| p.platform == platform)
        .context("CONFIG_INVALID")?;
    profile_proxy(profile)
}
pub(crate) fn profile_proxy(profile: &Profile) -> Result<Option<String>> {
    match &profile.credential_ref {
        Some(reference) => Ok(Some(
            String::from_utf8(entry(reference)?.get_secret().context("CREDENTIAL_ERROR")?)
                .context("CREDENTIAL_ERROR")?,
        )),
        None => Ok(profile.proxy.clone()),
    }
}

pub(crate) fn effective_proxy(profile: &Profile) -> Result<Option<String>> {
    if profile.bypass_proxy {
        Ok(None)
    } else {
        profile_proxy(profile)
    }
}

pub fn snapshot(settings: &Settings) -> Result<Value> {
    let mut profiles = serde_json::Map::new();
    for profile in settings
        .profiles
        .iter()
        .filter(|p| p.platform != "telegram")
    {
        let proxy = effective_proxy(profile)?;
        let mut value = json!({"user_agent": profile.user_agent, "proxy": proxy});
        profile.write_auth_snapshot(&mut value);
        if profile.platform == "youtube" {
            value["http_chunk_size"] = json!(profile.http_chunk_size);
        }
        profiles.insert(profile.platform.clone(), value);
    }
    Ok(Value::Object(profiles))
}

impl Profile {
    fn active_cookie(&self) -> &str {
        match self.account_auth {
            Some(AccountAuth::Login) => &self.login_cookie,
            Some(AccountAuth::Manual) => &self.cookie,
            None => "",
        }
    }

    fn write_auth_snapshot(&self, value: &mut Value) {
        value["cookie"] = json!(self.active_cookie());
        value["login_session"] = json!(self.account_auth == Some(AccountAuth::Login) && self.platform != "weixin");
    }
}

pub(crate) fn refresh_account_auth(settings: &Settings, platform: &str, snapshot: &mut Value) {
    if let Some(profile) = settings.profiles.iter().find(|p| p.platform == platform) {
        profile.write_auth_snapshot(&mut snapshot[platform]);
    }
}

fn valid_manual_cookie(cookie: &str) -> bool {
    if cookie.is_empty() {
        return true;
    }
    if cookie.len() > 60_000 || cookie.contains(['\r', '\n']) {
        return false;
    }
    let mut pairs = cookie.split(';').map(str::trim).filter(|part| !part.is_empty()).peekable();
    pairs.peek().is_some() && pairs.all(|part| {
        part.split_once('=').is_some_and(|(name, value)| {
            !name.is_empty()
                && name.bytes().all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+.^_`|~-".contains(&b))
                && value.bytes().all(|b| matches!(b, 32..=126))
        })
    })
}

fn normalize_manual_cookie<'a>(platform: &str, cookie: &'a str) -> Result<&'a str> {
    let cookie = cookie.trim();
    if platform != "douyin" {
        return Ok(cookie);
    }
    let Some((host, header)) = cookie.split_once(';') else {
        return Ok(cookie);
    };
    let host = host.trim();
    let host = host.strip_prefix('.').unwrap_or(host);
    if !host.eq_ignore_ascii_case("douyin.com") && !host.eq_ignore_ascii_case("www.douyin.com") {
        return Ok(cookie);
    }
    let header = header.trim();
    if cookie.len() > 60_000 || cookie.contains(['\r', '\n']) || header.is_empty()
        || !valid_manual_cookie(header)
    {
        bail!("CONFIG_INVALID");
    }
    Ok(header)
}

pub(crate) fn update_account_auth(
    db: &Connection,
    settings: &mut Settings,
    platform: &str,
    mode: AccountAuth,
    credential: Option<String>,
) -> Result<()> {
    let profile = settings
        .profiles
        .iter_mut()
        .find(|p| p.platform == platform && platform != "telegram")
        .context("CONFIG_INVALID")?;
    let manual = match (mode, credential.as_deref()) {
        (AccountAuth::Manual, Some(cookie)) => normalize_manual_cookie(platform, cookie)?,
        _ => &profile.cookie,
    };
    let login = match (mode, credential.as_deref()) {
        (AccountAuth::Login, Some(cookie)) => cookie,
        _ => &profile.login_cookie,
    };
    if mode == AccountAuth::Manual && !valid_manual_cookie(manual) {
        bail!("CONFIG_INVALID");
    }
    // Clearing an active credential leaves the account unconfigured.
    let active = match mode {
        AccountAuth::Login if login.is_empty() => profile.account_auth.filter(|active| *active != mode),
        AccountAuth::Manual if manual.is_empty() => profile.account_auth.filter(|active| *active != mode),
        _ => Some(mode),
    };
    if credential.is_none() && active != Some(mode) {
        bail!("PLATFORM_LOGIN_REQUIRED");
    }
    db.execute(
        "INSERT INTO platform_settings(platform,user_agent,cookie,login_cookie,account_auth) VALUES (?1,?2,?3,?4,?5)
         ON CONFLICT(platform) DO UPDATE SET cookie=excluded.cookie,login_cookie=excluded.login_cookie,account_auth=excluded.account_auth",
        rusqlite::params![platform, profile.user_agent, manual, login, active.map(AccountAuth::as_str)],
    )
    .context("DATABASE_ERROR")?;
    profile.cookie_configured = !manual.is_empty();
    profile.login_configured = !login.is_empty();
    profile.account_auth = active;
    match (mode, credential.as_ref()) {
        (AccountAuth::Login, Some(cookie)) => profile.login_cookie.clone_from(cookie),
        (AccountAuth::Manual, Some(_)) => profile.cookie = manual.to_owned(),
        _ => {}
    }
    Ok(())
}

pub(crate) fn douyin_snapshot(settings: &Settings) -> Result<Value> {
    let profile = settings
        .profiles
        .iter()
        .find(|profile| profile.platform == "douyin")
        .context("CONFIG_INVALID")?;
    let mut value = json!({"user_agent": profile.user_agent, "proxy": effective_proxy(profile)?});
    profile.write_auth_snapshot(&mut value);
    Ok(json!({"douyin": value}))
}

pub(crate) fn parse_proxy(platform: &str, proxy: &str) -> Result<(url::Url, u16)> {
    let parsed = url::Url::parse(proxy).context("CONFIG_INVALID")?;
    let authority = proxy
        .split_once("://")
        .context("CONFIG_INVALID")?
        .1
        .split('/')
        .next()
        .unwrap();
    let port: u16 = authority
        .rsplit_once(':')
        .context("CONFIG_INVALID")?
        .1
        .parse()
        .context("CONFIG_INVALID")?;
    if proxy.len() > 2000
        || !matches!(parsed.scheme(), "http" | "socks5")
        || platform == "telegram" && parsed.scheme() != "socks5"
        || parsed.host_str().is_none()
        || port == 0
        || parsed.query().is_some()
        || parsed.fragment().is_some()
        || parsed.path() != "/" && !parsed.path().is_empty()
    {
        bail!("CONFIG_INVALID");
    }
    Ok((parsed, port))
}

pub(crate) fn validate_profile_update(settings: &Settings, update: &ProfileUpdate) -> Result<()> {
    let profile = settings
        .profiles
        .iter()
        .find(|p| p.platform == update.platform)
        .context("CONFIG_INVALID")?;
    if let Some(value) = update.http_chunk_size {
        if update.platform != "youtube" || value.is_some_and(|n| !valid_chunk_size(n)) {
            bail!("CONFIG_INVALID");
        }
    }
    if update.user_agent.is_empty()
        || update.user_agent.len() > 2048
        || update.user_agent.contains(['\r', '\n'])
    {
        bail!("CONFIG_INVALID");
    }
    let cookie = update
        .cookie
        .as_deref()
        .map(str::trim)
        .unwrap_or(&profile.cookie);
    if cookie.len() > 60_000 || cookie.contains(['\r', '\n']) {
        bail!("CONFIG_INVALID");
    }
    if let Some(proxy) = update.proxy.as_deref().filter(|proxy| !proxy.is_empty()) {
        parse_proxy(&update.platform, proxy)?;
    }
    Ok(())
}

pub fn save_profile(db: &Connection, settings: &mut Settings, update: ProfileUpdate) -> Result<()> {
    validate_profile_update(settings, &update)?;
    let profile = settings
        .profiles
        .iter_mut()
        .find(|p| p.platform == update.platform)
        .context("CONFIG_INVALID")?;
    let http_chunk_size = update.http_chunk_size.unwrap_or(profile.http_chunk_size);
    let bypass_proxy = update.bypass_proxy.unwrap_or(profile.bypass_proxy);
    let cookie = update
        .cookie
        .as_deref()
        .map(|cookie| normalize_manual_cookie(&update.platform, cookie))
        .transpose()?
        .unwrap_or(&profile.cookie)
        .to_owned();
    let account_auth = if update.platform != "telegram" && update.cookie.is_some() {
        if cookie.is_empty() {
            profile.account_auth.filter(|mode| *mode != AccountAuth::Manual)
        } else {
            Some(AccountAuth::Manual)
        }
    } else {
        profile.account_auth
    };
    let mut reference = profile.credential_ref.clone();
    let mut new_credential = None;
    let mut public_proxy = profile.proxy.clone();
    if let Some(proxy) = update.proxy {
        reference = None;
        if proxy.is_empty() {
            public_proxy = None;
        } else {
            let (mut parsed, port) = parse_proxy(&update.platform, &proxy)?;
            if !parsed.username().is_empty() || parsed.password().is_some() {
                let id = Uuid::new_v4().to_string();
                entry(&id)?
                    .set_secret(proxy.as_bytes())
                    .context("CREDENTIAL_ERROR")?;
                reference = Some(id.clone());
                new_credential = Some(id);
            }
            parsed
                .set_password(None)
                .map_err(|_| anyhow::anyhow!("CONFIG_INVALID"))?;
            parsed
                .set_username("")
                .map_err(|_| anyhow::anyhow!("CONFIG_INVALID"))?;
            public_proxy = Some(format!(
                "{}://{}:{port}/",
                parsed.scheme(),
                parsed.host().context("CONFIG_INVALID")?
            ));
        }
    }
    // User decision: Cookies reside in SQLite. Only authenticated proxy URLs
    // use OS credential storage. Failed writes preserve the previous reference.
    let saved = db.execute(
        "INSERT INTO platform_settings(platform,user_agent,proxy,cookie,credential_ref,http_chunk_size,bypass_proxy,account_auth) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)
         ON CONFLICT(platform) DO UPDATE SET user_agent=excluded.user_agent,proxy=excluded.proxy,
         cookie=excluded.cookie,credential_ref=excluded.credential_ref,http_chunk_size=excluded.http_chunk_size,bypass_proxy=excluded.bypass_proxy,account_auth=excluded.account_auth",
        rusqlite::params![
            update.platform,
            update.user_agent,
            public_proxy,
            cookie,
            reference,
            http_chunk_size.map(|n| n as i64),
            bypass_proxy,
            account_auth.map(AccountAuth::as_str)
        ],
    );
    if saved.is_err() {
        if let Some(id) = new_credential {
            let _ = entry(&id)?.delete_credential();
        }
        bail!("DATABASE_ERROR");
    }
    let old_reference = profile.credential_ref.clone();
    *profile = Profile {
        platform: update.platform,
        user_agent: update.user_agent,
        proxy: public_proxy,
        bypass_proxy,
        cookie_configured: !cookie.is_empty(),
        login_configured: profile.login_configured,
        account_auth,
        cookie,
        login_cookie: std::mem::take(&mut profile.login_cookie),
        credential_ref: reference.clone(),
        http_chunk_size,
    };
    if let Some(old) = old_reference.filter(|old| Some(old) != reference.as_ref()) {
        entry(&old)?
            .delete_credential()
            .context("CREDENTIAL_CLEANUP_ERROR")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile_for<'a>(settings: &'a Settings, platform: &str) -> &'a Profile {
        settings
            .profiles
            .iter()
            .find(|profile| profile.platform == platform)
            .unwrap()
    }

    fn profile_for_mut<'a>(settings: &'a mut Settings, platform: &str) -> &'a mut Profile {
        settings
            .profiles
            .iter_mut()
            .find(|profile| profile.platform == platform)
            .unwrap()
    }
    #[test]
    fn bypass_proxy_migrates_preserves_addresses_and_freezes_task_routes() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE platform_settings (platform TEXT PRIMARY KEY, user_agent TEXT NOT NULL,
            proxy TEXT, cookie TEXT NOT NULL, credential_ref TEXT);
            INSERT INTO platform_settings VALUES ('youtube','fixture','http://127.0.0.1:7890/','session=fixture',NULL);")
            .unwrap();
        let mut settings = load(&db, PathBuf::new()).unwrap();
        assert!(settings.profiles.iter().all(|p| !p.bypass_proxy));
        let captured = snapshot(&settings).unwrap();
        for enabled in [true, false] {
            let update = serde_json::from_value(
                json!({"platform":"youtube","user_agent":"fixture","bypass_proxy":enabled}),
            )
            .unwrap();
            save_profile(&db, &mut settings, update).unwrap();
            let restored = load(&db, PathBuf::new()).unwrap();
            assert_eq!(profile_for(&restored, "youtube").bypass_proxy, enabled);
            assert_eq!(
                proxy(&restored, "youtube").unwrap().as_deref(),
                Some("http://127.0.0.1:7890/")
            );
            let current = snapshot(&restored).unwrap();
            assert_eq!(
                current["youtube"]["proxy"],
                if enabled {
                    Value::Null
                } else {
                    json!("http://127.0.0.1:7890/")
                }
            );
            assert_eq!(current["youtube"]["cookie"], "session=fixture");
            assert_eq!(captured["youtube"]["proxy"], "http://127.0.0.1:7890/");
            assert!(restored
                .profiles
                .iter()
                .filter(|p| p.platform != "youtube")
                .all(|p| !p.bypass_proxy));
            // Older callers omit the flag and must preserve the stored choice.
            save_profile(
                &db,
                &mut settings,
                serde_json::from_value(json!({"platform":"youtube","user_agent":"fixture"}))
                    .unwrap(),
            )
            .unwrap();
            assert_eq!(
                profile_for(&load(&db, PathBuf::new()).unwrap(), "youtube").bypass_proxy,
                enabled
            );
        }
        profile_for_mut(&mut settings, "youtube").credential_ref =
            Some(format!("bypass-fixture-{}", Uuid::new_v4()));
        profile_for_mut(&mut settings, "youtube").bypass_proxy = true;
        assert!(
            snapshot(&settings).unwrap()["youtube"]["proxy"].is_null(),
            "bypassed proxy must not read its credential"
        );
        profile_for_mut(&mut settings, "youtube").bypass_proxy = false;
        assert!(
            snapshot(&settings).is_err(),
            "active proxy must preserve credential error semantics"
        );
    }

    #[test]
    fn bypass_proxy_database_failure_preserves_previous_route() {
        let db = Connection::open_in_memory().unwrap();
        let mut settings = load(&db, PathBuf::new()).unwrap();
        db.execute_batch("CREATE TEMP TRIGGER reject_bypass BEFORE INSERT ON platform_settings BEGIN SELECT RAISE(ABORT,'fixture failure'); END;").unwrap();
        let update = serde_json::from_value(
            json!({"platform":"tiktok","user_agent":"fixture","bypass_proxy":true}),
        )
        .unwrap();
        assert_eq!(
            save_profile(&db, &mut settings, update)
                .unwrap_err()
                .to_string(),
            "DATABASE_ERROR"
        );
        assert!(!profile_for(&settings, "tiktok").bypass_proxy);
        assert!(!profile_for(&load(&db, PathBuf::new()).unwrap(), "tiktok").bypass_proxy);
    }

    #[test]
    fn obsolete_external_tool_settings_are_ignored_and_preserved() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE tdl_settings(id INTEGER PRIMARY KEY, enabled INTEGER, path TEXT, session_directory TEXT); INSERT INTO tdl_settings VALUES(1,1,'missing-tool','old-session');").unwrap();
        let settings = load(&db, PathBuf::from("downloads")).unwrap();
        assert!(serde_json::to_value(settings).unwrap().get("tdl").is_none());
        assert_eq!(
            db.query_row("SELECT session_directory FROM tdl_settings", [], |r| r
                .get::<_, String>(
                0
            ))
            .unwrap(),
            "old-session"
        );
        let fresh = Connection::open_in_memory().unwrap();
        load(&fresh, PathBuf::from("downloads")).unwrap();
        assert_eq!(
            fresh
                .query_row(
                    "SELECT count(*) FROM sqlite_master WHERE name='tdl_settings'",
                    [],
                    |r| r.get::<_, u32>(0)
                )
                .unwrap(),
            0
        );
    }
    #[test]
    fn proxy_editor_reads_persisted_addresses_and_preserves_failure_semantics() {
        let db = Connection::open_in_memory().unwrap();
        let mut settings = load(&db, PathBuf::new()).unwrap();
        assert!(proxy(&settings, "youtube").unwrap().is_none());
        assert_eq!(
            proxy(&settings, "invalid").unwrap_err().to_string(),
            "CONFIG_INVALID"
        );
        for value in ["http://127.0.0.1:80", "socks5://127.0.0.1:7890"] {
            save_profile(
                &db,
                &mut settings,
                ProfileUpdate {
                    platform: "youtube".into(),
                    user_agent: "fixture".into(),
                    cookie: None,
                    proxy: Some(value.into()),
                    bypass_proxy: None,
                    http_chunk_size: None,
                },
            )
            .unwrap();
            let restored = load(&db, PathBuf::new()).unwrap();
            assert_eq!(
                proxy(&restored, "youtube").unwrap(),
                Some(format!("{value}/"))
            );
            assert_eq!(
                snapshot(&restored).unwrap()["youtube"]["proxy"],
                format!("{value}/")
            );
            assert!(proxy(&restored, "tiktok").unwrap().is_none());
        }
        // A new, nonexistent reference never queries an existing user's credential.
        let reference = Uuid::new_v4().to_string();
        profile_for_mut(&mut settings, "youtube").credential_ref = Some(reference.clone());
        assert_eq!(
            proxy(&settings, "youtube").unwrap_err().to_string(),
            "CREDENTIAL_ERROR"
        );
        assert!(!serde_json::to_string(&settings)
            .unwrap()
            .contains(&reference));
    }

    #[test]
    fn youtube_chunk_modes_migrate_persist_and_snapshot_without_changing_credentials() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE platform_settings (platform TEXT PRIMARY KEY, user_agent TEXT NOT NULL,
            proxy TEXT, cookie TEXT NOT NULL, credential_ref TEXT);
            INSERT INTO platform_settings VALUES ('youtube','old-agent','http://127.0.0.1:7890/','session=fixture',NULL);")
            .unwrap();
        let mut settings = load(&db, PathBuf::new()).unwrap();
        assert!(snapshot(&settings).unwrap()["youtube"]["http_chunk_size"].is_null());
        assert!(snapshot(&settings).unwrap()["tiktok"]
            .get("http_chunk_size")
            .is_none());
        for value in [Some(10_485_760_u64), Some(0), Some(1_073_741_824), None] {
            let update =
                serde_json::from_value(json!({"platform":"youtube", "user_agent":"old-agent",
                "http_chunk_size":value}))
                .unwrap();
            save_profile(&db, &mut settings, update).unwrap();
            let restored = load(&db, PathBuf::new()).unwrap();
            assert_eq!(profile_for(&restored, "youtube").http_chunk_size, value);
            let captured = snapshot(&restored).unwrap();
            assert_eq!(captured["youtube"]["http_chunk_size"], json!(value));
            assert_eq!(captured["youtube"]["cookie"], "session=fixture");
            assert_eq!(captured["youtube"]["proxy"], "http://127.0.0.1:7890/");
            assert!(!serde_json::to_string(&restored)
                .unwrap()
                .contains("session=fixture"));
            // Old callers that omit the new field must preserve the saved mode.
            save_profile(
                &db,
                &mut settings,
                serde_json::from_value(json!({"platform":"youtube",
                "user_agent":"old-agent"}))
                .unwrap(),
            )
            .unwrap();
            assert_eq!(
                profile_for(&load(&db, PathBuf::new()).unwrap(), "youtube").http_chunk_size,
                value
            );
        }
    }

    #[test]
    fn invalid_chunk_settings_cannot_change_saved_cookie_or_mode() {
        let db = Connection::open_in_memory().unwrap();
        let mut settings = load(&db, PathBuf::new()).unwrap();
        for (platform, value) in [
            ("youtube", Some(1)),
            ("youtube", Some(1_048_577)),
            ("youtube", Some(1_074_790_400)),
            ("youtube", Some(u64::MAX)),
            ("douyin", None),
            ("tiktok", Some(0)),
        ] {
            let error = save_profile(
                &db,
                &mut settings,
                ProfileUpdate {
                    platform: platform.into(),
                    user_agent: "test".into(),
                    cookie: Some("secret=changed".into()),
                    proxy: None,
                    bypass_proxy: None,
                    http_chunk_size: Some(value),
                },
            )
            .unwrap_err();
            assert_eq!(error.to_string(), "CONFIG_INVALID");
            assert_eq!(snapshot(&settings).unwrap()[platform]["cookie"], "");
            assert!(profile_for(&load(&db, PathBuf::new()).unwrap(), "youtube")
                .http_chunk_size
                .is_none());
        }
        for value in [json!(-1), json!(true), json!(1.5), json!("10485760")] {
            assert!(
                serde_json::from_value::<ProfileUpdate>(json!({"platform":"youtube",
                "user_agent":"test", "http_chunk_size":value}))
                .is_err()
            );
        }
    }

    #[test]
    fn invalid_persisted_chunk_size_reports_database_error() {
        let db = Connection::open_in_memory().unwrap();
        load(&db, PathBuf::new()).unwrap();
        for (platform, value) in [
            ("youtube", -1_i64),
            ("youtube", 1),
            ("youtube", i64::MAX),
            ("douyin", 0),
        ] {
            db.execute("DELETE FROM platform_settings", []).unwrap();
            db.execute("INSERT INTO platform_settings (platform,user_agent,cookie,http_chunk_size) VALUES (?1,'test','',?2)",
                rusqlite::params![platform, value]).unwrap();
            assert_eq!(
                load(&db, PathBuf::new()).err().unwrap().to_string(),
                "DATABASE_ERROR"
            );
        }
    }

    #[test]
    fn cookie_boundary_whitespace_is_trimmed_for_all_platforms() {
        let db = Connection::open_in_memory().unwrap();
        let mut settings = load(&db, PathBuf::new()).unwrap();
        for platform in ["douyin", "xiaohongshu", "tiktok", "youtube", "weixin"] {
            for (input, expected) in [
                (
                    "\r\n \t session=fixture; preference=a b \r\n\t",
                    "session=fixture; preference=a b",
                ),
                ("\r\n \t\r\n", ""),
            ] {
                save_profile(
                    &db,
                    &mut settings,
                    ProfileUpdate {
                        bypass_proxy: None,
                        http_chunk_size: None,
                        platform: platform.into(),
                        user_agent: "test-agent".into(),
                        cookie: Some(input.into()),
                        proxy: None,
                    },
                )
                .unwrap();
                let restored = load(&db, PathBuf::new()).unwrap();
                assert_eq!(super::cookie(&restored, platform).unwrap(), expected);
                assert_eq!(snapshot(&restored).unwrap()[platform]["cookie"], expected);
                assert_eq!(
                    restored
                        .profiles
                        .iter()
                        .find(|p| p.platform == platform)
                        .unwrap()
                        .cookie_configured,
                    !expected.is_empty()
                );
            }
        }
    }
    #[test]
    fn douyin_copied_cookie_saves_without_the_domain_prefix() {
        let db = Connection::open_in_memory().unwrap();
        let mut settings = load(&db, PathBuf::new()).unwrap();
        let header = "sessionid=synthetic; fpk1=abc+/==; SEARCH_RESULT_LIST_TYPE=%22single%22;";
        for host in ["douyin.com", ".douyin.com", "WWW.DOUYIN.COM"] {
            let input = format!(" {host}; {header} ");
            update_account_auth(&db, &mut settings, "douyin", AccountAuth::Manual, Some(input))
                .unwrap();
            assert_eq!(super::cookie(&settings, "douyin").unwrap(), header);
        }
        for invalid in [
            "douyin.com;",
            "douyin.com; ;",
            "example.com; sessionid=synthetic",
            "douyin.com; invalid; sessionid=synthetic",
            "douyin.com;\nsessionid=synthetic",
        ] {
            assert!(update_account_auth(
                &db, &mut settings, "douyin", AccountAuth::Manual, Some(invalid.into()),
            ).is_err());
        }
        assert!(update_account_auth(
            &db, &mut settings, "tiktok", AccountAuth::Manual, Some(format!("douyin.com; {header}")),
        ).is_err());
        let restored = load(&db, PathBuf::new()).unwrap();
        assert_eq!(super::cookie(&restored, "douyin").unwrap(), header);
        assert_eq!(douyin_snapshot(&restored).unwrap()["douyin"]["cookie"], header);

        save_profile(
            &db,
            &mut settings,
            ProfileUpdate {
                platform: "douyin".into(),
                user_agent: "fixture-agent".into(),
                cookie: Some("douyin.com; sessionid=updated".into()),
                proxy: None,
                http_chunk_size: None,
                bypass_proxy: None,
            },
        ).unwrap();
        assert_eq!(super::cookie(&settings, "douyin").unwrap(), "sessionid=updated");
    }

    #[test]
    fn cookie_internal_newlines_are_rejected_without_changing_saved_values() {
        let db = Connection::open_in_memory().unwrap();
        let mut settings = load(&db, PathBuf::new()).unwrap();
        for platform in ["douyin", "xiaohongshu", "tiktok", "youtube", "weixin"] {
            save_profile(
                &db,
                &mut settings,
                ProfileUpdate {
                    bypass_proxy: None,
                    http_chunk_size: None,
                    platform: platform.into(),
                    user_agent: "test-agent".into(),
                    cookie: Some("session=fixture".into()),
                    proxy: None,
                },
            )
            .unwrap();
            for input in ["session=one\r\nother=two", "session=one\n\nother=two"] {
                assert!(save_profile(
                    &db,
                    &mut settings,
                    ProfileUpdate {
                        bypass_proxy: None,
                        http_chunk_size: None,
                        platform: platform.into(),
                        user_agent: "test-agent".into(),
                        cookie: Some(input.into()),
                        proxy: None,
                    },
                )
                .is_err());
                let restored = load(&db, PathBuf::new()).unwrap();
                assert_eq!(
                    super::cookie(&restored, platform).unwrap(),
                    "session=fixture"
                );
                assert_eq!(
                    super::cookie(&settings, platform).unwrap(),
                    "session=fixture"
                );
            }
        }
    }
    #[test]
    fn yuanbao_session_survives_restart_without_exposing_or_overwriting_manual_cookie() {
        let db = Connection::open_in_memory().unwrap();
        let mut settings = load(&db, PathBuf::new()).unwrap();
        let manual = "hy_token=manual-fixture";
        let imported = "hy_user=fixture; hy_token=scan-fixture; hy_source=web";
        let update = |user_agent: &str, cookie: Option<String>| ProfileUpdate {
            platform: "weixin".into(),
            user_agent: user_agent.into(),
            cookie,
            proxy: None,
            bypass_proxy: None,
            http_chunk_size: None,
        };
        save_profile(&db, &mut settings, update("original", Some(manual.into()))).unwrap();
        update_account_auth(&db, &mut settings, "weixin", AccountAuth::Login, Some(imported.into())).unwrap();
        save_profile(&db, &mut settings, update("new-agent", None)).unwrap();

        let mut restored = load(&db, PathBuf::new()).unwrap();
        let public = serde_json::to_value(&restored).unwrap();
        let weixin = public["profiles"].as_array().unwrap().iter()
            .find(|p| p["platform"] == "weixin").unwrap();
        assert_eq!(weixin["login_configured"], true);
        assert_eq!(weixin["account_auth"], "login");
        assert!(!public.to_string().contains("fixture"));
        assert_eq!(cookie(&restored, "weixin").unwrap(), manual);
        let captured = snapshot(&restored).unwrap();
        assert_eq!(captured["weixin"]["cookie"], imported);
        assert_eq!(captured["weixin"]["user_agent"], "new-agent");
        assert_eq!(captured["douyin"]["cookie"], "");

        assert!(update_account_auth(&db, &mut restored, "weixin", AccountAuth::Manual, Some("invalid header".into())).is_err());
        let mut restored = load(&db, PathBuf::new()).unwrap();
        assert_eq!(snapshot(&restored).unwrap()["weixin"]["cookie"], imported);
        update_account_auth(&db, &mut restored, "weixin", AccountAuth::Manual, None).unwrap();
        assert_eq!(snapshot(&restored).unwrap()["weixin"]["cookie"], manual);
        update_account_auth(&db, &mut restored, "weixin", AccountAuth::Login, None).unwrap();
        assert_eq!(snapshot(&restored).unwrap()["weixin"]["cookie"], imported);
        update_account_auth(&db, &mut restored, "weixin", AccountAuth::Login, Some(String::new())).unwrap();
        let restored = load(&db, PathBuf::new()).unwrap();
        assert_eq!(snapshot(&restored).unwrap()["weixin"]["cookie"], "");
        assert_eq!(profile_for(&restored, "weixin").account_auth, None);
        assert_eq!(cookie(&restored, "weixin").unwrap(), manual);
    }

    #[test]
    fn account_auth_migration_preserves_previous_effective_credential() {
        for (manual, yuanbao, legacy_mode, expected) in [
            ("", None, None, None),
            ("manual=test", None, None, Some(AccountAuth::Manual)),
            ("manual=test", Some("yuanbao=test"), Some("yuanbao"), Some(AccountAuth::Login)),
            ("manual=test", Some("yuanbao=test"), Some("browser"), None),
        ] {
            let db = Connection::open_in_memory().unwrap();
            db.execute_batch(
                "CREATE TABLE platform_settings(platform TEXT PRIMARY KEY, user_agent TEXT NOT NULL,
                    proxy TEXT, cookie TEXT NOT NULL, credential_ref TEXT);",
            ).unwrap();
            db.execute(
                "INSERT INTO platform_settings(platform,user_agent,cookie) VALUES ('weixin','agent',?1)",
                [manual],
            ).unwrap();
            if let Some(yuanbao) = yuanbao {
                db.execute_batch("ALTER TABLE platform_settings ADD COLUMN yuanbao_cookie TEXT NOT NULL DEFAULT '';").unwrap();
                db.execute("UPDATE platform_settings SET yuanbao_cookie=?1", [yuanbao]).unwrap();
            }
            if let Some(mode) = legacy_mode {
                db.execute_batch("ALTER TABLE platform_settings ADD COLUMN weixin_auth TEXT;").unwrap();
                db.execute("UPDATE platform_settings SET weixin_auth=?1", [mode]).unwrap();
            }
            let restored = load(&db, PathBuf::new()).unwrap();
            assert_eq!(profile_for(&restored, "weixin").account_auth, expected);
            assert_eq!(cookie(&restored, "weixin").unwrap(), manual);
        }
    }

    #[test]
    fn cookies_persist_in_sqlite_but_never_in_public_settings() {
        let db = Connection::open_in_memory().unwrap();
        let mut settings = load(&db, PathBuf::from("downloads")).unwrap();
        let cookie = format!("session={}", "x".repeat(8000));
        save_profile(
            &db,
            &mut settings,
            ProfileUpdate {
                bypass_proxy: None,
                http_chunk_size: None,
                platform: "douyin".into(),
                user_agent: "test-agent".into(),
                cookie: Some(cookie.clone()),
                proxy: Some(String::new()),
            },
        )
        .unwrap();
        assert_eq!(
            db.query_row("SELECT cookie FROM platform_settings", [], |r| r
                .get::<_, String>(0))
                .unwrap(),
            cookie
        );
        assert!(!serde_json::to_string(&settings)
            .unwrap()
            .contains("session="));
        let restored = load(&db, PathBuf::new()).unwrap();
        assert_eq!(super::cookie(&restored, "douyin").unwrap(), cookie);
        assert_eq!(super::cookie(&restored, "tiktok").unwrap(), "");
        assert!(super::cookie(&restored, "invalid").is_err());
        let captured = snapshot(&restored).unwrap();
        assert_eq!(captured["douyin"]["cookie"], cookie);
        assert_eq!(captured["tiktok"]["cookie"], "");
        save_profile(
            &db,
            &mut settings,
            ProfileUpdate {
                bypass_proxy: None,
                http_chunk_size: None,
                platform: "douyin".into(),
                user_agent: "next".into(),
                cookie: None,
                proxy: None,
            },
        )
        .unwrap();
        assert_eq!(snapshot(&settings).unwrap()["douyin"]["cookie"], cookie);
        save_profile(
            &db,
            &mut settings,
            ProfileUpdate {
                bypass_proxy: None,
                http_chunk_size: None,
                platform: "douyin".into(),
                user_agent: "next".into(),
                cookie: Some(String::new()),
                proxy: None,
            },
        )
        .unwrap();
        assert_eq!(snapshot(&settings).unwrap()["douyin"]["cookie"], "");
        assert_eq!(captured["douyin"]["cookie"], cookie);
    }
    #[test]
    fn invalid_proxy_does_not_change_saved_cookie() {
        let db = Connection::open_in_memory().unwrap();
        let mut settings = load(&db, PathBuf::new()).unwrap();
        for proxy in [
            "https://localhost:443",
            "http://localhost",
            "file:///tmp/a",
            "socks5://localhost:80/?secret=1",
        ] {
            assert!(save_profile(
                &db,
                &mut settings,
                ProfileUpdate {
                    bypass_proxy: None,
                    http_chunk_size: None,
                    platform: "douyin".into(),
                    user_agent: "agent".into(),
                    cookie: Some("session=x".into()),
                    proxy: Some(proxy.into())
                }
            )
            .is_err());
        }
        assert_eq!(snapshot(&settings).unwrap()["douyin"]["cookie"], "");
    }

    #[test]
    fn http_default_port_survives_save_and_reload() {
        let db = Connection::open_in_memory().unwrap();
        let mut settings = load(&db, PathBuf::new()).unwrap();
        save_profile(
            &db,
            &mut settings,
            ProfileUpdate {
                bypass_proxy: None,
                http_chunk_size: None,
                platform: "douyin".into(),
                user_agent: "agent".into(),
                cookie: None,
                proxy: Some("http://127.0.0.1:80".into()),
            },
        )
        .unwrap();
        let restored = load(&db, PathBuf::new()).unwrap();
        assert_eq!(
            snapshot(&restored).unwrap()["douyin"]["proxy"],
            "http://127.0.0.1:80/"
        );
    }
    #[test]
    fn youtube_settings_persist_independently() {
        let db = Connection::open_in_memory().unwrap();
        let mut settings = load(&db, PathBuf::new()).unwrap();
        save_profile(
            &db,
            &mut settings,
            ProfileUpdate {
                bypass_proxy: None,
                http_chunk_size: None,
                platform: "youtube".into(),
                user_agent: "fixture".into(),
                cookie: Some("session=fixture".into()),
                proxy: Some(String::new()),
            },
        )
        .unwrap();
        let restored = load(&db, PathBuf::new()).unwrap();
        assert_eq!(
            restored
                .profiles
                .iter()
                .map(|profile| profile.platform.as_str())
                .collect::<Vec<_>>(),
            [
                "douyin",
                "xiaohongshu",
                "tiktok",
                "youtube",
                "telegram",
                "weixin"
            ]
        );
        assert_eq!(
            super::cookie(&restored, "youtube").unwrap(),
            "session=fixture"
        );
        assert_eq!(super::cookie(&restored, "douyin").unwrap(), "");
        assert_eq!(super::cookie(&restored, "tiktok").unwrap(), "");
        assert_eq!(super::cookie(&restored, "xiaohongshu").unwrap(), "");
        assert!(!serde_json::to_string(&restored)
            .unwrap()
            .contains("session=fixture"));
    }
    #[test]
    fn existing_settings_migrate_with_default_five_and_preserve_directory() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(
            "CREATE TABLE app_settings(id INTEGER PRIMARY KEY, directory TEXT NOT NULL);
            INSERT INTO app_settings VALUES(1,'old-downloads');",
        )
        .unwrap();
        let restored = load(&db, PathBuf::from("default")).unwrap();
        assert_eq!(restored.download_directory, PathBuf::from("old-downloads"));
        assert_eq!(restored.max_downloads, 5);
        assert_eq!(restored.filename_template, crate::naming::DEFAULT_TEMPLATE);
        assert_eq!(load(&db, PathBuf::new()).unwrap().max_downloads, 5);
    }
    #[test]
    fn download_limit_migration_preserves_both_previous_schemas_and_allows_ten() {
        for (old_default, old_max) in [(2, 3), (3, 5)] {
            for previous in 1..=old_max {
                let db = Connection::open_in_memory().unwrap();
                db.execute_batch(&format!(
                    "CREATE TABLE app_settings(id INTEGER PRIMARY KEY CHECK(id=1), directory TEXT NOT NULL,
                       max_downloads INTEGER NOT NULL DEFAULT {old_default} CHECK(max_downloads BETWEEN 1 AND {old_max}),
                       filename_template TEXT NOT NULL DEFAULT '{{platform}}/{{user_name}}-{{content}}');",
                )).unwrap();
                db.execute(
                    "INSERT INTO app_settings VALUES(1,'saved-directory',?1,'custom/{link_id}')",
                    [previous],
                )
                .unwrap();
                let restored = load(&db, PathBuf::from("unused")).unwrap();
                assert_eq!(restored.max_downloads, previous);
                assert_eq!(
                    restored.download_directory,
                    PathBuf::from("saved-directory")
                );
                assert_eq!(restored.filename_template, "custom/{link_id}");
                for value in [6, 10] {
                    db.execute("UPDATE app_settings SET max_downloads=?1", [value])
                        .unwrap();
                    assert_eq!(load(&db, PathBuf::new()).unwrap().max_downloads, value);
                }
                for invalid in [0, 11] {
                    assert!(db
                        .execute("UPDATE app_settings SET max_downloads=?1", [invalid])
                        .is_err());
                }
                db.execute("DELETE FROM app_settings", []).unwrap();
                db.execute(
                    "INSERT INTO app_settings(id,directory) VALUES(1,'new-directory')",
                    [],
                )
                .unwrap();
                assert_eq!(load(&db, PathBuf::new()).unwrap().max_downloads, 5);
            }
        }
    }
    #[test]
    fn fresh_download_limit_defaults_to_five_before_and_after_directory_save() {
        let db = Connection::open_in_memory().unwrap();
        assert_eq!(
            load(&db, PathBuf::from("default")).unwrap().max_downloads,
            5
        );
        db.execute(
            "INSERT INTO app_settings(id,directory) VALUES(1,'saved-directory')",
            [],
        )
        .unwrap();
        assert_eq!(load(&db, PathBuf::new()).unwrap().max_downloads, 5);
        db.execute("UPDATE app_settings SET max_downloads=10", [])
            .unwrap();
        assert_eq!(load(&db, PathBuf::new()).unwrap().max_downloads, 10);
    }
    #[test]
    fn telegram_proxy_is_persisted_separately_and_excluded_from_engine_snapshot() {
        let db = Connection::open_in_memory().unwrap();
        let mut settings = load(&db, PathBuf::new()).unwrap();
        let edit = |proxy: &str| ProfileUpdate {
            platform: "telegram".into(),
            user_agent: "Telegram".into(),
            cookie: None,
            proxy: Some(proxy.into()),
            bypass_proxy: None,
            http_chunk_size: None,
        };
        save_profile(&db, &mut settings, edit("socks5://127.0.0.1:1080")).unwrap();
        let restored = load(&db, PathBuf::new()).unwrap();
        assert_eq!(
            proxy(&restored, "telegram").unwrap().as_deref(),
            Some("socks5://127.0.0.1:1080/")
        );
        assert!(proxy(&restored, "youtube").unwrap().is_none());
        assert!(snapshot(&restored).unwrap().get("telegram").is_none());
        assert!(save_profile(&db, &mut settings, edit("http://127.0.0.1:1080")).is_err());
        assert_eq!(
            proxy(&settings, "telegram").unwrap(),
            proxy(&restored, "telegram").unwrap()
        );
        save_profile(&db, &mut settings, edit("")).unwrap();
        assert!(proxy(&load(&db, PathBuf::new()).unwrap(), "telegram")
            .unwrap()
            .is_none());
    }
}
