use crate::{
    files::{finalize_to, Video},
    python_bridge::PythonBridge,
    settings::{self, Settings},
};
use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::sync::watch;
use uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Parsing,
    Ready,
    Queued,
    Downloading,
    Completed,
    Failed,
    Canceled,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub revision: u64,
    pub status: Status,
    pub video: Option<Value>,
    pub bytes: u64,
    pub total_bytes: Option<u64>,
    pub speed_bps: Option<f64>,
    pub error: Option<String>,
    pub path: Option<PathBuf>,
    pub created_at: u64,
    pub completed_at: Option<u64>,
    pub canceling: bool,
    #[serde(default)]
    pub file_missing: bool,
    #[serde(default)]
    pub queue_order: Option<u64>,
    #[serde(default)]
    pub download_requested: bool,
    #[serde(default)]
    pub cover_path: Option<PathBuf>,
    #[serde(default)]
    pub cover_error: Option<String>,
}

fn previously_downloaded(tasks: &[Task], task: &Task) -> bool {
    task.video.as_ref().is_some_and(|video| {
        tasks.iter().any(|old| {
            old.id != task.id
                && old.status == Status::Completed
                && old.video.as_ref().is_some_and(|v| {
                    v["platform"] == video["platform"] && v["video_id"] == video["video_id"]
                })
        })
    })
}

pub enum TaskEvent {
    Updated(Box<Task>),
    Removed(String),
}

struct Active {
    download_after_resolve: bool,
    profiles: Value,
    directory: PathBuf,
    cancel: watch::Sender<bool>,
    done: watch::Receiver<bool>,
}
struct Inner {
    db: Connection,
    tasks: Vec<Task>,
    settings: Settings,
    active: HashMap<String, Active>,
    downloads: HashSet<String>,
    closing: bool,
}
pub struct Core {
    _instance_lock: std::fs::File,
    inner: Mutex<Inner>,
    program: String,
    args: Vec<String>,
    emit: Arc<dyn Fn(TaskEvent) + Send + Sync>,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before epoch")
        .as_secs()
}
fn persist(db: &Connection, task: &Task) -> Result<()> {
    let mut stored = task.clone();
    // Covers may carry short-lived query tokens. Keep them only in memory.
    if let Some(video) = stored.video.as_mut() {
        video["cover_url"] = Value::Null;
    }
    db.execute(
        "INSERT INTO download_tasks(id,snapshot) VALUES (?1,?2)
        ON CONFLICT(id) DO UPDATE SET snapshot=excluded.snapshot",
        params![task.id, serde_json::to_string(&stored)?],
    )
    .context("DATABASE_ERROR")?;
    Ok(())
}
fn publish(inner: &mut Inner, task: Task, emit: &dyn Fn(TaskEvent)) -> Result<()> {
    persist(&inner.db, &task)?;
    if let Some(old) = inner.tasks.iter_mut().find(|t| t.id == task.id) {
        *old = task.clone();
    } else {
        inner.tasks.insert(0, task.clone());
    }
    emit(TaskEvent::Updated(Box::new(task)));
    Ok(())
}

