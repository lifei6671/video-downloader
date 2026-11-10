//! SQLite initialization, compatible local migration and asset persistence; no network calls.
use super::model::{Asset, AssetTranscript, Space};
use crate::desktop_core::{transcription::Snapshot, Status, Task};
use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{fs, io::Read, path::Path, time::UNIX_EPOCH};
use uuid::Uuid;

pub fn init(db: &Connection) -> Result<()> {
    let prepared = super::schema_migration::prepare(db)?;
    let tx = db.unchecked_transaction()?;
    init_in_transaction(&tx, prepared)?;
    tx.commit()?;
    Ok(())
}

/// The desktop startup includes its settings and account schema in this transaction.
pub(crate) fn init_in_transaction(
    tx: &Connection,
    prepared: super::schema_migration::Prepared,
) -> Result<()> {
    if tx.is_autocommit() {
        bail!("BACKUP_SCHEMA_REQUIRES_TRANSACTION");
    }
    super::schema_migration::check_unchanged(tx, &prepared)?;
    tx.execute_batch(include_str!("schema.sql"))?;
    tx.execute_batch(include_str!("sync-schema.sql"))?;
    tx.execute_batch(include_str!("telegram-schema.sql"))?;
    tx.execute_batch(include_str!("operation-display.sql"))?;
    // Reclaim captures left by older terminal paths, preserving uncertain/canceled work.
    tx.execute("DELETE FROM backup_operation_captures WHERE operation_id IN (SELECT operation_id FROM backup_operations WHERE kind='upload' AND state='completed')", [])?;
    // History tasks exist in the desktop database, but not in standalone backup stores.
    if tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='download_tasks')",
        [],
        |r| r.get::<_, bool>(0),
    )? {
        tx.execute_batch(include_str!("history-generation.sql"))?;
    }
    super::spaces::select_initial_default(tx)?;
    tx.execute(
        "INSERT OR IGNORE INTO backup_device(id,device_id) VALUES(1,?)",
        [Uuid::new_v4().to_string()],
    )?;
    if tx
        .prepare("PRAGMA foreign_key_check")?
        .query([])?
        .next()?
        .is_some()
    {
        bail!("BACKUP_SCHEMA_FOREIGN_KEY_ERROR");
    }
    // A running operation has an uncertain external outcome after restart.
    tx.execute("UPDATE backup_operations SET state='pending',last_error='BACKUP_RECONCILE_REQUIRED',updated_at=unixepoch() WHERE state='running'", [])?;
    tx.execute(
        "UPDATE asset_local_files SET local_state='missing' WHERE local_state='restoring'",
        [],
    )?;
    super::schema_migration::finish(tx, &prepared)?;
    Ok(())
}

pub fn device_id(db: &Connection) -> Result<String> {
    Ok(
        db.query_row("SELECT device_id FROM backup_device WHERE id=1", [], |r| {
            r.get(0)
        })?,
    )
}

/// A remote identity is always resolved within its owning space, never globally.
pub fn local_asset_id(db: &Connection, space: &str, remote: &str) -> Result<Option<String>> {
    Ok(db.query_row("SELECT local_asset_id FROM backup_space_assets WHERE space_id=?1 AND remote_asset_id=?2", params![space,remote], |r|r.get(0)).optional()?)
}

/// Bind an imported identity; a collision in another library gets a fresh local UUID.
pub fn bind_remote_asset(db: &Connection, space: &str, remote: &str) -> Result<String> {
    if let Some(id) = local_asset_id(db, space, remote)? {
        return Ok(id);
    }
    uuid::Uuid::parse_str(remote).context("BACKUP_INVALID_RECORD")?;
    let exists: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM media_assets WHERE asset_id=?)",
        [remote],
        |r| r.get(0),
    )?;
    let id = if exists {
        Uuid::new_v4().to_string()
    } else {
        remote.to_owned()
    };
    db.execute(
        "INSERT INTO media_assets VALUES(?1,NULL,'{}','',NULL,1,'active')",
        [&id],
    )?;
    db.execute(
        "INSERT INTO asset_local_files(asset_id,bytes,local_state) VALUES(?1,0,'missing')",
        [&id],
    )?;
    db.execute(
        "INSERT INTO backup_space_assets(space_id,remote_asset_id,local_asset_id) VALUES(?1,?2,?3)",
        params![space, remote, id],
    )?;
    Ok(id)
}

