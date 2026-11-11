use super::{douyin_author, Task};
use crate::{files, parse_cache, telegram_download};
use anyhow::{ensure, Context, Result};
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde_json::{json, Value};
use std::{collections::HashMap, path::Path};

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct Key<'a> {
    platform: &'a str,
    id: &'a str,
}

pub(super) fn key(video: &Value) -> Option<Key<'_>> {
    let platform = video["platform"].as_str()?;
    let profile_id = video["author_profile"]["id"]
        .as_str()
        .filter(|id| !id.is_empty());
    let id = match platform {
        "douyin" => match douyin_author::batch_author_id(video) {
            Some(batch) if profile_id.is_none_or(|id| id == batch) => batch,
            Some(_) => return None,
            None => profile_id.filter(|id| id.starts_with("MS4wLjAB"))?,
        },
        "tiktok" => profile_id.filter(|id| id.bytes().all(|c| c.is_ascii_digit()))?,
        "xiaohongshu" => profile_id?,
        "youtube" => profile_id.filter(|id| id.len() == 24 && id.starts_with("UC"))?,
        "telegram" => {
            let video_id = video["video_id"].as_str()?;
            let canonical = video["canonical_url"].as_str()?;
            if !telegram_download::validate_identity(video_id, canonical) {
                return None;
            }
            let (channel, _) = video_id.split_once('_')?;
            if profile_id.is_some_and(|id| id != channel) {
                return None;
            }
            channel
        }
        _ => return None,
    };
    if !(1..=256).contains(&id.len())
        || !id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_'))
    {
        return None;
    }
    Some(Key { platform, id })
}

pub(super) enum Migration {
    Create,
    AddPublicDetails,
    Current,
}

pub(super) fn prepare(db: &Connection, already_backed_up: bool) -> Result<Migration> {
    let tables: i64 = db.query_row(
        "SELECT count(*) FROM sqlite_master WHERE type='table' AND name IN ('authors','task_authors')",
        [],
        |row| row.get(0),
    )?;
    ensure!(matches!(tables, 0 | 2), "AUTHOR_SCHEMA_INVALID");
    if tables == 2 {
        let details: i64 = db.query_row(
            "SELECT count(*) FROM pragma_table_info('authors')
             WHERE name IN ('follower_count','following_count','likes_count','age','region','ip_location','age_observed_at')",
            [], |row| row.get(0),
        )?;
        ensure!(matches!(details, 0 | 7), "AUTHOR_SCHEMA_INVALID");
        if details == 7 {
            return Ok(Migration::Current);
        }
        if !already_backed_up {
            crate::backup::schema_migration::snapshot(db, "before-authors-v2")?;
        }
        return Ok(Migration::AddPublicDetails);
    }
    let existing: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%')",
        [],
        |row| row.get(0),
    )?;
    if existing && !already_backed_up {
        crate::backup::schema_migration::snapshot(db, "before-authors-v1")?;
    }
    Ok(Migration::Create)
}

fn add_public_details(db: &Connection) -> Result<()> {
    db.execute_batch(
        "ALTER TABLE authors ADD COLUMN follower_count INTEGER CHECK(follower_count BETWEEN 0 AND 9007199254740991);
         ALTER TABLE authors ADD COLUMN following_count INTEGER CHECK(following_count BETWEEN 0 AND 9007199254740991);
         ALTER TABLE authors ADD COLUMN likes_count INTEGER CHECK(likes_count BETWEEN 0 AND 9007199254740991);
         ALTER TABLE authors ADD COLUMN age INTEGER CHECK(age BETWEEN 1 AND 130);
         ALTER TABLE authors ADD COLUMN region TEXT;
         ALTER TABLE authors ADD COLUMN ip_location TEXT;
         ALTER TABLE authors ADD COLUMN age_observed_at INTEGER CHECK(age_observed_at>=0);",
    )?;
    Ok(())
}

pub(super) fn apply_migration(db: &Connection, migration: Migration) -> Result<()> {
    match migration {
        Migration::Create => migrate(db),
        Migration::AddPublicDetails => {
            add_public_details(db)?;
            adopt_legacy_tasks(db)
        }
        Migration::Current => adopt_legacy_tasks(db),
    }
}

