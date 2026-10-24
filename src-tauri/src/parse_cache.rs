use crate::files::{image_extension, Video};
use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;

pub struct Preview {
    pub video: Value,
    pub saved_at: u64,
    pub has_cover: bool,
}

pub fn init(db: &Connection) -> Result<bool> {
    let existed: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='parse_cache')",
            [],
            |r| r.get(0),
        )
        .context("DATABASE_ERROR")?;
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS parse_cache (
           cache_key TEXT PRIMARY KEY, platform TEXT NOT NULL, video_id TEXT NOT NULL,
           resolver TEXT NOT NULL, kind TEXT NOT NULL, metadata TEXT NOT NULL,
           saved_at INTEGER NOT NULL CHECK(saved_at>=0), revision TEXT NOT NULL, cover BLOB);
         CREATE TABLE IF NOT EXISTS parse_cache_images (
           cache_key TEXT NOT NULL, image_index INTEGER NOT NULL CHECK(image_index BETWEEN 0 AND 63),
           body BLOB NOT NULL CHECK(length(body) BETWEEN 1 AND 33554432),
           PRIMARY KEY(cache_key,image_index));
         CREATE TABLE IF NOT EXISTS parse_cache_aliases (
           url TEXT PRIMARY KEY, platform TEXT NOT NULL, video_id TEXT NOT NULL);",
    )
    .context("DATABASE_ERROR")?;
    let has_avatar: bool = db.prepare("PRAGMA table_info(parse_cache)")?
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?.iter().any(|name| name == "author_avatar");
    if !has_avatar {
        db.execute("ALTER TABLE parse_cache ADD COLUMN author_avatar BLOB", [])
            .context("DATABASE_ERROR")?;
    }
    Ok(!existed)
}

fn revision(resolver: &str) -> String {
    if resolver == "tdl" {
        return crate::tdl_bridge::PROTOCOL_REVISION.into();
    }
    let dtk: Value = serde_json::from_str(include_str!("../../vendor/dtk/manifest.json"))
        .expect("bundled manifest");
    format!(
        "{}:{}:{}",
        crate::python_bridge::ENGINE_VERSION,
        crate::python_bridge::YTDLP_VERSION,
        dtk["upstream_commit"].as_str().expect("bundled commit")
    )
}

fn key(video: &Value) -> Result<String> {
    let identity: Video = serde_json::from_value(video.clone()).context("INVALID_ENGINE_RESULT")?;
    identity.validate()?;
    Ok(format!(
        "{}_{}_{}",
        identity.platform, identity.video_id, identity.resolver
    ))
}

// Only extract/normalize a URL for looking up previously validated aliases.
// Platform recognition and short-link expansion remain in the Python engine.
pub fn share_url(input: &str) -> Option<String> {
    let start = input
        .match_indices("http://")
        .chain(input.match_indices("https://"))
        .map(|(start, _)| start)
        .min()?;
    let text = &input[start..];
    let end = text
        .find(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | '"'))
        .unwrap_or(text.len());
    if text[end..].contains("http://") || text[end..].contains("https://") {
        return None;
    }
    let text = text[..end].trim_end_matches([
        '.', ',', ';', '!', '?', '，', '。', '；', '！', '）', '》', ')',
    ]);
    let mut url = url::Url::parse(text).ok()?;
    if !url.username().is_empty() || url.password().is_some() || url.port().is_some() {
        return None;
    }
    url.set_scheme("https").ok()?;
    let video_id = if url.path() == "/watch" {
        url.query_pairs()
            .find(|(key, _)| key == "v")
            .map(|(_, value)| value.into_owned())
    } else {
        None
    };
    url.set_query(None);
    if let Some(id) = video_id {
        url.query_pairs_mut().append_pair("v", &id);
    }
    url.set_fragment(None);
    let path = url.path().trim_end_matches('/').to_owned();
    url.set_path(&path);
    Some(url.into())
}

