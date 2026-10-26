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
    #[serde(default)]
    pub http_chunk_size: Option<u64>,
    #[serde(skip_serializing)]
    pub credential_ref: Option<String>,
    #[serde(skip_serializing)]
    cookie: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Settings {
    pub download_directory: PathBuf,
    pub filename_template: String,
    pub max_downloads: u8,
    pub profiles: Vec<Profile>,
    #[serde(default)]
    pub tdl: crate::tdl_bridge::TdlSettings,
}

#[derive(Deserialize)]
pub struct ProfileUpdate {
    pub platform: String,
    pub user_agent: String,
    // None keeps saved data; empty string explicitly clears it.
    pub cookie: Option<String>,
    pub proxy: Option<String>,
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

pub fn load(db: &Connection, default_directory: PathBuf) -> Result<Settings> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS app_settings (id INTEGER PRIMARY KEY CHECK(id=1), directory TEXT NOT NULL);
         CREATE TABLE IF NOT EXISTS platform_settings (
           platform TEXT PRIMARY KEY, user_agent TEXT NOT NULL, proxy TEXT,
           cookie TEXT NOT NULL, credential_ref TEXT);
         CREATE TABLE IF NOT EXISTS tdl_settings (id INTEGER PRIMARY KEY CHECK(id=1), enabled INTEGER NOT NULL, path TEXT);",
    ).context("DATABASE_ERROR")?;
    let has_chunk_size = db
        .prepare("PRAGMA table_info(platform_settings)")?
        .query_map([], |r| r.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?
        .iter()
        .any(|name| name == "http_chunk_size");
    if !has_chunk_size {
        db.execute_batch("ALTER TABLE platform_settings ADD COLUMN http_chunk_size INTEGER")
            .context("DATABASE_ERROR")?;
    }
    let has_session = db
        .prepare("PRAGMA table_info(tdl_settings)")?
        .query_map([], |r| r.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?
        .iter()
        .any(|name| name == "session_directory");
    if !has_session {
        db.execute_batch("ALTER TABLE tdl_settings ADD COLUMN session_directory TEXT")
            .context("DATABASE_ERROR")?;
    }
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
    let has_template = db.prepare("PRAGMA table_info(app_settings)")?
        .query_map([], |r| r.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?.iter().any(|name| name == "filename_template");
    if !has_template {
        db.execute_batch("ALTER TABLE app_settings ADD COLUMN filename_template TEXT NOT NULL DEFAULT '{platform}/{user_name}-{content}'")
            .context("DATABASE_ERROR")?;
    }
    let filename_template = match db.query_row("SELECT filename_template FROM app_settings WHERE id=1", [], |r| r.get::<_, String>(0)) {
        Ok(value) => { crate::naming::validate(&value).context("DATABASE_ERROR")?; value },
        Err(rusqlite::Error::QueryReturnedNoRows) => crate::naming::DEFAULT_TEMPLATE.into(),
        Err(_) => bail!("DATABASE_ERROR"),
    };
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
    for platform in ["douyin", "tiktok", "youtube"] {
        let row = db.query_row(
            "SELECT user_agent,proxy,cookie,credential_ref,http_chunk_size FROM platform_settings WHERE platform=?",
            [platform],
            |r| {
                Ok(Profile {
                    platform: platform.into(),
                    user_agent: r.get(0)?,
                    proxy: r.get(1)?,
                    cookie_configured: !r.get::<_, String>(2)?.is_empty(),
                    cookie: r.get(2)?,
                    credential_ref: r.get(3)?,
                    http_chunk_size: r.get::<_, Option<u32>>(4)?.map(u64::from),
                })
            },
        );
        profiles.push(match row {
            Ok(profile) => profile,
            Err(rusqlite::Error::QueryReturnedNoRows) => Profile {
                platform: platform.into(),
                user_agent: "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/150.0.0.0 Safari/537.36 Edg/150.0.0.0".into(),
                proxy: None, cookie_configured: false, cookie: String::new(), credential_ref: None, http_chunk_size: None,
            },
            Err(_) => bail!("DATABASE_ERROR"),
        });
    }
    if profiles.iter().any(|p| p.http_chunk_size.is_some_and(|n| p.platform != "youtube" || !valid_chunk_size(n))) {
        bail!("DATABASE_ERROR");
    }
    let tdl = match db.query_row(
        "SELECT enabled,path,session_directory FROM tdl_settings WHERE id=1",
        [],
        |r| {
            Ok(crate::tdl_bridge::TdlSettings {
                enabled: r.get(0)?,
                path: r.get::<_, Option<String>>(1)?.map(PathBuf::from),
                session_directory: r.get::<_, Option<String>>(2)?.map(PathBuf::from),
            })
        },
    ) {
        Ok(value) => value,
        Err(rusqlite::Error::QueryReturnedNoRows) => crate::tdl_bridge::TdlSettings::default(),
        Err(_) => bail!("DATABASE_ERROR"),
    };
    Ok(Settings {
        download_directory,
        filename_template,
        max_downloads,
        profiles,
        tdl,
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
    let profile = settings.profiles.iter().find(|p| p.platform == platform).context("CONFIG_INVALID")?;
    match &profile.credential_ref {
        Some(reference) => Ok(Some(
            String::from_utf8(entry(reference)?.get_secret().context("CREDENTIAL_ERROR")?)
                .context("CREDENTIAL_ERROR")?,
        )),
        None => Ok(profile.proxy.clone()),
    }
}

pub fn snapshot(settings: &Settings) -> Result<Value> {
    let mut profiles = serde_json::Map::new();
    for profile in &settings.profiles {
        let proxy = proxy(settings, &profile.platform)?;
        let mut value = json!({"user_agent": profile.user_agent, "proxy": proxy, "cookie": profile.cookie});
        if profile.platform == "youtube" {
            value["http_chunk_size"] = json!(profile.http_chunk_size);
        }
        profiles.insert(profile.platform.clone(), value);
    }
    Ok(Value::Object(profiles))
}

pub fn save_profile(db: &Connection, settings: &mut Settings, update: ProfileUpdate) -> Result<()> {
    let profile = settings
        .profiles
        .iter_mut()
        .find(|p| p.platform == update.platform)
        .context("CONFIG_INVALID")?;
    let http_chunk_size = match update.http_chunk_size {
        Some(value) => {
            if update.platform != "youtube" || value.is_some_and(|n| !valid_chunk_size(n)) {
                bail!("CONFIG_INVALID");
            }
            value
        }
        None => profile.http_chunk_size,
    };
    if update.user_agent.is_empty()
        || update.user_agent.len() > 2048
        || update.user_agent.contains(['\r', '\n'])
    {
        bail!("CONFIG_INVALID");
    }
    let cookie = update
        .cookie
        .map(|cookie| cookie.trim().to_owned())
        .unwrap_or_else(|| profile.cookie.clone());
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
        "INSERT INTO platform_settings(platform,user_agent,proxy,cookie,credential_ref,http_chunk_size) VALUES (?1,?2,?3,?4,?5,?6)
         ON CONFLICT(platform) DO UPDATE SET user_agent=excluded.user_agent,proxy=excluded.proxy,
         cookie=excluded.cookie,credential_ref=excluded.credential_ref,http_chunk_size=excluded.http_chunk_size",
        rusqlite::params![
            update.platform,
            update.user_agent,
            public_proxy,
            cookie,
            reference,
            http_chunk_size.map(|n| n as i64)
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
    #[test]
    fn proxy_editor_reads_persisted_addresses_and_preserves_failure_semantics() {
        let db = Connection::open_in_memory().unwrap();
        let mut settings = load(&db, PathBuf::new()).unwrap();
        assert!(proxy(&settings, "youtube").unwrap().is_none());
        assert_eq!(proxy(&settings, "invalid").unwrap_err().to_string(), "CONFIG_INVALID");
        for value in ["http://127.0.0.1:80", "socks5://127.0.0.1:7890"] {
            save_profile(&db, &mut settings, ProfileUpdate {
                platform: "youtube".into(), user_agent: "fixture".into(), cookie: None,
                proxy: Some(value.into()), http_chunk_size: None,
            }).unwrap();
            let restored = load(&db, PathBuf::new()).unwrap();
            assert_eq!(proxy(&restored, "youtube").unwrap(), Some(format!("{value}/")));
            assert_eq!(snapshot(&restored).unwrap()["youtube"]["proxy"], format!("{value}/"));
            assert!(proxy(&restored, "tiktok").unwrap().is_none());
        }
        // A new, nonexistent reference never queries an existing user's credential.
        let reference = Uuid::new_v4().to_string();
        settings.profiles[2].credential_ref = Some(reference.clone());
        assert_eq!(proxy(&settings, "youtube").unwrap_err().to_string(), "CREDENTIAL_ERROR");
        assert!(!serde_json::to_string(&settings).unwrap().contains(&reference));
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
        assert!(snapshot(&settings).unwrap()["tiktok"].get("http_chunk_size").is_none());
        for value in [Some(10_485_760_u64), Some(0), Some(1_073_741_824), None] {
            let update = serde_json::from_value(json!({"platform":"youtube", "user_agent":"old-agent",
                "http_chunk_size":value})).unwrap();
            save_profile(&db, &mut settings, update).unwrap();
            let restored = load(&db, PathBuf::new()).unwrap();
            assert_eq!(restored.profiles[2].http_chunk_size, value);
            let captured = snapshot(&restored).unwrap();
            assert_eq!(captured["youtube"]["http_chunk_size"], json!(value));
            assert_eq!(captured["youtube"]["cookie"], "session=fixture");
            assert_eq!(captured["youtube"]["proxy"], "http://127.0.0.1:7890/");
            assert!(!serde_json::to_string(&restored).unwrap().contains("session=fixture"));
            // Old callers that omit the new field must preserve the saved mode.
            save_profile(&db, &mut settings, serde_json::from_value(json!({"platform":"youtube",
                "user_agent":"old-agent"})).unwrap()).unwrap();
            assert_eq!(load(&db, PathBuf::new()).unwrap().profiles[2].http_chunk_size, value);
        }
    }

    #[test]
    fn invalid_chunk_settings_cannot_change_saved_cookie_or_mode() {
        let db = Connection::open_in_memory().unwrap();
        let mut settings = load(&db, PathBuf::new()).unwrap();
        for (platform, value) in [("youtube", Some(1)), ("youtube", Some(1_048_577)),
            ("youtube", Some(1_074_790_400)), ("youtube", Some(u64::MAX)),
            ("douyin", None), ("tiktok", Some(0))] {
            let error = save_profile(&db, &mut settings, ProfileUpdate {
                platform: platform.into(), user_agent: "test".into(), cookie: Some("secret=changed".into()),
                proxy: None, http_chunk_size: Some(value),
            }).unwrap_err();
            assert_eq!(error.to_string(), "CONFIG_INVALID");
            assert_eq!(snapshot(&settings).unwrap()[platform]["cookie"], "");
            assert!(load(&db, PathBuf::new()).unwrap().profiles[2].http_chunk_size.is_none());
        }
        for value in [json!(-1), json!(true), json!(1.5), json!("10485760")] {
            assert!(serde_json::from_value::<ProfileUpdate>(json!({"platform":"youtube",
                "user_agent":"test", "http_chunk_size":value})).is_err());
        }
    }

    #[test]
    fn invalid_persisted_chunk_size_reports_database_error() {
        let db = Connection::open_in_memory().unwrap();
        load(&db, PathBuf::new()).unwrap();
        for (platform, value) in [("youtube", -1_i64), ("youtube", 1),
            ("youtube", i64::MAX), ("douyin", 0)] {
            db.execute("DELETE FROM platform_settings", []).unwrap();
            db.execute("INSERT INTO platform_settings (platform,user_agent,cookie,http_chunk_size) VALUES (?1,'test','',?2)",
                rusqlite::params![platform, value]).unwrap();
            assert_eq!(load(&db, PathBuf::new()).err().unwrap().to_string(), "DATABASE_ERROR");
        }
    }

    #[test]
    fn cookie_boundary_whitespace_is_trimmed_for_all_platforms() {
        let db = Connection::open_in_memory().unwrap();
        let mut settings = load(&db, PathBuf::new()).unwrap();
        for platform in ["douyin", "tiktok", "youtube"] {
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
    fn cookie_internal_newlines_are_rejected_without_changing_saved_values() {
        let db = Connection::open_in_memory().unwrap();
        let mut settings = load(&db, PathBuf::new()).unwrap();
        for platform in ["douyin", "tiktok", "youtube"] {
            save_profile(
                &db,
                &mut settings,
                ProfileUpdate {
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
    fn cookies_persist_in_sqlite_but_never_in_public_settings() {
        let db = Connection::open_in_memory().unwrap();
        let mut settings = load(&db, PathBuf::from("downloads")).unwrap();
        let cookie = format!("session={}", "x".repeat(8000));
        save_profile(
            &db,
            &mut settings,
            ProfileUpdate {
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
                http_chunk_size: None,
                platform: "youtube".into(),
                user_agent: "fixture".into(),
                cookie: Some("session=fixture".into()),
                proxy: Some(String::new()),
            },
        )
        .unwrap();
        let restored = load(&db, PathBuf::new()).unwrap();
        assert_eq!(restored.profiles.len(), 3);
        assert_eq!(
            super::cookie(&restored, "youtube").unwrap(),
            "session=fixture"
        );
        assert_eq!(super::cookie(&restored, "douyin").unwrap(), "");
        assert_eq!(super::cookie(&restored, "tiktok").unwrap(), "");
        assert!(!serde_json::to_string(&restored)
            .unwrap()
            .contains("session=fixture"));
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
        assert_eq!(restored.filename_template, crate::naming::DEFAULT_TEMPLATE);
        assert_eq!(load(&db, PathBuf::new()).unwrap().max_downloads, 2);
    }
}
