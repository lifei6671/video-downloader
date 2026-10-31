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
}
fn load(db: &Connection, id: &str) -> Result<Option<Snapshot>> {
    let value: Option<String> = db
        .query_row(
            "SELECT snapshot FROM video_transcripts WHERE task_id=?",
            [id],
            |r| r.get(0),
        )
        .optional()
        .context("DATABASE_ERROR")?;
    value
        .map(|text| serde_json::from_str(&text).context("DATABASE_ERROR"))
        .transpose()
}
fn store(db: &Connection, value: &Snapshot) -> Result<()> {
    persist_snapshot(db,value,false)
}
fn persist_snapshot(db:&Connection,value:&Snapshot,new_result:bool)->Result<()> {
    let tx = db.unchecked_transaction().context("DATABASE_ERROR")?;
    tx.execute("INSERT INTO video_transcripts VALUES(?1,?2) ON CONFLICT(task_id) DO UPDATE SET snapshot=excluded.snapshot",
               params![value.task_id, serde_json::to_string(value)?]).context("DATABASE_ERROR")?;
    // Startup initializes this schema before normal transcript writes.
    if new_result&&tx.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='media_assets')", [], |r| r.get::<_,bool>(0))? {
        crate::backup::repository::preserve_transcript(&tx, value)?;
    }
    tx.commit().context("DATABASE_ERROR")?;
    Ok(())
}
pub(super) fn init(db: &Connection) -> Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS video_transcripts(task_id TEXT PRIMARY KEY,snapshot TEXT NOT NULL);")?;
    let mut query = db.prepare("SELECT task_id FROM video_transcripts")?;
    let ids = query
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
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