pub fn spaces(db: &Connection) -> Result<Vec<Space>> {
    let mut q=db.prepare("SELECT space_id,library_id,endpoint,bucket,prefix,credential_ref,enabled FROM backup_spaces ORDER BY rowid DESC")?;
    let rows = q.query_map([], |r| {
        Ok(Space {
            space_id: r.get(0)?,
            library_id: r.get(1)?,
            endpoint: r.get(2)?,
            bucket: r.get(3)?,
            prefix: r.get(4)?,
            credential_ref: r.get(5)?,
            enabled: r.get(6)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Called within the same transaction that persists the completed task.
pub fn associate(db: &Connection, task: &mut Task) -> Result<()> {
    if task.status != Status::Completed || !task.files.is_empty() {
        return Ok(());
    }
    let Some(video) = task
        .video
        .as_ref()
        .filter(|v| v["kind"].as_str().unwrap_or("video") == "video")
    else {
        return Ok(());
    };
    let Some(path) = task.path.as_ref() else {
        return Ok(());
    };
    let Ok(file) = fs::symlink_metadata(path) else {
        return Ok(());
    };
    if !file.is_file() || file.file_type().is_symlink() || file.len() == 0 {
        return Ok(());
    }
    let asset_id = task.asset_id.clone().unwrap_or_else(|| task.id.clone());
    let is_new: bool = !db.query_row(
        "SELECT EXISTS(SELECT 1 FROM media_assets WHERE asset_id=?)",
        [&asset_id],
        |r| r.get::<_, bool>(0),
    )?;
    let filename = path
        .file_name()
        .and_then(|v| v.to_str())
        .context("BACKUP_INVALID_FILENAME")?;
    let mut metadata = serde_json::Map::new();
    // Never serialize arbitrary extractor data, signed URLs, or the task snapshot.
    for field in [
        "platform",
        "video_id",
        "title",
        "author",
        "duration_ms",
        "width",
        "height",
        "selected_format_id",
    ] {
        if let Some(value) = video.get(field) {
            metadata.insert(field.into(), value.clone());
        }
    }
    if let Some(url) = canonical_source(video) {
        metadata.insert("canonical_url".into(), url.into());
    }
    metadata.insert("completed_at".into(), task.completed_at.into());
    db.execute("INSERT INTO media_assets(asset_id,task_id,metadata_json,original_filename,content_bytes,asset_state) VALUES(?1,?2,?3,?4,?5,'active') ON CONFLICT(asset_id) DO UPDATE SET task_id=excluded.task_id",
        params![asset_id,task.id,Value::Object(metadata).to_string(),filename,i64::try_from(file.len())?])?;
    if !is_new && video["platform"] == "telegram" {
        let raw: String = db.query_row(
            "SELECT metadata_json FROM media_assets WHERE asset_id=?",
            [&asset_id],
            |r| r.get(0),
        )?;
        let mut metadata: Value = serde_json::from_str(&raw)?;
        let mut patch = super::provider::FieldPatch::new();
        for field in ["width", "height", "duration_ms"] {
            if metadata[field].as_u64().is_none_or(|value| value == 0)
                && video[field].as_u64().is_some_and(|value| value > 0)
            {
                metadata[field] = video[field].clone();
                patch.insert(field.into(), video[field].clone());
            }
        }
        if !patch.is_empty() {
            db.execute(
                "UPDATE media_assets SET metadata_json=?2 WHERE asset_id=?1",
                params![asset_id, metadata.to_string()],
            )?;
            super::sync_engine::mark_local(db, &asset_id, patch, false)?;
        }
    }
    let source = serde_json::to_string(&crate::desktop_core::transcription::Source::read(path)?)?;
    db.execute("INSERT OR IGNORE INTO asset_local_files(asset_id,path,bytes,source_json,local_state) VALUES(?1,?2,?3,?4,'present')",
        params![asset_id,path.to_string_lossy(),i64::try_from(file.len())?,source])?;
    // Preserve the cover independently before history can release its cache reference.
    let cover = crate::parse_cache::cover(db, video)
        .ok()
        .or_else(|| task.cover_path.as_ref().and_then(|p| fs::read(p).ok()));
    if let Some(cover) = cover {
        db.execute(
            "INSERT OR IGNORE INTO asset_covers(asset_id,body) VALUES(?1,?2)",
            params![asset_id, cover],
        )?;
    }
    task.asset_id = Some(asset_id);
    if is_new {
        super::sync_engine::register_download(
            db,
            &task.asset_id.clone().context("BACKUP_ASSET_NOT_FOUND")?,
        )?;
    }
    Ok(())
}

fn canonical_source(video: &Value) -> Option<String> {
    let mut url = url::Url::parse(video["canonical_url"].as_str()?).ok()?;
    if url.scheme() != "https" || !url.username().is_empty() || url.password().is_some() {
        return None;
    }
    let host = url.host_str()?;
    let valid = match video["platform"].as_str()? {
        "youtube" => matches!(host, "www.youtube.com" | "youtube.com" | "youtu.be"),
        "douyin" => matches!(host, "www.douyin.com" | "douyin.com"),
        "xiaohongshu" => host == "www.xiaohongshu.com",
        "tiktok" => matches!(host, "www.tiktok.com" | "tiktok.com"),
        "telegram" => host == "t.me",
        _ => false,
    };
    if !valid {
        return None;
    }
    let youtube_id = if video["platform"] == "youtube" {
        url.query_pairs()
            .find(|(k, _)| k == "v")
            .map(|(_, v)| v.into_owned())
    } else {
        None
    };
    url.set_query(None);
    url.set_fragment(None);
    if let Some(id) = youtube_id {
        url.query_pairs_mut().append_pair("v", &id);
    }
    Some(url.into())
}

/// Streaming hash must run outside Core.inner (or during startup before Core exists).
pub fn fingerprint(path: &Path) -> Result<(String, Value)> {
    fingerprint_with_cancel(path, || false)
}

pub fn fingerprint_with_cancel(
    path: &Path,
    canceled: impl Fn() -> bool,
) -> Result<(String, Value)> {
    if canceled() {
        bail!("CANCELED");
    }
    let before = fs::symlink_metadata(path)?;
    if !before.is_file() || before.file_type().is_symlink() {
        bail!("FILE_MISSING");
    }
    let mut file = fs::File::open(path)?;
    let mut hash = Sha256::new();
    let mut buffer = [0_u8; 65536];
    loop {
        if canceled() {
            bail!("CANCELED");
        }
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    let after = fs::symlink_metadata(path)?;
    if canceled() {
        bail!("CANCELED");
    }
    if before.len() != after.len()
        || before.modified()? != after.modified()?
        || !after.is_file()
        || after.file_type().is_symlink()
    {
        bail!("BACKUP_SOURCE_CHANGED");
    }
    let source = json!({"path":path.canonicalize()?,"size":after.len(),"mtime_ns":after.modified()?.duration_since(UNIX_EPOCH)?.as_nanos().to_string()});
    Ok((format!("{:x}", hash.finalize()), source))
}

pub fn save_fingerprint(db: &Connection, id: &str, hash: &str, source: &Value) -> Result<()> {
    let tx = db.unchecked_transaction()?;
    tx.execute(
        "UPDATE media_assets SET content_sha256=?2,content_bytes=?3 WHERE asset_id=?1",
        params![
            id,
            hash,
            source["size"]
                .as_u64()
                .and_then(|v| i64::try_from(v).ok())
                .context("BACKUP_INVALID_SIZE")?
        ],
    )?;
    tx.execute("UPDATE asset_local_files SET sha256=?2,source_json=?3,verified_at=unixepoch() WHERE asset_id=?1",params![id,hash,source.to_string()])?;
    tx.execute("UPDATE asset_transcripts SET content_sha256=CASE WHEN content_sha256=?3 OR source_json=?2 THEN ?3 ELSE NULL END WHERE asset_id=?1",params![id,source.to_string(),hash])?;
    tx.commit()?;
    Ok(())
}

pub fn preserve_transcript(db: &Connection, snapshot: &Snapshot) -> Result<()> {
    persist_transcript(db, snapshot, true)
}
fn persist_transcript(db: &Connection, snapshot: &Snapshot, local_change: bool) -> Result<()> {
    let Some(result) = snapshot.result.as_ref() else {
        return Ok(());
    };
    let asset: Option<(String, Option<String>, Option<String>)> = db.query_row(
        "SELECT a.asset_id,l.sha256,l.source_json FROM media_assets a JOIN asset_local_files l USING(asset_id) WHERE a.task_id=?1 OR a.asset_id=?1",
        [&snapshot.task_id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
    let Some((id, hash, source)) = asset else {
        return Ok(());
    };
    let previous = transcript(db, &id)?;
    let source_json = serde_json::to_value(&result.source)?.to_string();
    let verified_hash = if source.as_deref() == Some(&source_json) {
        hash
    } else {
        None
    };
    db.execute("INSERT INTO asset_transcripts(asset_id,content_sha256,text,model_revision,model_name,pipeline_revision,completed_at,source_json) VALUES(?1,?2,?3,?4,?5,?6,?7,?8) ON CONFLICT(asset_id) DO UPDATE SET content_sha256=excluded.content_sha256,text=excluded.text,model_revision=excluded.model_revision,model_name=excluded.model_name,pipeline_revision=excluded.pipeline_revision,completed_at=excluded.completed_at,source_json=excluded.source_json",
        params![id,verified_hash,result.text,result.model_revision,result.model_name,result.pipeline_revision,i64::try_from(result.completed_at)?,source_json])?;
    if local_change
        && serde_json::to_value(previous)? != serde_json::to_value(transcript(db, &id)?)?
    {
        super::sync_engine::mark_local(db, &id, Default::default(), true)?;
    }
    Ok(())
}

const ASSET_SELECT: &str = "SELECT a.asset_id,a.task_id,a.metadata_json,a.original_filename,a.content_sha256,a.content_bytes,a.asset_state,l.path,l.local_state FROM media_assets a JOIN asset_local_files l USING(asset_id)";

fn asset_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Asset> {
    let raw: String = r.get(2)?;
    let metadata = serde_json::from_str(&raw).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(2, rusqlite::types::Type::Text, Box::new(e))
    })?;
    Ok(Asset {
        asset_id: r.get(0)?,
        task_id: r.get(1)?,
        metadata,
        original_filename: r.get(3)?,
        content_sha256: r.get(4)?,
        content_bytes: sql_u64(r, 5)?,
        asset_state: r.get(6)?,
        path: r.get::<_, Option<String>>(7)?.map(Into::into),
        local_state: r.get(8)?,
    })
}

pub fn asset(db: &Connection, id: &str) -> Result<Option<Asset>> {
    Ok(db
        .query_row(
            &format!("{ASSET_SELECT} WHERE a.asset_id=?"),
            [id],
            asset_row,
        )
        .optional()?)
}

pub fn list_assets(db: &Connection) -> Result<Vec<Asset>> {
    let mut q = db.prepare(&format!("{ASSET_SELECT} ORDER BY a.rowid DESC"))?;
    let rows = q
        .query_map([], asset_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

pub fn transcript(db: &Connection, id: &str) -> Result<Option<AssetTranscript>> {
    Ok(db.query_row("SELECT asset_id,content_sha256,text,model_revision,model_name,pipeline_revision,completed_at FROM asset_transcripts WHERE asset_id=?",[id],|r|Ok(AssetTranscript{asset_id:r.get(0)?,content_sha256:r.get(1)?,text:r.get(2)?,model_revision:r.get(3)?,model_name:r.get(4)?,pipeline_revision:r.get(5)?,completed_at:sql_u64(r,6)?})).optional()?)
}

/// Endpoint identity is immutable; switching targets creates a new space ID.
pub fn save_space(db: &Connection, space: &Space) -> Result<()> {
    let identity: Option<(String, String, String, String)> = db
        .query_row(
            "SELECT library_id,endpoint,bucket,prefix FROM backup_spaces WHERE space_id=?",
            [&space.space_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()?;
    if identity.is_some_and(|v| {
        v != (
            space.library_id.clone(),
            space.endpoint.clone(),
            space.bucket.clone(),
            space.prefix.clone(),
        )
    }) {
        bail!("BACKUP_SPACE_CHANGED");
    }
    db.execute("INSERT INTO backup_spaces(space_id,library_id,endpoint,bucket,prefix,credential_ref,enabled) VALUES(?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(space_id) DO UPDATE SET credential_ref=excluded.credential_ref,enabled=excluded.enabled",params![space.space_id,space.library_id,space.endpoint,space.bucket,space.prefix,space.credential_ref,space.enabled])?;
    db.execute("INSERT OR IGNORE INTO backup_provider_config(space_id,provider_type,profile) VALUES(?,'s3','r2')",[&space.space_id])?;
    Ok(())
}

fn sql_u64(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<u64> {
    let value: i64 = row.get(index)?;
    u64::try_from(value).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(
            index,
            rusqlite::types::Type::Integer,
            Box::new(e),
        )
    })
}