pub fn get(db: &Connection, platform: &str, id: &str, resolver: &str) -> Result<Option<Preview>> {
    let row = db.query_row(
        "SELECT cache_key,metadata,saved_at,cover IS NOT NULL,author_avatar IS NOT NULL FROM parse_cache
         WHERE platform=?1 AND video_id=?2 AND revision=?4 AND
           (resolver=?3 OR (?3='auto' AND resolver=CASE WHEN kind='image_album' THEN 'dtk' ELSE 'yt-dlp' END))",
        params![platform,id,resolver,revision(resolver)],
        |r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,i64>(2)?,r.get::<_,bool>(3)?,r.get::<_,bool>(4)?)),
    ).optional().context("DATABASE_ERROR")?;
    let Some((stored_key, metadata, saved_at, has_cover, has_avatar)) = row else {
        return Ok(None);
    };
    let mut video: Value = serde_json::from_str(&metadata).context("DATABASE_ERROR")?;
    if let Some(profile) = video["author_profile"].as_object_mut() {
        profile.insert("avatar_cached".into(), has_avatar.into());
    }
    if key(&video)? != stored_key {
        bail!("DATABASE_ERROR");
    }
    if video["kind"] == "image_album" {
        let count = video["images"].as_array().context("DATABASE_ERROR")?.len();
        let available: i64 = db
            .query_row(
                "SELECT count(*) FROM parse_cache_images WHERE cache_key=?",
                [&stored_key],
                |r| r.get(0),
            )
            .context("DATABASE_ERROR")?;
        if count == 0 || count > 64 || available != count as i64 {
            return Ok(None);
        }
    }
    Ok(Some(Preview {
        video,
        saved_at: u64::try_from(saved_at).context("DATABASE_ERROR")?,
        has_cover,
    }))
}

pub fn find(db: &Connection, input: &str, resolver: &str) -> Result<Option<Preview>> {
    let Some(url) = share_url(input) else {
        return Ok(None);
    };
    let identity = db
        .query_row(
            "SELECT platform,video_id FROM parse_cache_aliases WHERE url=?",
            [url],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
        )
        .optional()
        .context("DATABASE_ERROR")?;
    match identity {
        Some((platform, id)) => get(db, &platform, &id, resolver),
        None => Ok(None),
    }
}

// A validated public message alias also identifies its channel. Reuse existing
// cache rows across messages; conflicting or obsolete rows are not authority.
pub fn tdl_channel(db: &Connection, input: &str) -> Result<Option<u64>> {
    let link = crate::tdl_bridge::message_link(input)?;
    let Some(peer) = crate::tdl_bridge::public_peer(&link) else {
        return Ok(link.channel);
    };
    let mut query = db
        .prepare(
            "SELECT a.url,a.video_id,c.metadata FROM parse_cache_aliases a JOIN parse_cache c
         ON a.platform=c.platform AND a.video_id=c.video_id
         WHERE a.platform='telegram' AND c.resolver='tdl' AND c.revision=?",
        )
        .context("DATABASE_ERROR")?;
    let rows = query
        .query_map([revision("tdl")], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })
        .context("DATABASE_ERROR")?;
    let mut channel = None;
    for row in rows {
        let (url, id, metadata) = row.context("DATABASE_ERROR")?;
        let Ok(alias) = crate::tdl_bridge::message_link(&url) else {
            continue;
        };
        if crate::tdl_bridge::public_peer(&alias).as_ref() != Some(&peer) {
            continue;
        }
        let video: Value = serde_json::from_str(&metadata).context("DATABASE_ERROR")?;
        let Some(canonical) = video["canonical_url"].as_str() else {
            continue;
        };
        if !crate::tdl_bridge::validate_identity(&id, canonical) {
            continue;
        }
        let identity = crate::tdl_bridge::message_link(canonical)?;
        if identity.message != alias.message {
            continue;
        }
        if channel.is_some() && channel != identity.channel {
            return Ok(None);
        }
        channel = identity.channel;
    }
    Ok(channel)
}

// Re-resolve a known public alias in tdl when starting a download, so the live
// service can bind that username to its freshly verified numeric channel ID.
pub fn tdl_source(db: &Connection, video: &Value) -> Result<String> {
    let canonical = video["canonical_url"]
        .as_str()
        .context("INVALID_ENGINE_RESULT")?;
    let expected = crate::tdl_bridge::message_link(canonical)?;
    let mut statement = db.prepare(
        "SELECT url FROM parse_cache_aliases WHERE platform='telegram' AND video_id=? ORDER BY url"
    ).context("DATABASE_ERROR")?;
    let aliases = statement
        .query_map(
            [video["video_id"]
                .as_str()
                .context("INVALID_ENGINE_RESULT")?],
            |r| r.get::<_, String>(0),
        )
        .context("DATABASE_ERROR")?;
    for alias in aliases {
        let alias = alias.context("DATABASE_ERROR")?;
        if let Ok(link) = crate::tdl_bridge::message_link(&alias) {
            if link.channel.is_none() && link.message == expected.message {
                // Preserve the database key for precise invalidation (including
                // telegram.me). The bridge normalizes its own CLI argument.
                return Ok(alias);
            }
        }
    }
    Ok(canonical.to_owned())
}