pub(crate) fn rebind_asset_source(db:&Connection,id:&str,hash:&str,source:&serde_json::Value)->Result<Vec<Snapshot>> {
    let text:Option<String>=db.query_row("SELECT text FROM asset_transcripts WHERE asset_id=?1 AND content_sha256=?2",params![id,hash],|r|r.get(0)).optional()?;
    let Some(text)=text else {return Ok(Vec::new());};
    let mut q=db.prepare("SELECT snapshot FROM video_transcripts WHERE task_id=?1 OR task_id IN (SELECT task_id FROM media_assets WHERE asset_id=?1)")?;
    let rows=q.query_map([id],|r|r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;drop(q);
    let source:Source=serde_json::from_value(source.clone())?;let mut updated=Vec::new();
    for row in rows {
        let mut snapshot:Snapshot=serde_json::from_str(&row)?;
        if let Some(result)=snapshot.result.as_mut().filter(|r|r.text==text) {
            result.source=source.clone();snapshot.source_changed=false;snapshot.sequence+=1;
            db.execute("UPDATE video_transcripts SET snapshot=?2 WHERE task_id=?1",params![snapshot.task_id,serde_json::to_string(&snapshot)?])?;updated.push(snapshot);
        }
    }
    if !updated.is_empty() {db.execute("UPDATE asset_transcripts SET source_json=?2 WHERE asset_id=?1",params![id,serde_json::to_string(&source)?])?;}
    Ok(updated)
}

fn asset_exists(db:&Connection,id:&str)->Result<bool> {
    Ok(db.query_row("SELECT EXISTS(SELECT 1 FROM media_assets WHERE asset_id=?)",[id],|r|r.get(0))?)
}
impl Core {
    pub fn transcripts(&self) -> Result<Vec<Snapshot>> {
        let inner = self.inner.lock().unwrap();
        let mut query = inner
            .db
            .prepare("SELECT task_id FROM video_transcripts")
            .context("DATABASE_ERROR")?;
        let ids = query
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        ids.iter()
            .map(|id| {
                let value = inner
                    .transcription
                    .as_ref()
                    .filter(|a| a.snapshot.task_id == *id)
                    .map(|a| a.snapshot.clone())
                    .or_else(|| inner.transcription_unsaved.get(id).cloned());
                Ok(value
                    .or(load(&inner.db, id)?)
                    .context("DATABASE_ERROR")?
                    .summary())
            })
            .collect()
    }
    pub fn transcript(&self, id: &str) -> Result<Option<Snapshot>> {
        let inner = self.inner.lock().unwrap();
        if !inner.tasks.iter().any(|t| t.id == id) && !asset_exists(&inner.db,id)? {
            bail!("TASK_NOT_FOUND");
        }
        let mut value = inner
            .transcription
            .as_ref()
            .filter(|a| a.snapshot.task_id == id)
            .map(|a| a.snapshot.clone())
            .or_else(|| inner.transcription_unsaved.get(id).cloned())
            .or(load(&inner.db, id)?);
        if let Some(value) = &mut value {
            value.source_changed = value
                .result
                .as_ref()
                .is_some_and(|r| Source::read(&r.source.path).ok().as_ref() != Some(&r.source));
        }
        Ok(value)
    }
    pub fn start_transcription(self: &Arc<Self>, id: &str, force: bool) -> Result<Snapshot> {
        let path = if self.list().iter().any(|t|t.id==id) {self.playback_path(id)?} else {
            self.backup_assets()?.into_iter().find(|a|a.asset_id==id).and_then(|a|a.path).context("FILE_MISSING")?
        };
        let source = Source::read(&path)?;
        let mut inner = self.inner.lock().unwrap();
        if inner.closing {
            bail!("INVALID_STATE");
        }
        if inner.dependency_cancel.is_some() {
            bail!("DEPENDENCY_BUSY");
        }
        if let Some(active) = &inner.transcription {
            if active.snapshot.task_id == id {
                return Ok(active.snapshot.summary());
            }
            bail!("TRANSCRIPTION_BUSY");
        }
        if !inner
            .tasks
            .iter()
            .any(|t| t.id == id && t.path.as_ref() == Some(&path))
            && !asset_exists(&inner.db,id)?
        {
            bail!("TASK_NOT_FOUND");
        }
        let old = inner
            .transcription_unsaved
            .get(id)
            .cloned()
            .or(load(&inner.db, id)?);
        if !force
            && old
                .as_ref()
                .and_then(|s| s.result.as_ref())
                .is_some_and(|r| r.source == source)
        {
            return Ok(old.unwrap().summary());
        }
        let result = old.as_ref().and_then(|v| v.result.clone());
        let snapshot = Snapshot {
            task_id: id.into(),
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
        inner.transcription_unsaved.remove(id);
        let (cancel, mut cancellation) = watch::channel(false);
        let (done, finished) = watch::channel(false);
        inner.transcription = Some(ActiveTranscription {
            snapshot: snapshot.clone(),
            cancel,
            done: finished,
        });
        (self.emit)(TaskEvent::Transcription(Box::new(snapshot.summary())));
        let core = self.clone();
        let task_id = id.to_owned();
        let operation = snapshot.operation_id.clone();
        inner.workers.retain(|w| !w.is_finished());
        inner.workers.push(tokio::spawn(async move {
            let stage_id = format!("asr-{operation}");
            let outcome: Result<(String, bool, String)> = async {
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
                            (progress_core.emit)(TaskEvent::Transcription(Box::new(active.snapshot.summary())));
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
                    Ok((text, v["no_speech"] == true, backend.to_owned()))
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
                    Ok((text, empty, backend)) => {
                        value.state = if empty { "no_speech" } else { "completed" }.into();
                        value.backend = Some(backend.clone());
                        if !empty {
                            let (model_revision, pipeline_revision) = if backend == "cuda" {
                                ("ce72677f84900f0dc57f498ace253bfb3c9155b6", "fun-asr-nano-audiocpp-50749561-fsmn-v1")
                            } else { (MODEL, "fun-asr-nano-cpu-fbbfe018-v1") };
                            value.result = Some(TranscriptResult { text, source, model_revision: model_revision.into(), model_name: Some("Fun-ASR-Nano".into()), pipeline_revision: pipeline_revision.into(), backend: Some(backend), completed_at: now() });
                        }
                    },
                    Err(error) => {
                        value.state = if error.to_string() == "CANCELED" { "canceled" } else { "failed" }.into();
                        value.error_code = Some(if error.to_string() == "TIMEOUT" { "TRANSCRIPTION_TIMEOUT".into() } else { error.to_string() });
                    }
                }
                value.has_result = value.result.is_some();
                if inner.tasks.iter().any(|t| t.id == task_id) || asset_exists(&inner.db,&task_id).unwrap_or(false) {
                    if persist_snapshot(&inner.db, &value,value.state=="completed").is_err() {
                        value.state = "failed".into(); value.error_code = Some("DATABASE_ERROR".into());
                        // A recognized result is not saved until SQLite commits. Keep
                        // this failure queryable without losing the prior saved text.
                        value.result = previous_result;
                        value.has_result = value.result.is_some();
                        inner.transcription_unsaved.insert(task_id.clone(), value.clone());
                    }
                    (core.emit)(TaskEvent::Transcription(Box::new(value.summary())));
                }
                inner.transcription = None;
                core.wake_backup();
            }
            let _ = done.send(true);
        }));
        Ok(snapshot.summary())
    }
    pub async fn cancel_transcription(&self, id: &str, operation: &str) -> Result<()> {
        let done = {
            let inner = self.inner.lock().unwrap();
            match inner.transcription.as_ref() {
                Some(active)
                    if active.snapshot.task_id == id
                        && active.snapshot.operation_id == operation =>
                {
                    let _ = active.cancel.send(true);
                    Some(active.done.clone())
                }
                _ => None,
            }
        };
        if let Some(mut done) = done {
            let _ = done.wait_for(|v| *v).await;
        }
        Ok(())
    }
    pub async fn remove_after_transcription(&self, id: &str, delete: bool) -> Result<()> {
        let operation = self
            .inner
            .lock()
            .unwrap()
            .transcription
            .as_ref()
            .filter(|a| a.snapshot.task_id == id)
            .map(|a| a.snapshot.operation_id.clone());
        if let Some(operation) = operation {
            self.cancel_transcription(id, &operation).await?;
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
    export_text(&result.text,Some(&result.source.path),target)
}

pub fn export_text(text:&str,source:Option<&Path>,target:&Path)->Result<()> {
    if let Some(source)=source {
    let resolved_target = target
        .parent()
        .context("IO_ERROR")?
        .canonicalize()
        .context("IO_ERROR")?
        .join(target.file_name().context("IO_ERROR")?);
    let same_path = if cfg!(windows) {
        resolved_target
            .to_string_lossy()
            .eq_ignore_ascii_case(&source.to_string_lossy())
    } else {
        resolved_target == source
    };
    if same_path {
        bail!("TRANSCRIPTION_EXPORT_SOURCE");
    }
    if target.exists()
        && source.exists()
        && same_file::is_same_file(source, target).context("IO_ERROR")?
    {
        bail!("TRANSCRIPTION_EXPORT_SOURCE");
    }
    }
    let parent = target.parent().context("IO_ERROR")?;
    let mut stage = tempfile::NamedTempFile::new_in(parent).context("IO_ERROR")?;
    stage.write_all(b"\xef\xbb\xbf").context("IO_ERROR")?;
    let text = if cfg!(windows) {
        text.replace('\n', "\r\n")
    } else {
        text.to_owned()
    };
    stage.write_all(text.as_bytes()).context("IO_ERROR")?;
    stage.as_file().sync_all().context("IO_ERROR")?;
    stage
        .persist(target)
        .map_err(|_| anyhow::anyhow!("IO_ERROR"))?;
    Ok(())
}