// Older builds can add tasks to a database that already has the author tables.
fn adopt_legacy_tasks(db: &Connection) -> Result<()> {
    let mut query = db.prepare(
        "SELECT d.id,d.snapshot FROM download_tasks d
         LEFT JOIN task_authors t ON t.task_id=d.id
         LEFT JOIN authors a ON a.platform=t.platform AND a.author_id=t.author_id
         WHERE t.task_id IS NULL
            OR (a.avatar IS NULL AND json_extract(d.snapshot,'$.video.author_profile.avatar_cached')=1)
         ORDER BY d.rowid DESC",
    )?;
    let tasks = query
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let has_batch_cache: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='douyin_author_avatars')",
        [],
        |row| row.get(0),
    )?;
    for (id, snapshot) in tasks {
        let task: Task = serde_json::from_str(&snapshot)?;
        let Some(video) = task.video.as_ref().filter(|_| id == task.id) else {
            continue;
        };
        let Some(author) = key(video) else {
            continue;
        };
        link_task(db, &task)?;
        if has_avatar(db, video) {
            continue;
        }
        if has_batch_cache && author.platform == "douyin" {
            let body: Option<Vec<u8>> = db
                .query_row(
                    "SELECT body FROM douyin_author_avatars
                     WHERE author_id=?1 AND typeof(body)='blob' AND length(body) BETWEEN 1 AND ?2",
                    params![author.id, parse_cache::MAX_AVATAR_BYTES as i64],
                    |row| row.get(0),
                )
                .optional()?;
            if let Some(body) = body.filter(|body| valid_avatar(body)) {
                write_avatar(db, author, &body)?;
                continue;
            }
        }
        let metadata: Option<String> = db
            .query_row(
                "SELECT metadata FROM parse_cache
                 WHERE platform=?1 AND video_id=?2 AND resolver=?3 AND author_avatar IS NOT NULL",
                params![author.platform, video["video_id"].as_str(), video["resolver"].as_str()],
                |row| row.get(0),
            )
            .optional()?;
        let cached = metadata.and_then(|metadata| serde_json::from_str::<Value>(&metadata).ok());
        if cached.as_ref().and_then(key) == Some(author) {
            if let Ok(body) = parse_cache::avatar(db, video) {
                write_avatar(db, author, &body)?;
            }
        }
    }
    Ok(())
}

pub(super) fn migrate(db: &Connection) -> Result<()> {
    db.execute_batch(
        "CREATE TABLE authors (
            platform TEXT NOT NULL CHECK(platform IN ('douyin','tiktok','xiaohongshu','youtube','telegram')),
            author_id TEXT NOT NULL CHECK(length(author_id) BETWEEN 1 AND 256),
            kind TEXT NOT NULL CHECK(kind IN ('user','channel')),
            nickname TEXT, handle TEXT, bio TEXT,
            avatar BLOB CHECK(avatar IS NULL OR (typeof(avatar)='blob' AND length(avatar) BETWEEN 1 AND 2097152)),
            profile_updated_at INTEGER NOT NULL CHECK(profile_updated_at>=0),
            PRIMARY KEY(platform,author_id)
         ) WITHOUT ROWID;
         CREATE INDEX authors_platform_nickname ON authors(platform,nickname);
         CREATE TABLE task_authors (
            task_id TEXT PRIMARY KEY NOT NULL REFERENCES download_tasks(id) ON DELETE CASCADE,
            platform TEXT NOT NULL, author_id TEXT NOT NULL,
            FOREIGN KEY(platform,author_id) REFERENCES authors(platform,author_id)
         ) WITHOUT ROWID;
         CREATE INDEX task_authors_author ON task_authors(platform,author_id);",
    )?;
    add_public_details(db)?;

    let mut tasks = db.prepare("SELECT id,snapshot FROM download_tasks ORDER BY rowid")?;
    let mut rows = tasks.query([])?;
    while let Some(row) = rows.next()? {
        let id: String = row.get(0)?;
        let task: Task = serde_json::from_str(&row.get::<_, String>(1)?)?;
        if id == task.id {
            link_task(db, &task)?;
        }
    }

    let mut cache = db.prepare(
        "SELECT platform,video_id,metadata,saved_at,
            CASE WHEN typeof(author_avatar)='blob' AND length(author_avatar) BETWEEN 1 AND ?1
                 THEN author_avatar END
         FROM parse_cache ORDER BY saved_at,cache_key",
    )?;
    let mut rows = cache.query([parse_cache::MAX_AVATAR_BYTES as i64])?;
    while let Some(row) = rows.next()? {
        let Ok(video) = serde_json::from_str::<Value>(&row.get::<_, String>(2)?) else {
            continue;
        };
        if video["platform"].as_str() != Some(row.get_ref(0)?.as_str()?)
            || video["video_id"].as_str() != Some(row.get_ref(1)?.as_str()?)
        {
            continue;
        }
        if let Some(key) = key(&video) {
            record(db, key, &video, row.get(3)?)?;
            if let Some(body) = row.get::<_, Option<Vec<u8>>>(4)? {
                if valid_avatar(&body) {
                    write_avatar(db, key, &body)?;
                }
            }
        }
    }

    let legacy_batch_avatars: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='douyin_author_avatars')",
        [],
        |row| row.get(0),
    )?;
    if legacy_batch_avatars {
        let mut batch = db.prepare(
            "SELECT author_id,body FROM douyin_author_avatars
             WHERE typeof(body)='blob' AND length(body) BETWEEN 1 AND ?1",
        )?;
        let mut rows = batch.query([parse_cache::MAX_AVATAR_BYTES as i64])?;
        while let Some(row) = rows.next()? {
            let id = row.get_ref(0)?.as_str()?;
            let body = row.get_ref(1)?.as_blob()?;
            if valid_avatar(body) {
                // Batch avatars already used the verified Douyin author key.
                write_avatar(
                    db,
                    Key {
                        platform: "douyin",
                        id,
                    },
                    body,
                )?;
            }
        }
    }
    Ok(())
}

