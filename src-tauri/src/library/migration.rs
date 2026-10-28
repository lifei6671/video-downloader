use crate::desktop_core::{transcription::Snapshot, Task};
use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension};

/// Also reconciles a committed download whose library registration failed.
/// Never mutates task snapshots, settings, or media files.
pub fn backfill(db: &mut Connection, tasks: &[Task]) -> Result<()> {
    let tx = db.transaction().context("DATABASE_ERROR")?;
    for task in tasks {
        preserve_task(&tx, task)?;
    }
    tx.commit().context("DATABASE_ERROR")?;
    Ok(())
}

pub fn preserve_task(db: &Connection, task: &Task) -> Result<()> {
    let Some(asset_id) = super::register_task(db, task)? else {
        return Ok(());
    };
    let snapshot: Option<String> = db
        .query_row(
            "SELECT snapshot FROM video_transcripts WHERE task_id=?1",
            [&task.id],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(snapshot) = snapshot {
        let mut snapshot: Snapshot = serde_json::from_str(&snapshot).context("DATABASE_ERROR")?;
        let file_id: Option<String> = db.query_row(
            "SELECT file_id FROM asset_task_links WHERE task_id=?1",
            [&task.id],
            |r| r.get(0),
        )?;
        if let Some(result) = &snapshot.result {
            super::save_recognized(db, &asset_id, file_id.as_deref(), result)?;
        }
        if let Some(file_id) = file_id {
            snapshot.task_id.clear();
            snapshot.asset_id = Some(asset_id);
            snapshot.file_id = Some(file_id.clone());
            snapshot.has_result = super::latest_recognition(db, &file_id)?.is_some();
            snapshot.result = None;
            snapshot.source_changed = false;
            if snapshot.state == "running" {
                snapshot.state = "interrupted".into();
                snapshot.phase = None;
                snapshot.completed_segments = None;
                snapshot.total_segments = None;
                snapshot.error_code = Some("TRANSCRIPTION_INTERRUPTED".into());
                snapshot.sequence += 1;
            }
            db.execute("INSERT INTO asset_transcriptions(file_id,snapshot) VALUES(?1,?2) ON CONFLICT(file_id) DO NOTHING", rusqlite::params![file_id,serde_json::to_string(&snapshot)?])?;
        }
        db.execute("DELETE FROM video_transcripts WHERE task_id=?1", [&task.id])?;
    }
    Ok(())
}
