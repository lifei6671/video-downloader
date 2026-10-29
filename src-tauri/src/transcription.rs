use super::{cleanup_staging, now, Core, TaskEvent};
use crate::python_bridge::PythonBridge;
use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
    time::UNIX_EPOCH,
};
use tokio::sync::watch;
use uuid::Uuid;

const MAX_TEXT: u64 = 16 * 1024 * 1024;
const MODEL: &str = "46e849502a867080d66d351b8dfb1018b607e509";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Source {
    pub path: PathBuf,
    pub size: u64,
    pub mtime_ns: String,
}
impl Source {
    pub(crate) fn read(path: &Path) -> Result<Self> {
        let path = path.canonicalize().context("FILE_MISSING")?;
        let metadata = path.metadata().context("FILE_MISSING")?;
        if !metadata.is_file() {
            bail!("FILE_MISSING");
        }
        Ok(Self {
            path,
            size: metadata.len(),
            mtime_ns: metadata
                .modified()?
                .duration_since(UNIX_EPOCH)?
                .as_nanos()
                .to_string(),
        })
    }
}
#[derive(Clone, Serialize, Deserialize)]
pub struct TranscriptResult {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timing: Option<crate::library::TranscriptTiming>,
    pub text: String,
    pub source: Source,
    pub model_revision: String,
    #[serde(default)]
    pub model_name: Option<String>,
    pub pipeline_revision: String,
    #[serde(default)]
    pub backend: Option<String>,
    pub completed_at: u64,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub task_id: String,
    #[serde(default)]
    pub asset_id: Option<String>,
    #[serde(default)]
    pub file_id: Option<String>,
    pub operation_id: String,
    pub sequence: u64,
    pub state: String,
    pub phase: Option<String>,
    #[serde(default)]
    pub backend: Option<String>,
    #[serde(default)]
    pub completed_segments: Option<u64>,
    #[serde(default)]
    pub total_segments: Option<u64>,
    pub error_code: Option<String>,
    pub updated_at: u64,
    pub result: Option<TranscriptResult>,
    pub has_result: bool,
    pub source_changed: bool,
}
impl Snapshot {
    pub fn summary(&self) -> Self {
        let mut value = self.clone();
        value.result = None;
        value
    }
}
pub(super) struct ActiveTranscription {
    pub snapshot: Snapshot,
    pub cancel: watch::Sender<bool>,
    pub done: watch::Receiver<bool>,
    pub source: Source,
}
impl ActiveTranscription {
    pub fn uses_task(&self, task: &super::Task) -> bool {
        self.snapshot.task_id == task.id
            || task.path.as_ref().is_some_and(|path| {
                same_file::is_same_file(path, &self.source.path).unwrap_or(false)
            })
    }
}
fn load(db: &Connection, file_id: &str) -> Result<Option<Snapshot>> {
    let mut value = load_summary(db, file_id)?;
    if let Some(value) = &mut value {
        value.result = crate::library::latest_recognition(db, file_id)?;
        value.has_result = value.result.is_some();
    }
    Ok(value)
}
fn load_summary(db: &Connection, file_id: &str) -> Result<Option<Snapshot>> {
    let value: Option<String> = db
        .query_row(
            "SELECT snapshot FROM asset_transcriptions WHERE file_id=?1",
            [file_id],
            |r| r.get(0),
        )
        .optional()
        .context("DATABASE_ERROR")?;
    value
        .map(|text| {
            let mut value: Snapshot = serde_json::from_str(&text).context("DATABASE_ERROR")?;
            value.result = None;
            Ok(value)
        })
        .transpose()
}
fn store(db: &Connection, value: &Snapshot) -> Result<()> {
    let file_id = value.file_id.as_deref().context("FILE_NOT_FOUND")?;
    let asset_id = value.asset_id.as_deref().context("ASSET_NOT_FOUND")?;
    let tx = db.unchecked_transaction().context("DATABASE_ERROR")?;
    if let Some(result) = &value.result {
        crate::library::save_recognized(&tx, asset_id, Some(file_id), result)?;
    }
    let mut stored = value.summary();
    stored.task_id.clear();
    stored.source_changed = false;
    tx.execute("INSERT INTO asset_transcriptions(file_id,snapshot) VALUES(?1,?2) ON CONFLICT(file_id) DO UPDATE SET snapshot=excluded.snapshot", params![file_id,serde_json::to_string(&stored)?]).context("DATABASE_ERROR")?;
    tx.commit().context("DATABASE_ERROR")?;
    Ok(())
}
pub(super) fn init(db: &Connection) -> Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS video_transcripts(task_id TEXT PRIMARY KEY,snapshot TEXT NOT NULL);")?;
    let ids = {
        let mut query = db.prepare("SELECT file_id FROM asset_transcriptions")?;
        let rows = query
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    for id in ids {
        if let Some(mut value) = load(db, &id)? {
            if value.state == "running" {
                value.state = "interrupted".into();
                value.phase = None;
                value.completed_segments = None;
                value.total_segments = None;
                value.error_code = Some("TRANSCRIPTION_INTERRUPTED".into());
                value.sequence += 1;
                store(db, &value)?;
            }
        }
    }
    Ok(())
}
fn legacy_file(db: &Connection, task_id: &str) -> Result<(String, String)> {
    db.query_row(
        "SELECT asset_id,file_id FROM asset_task_links WHERE task_id=?1 AND file_id IS NOT NULL",
        [task_id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .optional()?
    .context("TRANSCRIPTION_NOT_FOUND")
}
fn emit_snapshot(core: &Core, value: &Snapshot) {
    (core.emit)(TaskEvent::AssetTranscription(Box::new(value.summary())));
    if !value.task_id.is_empty() {
        (core.emit)(TaskEvent::Transcription(Box::new(value.summary())));
    }
}
impl Core {
    pub fn transcripts(&self) -> Result<Vec<Snapshot>> {
        let inner = self.inner.lock().unwrap();
        let links = {
            let mut query = inner.db.prepare(
                "SELECT task_id,file_id FROM asset_task_links WHERE file_id IS NOT NULL",
            )?;
            let rows = query
                .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            rows
        };
        let mut values = Vec::new();
        for (task_id, file_id) in links {
            let value = inner
                .transcription
                .as_ref()
                .filter(|a| a.snapshot.file_id.as_deref() == Some(&file_id))
                .map(|a| a.snapshot.clone())
                .or_else(|| inner.transcription_unsaved.get(&file_id).cloned())
                .or(load_summary(&inner.db, &file_id)?);
            if let Some(mut value) = value {
                value.task_id = task_id;
                values.push(value.summary());
            }
        }
        Ok(values)
    }
    pub fn asset_transcript(&self, asset_id: &str, file_id: &str) -> Result<Option<Snapshot>> {
        let inner = self.inner.lock().unwrap();
        let file = crate::library::asset_file(&inner.db, asset_id, file_id)?;
        let mut value = inner
            .transcription
            .as_ref()
            .filter(|a| a.snapshot.file_id.as_deref() == Some(file_id))
            .map(|a| a.snapshot.clone())
            .or_else(|| inner.transcription_unsaved.get(file_id).cloned())
            .or(load(&inner.db, file_id)?);
        if let Some(value) = &mut value {
            value.source_changed = value
                .result
                .as_ref()
                .is_some_and(|r| Source::read(&file.path).ok().as_ref() != Some(&r.source));
        }
        Ok(value)
    }
    pub fn transcript(&self, id: &str) -> Result<Option<Snapshot>> {
        let (asset_id, file_id) = {
            let inner = self.inner.lock().unwrap();
            if !inner.tasks.iter().any(|t| t.id == id) {
                bail!("TASK_NOT_FOUND");
            }
            let value = legacy_file(&inner.db, id);
            match value {
                Ok(value) => value,
                Err(error) if error.to_string() == "TRANSCRIPTION_NOT_FOUND" => return Ok(None),
                Err(error) => return Err(error),
            }
        };
        self.asset_transcript(&asset_id, &file_id).map(|value| {
            value.map(|mut value| {
                value.task_id = id.into();
                value
            })
        })
    }
    pub fn start_transcription(self: &Arc<Self>, id: &str, force: bool) -> Result<Snapshot> {
        let (asset_id, file_id) = {
            let inner = self.inner.lock().unwrap();
            let task = inner
                .tasks
                .iter()
                .find(|task| task.id == id)
                .context("TASK_NOT_FOUND")?;
            if task.status != super::Status::Completed
                || !task.files.is_empty()
                || task
                    .video
                    .as_ref()
                    .is_some_and(|v| v["kind"] == "image_album")
            {
                bail!("INVALID_STATE");
            }
            legacy_file(&inner.db, id)?
        };
        self.start_file_transcription(&asset_id, &file_id, Some(id), force, false)
            .map(|mut value| {
                value.task_id = id.into();
                value
            })
    }
    pub fn start_asset_transcription(
        self: &Arc<Self>,
        asset_id: &str,
        file_id: &str,
        force: bool,
    ) -> Result<Snapshot> {
        self.start_file_transcription(asset_id, file_id, None, force, false)
    }
    pub(super) fn start_mcp_asset_transcription(self: &Arc<Self>, asset_id: &str, file_id: &str, force: bool) -> Result<Snapshot> {
        self.start_file_transcription(asset_id, file_id, None, force, true)
    }
    fn start_file_transcription(
        self: &Arc<Self>,
        asset_id: &str,
        file_id: &str,
        legacy_id: Option<&str>,
        force: bool,
        require_mcp_write: bool,
    ) -> Result<Snapshot> {
        let mut inner = self.inner.lock().unwrap();
        if inner.closing { bail!("INVALID_STATE"); }
        if require_mcp_write {
            if !inner.mcp_write_enabled { bail!("WRITE_DISABLED"); }
            crate::mcp_read::require_collected(&inner.db, asset_id)?;
        }
        let file = crate::library::asset_file(&inner.db, asset_id, file_id)?;
        let source = Source::read(&file.path)?;
        if inner.dependency_cancel.is_some() {
            bail!("DEPENDENCY_BUSY");
        }
        if let Some(active) = &inner.transcription {
            if active.snapshot.file_id.as_deref() == Some(file_id) {
                return Ok(active.snapshot.summary());
            }
            bail!("TRANSCRIPTION_BUSY");
        }
        let old = inner
            .transcription_unsaved
            .get(file_id)
            .cloned()
            .or(load(&inner.db, file_id)?);
        if !force
            && old
                .as_ref()
                .and_then(|s| s.result.as_ref())
                .is_some_and(|r| r.source == source)
        {
            return Ok(old.unwrap().summary());
        }
        let result = old.as_ref().and_then(|v| v.result.clone());
        let task_id = match legacy_id {
            Some(id)=>id.to_owned(),
            None=>inner.db.query_row("SELECT task_id FROM asset_task_links WHERE file_id=?1 ORDER BY task_id LIMIT 1", [file_id], |r|r.get::<_,String>(0)).optional()?.unwrap_or_default(),
        };
        let snapshot = Snapshot {
            task_id,
            asset_id: Some(asset_id.into()),
            file_id: Some(file_id.into()),
            operation_id: Uuid::new_v4().to_string(),
            sequence: old.map_or(1, |v| v.sequence + 1),
            state: "running".into(),
            phase: Some("extracting_audio".into()),
            backend: None,
            completed_segments: None,
            total_segments: None,
            error_code: None,
            updated_at: now(),
            has_result: result.is_some(),
            result,
            source_changed: false,
        };
        store(&inner.db, &snapshot)?;
        inner.transcription_unsaved.remove(file_id);
        let (cancel, mut cancellation) = watch::channel(false);
        let (done, finished) = watch::channel(false);
        inner.transcription = Some(ActiveTranscription {
            snapshot: snapshot.clone(),
            cancel,
            done: finished,
            source: source.clone(),
        });
        emit_snapshot(self, &snapshot);
        let core = self.clone();
        let file_id = file_id.to_owned();
        let operation = snapshot.operation_id.clone();
        inner.workers.retain(|w| !w.is_finished());
        inner.workers.push(tokio::spawn(async move {
            let stage_id = format!("asr-{operation}");
            let outcome: Result<(String, bool, String, crate::library::TranscriptTiming)> = async {
                let stage = tempfile::Builder::new().prefix(&format!("yingqu-{stage_id}-")).tempdir().context("IO_ERROR")?;
                core.inner.lock().unwrap().db.execute("INSERT INTO task_staging VALUES(?1,?2)",
                    params![stage_id, stage.path().to_string_lossy()]).context("DATABASE_ERROR")?;
                let progress_core = core.clone();
                let progress_op = operation.clone();
                let result = PythonBridge::operation_cancellable(&core.program, &core.args, &operation, "transcribe_video",
                    json!({"source":source.path,"staging_dir":stage.path(),"runtime_directory":core.runtime_directory}),
                    move |event| {
                        let mut inner = progress_core.inner.lock().unwrap();
                        if let Some(active) = inner.transcription.as_mut().filter(|a| a.snapshot.operation_id == progress_op) {
                            active.snapshot.phase = event["phase"].as_str().map(str::to_owned);
                            active.snapshot.backend = event["backend"].as_str().map(str::to_owned);
                            active.snapshot.completed_segments = event["completed_segments"].as_u64();
                            active.snapshot.total_segments = event["total_segments"].as_u64();
                            active.snapshot.sequence += 1;
                            emit_snapshot(&progress_core,&active.snapshot);
                        }
                    }, async { let _ = cancellation.wait_for(|v| *v).await; }).await;
                let result = result.and_then(|v| {
                    if v["filename"] != "transcript.txt" { bail!("INVALID_ENGINE_RESULT"); }
                    let file = stage.path().join("transcript.txt");
                    let metadata = std::fs::symlink_metadata(&file).context("INVALID_ENGINE_RESULT")?;
                    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > MAX_TEXT || v["bytes"].as_u64() != Some(metadata.len()) { bail!("INVALID_ENGINE_RESULT"); }
                    let text = std::fs::read_to_string(file).context("INVALID_ENGINE_RESULT")?;
                    if text.trim().is_empty() != (v["no_speech"].as_bool().context("INVALID_ENGINE_RESULT")?) { bail!("INVALID_ENGINE_RESULT"); }
                    if Source::read(&source.path).ok().as_ref() != Some(&source) { bail!("TRANSCRIPTION_SOURCE_CHANGED"); }
                    let backend = v["backend"].as_str().filter(|b| matches!(*b, "cpu" | "cuda")).context("INVALID_ENGINE_RESULT")?;
                    let mut timing = crate::library::TranscriptTiming::artifact(stage.path(), &v["timing"], &text)?;
                    timing.operation_id = Some(operation.clone());
                    Ok((text, v["no_speech"] == true, backend.to_owned(), timing))
                });
                // Bridge has reaped descendants before any staging is removed.
                let cleanup = cleanup_staging(&core.inner.lock().unwrap().db, &stage_id, stage.path());
                cleanup?;
                result
            }.await;
            let mut inner = core.inner.lock().unwrap();
            if let Some(active) = inner.transcription.as_mut().filter(|a| a.snapshot.operation_id == operation) {
                let mut value = active.snapshot.clone();
                let previous_result = value.result.clone();
                value.phase = None; value.sequence += 1; value.updated_at = now();
                value.completed_segments = None; value.total_segments = None;
                match outcome {
                    Ok((text, empty, backend, timing)) => {
                        value.state = if empty { "no_speech" } else { "completed" }.into();
                        value.backend = Some(backend.clone());
                        if !empty {
                            let (model_revision, pipeline_revision) = if backend == "cuda" {
                                ("ce72677f84900f0dc57f498ace253bfb3c9155b6", "fun-asr-nano-audiocpp-50749561-media-zero-v2")
                            } else { (MODEL, "fun-asr-nano-cpu-fbbfe018-media-zero-v2") };
                            value.result = Some(TranscriptResult { timing: Some(timing), text, source, model_revision: model_revision.into(), model_name: Some("Fun-ASR-Nano".into()), pipeline_revision: pipeline_revision.into(), backend: Some(backend), completed_at: now() });
                        }
                    },
                    Err(error) => {
                        value.state = if error.to_string() == "CANCELED" { "canceled" } else { "failed" }.into();
                        value.error_code = Some(if error.to_string() == "TIMEOUT" { "TRANSCRIPTION_TIMEOUT".into() } else { error.to_string() });
                    }
                }
                value.has_result = value.result.is_some();
                {
                    if store(&inner.db, &value).is_err() {
                        value.state = "failed".into(); value.error_code = Some("DATABASE_ERROR".into());
                        // A recognized result is not saved until SQLite commits. Keep
                        // this failure queryable without losing the prior saved text.
                        value.result = previous_result;
                        value.has_result = value.result.is_some();
                        inner.transcription_unsaved.insert(file_id.clone(), value.clone());
                    }
                    emit_snapshot(&core,&value);
                    if value.state=="completed" {(core.emit)(TaskEvent::LibraryChanged(super::library_api::LibraryChange{scope:super::library_api::LibraryScope::Documents,asset_ids:vec![value.asset_id.clone().unwrap()]}));}
                }
                inner.transcription = None;
            }
            let _ = done.send(true);
        }));
        Ok(snapshot.summary())
    }
    pub async fn cancel_asset_transcription(&self, file_id: &str, operation: &str) -> Result<()> {
        let done = {
            let inner = self.inner.lock().unwrap();
            inner
                .transcription
                .as_ref()
                .filter(|active| {
                    active.snapshot.file_id.as_deref() == Some(file_id)
                        && active.snapshot.operation_id == operation
                })
                .map(|active| {
                    let _ = active.cancel.send(true);
                    active.done.clone()
                })
        };
        if let Some(mut done) = done {
            let _ = done.wait_for(|v| *v).await;
        }
        Ok(())
    }
    pub async fn cancel_transcription(&self, id: &str, operation: &str) -> Result<()> {
        let (_, file_id) = legacy_file(&self.inner.lock().unwrap().db, id)?;
        self.cancel_asset_transcription(&file_id, operation).await
    }
    pub async fn remove_after_transcription(&self, id: &str, delete: bool) -> Result<()> {
        let operation = {
            let inner = self.inner.lock().unwrap();
            let task = inner
                .tasks
                .iter()
                .find(|task| task.id == id)
                .context("TASK_NOT_FOUND")?;
            inner
                .transcription
                .as_ref()
                .filter(|active| active.uses_task(task))
                .map(|active| {
                    (
                        active.snapshot.file_id.clone().unwrap(),
                        active.snapshot.operation_id.clone(),
                    )
                })
        };
        if let Some((file_id, operation)) = operation {
            self.cancel_asset_transcription(&file_id, &operation)
                .await?;
        }
        if delete {
            self.delete_local_file(id)
        } else {
            self.remove_history(id)
        }
    }
    pub fn transcript_text(&self, id: &str) -> Result<TranscriptResult> {
        self.transcript(id)?
            .and_then(|v| v.result)
            .context("TRANSCRIPTION_NOT_FOUND")
    }
}

pub fn export_txt(result: &TranscriptResult, target: &Path) -> Result<()> {
    export_text(&result.text, target, std::slice::from_ref(&result.source.path))
}

pub(crate) fn export_text(text: &str, target: &Path, protected_sources: &[PathBuf]) -> Result<()> {
    let text = if cfg!(windows) { text.replace('\n', "\r\n") } else { text.to_owned() };
    let mut bytes = Vec::with_capacity(3 + text.len());
    bytes.extend_from_slice(b"\xef\xbb\xbf"); bytes.extend_from_slice(text.as_bytes());
    export_bytes(&bytes, target, protected_sources)
}

pub(crate) fn export_bytes(bytes: &[u8], target: &Path, protected_sources: &[PathBuf]) -> Result<()> {
    let resolved_target = target
        .parent()
        .context("IO_ERROR")?
        .canonicalize()
        .context("IO_ERROR")?
        .join(target.file_name().context("IO_ERROR")?);
    for source in protected_sources {
        let same_path = if cfg!(windows) {
            resolved_target.to_string_lossy().eq_ignore_ascii_case(&source.to_string_lossy())
        } else { resolved_target == *source };
        if same_path || (target.exists() && source.exists()
            && same_file::is_same_file(source, target).context("IO_ERROR")?) {
            bail!("TRANSCRIPTION_EXPORT_SOURCE");
        }
    }
    let parent = target.parent().context("IO_ERROR")?;
    let mut stage = tempfile::NamedTempFile::new_in(parent).context("IO_ERROR")?;
    stage.write_all(bytes).context("IO_ERROR")?;
    stage.as_file().sync_all().context("IO_ERROR")?;
    stage
        .persist(target)
        .map_err(|_| anyhow::anyhow!("IO_ERROR"))?;
    Ok(())
}