fn text(value: &Value, limit: usize) -> Option<String> {
    let text = value.as_str()?.trim();
    (!text.is_empty()).then(|| text.chars().take(limit).collect())
}

const PUBLIC_FIELDS: [&str; 6] = [
    "follower_count",
    "following_count",
    "likes_count",
    "age",
    "region",
    "ip_location",
];

fn count(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .filter(|value| (0..=9_007_199_254_740_991).contains(value))
}

pub(super) fn strip_public_details(video: &mut Value) {
    if key(video).is_some() {
        if let Some(profile) = video["author_profile"].as_object_mut() {
            for field in PUBLIC_FIELDS {
                profile.remove(field);
            }
            profile.remove("age_hidden");
        }
    }
}

fn record(db: &Connection, key: Key<'_>, video: &Value, at: i64) -> Result<()> {
    let profile = &video["author_profile"];
    let nickname = text(&profile["nickname"], 256).or_else(|| {
        if key.platform == "telegram" || profile["id"].is_null() {
            text(&video["author"], 256)
        } else {
            None
        }
    });
    let handle = text(&profile["handle"], 256);
    let bio = text(&profile["bio"], 4096);
    let public = if matches!(key.platform, "douyin" | "tiktok") {
        profile
    } else {
        &Value::Null
    };
    let age_hidden = public["age_hidden"] == true;
    let age = if age_hidden {
        None
    } else {
        age(&public["age"])
    };
    // A hidden observation must also win over replayed age data in the same second.
    let age_observed_at = (age_hidden || age.is_some()).then_some(at.max(0));
    let kind = if matches!(key.platform, "telegram" | "youtube") {
        "channel"
    } else {
        "user"
    };
    db.execute(
        "INSERT INTO authors(platform,author_id,kind,nickname,handle,bio,profile_updated_at,
           follower_count,following_count,likes_count,age,region,ip_location,age_observed_at)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)
         ON CONFLICT(platform,author_id) DO UPDATE SET
           nickname=CASE WHEN excluded.profile_updated_at>=authors.profile_updated_at
                THEN coalesce(excluded.nickname,authors.nickname) ELSE coalesce(authors.nickname,excluded.nickname) END,
           handle=CASE WHEN excluded.profile_updated_at>=authors.profile_updated_at
                THEN coalesce(excluded.handle,authors.handle) ELSE coalesce(authors.handle,excluded.handle) END,
           bio=CASE WHEN excluded.profile_updated_at>=authors.profile_updated_at
                THEN coalesce(excluded.bio,authors.bio) ELSE coalesce(authors.bio,excluded.bio) END,
           follower_count=CASE WHEN excluded.profile_updated_at>=authors.profile_updated_at
                THEN coalesce(excluded.follower_count,authors.follower_count) ELSE coalesce(authors.follower_count,excluded.follower_count) END,
           following_count=CASE WHEN excluded.profile_updated_at>=authors.profile_updated_at
                THEN coalesce(excluded.following_count,authors.following_count) ELSE coalesce(authors.following_count,excluded.following_count) END,
           likes_count=CASE WHEN excluded.profile_updated_at>=authors.profile_updated_at
                THEN coalesce(excluded.likes_count,authors.likes_count) ELSE coalesce(authors.likes_count,excluded.likes_count) END,
           age=CASE WHEN excluded.age_observed_at>coalesce(authors.age_observed_at,-1)
                OR (excluded.age_observed_at=authors.age_observed_at
                    AND (excluded.age IS NULL OR authors.age IS NOT NULL))
                THEN excluded.age ELSE authors.age END,
           age_observed_at=max(coalesce(excluded.age_observed_at,authors.age_observed_at),
                coalesce(authors.age_observed_at,excluded.age_observed_at)),
           region=CASE WHEN excluded.profile_updated_at>=authors.profile_updated_at
                THEN coalesce(excluded.region,authors.region) ELSE coalesce(authors.region,excluded.region) END,
           ip_location=CASE WHEN excluded.profile_updated_at>=authors.profile_updated_at
                THEN coalesce(excluded.ip_location,authors.ip_location) ELSE coalesce(authors.ip_location,excluded.ip_location) END,
           profile_updated_at=max(authors.profile_updated_at,excluded.profile_updated_at)",
        params![key.platform, key.id, kind, nickname, handle, bio, at.max(0),
            count(&public["follower_count"]), count(&public["following_count"]), count(&public["likes_count"]),
            age, text(&public["region"], 120), text(&public["ip_location"], 120), age_observed_at],
    )?;
    Ok(())
}