pub fn remember(db: &Connection, input: &str, video: &Value) -> Result<()> {
    if video["platform"] == "telegram" {
        let link = crate::tdl_bridge::message_link(input)?;
        if let Some(peer) = crate::tdl_bridge::public_peer(&link) {
            let canonical = crate::tdl_bridge::message_link(
                video["canonical_url"]
                    .as_str()
                    .context("INVALID_ENGINE_RESULT")?,
            )?;
            // A freshly established mapping replaces older aliases for a
            // different channel, without removing their numeric cache records.
            let mut query = db
                .prepare("SELECT url,video_id FROM parse_cache_aliases WHERE platform='telegram'")
                .context("DATABASE_ERROR")?;
            let aliases = query
                .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
                .context("DATABASE_ERROR")?
                .collect::<rusqlite::Result<Vec<_>>>()
                .context("DATABASE_ERROR")?;
            drop(query);
            for (url, id) in aliases {
                let Ok(old) = crate::tdl_bridge::message_link(&url) else {
                    continue;
                };
                if crate::tdl_bridge::public_peer(&old).as_ref() == Some(&peer)
                    && canonical
                        .channel
                        .is_some_and(|channel| !id.starts_with(&format!("{channel}_")))
                {
                    db.execute("DELETE FROM parse_cache_aliases WHERE url=?", [url])
                        .context("DATABASE_ERROR")?;
                }
            }
        }
    }
    if let Some(url) = share_url(input) {
        db.execute(
            "INSERT INTO parse_cache_aliases(url,platform,video_id) VALUES(?1,?2,?3)
            ON CONFLICT(url) DO UPDATE SET platform=excluded.platform,video_id=excluded.video_id",
            params![
                url,
                video["platform"]
                    .as_str()
                    .context("INVALID_ENGINE_RESULT")?,
                video["video_id"]
                    .as_str()
                    .context("INVALID_ENGINE_RESULT")?
            ],
        )
        .context("DATABASE_ERROR")?;
    }
    Ok(())
}

pub fn forget_tdl_channel(db: &Connection, input: &str, video: &Value) -> Result<()> {
    let peer = crate::tdl_bridge::public_peer(&crate::tdl_bridge::message_link(input)?);
    let channel = crate::tdl_bridge::message_link(
        video["canonical_url"]
            .as_str()
            .context("INVALID_ENGINE_RESULT")?,
    )?
    .channel;
    let mut query = db
        .prepare("SELECT url,video_id FROM parse_cache_aliases WHERE platform='telegram'")
        .context("DATABASE_ERROR")?;
    let aliases = query
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .context("DATABASE_ERROR")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("DATABASE_ERROR")?;
    drop(query);
    for (url, id) in aliases {
        let Ok(link) = crate::tdl_bridge::message_link(&url) else {
            continue;
        };
        if peer.is_some()
            && crate::tdl_bridge::public_peer(&link) == peer
            && channel.is_some_and(|channel| id.starts_with(&format!("{channel}_")))
        {
            db.execute("DELETE FROM parse_cache_aliases WHERE url=?", [url])
                .context("DATABASE_ERROR")?;
        }
    }
    Ok(())
}

// A fresh tdl HEAD can prove cached original-file metadata obsolete. Let retry resolve it again.
pub fn invalidate_tdl(db: &mut Connection, video: &Value) -> Result<()> {
    let cache_key = key(video)?;
    if video["platform"] != "telegram" || video["resolver"] != "tdl" {
        bail!("INVALID_ENGINE_RESULT");
    }
    let tx = db.transaction().context("DATABASE_ERROR")?;
    tx.execute(
        "DELETE FROM parse_cache_images WHERE cache_key=?",
        [&cache_key],
    )
    .context("DATABASE_ERROR")?;
    tx.execute("DELETE FROM parse_cache WHERE cache_key=?", [&cache_key])
        .context("DATABASE_ERROR")?;
    tx.execute(
        "DELETE FROM parse_cache_aliases WHERE platform='telegram' AND video_id=?",
        [video["video_id"]
            .as_str()
            .context("INVALID_ENGINE_RESULT")?],
    )
    .context("DATABASE_ERROR")?;
    tx.commit().context("DATABASE_ERROR")?;
    Ok(())
}