impl Core {
    pub fn open(
        db_path: &Path,
        directory: PathBuf,
        program: String,
        args: Vec<String>,
        emit: impl Fn(TaskEvent) + Send + Sync + 'static,
    ) -> Result<Arc<Self>> {
        let instance_lock = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(db_path.with_extension("lock"))
            .context("IO_ERROR")?;
        instance_lock.try_lock().context("APP_ALREADY_RUNNING")?;
        let db = Connection::open(db_path).context("DATABASE_ERROR")?;
        db.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;
            CREATE TABLE IF NOT EXISTS download_tasks(id TEXT PRIMARY KEY,snapshot TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS commit_intents(task_id TEXT PRIMARY KEY,source TEXT NOT NULL,
                target TEXT NOT NULL,size INTEGER NOT NULL);
            CREATE TABLE IF NOT EXISTS task_staging(task_id TEXT PRIMARY KEY,path TEXT NOT NULL);",
        )
        .context("DATABASE_ERROR")?;
        let settings = settings::load(&db, directory)?;
        let mut tasks = {
            let mut query =
                db.prepare("SELECT snapshot FROM download_tasks ORDER BY rowid DESC")?;
            let strings = query
                .query_map([], |r| r.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            strings
                .iter()
                .map(|s| serde_json::from_str::<Task>(s))
                .collect::<serde_json::Result<Vec<_>>>()?
        };
        for task in &mut tasks {
            // Older snapshots did not distinguish previews from submissions.
            task.download_requested |= task.queue_order.is_some()
                || task.bytes > 0
                || task.path.is_some()
                || matches!(
                    task.status,
                    Status::Queued | Status::Downloading | Status::Completed
                );
            if matches!(
                task.status,
                Status::Parsing | Status::Ready | Status::Queued | Status::Downloading
            ) || task.error.as_deref() == Some("DATABASE_ERROR")
            {
                task.status = Status::Failed;
                task.error = Some("INTERRUPTED".into());
                task.canceling = false;
                task.speed_bps = None;
                task.revision += 1;
                let intent = db.query_row(
                    "SELECT source,target,size FROM commit_intents WHERE task_id=?",
                    [&task.id],
                    |r| {
                        Ok((
                            r.get::<_, String>(0)?,
                            r.get::<_, String>(1)?,
                            r.get::<_, i64>(2)? as u64,
                        ))
                    },
                );
                match intent {
                    Ok((source, target, size))
                        if !Path::new(&source).exists()
                            && std::fs::symlink_metadata(&target).is_ok_and(|m| {
                                m.is_file() && !m.file_type().is_symlink() && m.len() == size
                            }) =>
                    {
                        task.status = Status::Completed;
                        task.error = None;
                        let target = PathBuf::from(target);
                        task.cover_path = ["jpg", "png", "webp"]
                            .iter()
                            .map(|ext| target.with_extension(format!("cover.{ext}")))
                            .find(|path| path.is_file());
                        task.path = Some(target);
                        task.bytes = size;
                        task.completed_at = Some(now());
                    }
                    Ok(_) | Err(rusqlite::Error::QueryReturnedNoRows) => {}
                    Err(_) => bail!("DATABASE_ERROR"),
                }
                persist(&db, task)?;
            }
        }
        // Clean only recorded task-owned directories; never sweep downloads.
        let staging = {
            let mut query = db.prepare("SELECT task_id,path FROM task_staging")?;
            let rows =
                query.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        for (id, path) in staging {
            let path = PathBuf::from(path);
            if path
                .file_name()
                .is_none_or(|n| !n.to_string_lossy().starts_with(&format!("yingqu-{id}-")))
            {
                bail!("INVALID_STAGING_PATH");
            }
            match std::fs::symlink_metadata(&path) {
                Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
                    #[cfg(windows)]
                    {
                        use std::os::windows::fs::MetadataExt;
                        if metadata.file_attributes() & 0x400 != 0 {
                            bail!("INVALID_STAGING_PATH");
                        }
                    }
                    std::fs::remove_dir_all(&path).context("IO_ERROR")?;
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                _ => bail!("INVALID_STAGING_PATH"),
            }
            db.execute("DELETE FROM task_staging WHERE task_id=?", [id])
                .context("DATABASE_ERROR")?;
        }
        Ok(Arc::new(Self {
            _instance_lock: instance_lock,
            inner: Mutex::new(Inner {
                db,
                tasks,
                settings,
                active: HashMap::new(),
                downloads: HashSet::new(),
                closing: false,
            }),
            program,
            args,
            emit: Arc::new(emit),
        }))
    }

    pub fn list(&self) -> Vec<Task> {
        self.inner
            .lock()
            .unwrap()
            .tasks
            .iter()
            .cloned()
            .map(|mut t| {
                t.file_missing =
                    t.status == Status::Completed && t.path.as_ref().is_none_or(|p| !p.is_file());
                t
            })
            .collect()
    }
    pub fn settings(&self) -> Settings {
        self.inner.lock().unwrap().settings.clone()
    }
    pub fn cookie(&self, platform: &str) -> Result<String> {
        settings::cookie(&self.inner.lock().unwrap().settings, platform)
    }
    pub fn cover(&self, id: &str) -> Result<Vec<u8>> {
        let inner = self.inner.lock().unwrap();
        let task = inner
            .tasks
            .iter()
            .find(|t| t.id == id)
            .context("TASK_NOT_FOUND")?;
        crate::files::read_cover(task.cover_path.as_deref().context("FILE_MISSING")?)
    }
    pub fn save_profile(&self, update: settings::ProfileUpdate) -> Result<Settings> {
        let mut inner = self.inner.lock().unwrap();
        let Inner { db, settings, .. } = &mut *inner;
        settings::save_profile(db, settings, update)?;
        Ok(settings.clone())
    }
    pub fn set_directory(&self, path: PathBuf) -> Result<Settings> {
        let path = path.canonicalize().context("IO_ERROR")?;
        if !path.is_dir() {
            bail!("IO_ERROR");
        }
        tempfile::NamedTempFile::new_in(&path)
            .context("IO_ERROR")?
            .close()
            .context("IO_ERROR")?;
        let mut inner = self.inner.lock().unwrap();
        inner.db.execute("INSERT INTO app_settings(id,directory) VALUES(1,?) ON CONFLICT(id) DO UPDATE SET directory=excluded.directory",
            [path.to_string_lossy().as_ref()]).context("DATABASE_ERROR")?;
        inner.settings.download_directory = path;
        Ok(inner.settings.clone())
    }

    pub fn resolve(self: &Arc<Self>, input: String) -> Result<Task> {
        self.resolve_task(input, false)
    }

    pub fn retry(self: &Arc<Self>, id: &str) -> Result<Task> {
        let input = {
            let inner = self.inner.lock().unwrap();
            let task = inner
                .tasks
                .iter()
                .find(|t| t.id == id)
                .context("TASK_NOT_FOUND")?;
            if !matches!(task.status, Status::Failed | Status::Canceled) {
                bail!("INVALID_STATE");
            }
            let video: Video =
                serde_json::from_value(task.video.clone().context("REPARSE_REQUIRED")?)
                    .context("INVALID_ENGINE_RESULT")?;
            video.validate()?;
            video.canonical_url
        };
        self.resolve_task(input, true)
    }

    pub fn remove_history(&self, id: &str) -> Result<()> {
        let mut inner = self.inner.lock().unwrap();
        let task = inner
            .tasks
            .iter()
            .find(|t| t.id == id)
            .context("TASK_NOT_FOUND")?;
        if !matches!(
            task.status,
            Status::Completed | Status::Failed | Status::Canceled
        ) || inner.active.contains_key(id)
            || inner.downloads.contains(id)
        {
            bail!("TASK_BUSY");
        }
        let tx = inner.db.transaction().context("DATABASE_ERROR")?;
        tx.execute("DELETE FROM commit_intents WHERE task_id=?", [id])
            .context("DATABASE_ERROR")?;
        tx.execute("DELETE FROM download_tasks WHERE id=?", [id])
            .context("DATABASE_ERROR")?;
        tx.commit().context("DATABASE_ERROR")?;
        // Staging records, if cleanup previously failed, remain available for
        // startup cleanup. Removing history never touches a completed video.
        inner.tasks.retain(|t| t.id != id);
        (self.emit)(TaskEvent::Removed(id.to_owned()));
        Ok(())
    }

    pub fn delete_local_file(&self, id: &str) -> Result<()> {
        let mut inner = self.inner.lock().unwrap();
        if inner.active.contains_key(id) || inner.downloads.contains(id) {
            bail!("TASK_BUSY");
        }
        let task = inner
            .tasks
            .iter_mut()
            .find(|t| t.id == id)
            .context("TASK_NOT_FOUND")?;
        if task.status != Status::Completed {
            bail!("INVALID_STATE");
        }
        let path = task.path.as_ref().context("FILE_MISSING")?;
        match std::fs::symlink_metadata(path) {
            Ok(metadata) => {
                if !metadata.is_file() || metadata.file_type().is_symlink() {
                    bail!("IO_ERROR");
                }
                std::fs::remove_file(path).context("IO_ERROR")?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => bail!("IO_ERROR"),
        }
        // Missing-file state is derived from the stored path on every reload.
        // No database write after deletion can turn a successful deletion into
        // a misleading failure; the original history remains intact.
        task.file_missing = true;
        task.revision += 1;
        (self.emit)(TaskEvent::Updated(Box::new(task.clone())));
        Ok(())
    }

    fn resolve_task(self: &Arc<Self>, input: String, download_after_resolve: bool) -> Result<Task> {
        if input.trim().is_empty() || input.len() > 8192 {
            bail!("INVALID_INPUT");
        }
        let mut inner = self.inner.lock().unwrap();
        if inner.closing || inner.tasks.iter().any(|t| t.status == Status::Parsing) {
            bail!("TASK_BUSY");
        }
        let profiles = settings::snapshot(&inner.settings)?;
        let task = Task {
            id: Uuid::new_v4().to_string(),
            revision: 1,
            status: Status::Parsing,
            video: None,
            bytes: 0,
            total_bytes: None,
            speed_bps: None,
            error: None,
            path: None,
            created_at: now(),
            completed_at: None,
            canceling: false,
            file_missing: false,
            queue_order: None,
            download_requested: download_after_resolve,
            cover_path: None,
            cover_error: None,
        };
        let (cancel, mut cancellation) = watch::channel(false);
        let (done, done_rx) = watch::channel(false);
        publish(&mut inner, task.clone(), &*self.emit)?;
        let directory = inner.settings.download_directory.clone();
        inner.active.insert(
            task.id.clone(),
            Active {
                download_after_resolve,
                profiles: profiles.clone(),
                directory,
                cancel,
                done: done_rx,
            },
        );
        let core = self.clone();
        let id = task.id.clone();
        tokio::spawn(async move {
            let result = PythonBridge::operation_cancellable(
                &core.program,
                &core.args,
                &id,
                "resolve_video",
                json!({"input":input,"profiles":profiles}),
                |_| {},
                async {
                    let _ = cancellation.wait_for(|v| *v).await;
                },
            )
            .await
            .and_then(|result| {
                let video: Video = serde_json::from_value(result["result"].clone())
                    .context("INVALID_ENGINE_RESULT")?;
                video.validate()?;
                Ok(result["result"].clone())
            });
            core.finish(&id, result, false);
            let _ = done.send(true);
        });
        Ok(task)
    }

    fn finish(self: &Arc<Self>, id: &str, result: Result<Value>, downloading: bool) {
        let mut inner = self.inner.lock().unwrap();
        let mut task = inner
            .tasks
            .iter()
            .find(|t| t.id == id)
            .expect("active task exists")
            .clone();
        if task.canceling {
            task.status = Status::Canceled;
            task.error = None;
        } else {
            match result {
                Ok(video) if !downloading => {
                    task.video = Some(video);
                    task.status = Status::Ready;
                }
                Ok(_) => task.status = Status::Completed,
                Err(error) => {
                    task.status = Status::Failed;
                    task.error = Some(
                        error
                            .to_string()
                            .split(':')
                            .next()
                            .unwrap_or("IO_ERROR")
                            .to_owned(),
                    );
                }
            }
        }
        if task.status == Status::Ready
            && !previously_downloaded(&inner.tasks, &task)
            && inner
                .active
                .get(id)
                .is_some_and(|a| a.download_after_resolve)
        {
            task.status = Status::Queued;
            task.queue_order = Some(
                inner
                    .tasks
                    .iter()
                    .filter_map(|t| t.queue_order)
                    .max()
                    .unwrap_or(0)
                    + 1,
            );
        }
        task.canceling = false;
        task.speed_bps = None;
        task.revision += 1;
        if publish(&mut inner, task.clone(), &*self.emit).is_err() {
            task.status = Status::Failed;
            task.error = Some("DATABASE_ERROR".into());
            *inner.tasks.iter_mut().find(|t| t.id == id).unwrap() = task.clone();
            (self.emit)(TaskEvent::Updated(Box::new(task.clone())));
        }
        if !matches!(task.status, Status::Ready | Status::Queued) {
            inner.active.remove(id);
        }
        if task.status == Status::Queued {
            self.dispatch(&mut inner);
        }
    }

    pub fn set_max_downloads(self: &Arc<Self>, value: u8) -> Result<Settings> {
        if !(1..=3).contains(&value) {
            bail!("CONFIG_INVALID");
        }
        let mut inner = self.inner.lock().unwrap();
        if inner.closing {
            bail!("TASK_BUSY");
        }
        inner
            .db
            .execute(
                "INSERT INTO app_settings(id,directory,max_downloads) VALUES(1,?1,?2)
            ON CONFLICT(id) DO UPDATE SET max_downloads=excluded.max_downloads",
                params![inner.settings.download_directory.to_string_lossy(), value],
            )
            .context("DATABASE_ERROR")?;
        inner.settings.max_downloads = value;
        self.dispatch(&mut inner);
        Ok(inner.settings.clone())
    }

    pub fn download(self: &Arc<Self>, id: &str, confirm_duplicate: bool) -> Result<()> {
        let mut inner = self.inner.lock().unwrap();
        if inner.closing {
            bail!("TASK_BUSY");
        }
        let mut task = inner
            .tasks
            .iter()
            .find(|t| t.id == id)
            .context("TASK_NOT_FOUND")?
            .clone();
        if task.status != Status::Ready {
            bail!("INVALID_STATE");
        }
        if !confirm_duplicate && previously_downloaded(&inner.tasks, &task) {
            bail!("ALREADY_DOWNLOADED");
        }
        task.status = Status::Queued;
        task.download_requested = true;
        task.queue_order = Some(
            inner
                .tasks
                .iter()
                .filter_map(|t| t.queue_order)
                .max()
                .unwrap_or(0)
                + 1,
        );
        task.revision += 1;
        publish(&mut inner, task, &*self.emit)?;
        self.dispatch(&mut inner);
        Ok(())
    }

    // This lock covers slot reservation and FIFO selection, including concurrent
    // submissions, settings changes, cancellation, and process completion.
    fn dispatch(self: &Arc<Self>, inner: &mut Inner) {
        while !inner.closing && inner.downloads.len() < usize::from(inner.settings.max_downloads) {
            let Some(id) = inner
                .tasks
                .iter()
                .filter(|t| t.status == Status::Queued)
                .min_by_key(|t| t.queue_order)
                .map(|t| t.id.clone())
            else {
                break;
            };
            if self.start_download(inner, &id).is_err() {
                let mut task = inner.tasks.iter().find(|t| t.id == id).unwrap().clone();
                task.status = Status::Failed;
                task.error = Some("DATABASE_ERROR".into());
                task.revision += 1;
                if publish(inner, task.clone(), &*self.emit).is_err() {
                    *inner.tasks.iter_mut().find(|t| t.id == id).unwrap() = task.clone();
                    (self.emit)(TaskEvent::Updated(Box::new(task)));
                }
                inner.active.remove(&id);
            }
        }
    }

    fn start_download(self: &Arc<Self>, inner: &mut Inner, id: &str) -> Result<()> {
        let mut task = inner.tasks.iter().find(|t| t.id == id).unwrap().clone();
        let active = inner
            .active
            .get(id)
            .expect("queued task has configuration snapshot");
        let profiles = active.profiles.clone();
        let directory = active.directory.clone();
        let video = task.video.clone().context("INVALID_STATE")?;
        let (cancel, mut cancellation) = watch::channel(false);
        let (done, done_rx) = watch::channel(false);
        task.status = Status::Downloading;
        task.revision += 1;
        publish(inner, task, &*self.emit)?;
        inner.downloads.insert(id.to_owned());
        let active = inner.active.get_mut(id).unwrap();
        active.cancel = cancel;
        active.done = done_rx;
        let core = self.clone();
        let id = id.to_owned();
        tokio::spawn(async move {
            let result = async {
                std::fs::create_dir_all(&directory).context("IO_ERROR")?;
                let stage = tempfile::Builder::new().prefix(&format!("yingqu-{id}-")).tempdir_in(&directory).context("IO_ERROR")?;
                core.inner.lock().unwrap().db.execute("INSERT INTO task_staging VALUES(?1,?2)",
                    params![id,stage.path().to_string_lossy()]).context("DATABASE_ERROR")?;
                let mut last = Instant::now() - Duration::from_secs(1);
                let result = PythonBridge::operation_cancellable(&core.program, &core.args, &id, "download_video",
                    json!({"input":video["canonical_url"],"profiles":profiles,
                        "expected":{"platform":video["platform"],"video_id":video["video_id"]},"staging_dir":stage.path()}),
                    |event| {
                        if last.elapsed() < Duration::from_millis(250) { return; }
                        last = Instant::now();
                        let mut inner = core.inner.lock().unwrap();
                        let task = inner.tasks.iter_mut().find(|t| t.id == id).unwrap();
                        if task.canceling { return; }
                        task.bytes = event["bytes"].as_u64().unwrap_or(0);
                        task.total_bytes = event["total_bytes"].as_u64();
                        task.speed_bps = event["speed_bps"].as_f64();
                        task.revision += 1;
                        (core.emit)(TaskEvent::Updated(Box::new(task.clone())));
                    },
                    async { let _ = cancellation.wait_for(|v| *v).await; }).await;
                let committed = result.and_then(|result| {
                    let mut inner = core.inner.lock().unwrap();
                    let mut task = inner.tasks.iter().find(|t| t.id == id).unwrap().clone();
                    if task.canceling { bail!("CANCELED"); }
                    let identity: Video = serde_json::from_value(video).context("INVALID_ENGINE_RESULT")?;
                    let extension = result["container"].as_str().context("INVALID_ENGINE_RESULT")?;
                    if !matches!(extension, "mp4" | "webm") { bail!("INVALID_MEDIA_CONTENT"); }
                    let target = directory.join(format!("{}_{}_{}.{}",identity.platform,identity.video_id,id,extension));
                    let file = finalize_to(&identity, &result, stage.path(), &target, |source,size| {
                        inner.db.execute("INSERT INTO commit_intents VALUES(?1,?2,?3,?4)", params![
                            id,source.to_string_lossy(),target.to_string_lossy(),i64::try_from(size).context("IO_ERROR")?]).context("DATABASE_ERROR")?;
                        Ok(())
                    })?;
                    if result["cover_filename"].is_string() {
                        match crate::files::finalize_cover(&result, stage.path(), &file.path) {
                            Ok(path) => task.cover_path = Some(path),
                            Err(_) => task.cover_error = Some("COVER_UNAVAILABLE".into()),
                        }
                    } else {
                        task.cover_error = Some("COVER_UNAVAILABLE".into());
                    }
                    task.path = Some(file.path);
                    task.bytes = file.bytes;
                    task.total_bytes = Some(file.bytes);
                    task.completed_at = Some(now());
                    task.status = Status::Completed;
                    task.speed_bps = None;
                    task.revision += 1;
                    // Cancellation and file/DB commit share this lock.
                    publish(&mut inner, task, &*core.emit)?;
                    Ok(Value::Null)
                });
                stage.close().context("IO_ERROR")?;
                core.inner.lock().unwrap().db.execute("DELETE FROM task_staging WHERE task_id=?",[&id]).context("DATABASE_ERROR")?;
                committed
            }.await;
            // Preserve an already committed success, including when staging cleanup fails.
            if !core
                .list()
                .iter()
                .any(|t| t.id == id && t.status == Status::Completed)
            {
                core.finish(&id, result, true);
            }
            {
                let mut inner = core.inner.lock().unwrap();
                inner.active.remove(&id);
                inner.downloads.remove(&id);
                core.dispatch(&mut inner);
            }
            let _ = done.send(true);
        });
        Ok(())
    }

    pub async fn cancel(&self, id: &str) -> Result<()> {
        let mut done = {
            let mut inner = self.inner.lock().unwrap();
            let mut task = inner
                .tasks
                .iter()
                .find(|t| t.id == id)
                .context("TASK_NOT_FOUND")?
                .clone();
            if matches!(
                task.status,
                Status::Completed | Status::Failed | Status::Canceled
            ) {
                return Ok(());
            }
            if matches!(task.status, Status::Ready | Status::Queued) {
                task.status = Status::Canceled;
                task.revision += 1;
                publish(&mut inner, task, &*self.emit)?;
                inner.active.remove(id);
                return Ok(());
            }
            task.canceling = true;
            task.revision += 1;
            // Cancellation must still reach the process if persistence fails.
            let saved = publish(&mut inner, task.clone(), &*self.emit);
            if saved.is_err() {
                *inner.tasks.iter_mut().find(|t| t.id == id).unwrap() = task;
            }
            let active = inner.active.get(id).context("INVALID_STATE")?;
            let _ = active.cancel.send(true);
            active.done.clone()
        };
        let _ = done.wait_for(|v| *v).await;
        Ok(())
    }
    pub async fn shutdown(self: &Arc<Self>) {
        let active = {
            let mut inner = self.inner.lock().unwrap();
            inner.closing = true;
            inner
                .active
                .iter()
                .map(|(id, a)| (id.clone(), a.done.clone()))
                .collect::<Vec<_>>()
        };
        let cancellations = active
            .iter()
            .map(|(id, _)| {
                let core = self.clone();
                let id = id.clone();
                tokio::spawn(async move {
                    let _ = core.cancel(&id).await;
                })
            })
            .collect::<Vec<_>>();
        for cancellation in cancellations {
            let _ = cancellation.await;
        }
        for (_, mut done) in active {
            let _ = done.wait_for(|v| *v).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn core(root: &Path) -> Arc<Core> {
        let project = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let python = project.join(if cfg!(windows) {
            ".venv/Scripts/python.exe"
        } else {
            ".venv/bin/python"
        });
        Core::open(
            &root.join("tasks.sqlite"),
            root.join("downloads"),
            python.to_string_lossy().into_owned(),
            vec![project
                .join("tests/fixtures/desktop_engine.py")
                .to_string_lossy()
                .into_owned()],
            |_| {},
        )
        .unwrap()
    }
    async fn wait(core: &Core, id: &str, status: Status) -> Task {
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let task = core.list().into_iter().find(|t| t.id == id).unwrap();
                if task.status == status {
                    return task;
                }
                assert_ne!(task.status, Status::Failed, "{:?}", task.error);
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap()
    }
    #[tokio::test]
    async fn real_process_single_task_commit_and_restart() {
        let root = tempfile::tempdir().unwrap();
        let app = core(root.path());
        let task = app.resolve("valid".into()).unwrap();
        assert!(app.resolve("other".into()).is_err());
        let ready = wait(&app, &task.id, Status::Ready).await;
        assert!(ready.path.is_none());
        app.download(&task.id, false).unwrap();
        assert!(app.download(&task.id, false).is_err());
        let complete = wait(&app, &task.id, Status::Completed).await;
        let path = complete.path.unwrap();
        assert!(path.is_file());
        assert_eq!(complete.bytes, std::fs::metadata(&path).unwrap().len());
        app.shutdown().await;
        drop(app);
        let restarted = core(root.path());
        assert_eq!(restarted.list()[0].status, Status::Completed);
        std::fs::remove_file(path).unwrap();
        assert!(restarted.list()[0].file_missing);
    }
    #[tokio::test]
    async fn cancellation_waits_for_exit_and_removes_partial_file() {
        let root = tempfile::tempdir().unwrap();
        let app = core(root.path());
        let parsing = app.resolve("block".into()).unwrap();
        app.cancel(&parsing.id).await.unwrap();
        assert_eq!(app.list()[0].status, Status::Canceled);
        let task = app.resolve("slow".into()).unwrap();
        wait(&app, &task.id, Status::Ready).await;
        app.download(&task.id, false).unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            while app.list().iter().find(|t| t.id == task.id).unwrap().bytes == 0 {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        app.cancel(&task.id).await.unwrap();
        assert_eq!(
            app.list().iter().find(|t| t.id == task.id).unwrap().status,
            Status::Canceled
        );
        assert_eq!(
            std::fs::read_dir(root.path().join("downloads"))
                .unwrap()
                .count(),
            0
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(
            std::fs::read_dir(root.path().join("downloads"))
                .unwrap()
                .count(),
            0
        );
    }
    #[tokio::test]
    async fn ready_restart_becomes_interrupted_and_input_is_not_stored() {
        let root = tempfile::tempdir().unwrap();
        let app = core(root.path());
        let task = app.resolve("private share text".into()).unwrap();
        wait(&app, &task.id, Status::Ready).await;
        drop(app);
        let restarted = core(root.path());
        assert_eq!(restarted.list()[0].error.as_deref(), Some("INTERRUPTED"));
        let db = Connection::open(root.path().join("tasks.sqlite")).unwrap();
        let text: String = db
            .query_row("SELECT snapshot FROM download_tasks", [], |r| r.get(0))
            .unwrap();
        assert!(!text.contains("private share text"));
    }
    #[test]
    fn recovery_requires_commit_evidence_and_database_failure_prevents_move() {
        let root = tempfile::tempdir().unwrap();
        let app = core(root.path());
        let source = root.path().join("video.mp4");
        let target = root.path().join("finished.mp4");
        std::fs::write(&target, b"complete").unwrap();
        let mut task = Task {
            id: "recover".into(),
            revision: 1,
            status: Status::Downloading,
            video: None,
            bytes: 0,
            total_bytes: None,
            speed_bps: None,
            error: None,
            path: None,
            created_at: now(),
            completed_at: None,
            canceling: false,
            file_missing: false,
            queue_order: None,
            download_requested: false,
            cover_path: None,
            cover_error: None,
        };
        {
            let inner = app.inner.lock().unwrap();
            persist(&inner.db, &task).unwrap();
            inner
                .db
                .execute(
                    "INSERT INTO commit_intents VALUES(?1,?2,?3,8)",
                    params![task.id, source.to_string_lossy(), target.to_string_lossy()],
                )
                .unwrap();
            task.id = "unproved".into();
            persist(&inner.db, &task).unwrap();
        }
        drop(app);
        let app = core(root.path());
        assert_eq!(
            app.list()
                .iter()
                .find(|t| t.id == "recover")
                .unwrap()
                .status,
            Status::Completed
        );
        assert_eq!(
            app.list()
                .iter()
                .find(|t| t.id == "unproved")
                .unwrap()
                .status,
            Status::Failed
        );
        std::fs::write(&source, b"\0\0\0\x18ftypmp42").unwrap();
        let identity = Video {
            platform: "douyin".into(),
            video_id: "123".into(),
            canonical_url: String::new(),
        };
        let result = json!({"platform":"douyin","video_id":"123","container":"mp4","filename":"video.mp4","bytes":12});
        assert!(finalize_to(
            &identity,
            &result,
            root.path(),
            &root.path().join("new.mp4"),
            |_, _| bail!("DATABASE_ERROR")
        )
        .is_err());
        assert!(!root.path().join("new.mp4").exists());
        assert!(source.exists());
    }
    async fn enqueue(app: &Arc<Core>, video_id: &str) -> Task {
        let task = app.resolve(format!("hold:{video_id}")).unwrap();
        wait(app, &task.id, Status::Ready).await;
        app.download(&task.id, false).unwrap();
        task
    }
    async fn started(root: &Path, video_id: &str) {
        tokio::time::timeout(Duration::from_secs(5), async {
            while !root.join(format!("started-{video_id}")).exists() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
    }
    fn release(root: &Path, video_id: &str) {
        std::fs::write(root.join(format!("release-{video_id}")), b"go").unwrap();
    }

    #[tokio::test]
    async fn fifo_completion_failure_and_queued_cancellation_release_slots() {
        let root = tempfile::tempdir().unwrap();
        let app = core(root.path());
        assert_eq!(app.settings().max_downloads, 2);
        let a = enqueue(&app, "8001").await;
        let b = enqueue(&app, "7002").await;
        started(root.path(), "8001").await;
        started(root.path(), "7002").await;
        let c = enqueue(&app, "8003").await;
        let d = enqueue(&app, "8004").await;
        let e = enqueue(&app, "8005").await;
        assert_eq!(
            app.list()
                .iter()
                .filter(|t| t.status == Status::Downloading)
                .count(),
            2
        );
        assert_eq!(
            app.list()
                .iter()
                .filter(|t| t.status == Status::Queued)
                .count(),
            3
        );
        assert!(!root.path().join("started-8003").exists());
        assert!(app.download(&c.id, false).is_err());
        app.cancel(&d.id).await.unwrap();
        release(root.path(), "8001");
        wait(&app, &a.id, Status::Completed).await;
        started(root.path(), "8003").await;
        assert!(!root.path().join("started-8004").exists());
        assert!(!root.path().join("started-8005").exists());
        release(root.path(), "7002");
        tokio::time::timeout(Duration::from_secs(5), async {
            while app.list().iter().find(|t| t.id == b.id).unwrap().status != Status::Failed {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            app.list()
                .iter()
                .find(|t| t.id == b.id)
                .unwrap()
                .error
                .as_deref(),
            Some("NETWORK_ERROR")
        );
        started(root.path(), "8005").await;
        app.cancel(&c.id).await.unwrap();
        assert_eq!(
            app.list().iter().find(|t| t.id == e.id).unwrap().status,
            Status::Downloading
        );
        release(root.path(), "8005");
        wait(&app, &e.id, Status::Completed).await;
        app.shutdown().await;
        assert_eq!(
            std::fs::read_dir(root.path().join("downloads"))
                .unwrap()
                .count(),
            2
        );
    }

    #[tokio::test]
    async fn changing_limit_preserves_running_tasks_and_persists_without_resetting_directory() {
        let root = tempfile::tempdir().unwrap();
        let app = core(root.path());
        app.set_max_downloads(1).unwrap();
        let a = enqueue(&app, "8101").await;
        let b = enqueue(&app, "8102").await;
        let c = enqueue(&app, "8103").await;
        let d = enqueue(&app, "8104").await;
        assert_eq!(
            app.list()
                .iter()
                .filter(|t| t.status == Status::Downloading)
                .count(),
            1
        );
        assert!(app.set_max_downloads(0).is_err());
        assert!(app.set_max_downloads(4).is_err());
        assert_eq!(app.settings().max_downloads, 1);
        app.set_max_downloads(3).unwrap();
        for id in ["8101", "8102", "8103"] {
            started(root.path(), id).await;
        }
        assert!(!root.path().join("started-8104").exists());
        app.set_max_downloads(1).unwrap();
        assert_eq!(
            app.list()
                .iter()
                .filter(|t| t.status == Status::Downloading)
                .count(),
            3
        );
        app.cancel(&a.id).await.unwrap();
        app.cancel(&b.id).await.unwrap();
        assert_eq!(
            app.list().iter().find(|t| t.id == c.id).unwrap().status,
            Status::Downloading
        );
        assert_eq!(
            app.list().iter().find(|t| t.id == d.id).unwrap().status,
            Status::Queued
        );
        app.cancel(&c.id).await.unwrap();
        started(root.path(), "8104").await;
        app.set_max_downloads(2).unwrap();
        app.set_directory(root.path().to_owned()).unwrap();
        assert_eq!(app.settings().max_downloads, 2);
        app.shutdown().await;
        assert!(app.list().iter().all(|t| t.status == Status::Canceled));
        drop(app);
        let app = core(root.path());
        assert_eq!(app.settings().max_downloads, 2);
        assert_eq!(
            app.settings().download_directory,
            root.path().canonicalize().unwrap()
        );
    }

    #[tokio::test]
    async fn queued_task_keeps_its_configuration_and_shutdown_never_dispatches_it() {
        let root = tempfile::tempdir().unwrap();
        let app = core(root.path());
        app.set_max_downloads(1).unwrap();
        let a = enqueue(&app, "8201").await;
        let old = app.settings().profiles[0].user_agent.clone();
        let b = enqueue(&app, "8202").await;
        app.save_profile(settings::ProfileUpdate {
            platform: "douyin".into(),
            user_agent: "new-agent".into(),
            cookie: None,
            proxy: None,
        })
        .unwrap();
        app.cancel(&a.id).await.unwrap();
        started(root.path(), "8202").await;
        let captured: Value = serde_json::from_str(
            &std::fs::read_to_string(root.path().join("started-8202")).unwrap(),
        )
        .unwrap();
        assert_eq!(captured["douyin"]["user_agent"], old);
        let c = enqueue(&app, "8203").await;
        let parsing = app.resolve("block".into()).unwrap();
        app.shutdown().await;
        for id in [&b.id, &c.id, &parsing.id] {
            assert_eq!(
                app.list().iter().find(|t| &t.id == id).unwrap().status,
                Status::Canceled
            );
        }
        assert!(!root.path().join("started-8203").exists());
        assert_eq!(
            std::fs::read_dir(root.path().join("downloads"))
                .unwrap()
                .count(),
            0
        );
    }

    #[tokio::test]
    async fn restart_interrupts_persisted_queue_without_starting_engine() {
        let root = tempfile::tempdir().unwrap();
        let app = core(root.path());
        let task = app.resolve("valid".into()).unwrap();
        let mut ready = wait(&app, &task.id, Status::Ready).await;
        ready.status = Status::Queued;
        ready.queue_order = Some(1);
        persist(&app.inner.lock().unwrap().db, &ready).unwrap();
        drop(app);
        let app = core(root.path());
        assert_eq!(app.list()[0].status, Status::Failed);
        assert_eq!(app.list()[0].error.as_deref(), Some("INTERRUPTED"));
        assert!(app.inner.lock().unwrap().active.is_empty());
    }
    #[tokio::test]
    async fn retry_reparses_with_new_settings_and_preserves_old_record() {
        let root = tempfile::tempdir().unwrap();
        let app = core(root.path());
        let old = enqueue(&app, "8301").await;
        started(root.path(), "8301").await;
        assert!(app.remove_history(&old.id).is_err());
        assert!(app.delete_local_file(&old.id).is_err());
        app.cancel(&old.id).await.unwrap();
        std::fs::remove_file(root.path().join("started-8301")).unwrap();
        app.save_profile(settings::ProfileUpdate {
            platform: "douyin".into(),
            user_agent: "retry-agent".into(),
            cookie: None,
            proxy: None,
        })
        .unwrap();
        let retry = app.retry(&old.id).unwrap();
        assert_ne!(retry.id, old.id);
        assert!(retry.download_requested);
        started(root.path(), "8301").await;
        let profile: Value = serde_json::from_str(
            &std::fs::read_to_string(root.path().join("started-8301")).unwrap(),
        )
        .unwrap();
        assert_eq!(profile["douyin"]["user_agent"], "retry-agent");
        assert_eq!(
            app.list().iter().find(|t| t.id == old.id).unwrap().status,
            Status::Canceled
        );
        release(root.path(), "8301");
        wait(&app, &retry.id, Status::Completed).await;
        app.shutdown().await;
        assert!(app.retry(&retry.id).is_err());
    }

    #[tokio::test]
    async fn history_removal_preserves_file_and_deletion_preserves_history() {
        let root = tempfile::tempdir().unwrap();
        let app = core(root.path());
        let first = app.resolve("valid".into()).unwrap();
        wait(&app, &first.id, Status::Ready).await;
        app.download(&first.id, false).unwrap();
        let completed = wait(&app, &first.id, Status::Completed).await;
        app.shutdown().await;
        let path = completed.path.unwrap();
        app.inner.lock().unwrap().db.execute_batch("CREATE TRIGGER reject_removal BEFORE DELETE ON download_tasks BEGIN SELECT RAISE(ABORT, 'test'); END;").unwrap();
        assert!(app.remove_history(&first.id).is_err());
        assert!(app.list().iter().any(|t| t.id == first.id));
        assert!(path.exists());
        app.inner
            .lock()
            .unwrap()
            .db
            .execute_batch("DROP TRIGGER reject_removal")
            .unwrap();
        app.remove_history(&first.id).unwrap();
        assert!(path.exists());
        drop(app);
        let app = core(root.path());
        assert!(app.list().is_empty());
        let second = app.resolve("valid".into()).unwrap();
        wait(&app, &second.id, Status::Ready).await;
        app.download(&second.id, true).unwrap();
        let completed = wait(&app, &second.id, Status::Completed).await;
        app.shutdown().await;
        let second_path = completed.path.unwrap();
        std::fs::remove_file(&second_path).unwrap();
        std::fs::create_dir(&second_path).unwrap();
        assert!(app.delete_local_file(&second.id).is_err());
        assert!(second_path.is_dir());
        std::fs::remove_dir(&second_path).unwrap();
        std::fs::write(&second_path, b"video").unwrap();
        app.delete_local_file(&second.id).unwrap();
        app.delete_local_file(&second.id).unwrap();
        assert!(!second_path.exists());
        assert!(path.exists());
        drop(app);
        let app = core(root.path());
        assert_eq!(app.list().len(), 1);
        assert_eq!(app.list()[0].status, Status::Completed);
        assert!(app.list()[0].file_missing);
    }

    #[tokio::test]
    async fn canceled_parse_requires_new_input() {
        let root = tempfile::tempdir().unwrap();
        let app = core(root.path());
        let task = app.resolve("block".into()).unwrap();
        app.cancel(&task.id).await.unwrap();
        assert_eq!(
            app.retry(&task.id).err().unwrap().to_string(),
            "REPARSE_REQUIRED"
        );
        app.remove_history(&task.id).unwrap();
        assert!(app.list().is_empty());
    }
    #[tokio::test]
    async fn previews_are_not_downloads_and_legacy_submissions_survive_restart() {
        let root = tempfile::tempdir().unwrap();
        let app = core(root.path());
        let preview = app.resolve("valid".into()).unwrap();
        assert!(!preview.download_requested);
        let ready = wait(&app, &preview.id, Status::Ready).await;
        assert!(!ready.download_requested);
        app.cancel(&preview.id).await.unwrap();
        assert!(!app.list()[0].download_requested);
        let task = app.resolve("valid".into()).unwrap();
        wait(&app, &task.id, Status::Ready).await;
        app.download(&task.id, false).unwrap();
        assert!(
            app.list()
                .iter()
                .find(|t| t.id == task.id)
                .unwrap()
                .download_requested
        );
        wait(&app, &task.id, Status::Completed).await;
        app.shutdown().await;
        // Remove the new field to exercise migration of pre-fix snapshots.
        {
            let inner = app.inner.lock().unwrap();
            for task in &inner.tasks {
                let mut snapshot = serde_json::to_value(task).unwrap();
                snapshot
                    .as_object_mut()
                    .unwrap()
                    .remove("download_requested");
                inner
                    .db
                    .execute(
                        "UPDATE download_tasks SET snapshot=? WHERE id=?",
                        params![snapshot.to_string(), task.id],
                    )
                    .unwrap();
            }
        }
        drop(app);
        let app = core(root.path());
        let tasks = app.list();
        assert!(
            !tasks
                .iter()
                .find(|t| t.id == preview.id)
                .unwrap()
                .download_requested
        );
        assert!(
            tasks
                .iter()
                .find(|t| t.id == task.id)
                .unwrap()
                .download_requested
        );
    }
    #[tokio::test]
    async fn downloaded_cover_is_available_after_restart_and_video_deletion() {
        let root = tempfile::tempdir().unwrap();
        let app = core(root.path());
        let task = app
            .resolve("https://www.douyin.com/video/456".into())
            .unwrap();
        wait(&app, &task.id, Status::Ready).await;
        app.download(&task.id, false).unwrap();
        let done = wait(&app, &task.id, Status::Completed).await;
        assert!(done.cover_path.unwrap().is_file());
        assert!(done.cover_error.is_none());
        app.shutdown().await;
        drop(app);
        let app = core(root.path());
        assert_eq!(app.cover(&task.id).unwrap(), b"\x89PNG\r\n\x1a\nfixture");
        app.delete_local_file(&task.id).unwrap();
        assert!(app.cover(&task.id).is_ok());
        app.remove_history(&task.id).unwrap();
        assert!(app.cover(&task.id).is_err());
    }
    #[tokio::test]
    async fn duplicate_requires_confirmation_and_retry_cannot_bypass_it() {
        let root = tempfile::tempdir().unwrap();
        let app = core(root.path());
        let first = app.resolve("valid".into()).unwrap();
        wait(&app, &first.id, Status::Ready).await;
        app.download(&first.id, false).unwrap();
        let original = wait(&app, &first.id, Status::Completed).await;
        let second = app.resolve("valid".into()).unwrap();
        wait(&app, &second.id, Status::Ready).await;
        assert_eq!(
            app.download(&second.id, false).err().unwrap().to_string(),
            "ALREADY_DOWNLOADED"
        );
        let blocked = app.list().into_iter().find(|t| t.id == second.id).unwrap();
        assert_eq!(blocked.status, Status::Ready);
        assert!(!blocked.download_requested);
        app.cancel(&second.id).await.unwrap();
        let retry = app.retry(&second.id).unwrap();
        wait(&app, &retry.id, Status::Ready).await;
        assert_eq!(
            app.download(&retry.id, false).err().unwrap().to_string(),
            "ALREADY_DOWNLOADED"
        );
        app.download(&retry.id, true).unwrap();
        let repeated = wait(&app, &retry.id, Status::Completed).await;
        assert_ne!(original.path, repeated.path);
        assert!(original.path.unwrap().is_file());
        let mut other_platform = repeated.clone();
        other_platform.id = "other-platform".into();
        other_platform.video.as_mut().unwrap()["platform"] = json!("tiktok");
        assert!(!previously_downloaded(
            std::slice::from_ref(&repeated),
            &other_platform
        ));
        let mut failed = repeated.clone();
        failed.id = "failed-copy".into();
        failed.status = Status::Failed;
        assert!(!previously_downloaded(&[failed], &repeated));
        app.shutdown().await;
        drop(app);
        let app = core(root.path());
        let after_restart = app.resolve("valid".into()).unwrap();
        wait(&app, &after_restart.id, Status::Ready).await;
        assert_eq!(
            app.download(&after_restart.id, false)
                .err()
                .unwrap()
                .to_string(),
            "ALREADY_DOWNLOADED"
        );
        app.shutdown().await;
    }
}