pub(super) fn link_task(db: &Connection, task: &Task) -> Result<()> {
    if let Some((video, key)) = task
        .video
        .as_ref()
        .and_then(|video| Some((video, key(video)?)))
    {
        let observed_at = if matches!(key.platform, "douyin" | "telegram")
            && (video["author_scan_id"].is_string() || key.platform == "telegram")
        {
            task.completed_at.or(task.cached_at)
        } else {
            task.cached_at
        };
        record(
            db,
            key,
            video,
            i64::try_from(observed_at.unwrap_or(task.created_at)).unwrap_or(0),
        )?;
        db.execute(
            "INSERT INTO task_authors(task_id,platform,author_id) VALUES(?1,?2,?3)
             ON CONFLICT(task_id) DO UPDATE SET platform=excluded.platform,author_id=excluded.author_id
             WHERE task_authors.platform<>excluded.platform OR task_authors.author_id<>excluded.author_id",
            params![task.id, key.platform, key.id],
        )?;
    } else {
        db.execute("DELETE FROM task_authors WHERE task_id=?", [&task.id])?;
    }
    Ok(())
}

fn age(value: &Value) -> Option<i64> {
    value.as_i64().filter(|value| (1..=130).contains(value))
}

pub(super) struct DownloadedProfile<'a> {
    key: Key<'a>,
    result: &'a Value,
    legacy_avatar: Option<Vec<u8>>,
}

pub(super) fn downloaded_profile<'a>(
    db: &Connection,
    video: &mut Value,
    result: &'a Value,
) -> Option<DownloadedProfile<'a>> {
    let incoming = key(result)?;
    if !matches!(incoming.platform, "douyin" | "tiktok") || video["platform"] != incoming.platform {
        return None;
    }
    let profile = &result["author_profile"];
    let has_details = ["bio", "region", "ip_location"].iter().any(|field| {
        profile[field]
            .as_str()
            .is_some_and(|value| !value.trim().is_empty())
    }) || PUBLIC_FIELDS[..3]
        .iter()
        .any(|field| count(&profile[field]).is_some())
        || age(&profile["age"]).is_some()
        || profile["age_hidden"] == true;
    if !has_details {
        return None;
    }
    let legacy_avatar = match key(video) {
        Some(existing) if existing != incoming => return None,
        Some(_) => None,
        None => {
            // Preserve the avatar already cached for this exact work.
            let avatar = video["video_id"].as_str().and_then(|id| {
                let resolver = video["resolver"].as_str().unwrap_or("yt-dlp");
                parse_cache::get(db, incoming.platform, id, resolver)
                    .ok()
                    .flatten()
                    .filter(|preview| key(&preview.video).is_none_or(|cached| cached == incoming))
                    .and_then(|preview| parse_cache::avatar(db, &preview.video).ok())
            });
            if !video["author_profile"].is_object() {
                video["author_profile"] = json!({"kind":"user"});
            }
            // The caller has already verified the returned work identity and file.
            video["author_profile"]["id"] = incoming.id.into();
            avatar
        }
    };
    Some(DownloadedProfile {
        key: incoming,
        result,
        legacy_avatar,
    })
}