pub fn save(
    db: &mut Connection,
    video: &Value,
    images: Vec<Vec<u8>>,
    cover: Option<Vec<u8>>,
    saved_at: u64,
) -> Result<Preview> {
    let identity: Video = serde_json::from_value(video.clone()).context("INVALID_ENGINE_RESULT")?;
    let cache_key = key(video)?;
    let mut metadata = video.clone();
    metadata["kind"] = identity.kind.clone().into();
    metadata["resolver"] = identity.resolver.clone().into();
    metadata["cover_url"] = Value::Null;
    if identity.kind == "image_album" {
        let records = metadata["images"]
            .as_array_mut()
            .context("INVALID_ENGINE_RESULT")?;
        if records.is_empty() || records.len() > 64 || records.len() != images.len() {
            bail!("INCOMPLETE_MEDIA");
        }
        for (index, (record, body)) in records.iter_mut().zip(&images).enumerate() {
            let ext = image_extension(body).context("INVALID_MEDIA_CONTENT")?;
            record["preview_filename"] = format!("preview-{:03}.{ext}", index + 1).into();
        }
    }
    let object = metadata.as_object_mut().context("INVALID_ENGINE_RESULT")?;
    for field in [
        "cover_filename",
        "cover_bytes",
        "cover_error",
        "source_url",
        "cookie_warning",
        "author_avatar_filename",
        "author_avatar_bytes",
    ] {
        object.remove(field);
    }
    let has_cover = cover.is_some();
    let tx = db.transaction().context("DATABASE_ERROR")?;
    tx.execute(
        "INSERT INTO parse_cache(cache_key,platform,video_id,resolver,kind,metadata,saved_at,revision,cover)
        VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9) ON CONFLICT(cache_key) DO UPDATE SET
        author_avatar=CASE WHEN json_extract(parse_cache.metadata,'$.author_profile.id')=json_extract(excluded.metadata,'$.author_profile.id')
            THEN parse_cache.author_avatar ELSE NULL END,
        metadata=excluded.metadata,kind=excluded.kind,saved_at=excluded.saved_at,revision=excluded.revision,cover=excluded.cover",
        params![
            cache_key,
            identity.platform,
            identity.video_id,
            identity.resolver,
            identity.kind,
            metadata.to_string(),
            i64::try_from(saved_at).context("DATABASE_ERROR")?,
            revision(&identity.resolver),
            cover
        ],
    )
    .context("DATABASE_ERROR")?;
    tx.execute(
        "DELETE FROM parse_cache_images WHERE cache_key=?",
        [&cache_key],
    )
    .context("DATABASE_ERROR")?;
    for (index, body) in images.iter().enumerate() {
        tx.execute(
            "INSERT INTO parse_cache_images VALUES(?1,?2,?3)",
            params![cache_key, index as i64, body],
        )
        .context("DATABASE_ERROR")?;
    }
    remember(
        &tx,
        video["canonical_url"]
            .as_str()
            .context("INVALID_ENGINE_RESULT")?,
        video,
    )?;
    if let Some(url) = video["source_url"].as_str() {
        remember(&tx, url, video)?;
    }
    tx.commit().context("DATABASE_ERROR")?;
    Ok(Preview {
        video: metadata,
        saved_at,
        has_cover,
    })
}

pub fn image(db: &Connection, video: &Value, index: usize) -> Result<Vec<u8>> {
    let body: Vec<u8> = db
        .query_row(
            "SELECT body FROM parse_cache_images WHERE cache_key=?1 AND image_index=?2",
            params![key(video)?, i64::try_from(index).context("INVALID_INPUT")?],
            |r| r.get(0),
        )
        .optional()
        .context("DATABASE_ERROR")?
        .context("FILE_MISSING")?;
    if body.len() > 32 * 1024 * 1024 || image_extension(&body).is_none() {
        bail!("INVALID_MEDIA_CONTENT");
    }
    Ok(body)
}

