use super::{now, sql_integer};
use crate::desktop_core::transcription::Source;
use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Deserialize)]
pub struct LocalMetadata {
    pub width: u32,
    pub height: u32,
    pub duration_ms: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct ImportedVideo {
    pub asset_id: String,
    pub file_id: String,
    pub already_present: bool,
}

pub(crate) fn relink_local(
    db: &Connection,
    asset_id: &str,
    file_id: &str,
    source: &Source,
    metadata: &LocalMetadata,
) -> Result<ImportedVideo> {
    super::asset_file(db, asset_id, file_id)?;
    if metadata.width == 0
        || metadata.height == 0
        || source.size == 0
        || metadata.duration_ms == Some(0)
    {
        bail!("LOCAL_VIDEO_INVALID");
    }
    if db.query_row(
        "SELECT EXISTS(SELECT 1 FROM asset_files WHERE asset_id=?1 AND id!=?2 AND path=?3)",
        params![
            asset_id,
            file_id,
            source.path.to_str().context("INVALID_INPUT")?
        ],
        |r| r.get::<_, bool>(0),
    )? {
        bail!("FILE_ALREADY_LINKED");
    }
    db.execute("UPDATE asset_files SET path=?1,size=?2,mtime_ns=?3,duration_ms=?4,width=?5,height=?6 WHERE id=?7",params![source.path.to_str().context("INVALID_INPUT")?,sql_integer(source.size)?,source.mtime_ns,metadata.duration_ms.map(sql_integer).transpose()?,metadata.width,metadata.height,file_id])?;
    Ok(ImportedVideo {
        asset_id: asset_id.into(),
        file_id: file_id.into(),
        already_present: true,
    })
}

/// Probe and source-fingerprint verification belong to the Core operation.
pub(crate) fn register_local(
    db: &Connection,
    source: &Source,
    metadata: &LocalMetadata,
) -> Result<ImportedVideo> {
    if metadata.width == 0
        || metadata.height == 0
        || source.size == 0
        || metadata.duration_ms == Some(0)
    {
        bail!("LOCAL_VIDEO_INVALID");
    }
    let existing = {
        let mut query = db.prepare("SELECT id,asset_id,path FROM asset_files")?;
        let rows = query
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    PathBuf::from(r.get::<_, String>(2)?),
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.into_iter().find(|(_, _, path)| {
            *path == source.path || same_file::is_same_file(path, &source.path).unwrap_or(false)
        })
    };
    if let Some((file_id, asset_id, _)) = existing {
        return Ok(ImportedVideo {
            asset_id,
            file_id,
            already_present: true,
        });
    }
    let tx = db.unchecked_transaction()?;
    let asset_id = Uuid::new_v4().to_string();
    let file_id = Uuid::new_v4().to_string();
    let title = source
        .path
        .file_stem()
        .and_then(|s| s.to_str())
        .context("INVALID_INPUT")?;
    tx.execute(
        "INSERT INTO library_assets(id,title,created_at,updated_at) VALUES(?1,?2,?3,?3)",
        params![asset_id, title, sql_integer(now())?],
    )?;
    tx.execute("INSERT INTO asset_files(id,asset_id,path,size,mtime_ns,duration_ms,width,height) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)", params![file_id,asset_id,source.path.to_str().context("INVALID_INPUT")?,sql_integer(source.size)?,source.mtime_ns,metadata.duration_ms.map(sql_integer).transpose()?,metadata.width,metadata.height])?;
    tx.commit()?;
    Ok(ImportedVideo {
        asset_id,
        file_id,
        already_present: false,
    })
}
