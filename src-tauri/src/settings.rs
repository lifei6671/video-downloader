use anyhow::{bail, Context, Result};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Clone, Serialize, Deserialize)]
pub struct Profile {
    pub platform: String,
    pub user_agent: String,
    pub proxy: Option<String>,
    pub cookie_configured: bool,
    #[serde(skip_serializing)]
    pub credential_ref: Option<String>,
    #[serde(skip_serializing)]
    cookie: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Settings {
    pub download_directory: PathBuf,
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
}

fn entry(reference: &str) -> Result<keyring::Entry> {
    keyring::Entry::new("com.yingqu.desktop", reference).context("CREDENTIAL_ERROR")
}

pub fn load(db: &Connection, default_directory: PathBuf) -> Result<Settings> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS app_settings (id INTEGER PRIMARY KEY CHECK(id=1), directory TEXT NOT NULL);
         CREATE TABLE IF NOT EXISTS platform_settings (
           platform TEXT PRIMARY KEY, user_agent TEXT NOT NULL, proxy TEXT,
           cookie TEXT NOT NULL, credential_ref TEXT);",
    ).context("DATABASE_ERROR")?;
    let has_limit = db
        .prepare("PRAGMA table_info(app_settings)")?
        .query_map([], |r| r.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?
        .iter()
        .any(|name| name == "max_downloads");
    if !has_limit {
        db.execute_batch("ALTER TABLE app_settings ADD COLUMN max_downloads INTEGER NOT NULL DEFAULT 2 CHECK(max_downloads BETWEEN 1 AND 3);")
            .context("DATABASE_ERROR")?;
    }
    let max_downloads = match db.query_row(
        "SELECT max_downloads FROM app_settings WHERE id=1",
        [],
        |r| r.get::<_, u8>(0),
    ) {
        Ok(value @ 1..=3) => value,
        Err(rusqlite::Error::QueryReturnedNoRows) => 2,
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
    let mut profiles = Vec::new();
    for platform in ["douyin", "tiktok"] {
        let row = db.query_row(
            "SELECT user_agent,proxy,cookie,credential_ref FROM platform_settings WHERE platform=?",
            [platform],
            |r| {
                Ok(Profile {
                    platform: platform.into(),
                    user_agent: r.get(0)?,
                    proxy: r.get(1)?,
                    cookie_configured: !r.get::<_, String>(2)?.is_empty(),
                    cookie: r.get(2)?,
                    credential_ref: r.get(3)?,
                })
            },
        );
        profiles.push(match row {
            Ok(profile) => profile,
            Err(rusqlite::Error::QueryReturnedNoRows) => Profile {
                platform: platform.into(),
                user_agent: "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/150.0.0.0 Safari/537.36 Edg/150.0.0.0".into(),
                proxy: None, cookie_configured: false, cookie: String::new(), credential_ref: None,
            },
            Err(_) => bail!("DATABASE_ERROR"),
        });
    }
    Ok(Settings {
        download_directory,
        max_downloads,
        profiles,
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

pub fn snapshot(settings: &Settings) -> Result<Value> {
    let mut profiles = serde_json::Map::new();
    for profile in &settings.profiles {
        let proxy = match &profile.credential_ref {
            Some(reference) => Some(
                String::from_utf8(entry(reference)?.get_secret().context("CREDENTIAL_ERROR")?)
                    .context("CREDENTIAL_ERROR")?,
            ),
            None => profile.proxy.clone(),
        };
        profiles.insert(
            profile.platform.clone(),
            json!({
                "user_agent": profile.user_agent, "proxy": proxy, "cookie": profile.cookie
            }),
        );
    }
    Ok(Value::Object(profiles))
}

pub fn save_profile(db: &Connection, settings: &mut Settings, update: ProfileUpdate) -> Result<()> {
    let profile = settings
        .profiles
        .iter_mut()
        .find(|p| p.platform == update.platform)
        .context("CONFIG_INVALID")?;
    if update.user_agent.is_empty()
        || update.user_agent.len() > 2048
        || update.user_agent.contains(['\r', '\n'])
    {
        bail!("CONFIG_INVALID");
    }
    let cookie = update.cookie.unwrap_or_else(|| profile.cookie.clone());
    if cookie.len() > 60_000 || cookie.contains(['\r', '\n']) {
        bail!("CONFIG_INVALID");
    }
    let mut reference = profile.credential_ref.clone();
    let mut new_credential = None;
    let mut public_proxy = profile.proxy.clone();
    if let Some(proxy) = update.proxy {
        reference = None;
        if proxy.is_empty() {
            public_proxy = None;
        } else {
            let mut parsed = url::Url::parse(&proxy).context("CONFIG_INVALID")?;
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
                || parsed.host_str().is_none()
                || port == 0
                || parsed.query().is_some()
                || parsed.fragment().is_some()
                || parsed.path() != "/" && !parsed.path().is_empty()
            {
                bail!("CONFIG_INVALID");
            }
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
        "INSERT INTO platform_settings VALUES (?1,?2,?3,?4,?5)
         ON CONFLICT(platform) DO UPDATE SET user_agent=excluded.user_agent,proxy=excluded.proxy,
         cookie=excluded.cookie,credential_ref=excluded.credential_ref",
        rusqlite::params![
            update.platform,
            update.user_agent,
            public_proxy,
            cookie,
            reference
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
        cookie_configured: !cookie.is_empty(),
        cookie,
        credential_ref: reference.clone(),
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
    #[test]
    fn cookies_persist_in_sqlite_but_never_in_public_settings() {
        let db = Connection::open_in_memory().unwrap();
        let mut settings = load(&db, PathBuf::from("downloads")).unwrap();
        let cookie = format!("session={}", "x".repeat(8000));
        save_profile(
            &db,
            &mut settings,
            ProfileUpdate {
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
    fn existing_settings_migrate_with_default_two_and_preserve_directory() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(
            "CREATE TABLE app_settings(id INTEGER PRIMARY KEY, directory TEXT NOT NULL);
            INSERT INTO app_settings VALUES(1,'old-downloads');",
        )
        .unwrap();
        let restored = load(&db, PathBuf::from("default")).unwrap();
        assert_eq!(restored.download_directory, PathBuf::from("old-downloads"));
        assert_eq!(restored.max_downloads, 2);
        assert_eq!(load(&db, PathBuf::new()).unwrap().max_downloads, 2);
    }
}