// Add a downloaded cover without replacing newer parsed metadata or restoring an
// invalidated cache entry. A cache write failure never changes download success.
pub fn save_cover(db: &Connection, video: &Value, body: &[u8]) -> Result<bool> {
    if body.len() > 10 * 1024 * 1024
        || !matches!(image_extension(body), Some("jpeg" | "png" | "webp"))
    {
        bail!("INVALID_MEDIA_CONTENT");
    }
    Ok(db.execute(
        "UPDATE parse_cache SET cover=?1 WHERE cache_key=?2 AND revision=?3",
        params![body, key(video)?, revision(video["resolver"].as_str().context("INVALID_ENGINE_RESULT")?)],
    ).context("DATABASE_ERROR")? != 0)
}

// Task removal can release shared cover/avatar bytes while preserving reusable
// album images, metadata and aliases. The caller checks for remaining references.
pub fn clear_cover(db: &Connection, video: &Value) -> Result<()> {
    db.execute(
        "UPDATE parse_cache SET cover=NULL,author_avatar=NULL WHERE cache_key=?",
        [key(video)?],
    )
    .context("DATABASE_ERROR")?;
    Ok(())
}

pub fn cover(db: &Connection, video: &Value) -> Result<Vec<u8>> {
    cached_image(db, video, "SELECT cover FROM parse_cache WHERE cache_key=?", 10 * 1024 * 1024)
}

pub fn save_avatar(db: &Connection, video: &Value, body: &[u8]) -> Result<bool> {
    if body.len() > 2 * 1024 * 1024
        || !matches!(image_extension(body), Some("jpeg" | "png" | "webp")) {
        bail!("INVALID_MEDIA_CONTENT");
    }
    Ok(db.execute("UPDATE parse_cache SET author_avatar=?1 WHERE cache_key=?2",
        params![body, key(video)?]).context("DATABASE_ERROR")? != 0)
}

pub fn avatar(db: &Connection, video: &Value) -> Result<Vec<u8>> {
    cached_image(db, video, "SELECT author_avatar FROM parse_cache WHERE cache_key=?", 2 * 1024 * 1024)
}