pub(super) fn save_downloaded_profile(
    db: &Connection,
    profile: DownloadedProfile<'_>,
    at: u64,
) -> Result<()> {
    record(
        db,
        profile.key,
        profile.result,
        i64::try_from(at).context("DATABASE_ERROR")?,
    )
    .context("DATABASE_ERROR")?;
    if let Some(avatar) = profile.legacy_avatar {
        db.execute(
            "UPDATE authors SET avatar=?3 WHERE platform=?1 AND author_id=?2 AND avatar IS NULL",
            params![profile.key.platform, profile.key.id, avatar],
        )
        .context("DATABASE_ERROR")?;
    }
    Ok(())
}

fn valid_avatar(body: &[u8]) -> bool {
    body.len() <= parse_cache::MAX_AVATAR_BYTES
        && matches!(files::image_extension(body), Some("jpeg" | "png" | "webp"))
}

fn write_avatar(db: &Connection, key: Key<'_>, body: &[u8]) -> Result<()> {
    db.execute(
        "UPDATE authors SET avatar=?3 WHERE platform=?1 AND author_id=?2",
        params![key.platform, key.id, body],
    )?;
    Ok(())
}

pub(super) fn save_avatar(db: &Connection, video: &Value, body: &[u8], at: u64) -> Result<()> {
    ensure!(valid_avatar(body), "INVALID_MEDIA_CONTENT");
    if let Some(key) = key(video) {
        record(db, key, video, i64::try_from(at).unwrap_or(0))?;
        write_avatar(db, key, body)?;
    } else {
        parse_cache::save_avatar(db, video, body)?;
    }
    Ok(())
}

pub(super) fn save_staged_avatar(
    db: &Connection,
    video: &Value,
    result: &Value,
    stage: &Path,
    at: u64,
) -> Result<()> {
    if result["author_avatar_filename"] != "author-avatar" {
        return Ok(());
    }
    let expected = result["author_avatar_bytes"]
        .as_u64()
        .context("INVALID_ENGINE_RESULT")?;
    ensure!(
        (1..=parse_cache::MAX_AVATAR_BYTES as u64).contains(&expected),
        "INVALID_MEDIA_CONTENT"
    );
    let body = files::read_cover(&stage.join("author-avatar"))?;
    ensure!(expected == body.len() as u64, "INCOMPLETE_MEDIA");
    save_avatar(db, video, &body, at)
}

pub(super) fn has_avatar(db: &Connection, video: &Value) -> bool {
    if let Some(key) = key(video) {
        db.query_row(
            "SELECT EXISTS(SELECT 1 FROM authors WHERE platform=?1 AND author_id=?2 AND avatar IS NOT NULL)",
            params![key.platform, key.id],
            |row| row.get(0),
        ).unwrap_or(false)
    } else {
        parse_cache::avatar(db, video).is_ok()
    }
}

pub(super) fn avatar(db: &Connection, task_id: &str, video: &Value) -> Result<Vec<u8>> {
    if key(video).is_none() {
        return parse_cache::avatar(db, video);
    }
    let body: Option<Vec<u8>> = db
        .query_row(
            "SELECT a.avatar FROM task_authors t JOIN authors a USING(platform,author_id)
         WHERE t.task_id=?1 AND a.avatar IS NOT NULL",
            [task_id],
            |row| row.get(0),
        )
        .optional()?;
    let body = body.context("FILE_MISSING")?;
    ensure!(valid_avatar(&body), "INVALID_MEDIA_CONTENT");
    Ok(body)
}

