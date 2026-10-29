use super::{now, optional_unsigned, require_asset, sql_integer, unsigned};
use crate::{
    desktop_core::{Status, Task},
    parse_cache,
};
use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize)]
pub struct Asset {
    pub id: String,
    pub title: String,
    pub author: Option<String>,
    pub platform: Option<String>,
    pub video_id: Option<String>,
    pub canonical_url: Option<String>,
    pub created_at: u64,
    pub updated_at: u64,
    pub archived_at: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct AssetFile {
    pub id: String,
    pub asset_id: String,
    pub path: PathBuf,
    pub size: u64,
    pub mtime_ns: Option<String>,
    pub duration_ms: Option<u64>,
    pub width: Option<u64>,
    pub height: Option<u64>,
    pub missing: bool,
    pub changed: bool,
}

pub(super) fn asset_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Asset> {
    Ok(Asset {
        id: row.get(0)?,
        title: row.get(1)?,
        author: row.get(2)?,
        platform: row.get(3)?,
        video_id: row.get(4)?,
        canonical_url: row.get(5)?,
        created_at: unsigned(row, 6)?,
        updated_at: unsigned(row, 7)?,
        archived_at: optional_unsigned(row, 8)?,
    })
}

pub fn asset(db: &Connection, id: &str) -> Result<Asset> {
    db.query_row("SELECT id,title,author,platform,video_id,canonical_url,created_at,updated_at,archived_at FROM library_assets WHERE id=?1", [id], asset_row)
        .optional()?.context("ASSET_NOT_FOUND")
}

pub fn list_assets(db: &Connection, archived: bool, offset: u32, limit: u32) -> Result<Vec<Asset>> {
    if limit == 0 || limit > 100 {
        bail!("INVALID_INPUT");
    }
    let mut query = db.prepare("SELECT id,title,author,platform,video_id,canonical_url,created_at,updated_at,archived_at FROM library_assets WHERE (archived_at IS NOT NULL)=?1 ORDER BY created_at DESC,id DESC LIMIT ?2 OFFSET ?3")?;
    let rows = query
        .query_map(params![archived, limit, offset], asset_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

pub fn archive_asset(db: &Connection, id: &str, archived: bool) -> Result<()> {
    require_asset(db, id)?;
    db.execute(
        "UPDATE library_assets SET archived_at=?1,updated_at=?2 WHERE id=?3 AND (archived_at IS NOT NULL)!=?4",
        params![
            archived.then(now).map(sql_integer).transpose()?,
            sql_integer(now())?,
            id,
            archived
        ],
    )?;
    Ok(())
}

#[derive(Debug, Serialize)]
pub struct TaskCollectionState {
    pub task_id: String,
    pub asset_id: String,
    pub collected: bool,
}

/// Resolve a whole task page in one query; unregistered tasks are omitted.
pub fn task_collection_states(db: &Connection, task_ids: &[String]) -> Result<Vec<TaskCollectionState>> {
    if task_ids.len() > 10_000 || task_ids.iter().any(|id| id.is_empty()) {
        bail!("INVALID_INPUT");
    }
    let mut query = db.prepare("SELECT l.task_id,l.asset_id,a.archived_at IS NULL FROM asset_task_links l JOIN library_assets a ON a.id=l.asset_id WHERE l.task_id IN (SELECT value FROM json_each(?1)) ORDER BY l.task_id")?;
    let rows = query.query_map([serde_json::to_string(task_ids)?], |row| Ok(TaskCollectionState {
        task_id: row.get(0)?, asset_id: row.get(1)?, collected: row.get(2)?,
    }))?.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

pub fn set_task_collected(db: &Connection, task_id: &str, collected: bool) -> Result<TaskCollectionState> {
    let asset_id = asset_for_task(db, task_id)?.context("ASSET_NOT_FOUND")?;
    archive_asset(db, &asset_id, !collected)?;
    Ok(TaskCollectionState { task_id: task_id.into(), asset_id, collected })
}

pub fn asset_for_task(db: &Connection, task_id: &str) -> Result<Option<String>> {
    Ok(db
        .query_row(
            "SELECT asset_id FROM asset_task_links WHERE task_id=?1",
            [task_id],
            |r| r.get(0),
        )
        .optional()?)
}

pub fn asset_files(db: &Connection, asset_id: &str) -> Result<Vec<AssetFile>> {
    require_asset(db, asset_id)?;
    let mut query = db.prepare("SELECT id,asset_id,path,size,mtime_ns,duration_ms,width,height FROM asset_files WHERE asset_id=?1 ORDER BY id")?;
    let rows = query.query_map([asset_id], |row| {
        let path = PathBuf::from(row.get::<_, String>(2)?);
        let size = unsigned(row, 3)?;
        let mtime_ns: Option<String> = row.get(4)?;
        let metadata = std::fs::metadata(&path).ok().filter(|m| m.is_file());
        let changed = metadata.as_ref().is_some_and(|m| {
            m.len() != size
                || mtime_ns
                    .as_ref()
                    .is_some_and(|t| modified(m).as_ref() != Some(t))
        });
        Ok(AssetFile {
            id: row.get(0)?,
            asset_id: row.get(1)?,
            path,
            size,
            mtime_ns,
            duration_ms: optional_unsigned(row, 5)?,
            width: optional_unsigned(row, 6)?,
            height: optional_unsigned(row, 7)?,
            missing: metadata.is_none(),
            changed,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn asset_file(db: &Connection, asset_id: &str, file_id: &str) -> Result<AssetFile> {
    asset_files(db, asset_id)?
        .into_iter()
        .find(|file| file.id == file_id)
        .context("FILE_NOT_FOUND")
}

fn modified(metadata: &std::fs::Metadata) -> Option<String> {
    metadata
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|v| v.as_nanos().to_string())
}

fn path_string(path: &Path) -> Result<&str> {
    path.to_str().context("INVALID_INPUT")
}

pub fn relink_file(db: &Connection, file_id: &str, path: &Path) -> Result<()> {
    let path = path.canonicalize().context("FILE_MISSING")?;
    let metadata = std::fs::metadata(&path).context("FILE_MISSING")?;
    if !metadata.is_file() {
        bail!("INVALID_INPUT");
    }
    if db.execute(
        "UPDATE asset_files SET path=?1,size=?2,mtime_ns=?3 WHERE id=?4",
        params![
            path_string(&path)?,
            sql_integer(metadata.len())?,
            modified(&metadata),
            file_id
        ],
    )? == 0
    {
        bail!("FILE_NOT_FOUND");
    }
    // Recognized documents retain their original source fingerprint.
    Ok(())
}

/// Called inside the caller's transaction after a download has been committed.
/// Returns None for previews, failed tasks and image albums.
pub fn register_task(db: &Connection, task: &Task) -> Result<Option<String>> {
    if task.status != Status::Completed
        || !task.files.is_empty()
        || task
            .video
            .as_ref()
            .is_some_and(|v| v["kind"] == "image_album")
    {
        return Ok(None);
    }
    if let Some(id) = asset_for_task(db, &task.id)? {
        retain_cover(db, &id, task)?;
        return Ok(Some(id));
    }
    let video = task.video.as_ref();
    let identity = video
        .and_then(|v| Some((v["platform"].as_str()?, v["video_id"].as_str()?)))
        .filter(|(platform, id)| !platform.is_empty() && !id.is_empty());
    let existing: Option<String> = match identity {
        Some((platform, id)) => db
            .query_row(
                "SELECT id FROM library_assets WHERE platform=?1 AND video_id=?2",
                params![platform, id],
                |r| r.get(0),
            )
            .optional()?,
        None => None,
    };
    let id = existing.unwrap_or_else(|| Uuid::new_v4().to_string());
    db.execute("INSERT OR IGNORE INTO library_assets(id,title,author,platform,video_id,canonical_url,created_at,updated_at,archived_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?8)",
        params![id, video.and_then(|v| v["title"].as_str()).unwrap_or("已下载视频"),
            video.and_then(|v| v["author"].as_str()), identity.map(|v| v.0), identity.map(|v| v.1),
            video.and_then(|v| v["canonical_url"].as_str()), sql_integer(task.completed_at.unwrap_or(task.created_at))?, sql_integer(now())?])?;
    let file_id = if let Some(path) = &task.path {
        let stored_path = path.canonicalize().unwrap_or_else(|_| path.clone());
        let existing: Option<String> = db
            .query_row(
                "SELECT id FROM asset_files WHERE asset_id=?1 AND path=?2",
                params![id, path_string(&stored_path)?],
                |r| r.get(0),
            )
            .optional()?;
        let file_id = existing.unwrap_or_else(|| Uuid::new_v4().to_string());
        let metadata = std::fs::metadata(&stored_path).ok();
        db.execute("INSERT OR IGNORE INTO asset_files(id,asset_id,path,size,mtime_ns,duration_ms,width,height) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
            params![file_id,id,path_string(&stored_path)?,sql_integer(task.bytes)?,metadata.as_ref().and_then(modified),
                video.and_then(|v| v["duration_ms"].as_u64()).map(sql_integer).transpose()?,video.and_then(|v| v["width"].as_u64()).map(sql_integer).transpose()?,video.and_then(|v| v["height"].as_u64()).map(sql_integer).transpose()?])?;
        Some(file_id)
    } else {
        None
    };
    db.execute(
        "INSERT INTO asset_task_links(task_id,asset_id,file_id) VALUES(?1,?2,?3)",
        params![task.id, id, file_id],
    )?;
    retain_cover(db, &id, task)?;
    Ok(Some(id))
}

pub fn retain_cover(db: &Connection, asset_id: &str, task: &Task) -> Result<()> {
    let exists: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM asset_resources WHERE asset_id=?1 AND kind='cover')",
        [asset_id],
        |r| r.get(0),
    )?;
    if exists {
        return Ok(());
    }
    let body = match task
        .video
        .as_ref()
        .filter(|_| task.cover_cached)
        .map(|v| parse_cache::cover(db, v))
    {
        Some(Ok(body)) => Some(body),
        Some(Err(error))
            if matches!(
                error.to_string().as_str(),
                "FILE_MISSING" | "INVALID_MEDIA_CONTENT"
            ) =>
        {
            None
        }
        Some(Err(error)) => return Err(error),
        None => None,
    };
    let body = match (body, &task.cover_path) {
        (Some(body), _) => Some(body),
        (None, Some(path)) => match std::fs::File::open(path) {
            Ok(file) => {
                use std::io::Read;
                let mut body = Vec::new();
                file.take(10 * 1024 * 1024 + 1)
                    .read_to_end(&mut body)
                    .context("IO_ERROR")?;
                Some(body)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error).context("IO_ERROR"),
        },
        _ => None,
    };
    if let Some(body) =
        body.filter(|b| b.len() <= 10 * 1024 * 1024 && crate::files::image_extension(b).is_some())
    {
        db.execute(
            "INSERT OR IGNORE INTO asset_resources(asset_id,kind,body) VALUES(?1,'cover',?2)",
            params![asset_id, body],
        )?;
    }
    Ok(())
}

pub fn asset_cover(db: &Connection, asset_id: &str) -> Result<Vec<u8>> {
    require_asset(db, asset_id)?;
    db.query_row(
        "SELECT body FROM asset_resources WHERE asset_id=?1 AND kind='cover'",
        [asset_id],
        |r| r.get(0),
    )
    .optional()?
    .context("COVER_UNAVAILABLE")
}
