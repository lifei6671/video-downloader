//! Persistent learning material. Download history is only one source of assets.
mod assets;
mod analysis;
mod comparison;
pub(crate) mod topics;
pub(crate) mod ideas;
mod ai_source;
pub use ai_source::AiSource;
pub use ideas::{LibraryIdea,IdeaSummary,IdeaDetail,IdeaInput,IdeaReference,IdeaReferenceKind,IdeaReferenceInput,IdeaReferenceView,WorkbenchIdeas,idea_summaries,library_idea,prepare_idea_reference,create_idea,edit_idea};
pub use topics::{ResearchTopic,ResearchTopicDetail,TopicMember,TopicMemberKind,TopicMemberInput,TopicInput,research_topics,research_topic,create_research_topic,edit_research_topic,add_topic_member,remove_topic_member,reorder_topic_members};
pub use comparison::{ComparisonPin, ComparisonSource, NoteComparison, NewComparison, ComparisonEdit, comparison_source, create_comparison, edit_comparison};
pub use analysis::{AnalysisInput, AnalysisContent, NoteAnalysis, NewAnalysis, Observation, ObservationInput, ObservationKind, ObservationEvidence, create_analysis, edit_analysis};
mod backup;
mod clips;
mod keyframes;
pub use keyframes::{
    delete_keyframe, keyframe_image, keyframes, sample_points, validate_frame_image, Keyframe,
    KeyframeData, MAX_FRAME_BYTES,
};
pub(crate) use keyframes::{store_frames, FrameBatch};
mod content_export;
mod documents;
mod local_import;
mod migration;
mod organization;
mod search;
mod timing;
mod transfer;
mod views;
mod workbench;
pub use clips::{clip_playback, clips, create_clip, delete_clip, Clip, ClipInput, ClipRange};
pub use timing::{TranscriptSegment, TranscriptTiming};

pub use assets::*;
pub use backup::*;
pub use content_export::*;
pub use documents::*;
pub(crate) use local_import::register_local;
pub(crate) use local_import::relink_local;
pub use local_import::*;
pub use migration::*;
pub use organization::*;
pub use search::*;
pub use transfer::{TransferCallback, TransferProgress};
pub use views::*;
pub use workbench::*;

use anyhow::{bail, Context, Result};
use rusqlite::{Connection, OptionalExtension};
use std::path::{Path, PathBuf};
use uuid::Uuid;

const VERSION: i64 = 11;

/// Initialize before any legacy schema or recovery changes so the backup is a
/// consistent copy of the entire pre-upgrade database, including its WAL.
pub fn initialize(db: &mut Connection, db_path: &Path) -> Result<Option<PathBuf>> {
    db.execute_batch("PRAGMA foreign_keys=ON;")
        .context("DATABASE_ERROR")?;
    let exists: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='library_schema')",
        [],
        |row| row.get(0),
    )?;
    if exists {
        let version: Option<i64> = db
            .query_row("SELECT version FROM library_schema WHERE id=1", [], |r| {
                r.get(0)
            })
            .optional()?;
        if matches!(version, Some(1..=10)) {
            let backup = backup_database(db, db_path)?;
            let tx = db.transaction().context("DATABASE_ERROR")?;
            if version == Some(1) {
                tx.execute_batch(include_str!("schema-v2.sql"))?;
            }
            if version.is_some_and(|v| v < 3) {
                tx.execute_batch(include_str!("schema-v3.sql"))?;
            }
            if version.is_some_and(|v| v < 4) {
                tx.execute_batch(include_str!("schema-v4.sql"))?;
            }
            if version.is_some_and(|v| v < 5) {
                tx.execute_batch(include_str!("schema-v5.sql"))?;
            }
            if version.is_some_and(|v| v < 6) { tx.execute_batch(include_str!("schema-v6.sql"))?; }
            if version.is_some_and(|v| v < 7) { tx.execute_batch(include_str!("schema-v7.sql"))?; }
            if version.is_some_and(|v| v < 8) { tx.execute_batch(include_str!("schema-v8.sql"))?; }
            if version.is_some_and(|v| v < 9) { tx.execute_batch(include_str!("schema-v9.sql"))?; }
            if version.is_some_and(|v| v < 10) { tx.execute_batch(include_str!("schema-v10.sql"))?; }
            tx.execute_batch(include_str!("schema-v11.sql"))?;
            tx.commit()?;
            return Ok(Some(backup));
        }
        if version != Some(VERSION) {
            bail!("LIBRARY_SCHEMA_UNSUPPORTED");
        }
        return Ok(None);
    }
    let has_tables: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%')",
        [], |row| row.get(0),
    )?;
    let backup = if has_tables {
        Some(backup_database(db, db_path)?)
    } else {
        None
    };
    let tx = db.transaction().context("DATABASE_ERROR")?;
    tx.execute_batch(include_str!("schema.sql"))
        .context("DATABASE_ERROR")?;
    tx.execute_batch(include_str!("schema-v2.sql"))?;
    tx.execute_batch(include_str!("schema-v3.sql"))?;
    tx.execute_batch(include_str!("schema-v4.sql"))?;
    tx.execute_batch(include_str!("schema-v5.sql"))?;
    tx.execute_batch(include_str!("schema-v6.sql"))?;
    tx.execute_batch(include_str!("schema-v7.sql"))?;
    tx.execute_batch(include_str!("schema-v8.sql"))?;
    tx.execute_batch(include_str!("schema-v9.sql"))?;
    tx.execute_batch(include_str!("schema-v10.sql"))?;
    tx.execute_batch(include_str!("schema-v11.sql"))?;
    tx.commit().context("DATABASE_ERROR")?;
    Ok(backup)
}

fn backup_database(db: &Connection, db_path: &Path) -> Result<PathBuf> {
    let path = db_path.with_extension(format!("pre-library-v11-{}.sqlite3", Uuid::new_v4()));
    db.execute("VACUUM main INTO ?1", [path.to_str().context("IO_ERROR")?])
        .context("LIBRARY_BACKUP_FAILED")?;
    Ok(path)
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock before epoch")
        .as_secs()
}

fn sql_integer(value: u64) -> Result<i64> {
    i64::try_from(value).context("INVALID_INPUT")
}

fn unsigned(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<u64> {
    let value: i64 = row.get(index)?;
    u64::try_from(value).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(index, value))
}

fn optional_unsigned(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<Option<u64>> {
    row.get::<_, Option<i64>>(index)?
        .map(|value| {
            u64::try_from(value).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(index, value))
        })
        .transpose()
}

fn name(value: &str) -> Result<&str> {
    let value = value.trim();
    if value.is_empty() || value.chars().count() > 128 {
        bail!("INVALID_INPUT");
    }
    Ok(value)
}

fn require_asset(db: &Connection, id: &str) -> Result<()> {
    if !db.query_row(
        "SELECT EXISTS(SELECT 1 FROM library_assets WHERE id=?1)",
        [id],
        |r| r.get::<_, bool>(0),
    )? {
        bail!("ASSET_NOT_FOUND");
    }
    Ok(())
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod timing_tests;

#[cfg(test)]
mod clip_tests;

#[cfg(test)]
mod keyframe_tests;

#[cfg(test)]
mod analysis_tests;

#[cfg(test)]
mod comparison_tests;

#[cfg(test)]
mod topic_tests;

#[cfg(test)]
mod idea_tests;
