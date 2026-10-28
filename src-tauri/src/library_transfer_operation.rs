use super::{Core, Status, TaskEvent};
use crate::library::{self, TransferProgress};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::watch;
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TransferKind {
    Backup,
    Restore,
}
#[derive(Clone, Debug, Serialize)]
pub struct TransferSnapshot {
    pub id: String,
    pub sequence: u64,
    pub order: u64,
    pub kind: TransferKind,
    pub include_media: bool,
    pub state: &'static str,
    pub phase: &'static str,
    pub completed_files: usize,
    pub total_files: usize,
    pub bytes: u64,
    pub canceling: bool,
    pub path: String,
    pub error: Option<String>,
    pub result: Option<serde_json::Value>,
}
pub(super) struct LibraryTransfer {
    pub snapshot: TransferSnapshot,
    pub cancel: watch::Sender<bool>,
}
#[derive(Debug, Serialize)]
pub struct BackupInfo {
    pub ideas: u64,
    pub topics: u64,
    pub assets: u64,
    pub documents: u64,
    pub notes: u64,
    pub files: u64,
    pub available_files: u64,
    pub media_bytes: u64,
    pub empty: bool,
    pub busy: bool,
}

impl Core {
    pub fn library_transfer(&self) -> Option<TransferSnapshot> {
        self.inner
            .lock()
            .unwrap()
            .library_transfer
            .as_ref()
            .map(|a| a.snapshot.clone())
    }
    pub fn library_backup_info(&self) -> Result<BackupInfo> {
        let inner = self.inner.lock().unwrap();
        let db = &inner.db;
        let count = |table: &str| -> Result<u64> {
            let value: i64 = db
                .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
                .context("DATABASE_ERROR")?;
            Ok(value as u64)
        };
        let mut query = db
            .prepare("SELECT path FROM asset_files")
            .context("DATABASE_ERROR")?;
        let mut available_files = 0;
        let mut media_bytes = 0;
        for path in query
            .query_map([], |r| r.get::<_, String>(0))
            .context("DATABASE_ERROR")?
        {
            if let Ok(metadata) = std::fs::metadata(path.context("DATABASE_ERROR")?) {
                if metadata.is_file() {
                    available_files += 1;
                    media_bytes += metadata.len();
                }
            }
        }
        let empty = match library::require_empty_library(db) {
            Ok(()) => true,
            Err(error) if error.to_string() == "LIBRARY_NOT_EMPTY" => false,
            Err(error) => return Err(error).context("DATABASE_ERROR"),
        };
        Ok(BackupInfo {
            ideas: count("library_ideas")?,
            topics: count("research_topics")?,
            assets: count("library_assets")?,
            documents: count("asset_documents")?,
            notes: count("asset_notes")?,
            files: count("asset_files")?,
            available_files,
            media_bytes,
            empty,
            busy: inner.closing
                || inner.keyframe_run.as_ref().is_some_and(|run|run.snapshot.state=="running")
                || inner.video_import.is_some()
                || inner.transcription.is_some()
                || inner.tasks.iter().any(|t| {
                    matches!(
                        t.status,
                        Status::Parsing | Status::Queued | Status::Downloading
                    )
                }),
        })
    }
    pub fn start_library_backup(
        self: &Arc<Self>,
        target: PathBuf,
        include_media: bool,
    ) -> Result<TransferSnapshot> {
        self.start_library_transfer(TransferKind::Backup, None, target, include_media)
    }
    pub fn start_library_restore(
        self: &Arc<Self>,
        package: PathBuf,
        target: PathBuf,
    ) -> Result<TransferSnapshot> {
        let info = self.library_backup_info()?;
        if !info.empty {
            bail!("LIBRARY_NOT_EMPTY");
        }
        if info.busy {
            bail!("LIBRARY_BUSY");
        }
        self.start_library_transfer(TransferKind::Restore, Some(package), target, false)
    }
    fn start_library_transfer(
        self: &Arc<Self>,
        kind: TransferKind,
        package: Option<PathBuf>,
        target: PathBuf,
        include_media: bool,
    ) -> Result<TransferSnapshot> {
        let mut snapshot = TransferSnapshot {
            id: Uuid::new_v4().to_string(),
            sequence: 0,
            order: 0,
            kind,
            include_media,
            state: "running",
            phase: "preparing",
            completed_files: 0,
            total_files: 0,
            bytes: 0,
            canceling: false,
            path: target.to_string_lossy().into_owned(),
            error: None,
            result: None,
        };
        {
            let mut inner = self.inner.lock().unwrap();
            if inner.closing {
                bail!("INVALID_STATE");
            }
            if inner
                .library_transfer
                .as_ref()
                .is_some_and(|a| a.snapshot.state == "running")
            {
                bail!("LIBRARY_TRANSFER_BUSY");
            }
            snapshot.order = inner
                .library_transfer
                .as_ref()
                .map_or(1, |a| a.snapshot.order + 1);
            let (cancel, cancellation) = watch::channel(false);
            inner.library_transfer = Some(LibraryTransfer {
                snapshot: snapshot.clone(),
                cancel,
            });
            let core = self.clone();
            let worker_snapshot = snapshot.clone();
            inner.workers.retain(|worker| !worker.is_finished());
            inner.workers.push(tokio::spawn(async move {
                let work = core.clone();
                let id = worker_snapshot.id.clone();
                let outcome = tokio::task::spawn_blocking(move || -> Result<serde_json::Value> {
                    let mut last = Instant::now() - Duration::from_secs(1);
                    let mut notify = |progress: TransferProgress| -> Result<()> {
                        let update = {
                            let mut inner = work.inner.lock().unwrap();
                            if *cancellation.borrow() {
                                bail!("CANCELED");
                            }
                            let active =
                                inner.library_transfer.as_mut().context("INVALID_STATE")?;
                            let send = active.snapshot.phase != progress.phase
                                || active.snapshot.completed_files != progress.completed_files
                                || last.elapsed() >= Duration::from_millis(100);
                            active.snapshot.sequence += 1;
                            active.snapshot.phase = progress.phase;
                            active.snapshot.completed_files = progress.completed_files;
                            active.snapshot.total_files = progress.total_files;
                            active.snapshot.bytes = progress.bytes;
                            if send {
                                last = Instant::now();
                                Some(active.snapshot.clone())
                            } else {
                                None
                            }
                        };
                        if let Some(update) = update {
                            (work.emit)(TaskEvent::LibraryTransfer(update));
                        }
                        Ok(())
                    };
                    let result = match kind {
                        TransferKind::Backup => serde_json::to_value(work.backup_library(
                            &target,
                            include_media,
                            &mut notify,
                        )?),
                        TransferKind::Restore => serde_json::to_value(work.restore_library(
                            package.as_deref().context("INVALID_INPUT")?,
                            &target,
                            &mut notify,
                        )?),
                    };
                    result.context("IO_ERROR")
                })
                .await
                .unwrap_or_else(|_| Err(anyhow::anyhow!("IO_ERROR")));
                let update = {
                    let mut inner = core.inner.lock().unwrap();
                    let closing = inner.closing;
                    let active = inner.library_transfer.as_mut().unwrap();
                    debug_assert_eq!(active.snapshot.id, id);
                    active.snapshot.sequence += 1;
                    active.snapshot.canceling = false;
                    match outcome {
                        Ok(result) => {
                            active.snapshot.state = "completed";
                            active.snapshot.result = Some(result);
                        }
                        Err(error) => {
                            let code = error.to_string();
                            if code == "CANCELED" || (closing && code == "INVALID_STATE") {
                                active.snapshot.state = "canceled";
                            } else {
                                active.snapshot.state = "failed";
                                active.snapshot.error = Some(code);
                            }
                        }
                    }
                    active.snapshot.clone()
                };
                (core.emit)(TaskEvent::LibraryTransfer(update));
            }));
        }
        (self.emit)(TaskEvent::LibraryTransfer(snapshot.clone()));
        Ok(snapshot)
    }
    pub fn cancel_library_transfer(&self, id: &str) -> Result<()> {
        let update = {
            let mut inner = self.inner.lock().unwrap();
            let active = inner
                .library_transfer
                .as_mut()
                .filter(|a| a.snapshot.id == id)
                .context("LIBRARY_TRANSFER_NOT_FOUND")?;
            if active.snapshot.state != "running" {
                return Ok(());
            }
            if active.snapshot.phase == "committing" {
                bail!("LIBRARY_TRANSFER_COMMITTING");
            }
            let _ = active.cancel.send(true);
            active.snapshot.canceling = true;
            active.snapshot.sequence += 1;
            active.snapshot.clone()
        };
        (self.emit)(TaskEvent::LibraryTransfer(update));
        Ok(())
    }
}

#[cfg(test)]
#[path = "library_transfer_operation_tests.rs"]
mod tests;
