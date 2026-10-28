use super::{
    library_api::{LibraryChange, LibraryScope},
    Core, Status, TaskEvent,
};
use crate::library::{self, WorkbenchLibrary};
use anyhow::{Context, Result};
use serde::Serialize;
use std::collections::HashMap;

#[derive(Clone, Serialize)]
pub struct WorkbenchOperation {
    pub id: String,
    pub kind: &'static str,
    pub title: String,
    pub state: String,
    pub phase: Option<String>,
    pub error: Option<String>,
    pub asset_id: Option<String>,
    pub file_id: Option<String>,
    pub task_id: Option<String>,
}

#[derive(Serialize)]
pub struct Workbench {
    #[serde(flatten)]
    pub library: WorkbenchLibrary,
    pub operations: Vec<WorkbenchOperation>,
}

impl Core {
    pub fn record_library_visit(&self, id: &str) -> Result<()> {
        {
            let inner = self.inner.lock().unwrap();
            if inner.closing {
                anyhow::bail!("INVALID_STATE");
            }
            library::record_visit(&inner.db, id).map_err(super::library_api::database_error)?;
        }
        (self.emit)(TaskEvent::LibraryChanged(LibraryChange {
            scope: LibraryScope::Visits,
            asset_ids: vec![id.into()],
        }));
        Ok(())
    }

    pub fn workbench(&self) -> Result<Workbench> {
        let inner = self.inner.lock().unwrap();
        let db = &inner.db;
        let library = library::workbench_library(db).context("DATABASE_ERROR")?;
        let mut operations = Vec::new();
        // Summary snapshots only; no document text, source paths or media URLs leave this endpoint.
        let mut query = db.prepare("SELECT file_id,snapshot FROM asset_transcriptions WHERE json_extract(snapshot,'$.state') IN ('running','failed','interrupted')").context("DATABASE_ERROR")?;
        let mut snapshots = query
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
            .map(|row| {
                let (id, json) = row?;
                Ok((
                    id,
                    serde_json::from_str::<super::transcription::Snapshot>(&json)?,
                ))
            })
            .collect::<Result<HashMap<_, _>>>()
            .context("DATABASE_ERROR")?;
        for (id, snapshot) in &inner.transcription_unsaved {
            snapshots.insert(id.clone(), snapshot.summary());
        }
        if let Some(active) = &inner.transcription {
            if let Some(id) = &active.snapshot.file_id {
                snapshots.insert(id.clone(), active.snapshot.summary());
            }
        }
        for (file_id, snapshot) in snapshots {
            if !matches!(
                snapshot.state.as_str(),
                "running" | "failed" | "interrupted"
            ) {
                continue;
            }
            let owner = db.query_row("SELECT a.id,a.title FROM asset_files f JOIN library_assets a ON a.id=f.asset_id WHERE f.id=?1", [&file_id], |r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?))).context("DATABASE_ERROR")?;
            operations.push(WorkbenchOperation {
                id: snapshot.operation_id,
                kind: "transcription",
                title: owner.1,
                state: snapshot.state,
                phase: snapshot.phase,
                error: snapshot.error_code,
                asset_id: Some(owner.0),
                file_id: Some(file_id),
                task_id: None,
            });
        }
        operations.sort_by(|a, b| a.id.cmp(&b.id));
        if let Some(active) = &inner.video_import {
            let mut operation = active.operation.clone();
            if *active.cancel.borrow() {
                operation.phase = Some("canceling".into());
            }
            operations.push(operation);
        } else if let Some(failed) = &inner.last_import_failure {
            operations.push(failed.clone());
        }
        if let Some(active) = &inner.library_transfer {
            if matches!(active.snapshot.state, "running" | "failed") {
                let backup =
                    active.snapshot.kind == super::library_transfer_operation::TransferKind::Backup;
                operations.push(WorkbenchOperation {
                    id: active.snapshot.id.clone(),
                    kind: if backup { "backup" } else { "restore" },
                    title: if backup {
                        "备份知识库"
                    } else {
                        "恢复知识库"
                    }
                    .into(),
                    state: active.snapshot.state.into(),
                    phase: Some(active.snapshot.phase.into()),
                    error: active.snapshot.error.clone(),
                    asset_id: None,
                    file_id: None,
                    task_id: None,
                });
            }
        }
        if let Some(run)=&inner.keyframe_run { if matches!(run.snapshot.state,"running"|"failed") {
            operations.push(WorkbenchOperation {id:run.snapshot.id.clone(),kind:"keyframes",title:"提取关键帧".into(),state:run.snapshot.state.into(),phase:Some(if run.snapshot.canceling {"canceling"} else {"extracting_frames"}.into()),error:run.snapshot.error.clone(),asset_id:Some(run.snapshot.asset_id.clone()),file_id:Some(run.snapshot.file_id.clone()),task_id:None});
        }}
        for task in &inner.tasks {
            if !matches!(
                task.status,
                Status::Parsing | Status::Queued | Status::Downloading | Status::Failed
            ) && !(task.status == Status::Completed && task.error.is_some())
            {
                continue;
            }
            operations.push(WorkbenchOperation {
                id: task.id.clone(),
                kind: "download",
                title: task
                    .video
                    .as_ref()
                    .and_then(|v| v.get("title"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("视频解析")
                    .into(),
                state: if task.status == Status::Failed || task.error.is_some() {
                    "failed"
                } else {
                    "running"
                }
                .into(),
                phase: Some(
                    if task.canceling {
                        "canceling"
                    } else if task.merging {
                        "merging"
                    } else {
                        match task.status {
                            Status::Parsing => "parsing",
                            Status::Queued => "queued",
                            _ => "downloading",
                        }
                    }
                    .into(),
                ),
                error: task.error.clone(),
                asset_id: None,
                file_id: None,
                task_id: Some(task.id.clone()),
            });
        }
        Ok(Workbench {
            library,
            operations,
        })
    }
}