pub(super) fn release_avatar(db: &Connection, key: Key<'_>) -> Result<()> {
    db.execute(
        "UPDATE authors SET avatar=NULL WHERE platform=?1 AND author_id=?2 AND avatar IS NOT NULL
         AND NOT EXISTS(SELECT 1 FROM task_authors WHERE platform=?1 AND author_id=?2)",
        params![key.platform, key.id],
    )?;
    Ok(())
}

const PROFILE_SELECT: &str =
    "SELECT t.task_id,a.author_id,a.kind,a.nickname,a.handle,a.bio,a.avatar IS NOT NULL,
       a.follower_count,a.following_count,a.likes_count,a.age,a.region,a.ip_location,a.profile_updated_at
     FROM task_authors t JOIN authors a USING(platform,author_id)";

fn display_profile(row: &Row<'_>) -> rusqlite::Result<Value> {
    Ok(json!({
        "id": row.get::<_, String>(1)?,
        "kind": row.get::<_, String>(2)?,
        "nickname": row.get::<_, Option<String>>(3)?,
        "handle": row.get::<_, Option<String>>(4)?,
        "bio": row.get::<_, Option<String>>(5)?,
        "avatar_cached": row.get::<_, bool>(6)?,
        "follower_count": row.get::<_, Option<i64>>(7)?,
        "following_count": row.get::<_, Option<i64>>(8)?,
        "likes_count": row.get::<_, Option<i64>>(9)?,
        "age": row.get::<_, Option<i64>>(10)?,
        "region": row.get::<_, Option<String>>(11)?,
        "ip_location": row.get::<_, Option<String>>(12)?,
        "profile_updated_at": row.get::<_, i64>(13)?,
    }))
}

pub(super) fn display_profiles(db: &Connection) -> Result<HashMap<String, Value>> {
    let mut query = db.prepare(PROFILE_SELECT)?;
    let profiles = query.query_map([], |row| Ok((row.get(0)?, display_profile(row)?)))?;
    Ok(profiles.collect::<rusqlite::Result<_>>()?)
}

