use super::{asset_file, name, now, require_asset, sql_integer, unsigned};
use crate::desktop_core::transcription::Source;
use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClipRange {
    pub start_ms: u64,
    pub end_ms: u64,
}
impl ClipRange {
    fn validate(&self) -> Result<()> {
        if self.start_ms >= self.end_ms {
            bail!("CLIP_RANGE_INVALID");
        }
        sql_integer(self.end_ms)?;
        Ok(())
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClipInput {
    pub file_id: String,
    pub title: String,
    pub reason: String,
    pub start_ms: u64,
    pub end_ms: u64,
    /// Client-specific HTML media range, measured after a real seek. Rechecked on every play.
    pub playable: ClipRange,
    pub document_id: Option<String>,
    pub quote: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Clip {
    pub id: String,
    pub asset_id: String,
    pub file_id: String,
    pub title: String,
    pub reason: String,
    pub start_ms: u64,
    pub end_ms: u64,
    pub document_id: Option<String>,
    pub document_version: Option<u64>,
    pub quote: Option<String>,
    pub created_at: u64,
    pub missing: bool,
    pub source_changed: bool,
}
pub(super) struct ClipRecord {
    pub clip: Clip,
    pub source: Source,
}

fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<(Clip, String)> {
    Ok((
        Clip {
            id: r.get(0)?,
            asset_id: r.get(1)?,
            file_id: r.get(2)?,
            title: r.get(3)?,
            reason: r.get(4)?,
            start_ms: unsigned(r, 5)?,
            end_ms: unsigned(r, 6)?,
            document_id: r.get(8)?,
            document_version: super::optional_unsigned(r, 9)?,
            quote: r.get(10)?,
            created_at: unsigned(r, 11)?,
            missing: false,
            source_changed: false,
        },
        r.get(7)?,
    ))
}
pub(super) fn records(db: &Connection, asset: &str) -> Result<Vec<ClipRecord>> {
    require_asset(db, asset)?;
    let mut query=db.prepare("SELECT id,asset_id,file_id,title,reason,start_ms,end_ms,source_json,document_id,document_version,quote,created_at FROM asset_clips WHERE asset_id=?1 ORDER BY created_at DESC,id DESC")?;
    let rows = query
        .query_map([asset], row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    rows.into_iter()
        .map(|(mut clip, source)| {
            let source: Source = serde_json::from_str(&source)?;
            let file = asset_file(db, asset, &clip.file_id)?;
            let current = Source::read(&file.path).ok();
            clip.missing = current.is_none();
            clip.source_changed = current.as_ref() != Some(&source);
            Ok(ClipRecord { clip, source })
        })
        .collect()
}
pub fn clips(db: &Connection, asset: &str) -> Result<Vec<Clip>> {
    Ok(records(db, asset)?.into_iter().map(|r| r.clip).collect())
}

pub fn create_clip(db: &Connection, asset: &str, input: &ClipInput) -> Result<String> {
    let title = name(&input.title)?;
    if input.reason.len() > 16 * 1024 {
        bail!("CLIP_REASON_TOO_LONG");
    }
    let range = ClipRange {
        start_ms: input.start_ms,
        end_ms: input.end_ms,
    };
    range.validate()?;
    input.playable.validate()?;
    if range.start_ms < input.playable.start_ms || range.end_ms > input.playable.end_ms {
        bail!("CLIP_RANGE_UNAVAILABLE");
    }
    let file = asset_file(db, asset, &input.file_id)?;
    if file.changed {
        bail!("CLIP_SOURCE_CHANGED");
    }
    let source = Source::read(&file.path)?;
    let version = match (&input.document_id, &input.quote) {
        (Some(id), Some(quote)) if !quote.trim().is_empty() => {
            let document = super::read_document(db, asset, id)?;
            if !document.text.contains(quote) {
                bail!("INVALID_CITATION");
            }
            let version = document.summary.version;
            let mut origin = document;
            while origin.source.is_none() {
                let Some(id) = origin.summary.source_document_id.as_deref() else {
                    break;
                };
                origin = super::read_document(db, asset, id)?;
            }
            if let Some(original) = &origin.source {
                if origin.summary.file_id.as_deref() != Some(&input.file_id) || original != &source
                {
                    bail!("CLIP_SOURCE_MISMATCH");
                }
            }
            Some(version)
        }
        (None, None) => None,
        _ => bail!("INVALID_CITATION"),
    };
    let id = Uuid::new_v4().to_string();
    db.execute(
        "INSERT INTO asset_clips VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
        params![
            id,
            asset,
            input.file_id,
            title,
            input.reason,
            sql_integer(input.start_ms)?,
            sql_integer(input.end_ms)?,
            serde_json::to_string(&source)?,
            input.document_id,
            version.map(sql_integer).transpose()?,
            input.quote,
            sql_integer(now())?
        ],
    )?;
    Ok(id)
}

pub fn delete_clip(db: &Connection, asset: &str, id: &str) -> Result<()> {
    require_asset(db, asset)?;
    if db.execute(
        "DELETE FROM asset_clips WHERE asset_id=?1 AND id=?2",
        params![asset, id],
    )? != 1
    {
        bail!("CLIP_NOT_FOUND");
    }
    Ok(())
}

pub fn clip_playback(db: &Connection, asset: &str, id: &str) -> Result<(Clip, PathBuf)> {
    let record = records(db, asset)?
        .into_iter()
        .find(|r| r.clip.id == id)
        .context("CLIP_NOT_FOUND")?;
    if record.clip.missing {
        bail!("FILE_MISSING");
    }
    if record.clip.source_changed {
        bail!("CLIP_SOURCE_CHANGED");
    }
    let file = asset_file(db, asset, &record.clip.file_id)?;
    Ok((record.clip, file.path))
}

pub(super) fn validate_reference(db: &Connection, clip: &Clip) -> Result<()> {
    name(&clip.title)?;
    if clip.reason.len() > 16 * 1024 {
        bail!("CLIP_REASON_TOO_LONG");
    }
    ClipRange {
        start_ms: clip.start_ms,
        end_ms: clip.end_ms,
    }
    .validate()?;
    let owner: String = db.query_row(
        "SELECT asset_id FROM asset_files WHERE id=?1",
        [&clip.file_id],
        |r| r.get(0),
    )?;
    if owner != clip.asset_id {
        bail!("FILE_NOT_FOUND");
    }
    match (&clip.document_id, clip.document_version, &clip.quote) {
        (None, None, None) => {}
        (Some(id), Some(version), Some(quote)) if !quote.trim().is_empty() => {
            let row: Option<(String, u64, String)> = db
                .query_row(
                    "SELECT asset_id,version,text FROM asset_documents WHERE id=?1",
                    [id],
                    |r| Ok((r.get(0)?, unsigned(r, 1)?, r.get(2)?)),
                )
                .optional()?;
            if !row.is_some_and(|(asset, v, text)| {
                asset == clip.asset_id && v == version && text.contains(quote)
            }) {
                bail!("INVALID_CITATION");
            }
            let mut document = super::read_document(db, &clip.asset_id, id)?;
            while document.summary.file_id.is_none() {
                let Some(parent) = document.summary.source_document_id.as_deref() else {
                    break;
                };
                document = super::read_document(db, &clip.asset_id, parent)?;
            }
            if document
                .summary
                .file_id
                .as_ref()
                .is_some_and(|file| file != &clip.file_id)
            {
                bail!("CLIP_SOURCE_MISMATCH");
            }
        }
        _ => bail!("INVALID_CITATION"),
    }
    Ok(())
}
