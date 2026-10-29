//! Core-owned extraction workers; PythonBridge owns and reaps the actual FFmpeg tree.
use super::{cleanup_staging, transcription::Source, Core, TaskEvent};
use crate::{library, python_bridge::PythonBridge};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;
use tokio::sync::watch;
use uuid::Uuid;

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum KeyframeRequest {
    Position { file_id: String, position_ms: u64 },
    Clip { clip_id: String, count: u32 },
    Range { file_id: String, start_ms: u64, end_ms: u64, #[serde(default = "default_count")] count: u32 },
}
pub(crate) fn default_count() -> u32 { 5 }
#[derive(Clone, Debug, Serialize)]
pub struct KeyframeSnapshot {
    pub id: String,
    pub sequence: u64,
    pub asset_id: String,
    pub file_id: String,
    pub clip_id: Option<String>,
    pub start_ms: u64,
    pub end_ms: Option<u64>,
    pub requested_ms: Vec<u64>,
    pub state: &'static str,
    pub completed: u64,
    pub total: u64,
    pub canceling: bool,
    pub error: Option<String>,
    pub frame_ids: Vec<String>,
}
pub(super) struct KeyframeRun {
    pub snapshot: KeyframeSnapshot,
    pub cancel: watch::Sender<bool>,
    pub done: watch::Receiver<bool>,
}
fn emit(core: &Core, snapshot: KeyframeSnapshot) {
    let asset = snapshot.asset_id.clone();
    (core.emit)(TaskEvent::Keyframes(Box::new(snapshot)));
    (core.emit)(TaskEvent::LibraryChanged(
        super::library_api::LibraryChange {
            scope: super::library_api::LibraryScope::Operations,
            asset_ids: vec![asset],
        },
    ));
}

impl Core {
    pub fn keyframe_operation(&self) -> Option<KeyframeSnapshot> {
        self.inner
            .lock()
            .unwrap()
            .keyframe_run
            .as_ref()
            .map(|run| run.snapshot.clone())
    }
    pub fn start_keyframes(
        self: &Arc<Self>,
        asset: &str,
        request: KeyframeRequest,
    ) -> Result<KeyframeSnapshot> {
        self.start_keyframes_inner(asset, request, false)
    }
    pub(super) fn start_mcp_keyframes(self: &Arc<Self>, asset: &str, request: KeyframeRequest) -> Result<KeyframeSnapshot> {
        self.start_keyframes_inner(asset, request, true)
    }
    fn start_keyframes_inner(self: &Arc<Self>, asset: &str, request: KeyframeRequest, require_mcp_write: bool) -> Result<KeyframeSnapshot> {
        let id = Uuid::new_v4().to_string();
        let mut inner = self.inner.lock().unwrap();
        let (snapshot, source, start, end, points, mut cancellation, done) = {
            if inner.closing {
                bail!("INVALID_STATE");
            }
            if require_mcp_write {
                if !inner.mcp_write_enabled { bail!("WRITE_DISABLED"); }
                crate::mcp_read::require_collected(&inner.db, asset)?;
            }
            if inner
                .keyframe_run
                .as_ref()
                .is_some_and(|run| run.snapshot.state == "running")
            {
                bail!("KEYFRAME_BUSY");
            }
            if inner.dependency_cancel.is_some() {
                bail!("DEPENDENCY_BUSY");
            }
            let (file_id, clip_id, start, end, points) = match request {
                KeyframeRequest::Position {
                    file_id,
                    position_ms,
                } => {
                    if position_ms > 9_007_199_254_740_991 {
                        bail!("KEYFRAME_RANGE_INVALID");
                    }
                    (file_id, None, position_ms, None, vec![position_ms])
                }
                KeyframeRequest::Clip { clip_id, count } => {
                    let (clip, _) = library::clip_playback(&inner.db, asset, &clip_id)?;
                    let points = library::sample_points(clip.start_ms, clip.end_ms, count)?;
                    (
                        clip.file_id,
                        Some(clip_id),
                        clip.start_ms,
                        Some(clip.end_ms),
                        points,
                    )
                }
                KeyframeRequest::Range { file_id, start_ms, end_ms, count } => {
                    if !matches!(count, 3 | 5 | 9) { bail!("KEYFRAME_RANGE_INVALID"); }
                    let points = library::sample_points(start_ms, end_ms, count)?;
                    let file = library::asset_file(&inner.db, asset, &file_id)?;
                    if file.duration_ms.is_some_and(|duration| end_ms > duration) { bail!("KEYFRAME_RANGE_INVALID"); }
                    (file_id, None, start_ms, Some(end_ms), points)
                }
            };
            let file = library::asset_file(&inner.db, asset, &file_id)?;
            let source = Source::read(&file.path)?;
            if file.changed {
                bail!("KEYFRAME_SOURCE_CHANGED");
            }
            let snapshot = KeyframeSnapshot {
                id: id.clone(),
                sequence: 1,
                asset_id: asset.into(),
                file_id,
                clip_id,
                start_ms: start,
                end_ms: end,
                requested_ms: points.clone(),
                state: "running",
                completed: 0,
                total: points.len() as u64,
                canceling: false,
                error: None,
                frame_ids: vec![],
            };
            let (cancel, cancellation) = watch::channel(false);
            let (done, finished) = watch::channel(false);
            inner.keyframe_run = Some(KeyframeRun {
                snapshot: snapshot.clone(),
                cancel,
                done: finished,
            });
            (snapshot, source, start, end, points, cancellation, done)
        };
        let core = self.clone();
        let running = snapshot.clone();
        let worker = tokio::spawn(async move {
            let stage_id = format!("keyframes-{}", running.id);
            let result: Result<Vec<String>> = async {
                let stage = tempfile::Builder::new().prefix(&format!("yingqu-{stage_id}-")).tempdir_in(core.runtime_directory.parent().context("IO_ERROR")?).context("IO_ERROR")?;
                core.inner.lock().unwrap().db.execute("INSERT INTO task_staging VALUES(?1,?2)",rusqlite::params![stage_id,stage.path().to_string_lossy()]).context("DATABASE_ERROR")?;
                let progress_core = core.clone(); let operation_id=running.id.clone();
                let outcome = PythonBridge::operation_cancellable(&core.program,&core.args,&running.id,"extract_keyframes",json!({"source":source.path,"staging_dir":stage.path(),"runtime_directory":core.runtime_directory,"positions_ms":points,"end_ms":end}),move |progress| {
                    let snapshot = {let mut inner=progress_core.inner.lock().unwrap();
                        inner.keyframe_run.as_mut().filter(|run|run.snapshot.id==operation_id).map(|run| {run.snapshot.completed=progress["completed"].as_u64().unwrap_or(0);run.snapshot.sequence+=1;run.snapshot.clone()})};
                    if let Some(snapshot)=snapshot {(progress_core.emit)(TaskEvent::Keyframes(Box::new(snapshot)));}
                },async {let _=cancellation.wait_for(|cancel|*cancel).await;}).await;
                let batch = library::FrameBatch {asset_id:running.asset_id.clone(),file_id:running.file_id.clone(),clip_id:running.clip_id.clone(),id:running.id.clone(),source:source.clone(),start_ms:start,end_ms:end,points};
                let prepared = outcome.and_then(|result|batch.read(stage.path(),&result));
                // Reap, read bounded artifacts and remove staging before the atomic commit.
                cleanup_staging(&core.inner.lock().unwrap().db,&stage_id,stage.path())?;
                let frames = prepared?;
                {
                    let inner=core.inner.lock().unwrap();
                    if inner.closing || *cancellation.borrow() {bail!("CANCELED");}
                    let current=library::asset_file(&inner.db,&running.asset_id,&running.file_id)?;
                    if Source::read(&current.path).ok().as_ref()!=Some(&source) || Source::read(&source.path).ok().as_ref()!=Some(&source) {bail!("KEYFRAME_SOURCE_CHANGED");}
                    library::store_frames(&inner.db,&frames).map_err(super::library_api::database_error)
                }
            }.await;
            let snapshot = {
                let mut inner = core.inner.lock().unwrap();
                let run = inner.keyframe_run.as_mut().expect("owned keyframe run");
                run.snapshot.sequence += 1;
                run.snapshot.canceling = false;
                match result {
                    Ok(ids) => {
                        run.snapshot.state = "completed";
                        run.snapshot.frame_ids = ids;
                    }
                    Err(error) => {
                        let code = error.to_string();
                        run.snapshot.state = if code == "CANCELED" {
                            "canceled"
                        } else {
                            "failed"
                        };
                        run.snapshot.error = Some(code);
                    }
                }
                run.snapshot.clone()
            };
            if snapshot.state == "completed" {
                (core.emit)(TaskEvent::LibraryChanged(
                    super::library_api::LibraryChange {
                        scope: super::library_api::LibraryScope::Keyframes,
                        asset_ids: vec![snapshot.asset_id.clone()],
                    },
                ));
            }
            emit(&core, snapshot);
            let _ = done.send(true);
        });
        inner.workers.retain(|w| !w.is_finished());
        inner.workers.push(worker);
        drop(inner);
        emit(self, snapshot.clone());
        Ok(snapshot)
    }
    pub async fn cancel_keyframes(&self, id: &str) -> Result<()> {
        let (mut done, snapshot) = {
            let mut inner = self.inner.lock().unwrap();
            let run = inner
                .keyframe_run
                .as_mut()
                .filter(|run| run.snapshot.id == id)
                .context("KEYFRAME_OPERATION_NOT_FOUND")?;
            if run.snapshot.state != "running" {
                return Ok(());
            }
            run.snapshot.canceling = true;
            run.snapshot.sequence += 1;
            let _ = run.cancel.send(true);
            (run.done.clone(), run.snapshot.clone())
        };
        emit(self, snapshot);
        let _ = done.wait_for(|done| *done).await;
        Ok(())
    }
}

#[cfg(test)]
#[path = "keyframe_tests.rs"]
mod tests;