pub(super) fn display_task(db: &Connection, mut task: Task) -> Task {
    if let Some(video) = task.video.as_mut() {
        let profile = db
            .prepare_cached(&format!("{PROFILE_SELECT} WHERE t.task_id=?1"))
            .and_then(|mut query| query.query_row([&task.id], display_profile));
        if let Ok(profile) = profile {
            video["author_profile"] = profile;
        }
    }
    task
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downloaded_bio_updates_only_the_matching_author_and_survives_missing_signature() {
        let mut db = Connection::open_in_memory().unwrap();
        db.execute_batch(
            "CREATE TABLE download_tasks(id TEXT PRIMARY KEY,snapshot TEXT NOT NULL);",
        )
        .unwrap();
        parse_cache::init(&db).unwrap();
        migrate(&db).unwrap();

        let mut video = json!({"platform":"tiktok", "video_id":"99", "kind":"video",
            "canonical_url":"https://www.tiktok.com/@sample/video/99", "resolver":"yt-dlp"});
        let avatar = b"\x89PNG\r\n\x1a\nfixture";
        parse_cache::save(&mut db, &video, vec![], None, 10).unwrap();
        assert!(parse_cache::save_avatar(&db, &video, avatar).unwrap());
        let incoming = json!({"platform":"tiktok", "author_profile":{"id":"123","bio":" 新简介 "}});
        let downloaded = downloaded_profile(&db, &mut video, &incoming).unwrap();
        let author = downloaded.key;
        assert!(key(&video) == Some(author));
        record(&db, author, &video, 10).unwrap();
        save_downloaded_profile(&db, downloaded, 20).unwrap();
        for profile in [
            json!({"id":"456","bio":"别人的简介"}),
            json!({"id":"123","bio":"  "}),
            json!({"id":"123"}),
        ] {
            let result = json!({"platform":"tiktok", "author_profile":profile});
            assert!(downloaded_profile(&db, &mut video, &result).is_none());
            record(&db, author, &video, 30).unwrap();
        }
        let stored: (String, Vec<u8>) = db
            .query_row("SELECT bio,avatar FROM authors", [], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .unwrap();
        assert_eq!(stored, ("新简介".to_string(), avatar.to_vec()));
    }

    #[test]
    fn upgrades_existing_author_table_and_preserves_optional_public_details() {
        let root = tempfile::tempdir().unwrap();
        let mut db = Connection::open(root.path().join("authors.sqlite")).unwrap();
        db.execute_batch(
            "CREATE TABLE download_tasks(id TEXT PRIMARY KEY,snapshot TEXT NOT NULL);",
        )
        .unwrap();
        parse_cache::init(&db).unwrap();
        migrate(&db).unwrap();
        for column in PUBLIC_FIELDS.into_iter().chain(["age_observed_at"]) {
            db.execute_batch(&format!("ALTER TABLE authors DROP COLUMN {column}"))
                .unwrap();
        }
        db.execute(
            "INSERT INTO authors(platform,author_id,kind,bio,profile_updated_at)
            VALUES('tiktok','123','user','旧简介',10)",
            [],
        )
        .unwrap();
        db.execute_batch(
            "INSERT INTO download_tasks VALUES('task','{}');
            INSERT INTO task_authors VALUES('task','tiktok','123');",
        )
        .unwrap();

        let upgrade = prepare(&db, false).unwrap();
        assert!(matches!(upgrade, Migration::AddPublicDetails));
        let backup = std::fs::read_dir(root.path())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| {
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("authors.sqlite.before-authors-v2-")
            })
            .unwrap();
        let before = Connection::open(backup).unwrap();
        assert!(before
            .prepare("SELECT follower_count FROM authors")
            .is_err());
        let tx = db.transaction().unwrap();
        apply_migration(&tx, upgrade).unwrap();
        tx.commit().unwrap();
        assert!(matches!(prepare(&db, false).unwrap(), Migration::Current));
        let initial = &display_profiles(&db).unwrap()["task"];
        assert_eq!(initial["bio"], "旧简介");
        assert!(initial["follower_count"].is_null());

        let mut video = json!({"platform":"tiktok","author_profile":{"id":"123"}});
        let fresh = json!({"platform":"tiktok","author_profile":{
            "id":"123", "follower_count":0, "following_count":8, "age":28, "region":"US"}});
        let incoming = downloaded_profile(&db, &mut video, &fresh).unwrap();
        save_downloaded_profile(&db, incoming, 20).unwrap();
        let partial = json!({"platform":"tiktok","author_profile":{"id":"123","likes_count":15}});
        let stale = json!({"platform":"tiktok","author_profile":{"id":"123","follower_count":9}});
        let key = key(&fresh).unwrap();
        record(&db, key, &partial, 30).unwrap();
        record(&db, key, &stale, 15).unwrap();
        let updated = &display_profiles(&db).unwrap()["task"];
        assert_eq!(updated["follower_count"], 0);
        assert_eq!(updated["following_count"], 8);
        assert_eq!(updated["likes_count"], 15);
        assert_eq!(updated["age"], 28);
        assert_eq!(updated["region"], "US");
        assert!(updated["ip_location"].is_null());

        let hidden = json!({"platform":"tiktok","author_profile":{"id":"123","age_hidden":true}});
        let incoming = downloaded_profile(&db, &mut video, &hidden).unwrap();
        save_downloaded_profile(&db, incoming, 35).unwrap();
        for at in [20, 35] {
            record(&db, key, &fresh, at).unwrap();
            assert!(display_profiles(&db).unwrap()["task"]["age"].is_null());
        }
        record(&db, key, &fresh, 36).unwrap();
        assert_eq!(display_profiles(&db).unwrap()["task"]["age"], 28);
    }

    #[test]
    fn failed_author_migration_rolls_back_new_tables() {
        let mut db = Connection::open_in_memory().unwrap();
        db.execute_batch(
            "CREATE TABLE download_tasks(id TEXT PRIMARY KEY,snapshot TEXT NOT NULL);
            INSERT INTO download_tasks VALUES('bad','{}');",
        )
        .unwrap();
        {
            let tx = db.transaction().unwrap();
            assert!(migrate(&tx).is_err());
        }
        assert!(matches!(prepare(&db, false).unwrap(), Migration::Create));
        assert!(db.prepare("SELECT * FROM authors").is_err());
        assert!(db.prepare("SELECT * FROM task_authors").is_err());
        assert_eq!(
            db.query_row("SELECT snapshot FROM download_tasks", [], |row| row
                .get::<_, String>(0))
                .unwrap(),
            "{}"
        );
    }
}