fn cached_image(db: &Connection, video: &Value, query: &str, limit: usize) -> Result<Vec<u8>> {
    let body: Option<Vec<u8>> = db
        .query_row(
            query,
            [key(video)?],
            |r| r.get(0),
        )
        .optional()
        .context("DATABASE_ERROR")?
        .flatten();
    let body = body.context("FILE_MISSING")?;
    if body.len() > limit
        || !matches!(image_extension(&body), Some("jpeg" | "png" | "webp"))
    {
        bail!("INVALID_MEDIA_CONTENT");
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn author_avatar_migrates_persists_and_is_removed_with_last_reference() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("old.db");
        let mut db = Connection::open(&path).unwrap();
        db.execute_batch("CREATE TABLE parse_cache(cache_key TEXT PRIMARY KEY,platform TEXT NOT NULL,video_id TEXT NOT NULL,resolver TEXT NOT NULL,kind TEXT NOT NULL,metadata TEXT NOT NULL,saved_at INTEGER NOT NULL,revision TEXT NOT NULL,cover BLOB);").unwrap();
        init(&db).unwrap();
        init(&db).unwrap();
        let mut video = json!({"platform":"douyin","video_id":"123","canonical_url":"https://www.douyin.com/video/123","resolver":"yt-dlp","kind":"video",
            "author_profile":{"id":"author-1","nickname":"昵称","handle":"handle","kind":"user"},
            "author_avatar_filename":"author-avatar","author_avatar_bytes":15});
        let body = b"\x89PNG\r\n\x1a\nfixture";
        let saved = save(&mut db, &video, vec![], None, 100).unwrap();
        assert!(saved.video.get("author_avatar_filename").is_none());
        assert!(saved.video.get("author_avatar_bytes").is_none());
        assert!(save_avatar(&db, &video, b"<html>login</html>").is_err());
        assert!(save_avatar(&db, &video, body).unwrap());
        drop(db);
        let mut db = Connection::open(&path).unwrap();
        init(&db).unwrap();
        let hit = get(&db, "douyin", "123", "yt-dlp").unwrap().unwrap();
        assert_eq!(hit.video["author_profile"]["nickname"], "昵称");
        assert_eq!(hit.video["author_profile"]["avatar_cached"], true);
        assert_eq!(avatar(&db, &hit.video).unwrap(), body);
        save(&mut db, &video, vec![], None, 101).unwrap();
        assert_eq!(avatar(&db, &video).unwrap(), body);
        video["author_profile"]["id"] = "another-author".into();
        save(&mut db, &video, vec![], None, 102).unwrap();
        assert!(avatar(&db, &video).is_err());
        save_avatar(&db, &video, body).unwrap();
        clear_cover(&db, &video).unwrap();
        assert!(avatar(&db, &video).is_err());
        let hit = get(&db, "douyin", "123", "yt-dlp").unwrap().unwrap();
        assert_eq!(hit.video["author_profile"]["avatar_cached"], false);
        assert_eq!(hit.video["author_profile"]["id"], "another-author");
    }

    #[test]
    fn telegram_channel_mapping_reuses_valid_aliases_across_messages_and_restart() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("cache.sqlite");
        let mut db = Connection::open(&path).unwrap();
        init(&db).unwrap();
        let video = json!({"platform":"telegram","video_id":"100_9","resolver":"tdl","kind":"video",
            "canonical_url":"https://t.me/c/100/9","selected_format_id":"original",
            "formats":[{"format_id":"original","container":"mp4","filesize":4096}]});
        save(&mut db, &video, vec![], None, 1).unwrap();
        remember(&db, "分享文案 https://telegram.me/Public_Channel/9", &video).unwrap();
        assert!(find(&db, "https://t.me/public_channel/10", "tdl")
            .unwrap()
            .is_none());
        assert_eq!(
            tdl_channel(&db, "https://t.me/public_channel/10").unwrap(),
            Some(100)
        );
        assert_eq!(
            tdl_channel(&db, "https://t.me/new_channel/10").unwrap(),
            None
        );
        drop(db);
        let db = Connection::open(&path).unwrap();
        assert_eq!(
            tdl_channel(&db, "https://telegram.me/PUBLIC_CHANNEL/11").unwrap(),
            Some(100)
        );
        forget_tdl_channel(&db, "https://t.me/public_channel/9", &video).unwrap();
        assert_eq!(
            tdl_channel(&db, "https://t.me/public_channel/10").unwrap(),
            None
        );
        assert!(get(&db, "telegram", "100_9", "tdl").unwrap().is_some());
        remember(&db, "https://t.me/public_channel/9", &video).unwrap();
        db.execute("UPDATE parse_cache SET revision='obsolete'", [])
            .unwrap();
        assert_eq!(
            tdl_channel(&db, "https://t.me/public_channel/10").unwrap(),
            None
        );
    }

    #[test]
    fn freshly_resolved_telegram_username_replaces_channel_mapping_without_deleting_numeric_cache()
    {
        let mut db = Connection::open_in_memory().unwrap();
        init(&db).unwrap();
        for channel in [100, 101] {
            let message = if channel == 100 { 9 } else { 10 };
            let video = json!({"platform":"telegram","video_id":format!("{channel}_{message}"),"resolver":"tdl","kind":"video",
                "canonical_url":format!("https://t.me/c/{channel}/{message}"),"selected_format_id":"original",
                "formats":[{"format_id":"original","container":"mp4","filesize":4096}]});
            save(&mut db, &video, vec![], None, channel).unwrap();
            remember(&db, &format!("https://t.me/channel/{message}"), &video).unwrap();
        }
        assert_eq!(
            tdl_channel(&db, "https://t.me/channel/11").unwrap(),
            Some(101)
        );
        assert!(get(&db, "telegram", "100_9", "tdl").unwrap().is_some());
    }

    #[test]
    fn downloaded_cover_updates_existing_cache_without_overwriting_metadata_or_reviving_entries() {
        let mut db = Connection::open_in_memory().unwrap();
        init(&db).unwrap();
        let video = json!({"platform":"telegram","video_id":"100_9","resolver":"tdl","kind":"video",
            "canonical_url":"https://t.me/c/100/9","title":"current","selected_format_id":"original",
            "formats":[{"format_id":"original","container":"mp4","filesize":4096}]});
        let body = b"\x89PNG\r\n\x1a\nfixture";
        assert!(!save_cover(&db, &video, body).unwrap());
        save(&mut db, &video, vec![], None, 100).unwrap();
        let mut stale = video.clone();
        stale["title"] = "old".into();
        assert!(save_cover(&db, &stale, body).unwrap());
        let hit = find(&db, "https://t.me/c/100/9", "tdl").unwrap().unwrap();
        assert_eq!(hit.video["title"], "current");
        assert_eq!(hit.saved_at, 100);
        assert!(hit.has_cover);
        assert_eq!(cover(&db, &video).unwrap(), body);
        assert!(save_cover(&db, &video, b"<html>error</html>").is_err());
        db.execute("UPDATE parse_cache SET revision='obsolete',cover=NULL", [])
            .unwrap();
        assert!(!save_cover(&db, &video, body).unwrap());
        assert!(cover(&db, &video).is_err());
    }

    #[test]
    fn aliases_assets_and_resolver_revision_are_isolated() {
        let mut db = Connection::open_in_memory().unwrap();
        assert!(init(&db).unwrap());
        let video = json!({"platform":"douyin","video_id":"123","kind":"video","resolver":"yt-dlp",
            "canonical_url":"https://www.douyin.com/video/123","title":"cached",
            "cover_url":"https://example.invalid/cover?private_token=never_store"});
        let cover_body = b"\x89PNG\r\n\x1a\nfixture".to_vec();
        save(&mut db, &video, vec![], Some(cover_body.clone()), 100).unwrap();
        remember(
            &db,
            "private share text https://v.douyin.com/share/?tracking=secret",
            &video,
        )
        .unwrap();
        let hit = find(&db, "new share text https://v.douyin.com/share/。", "auto")
            .unwrap()
            .unwrap();
        assert_eq!(hit.video["title"], "cached");
        assert!(hit.video["cover_url"].is_null());
        assert_eq!(cover(&db, &hit.video).unwrap(), cover_body);
        assert!(find(
            &db,
            "https://www.douyin.com/video/123?tracking=https://example.invalid",
            "auto"
        )
        .unwrap()
        .is_some());
        assert!(find(&db, "https://www.douyin.com/video/123", "dtk")
            .unwrap()
            .is_none());
        let mut dtk = video.clone();
        dtk["resolver"] = "dtk".into();
        dtk["title"] = "other engine".into();
        save(&mut db, &dtk, vec![], None, 101).unwrap();
        assert_eq!(
            find(&db, "https://www.douyin.com/video/123", "dtk")
                .unwrap()
                .unwrap()
                .video["title"],
            "other engine"
        );
        assert_eq!(
            find(&db, "https://www.douyin.com/video/123", "auto")
                .unwrap()
                .unwrap()
                .video["title"],
            "cached"
        );
        assert!(find(
            &db,
            "https://www.douyin.com/video/123 https://v.douyin.com/share/",
            "auto"
        )
        .unwrap()
        .is_none());
        db.execute("UPDATE parse_cache SET revision='old parser'", [])
            .unwrap();
        assert!(find(&db, "https://www.douyin.com/video/123", "auto")
            .unwrap()
            .is_none());
        assert!(!init(&db).unwrap());
        let stored: String = db
            .query_row("SELECT metadata FROM parse_cache LIMIT 1", [], |r| r.get(0))
            .unwrap();
        assert!(!stored.contains("private_token"));
        let urls: String = db
            .query_row(
                "SELECT group_concat(url) FROM parse_cache_aliases",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(!urls.contains("share text") && !urls.contains("tracking"));
    }
    #[test]
    fn youtube_watch_aliases_keep_distinct_video_ids() {
        let first = share_url("https://www.youtube.com/watch?v=BaW_jenozKc&list=ignored").unwrap();
        let second = share_url("https://www.youtube.com/watch?v=YE7VzlLtp-4&tracking=x").unwrap();
        assert_eq!(first, "https://www.youtube.com/watch?v=BaW_jenozKc");
        assert_ne!(first, second);
        let mut db = Connection::open_in_memory().unwrap();
        init(&db).unwrap();
        let video = serde_json::json!({"platform":"youtube","video_id":"BaW_jenozKc",
            "canonical_url":first,"kind":"video","resolver":"yt-dlp",
            "cookie_warning":"BROWSER_COOKIES_UNAVAILABLE"});
        let stored = save(&mut db, &video, vec![], None, 1).unwrap();
        assert!(stored.video["cookie_warning"].is_null());
        assert!(find(&db, &first, "auto").unwrap().is_some());
        assert!(find(&db, &second, "auto").unwrap().is_none());
    }
}
