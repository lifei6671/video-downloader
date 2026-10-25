use crate::{
    files::{finalize_album, finalize_to, validate_album, AlbumFile, Video},
    parse_cache,
    python_bridge::PythonBridge,
    settings::{self, Settings},
    tdl_bridge::{self, TdlSettings},
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
#[path = "transcription.rs"]
pub mod transcription;

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
    #[serde(default)]
    pub total_bytes_estimate: Option<u64>,
    pub speed_bps: Option<f64>,
    #[serde(default)]
    pub eta_seconds: Option<f64>,
    #[serde(default)]
    pub merging: bool,
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
    #[serde(default)]
    pub files: Vec<AlbumFile>,
    #[serde(default)]
    pub completed_items: Option<u64>,
    #[serde(default)]
    pub total_items: Option<u64>,
    #[serde(default)]
    pub cache_hit: bool,
    #[serde(default)]
    pub cached_at: Option<u64>,
    #[serde(default)]
    pub cover_cached: bool,
    // Only a validated, normalized Telegram URL; never the pasted share text.
    #[serde(default)]
    pub telegram_link: Option<String>,
}

#[derive(Default)]
struct YoutubeTransferRate {
    sample: Option<(Instant, u64)>,
    speed: Option<f64>,
}

impl YoutubeTransferRate {
    fn update(&mut self, event: &mut Value, at: Instant) {
        let count = event["bytes"].as_u64().unwrap_or(0);
        if event["phase"] == "merging" {
            *self = Self::default();
        } else if let Some((started, bytes)) = self.sample {
            let elapsed = at.duration_since(started);
            if elapsed >= Duration::from_secs(1) {
                // Aggregate counters stay monotonic across HTTP chunks and audio/video streams.
                self.speed = Some((count - bytes) as f64 / elapsed.as_secs_f64());
                self.sample = Some((at, count));
            }
        } else {
            self.sample = Some((at, count));
        }
        let eta = self.speed.filter(|speed| *speed > 0.0).and_then(|speed| {
            event["total_bytes"].as_u64().or_else(|| event["total_bytes_estimate"].as_u64())
                .map(|total| total.saturating_sub(count) as f64 / speed)
                .filter(|eta| *eta <= 604800.0)
        });
        event["speed_bps"] = self.speed.into();
        event["eta_seconds"] = eta.into();
    }
}

fn file_missing(task: &Task) -> bool {
    task.path.as_ref().is_none_or(|path| {
        if task.files.is_empty() {
            !path.is_file()
        } else {
            !std::fs::symlink_metadata(path)
                .is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink())
                || task.files.iter().any(|file| {
                    !std::fs::symlink_metadata(path.join(&file.filename)).is_ok_and(|m| {
                        m.is_file() && !m.file_type().is_symlink() && m.len() == file.bytes
                    })
                })
        }
    })
}

fn same_download_variant(old: &Value, video: &Value) -> bool {
    let old_kind = old["kind"].as_str().unwrap_or("video");
    let kind = video["kind"].as_str().unwrap_or("video");
    old["platform"] == video["platform"]
        && old["video_id"] == video["video_id"]
        && old_kind == kind
        && (kind == "image_album" || video["platform"] == "telegram"
            // Legacy records without a format represent an unspecified variant.
            || old["selected_format_id"].as_str().is_none()
            || video["selected_format_id"].as_str().is_none()
            || old["selected_format_id"] == video["selected_format_id"])
}

fn previously_downloaded(tasks: &[Task], task: &Task) -> bool {
    task.video.as_ref().is_some_and(|video| {
        tasks.iter().any(|old| {
            old.id != task.id
                && old.status == Status::Completed
                && !file_missing(old)
                && old
                    .video
                    .as_ref()
                    .is_some_and(|v| same_download_variant(v, video))
        })
    })
}

pub enum TaskEvent {
    Updated(Box<Task>),
    Removed(String),
    Transcription(Box<transcription::Snapshot>),
}

struct Active {
    tdl: Option<TdlSettings>,
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
    workers: Vec<tokio::task::JoinHandle<()>>,
    closing: bool,
    dependency_cancel: Option<watch::Sender<bool>>,
    transcription: Option<transcription::ActiveTranscription>,
    transcription_unsaved: HashMap<String, transcription::Snapshot>,
    tdl_login_cancel: Option<watch::Sender<bool>>,
    tdl_login_control: Option<tdl_bridge::LoginControl>,
}
pub struct Core {
    _instance_lock: std::fs::File,
    inner: Mutex<Inner>,
    program: String,
    args: Vec<String>,
    runtime_directory: PathBuf,
    tdl_session: tdl_bridge::Session,
    emit: Arc<dyn Fn(TaskEvent) + Send + Sync>,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before epoch")
        .as_secs()
}
fn remove_staging_directory(path: &Path) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        // A terminating native process or filesystem filter can briefly retain a
        // handle even after the Job has no active processes. Retry only these
        // Windows sharing/lock violations, for at most 310 ms in total.
        let mut delay = 10;
        loop {
            match std::fs::remove_dir_all(path) {
                Err(error) if matches!(error.raw_os_error(), Some(32 | 33)) && delay <= 160 => {
                    std::thread::sleep(std::time::Duration::from_millis(delay));
                    delay *= 2;
                }
                result => return result,
            }
        }
    }
    #[cfg(not(windows))]
    std::fs::remove_dir_all(path)
}

fn cleanup_staging(db: &Connection, id: &str, path: &Path) -> Result<()> {
    if path
        .file_name()
        .is_none_or(|n| !n.to_string_lossy().starts_with(&format!("yingqu-{id}-")))
    {
        bail!("INVALID_STAGING_PATH");
    }
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                if metadata.file_attributes() & 0x400 != 0 {
                    bail!("INVALID_STAGING_PATH");
                }
            }
            remove_staging_directory(path).context("IO_ERROR")?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        _ => bail!("INVALID_STAGING_PATH"),
    }
    db.execute("DELETE FROM task_staging WHERE task_id=?", [id])
        .context("DATABASE_ERROR")?;
    Ok(())
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

fn apply_video_format(video: &mut Value, format_id: &str) -> Result<()> {
    let format = video["formats"]
        .as_array()
        .context("FORMAT_UNAVAILABLE")?
        .iter()
        .find(|f| f["format_id"].as_str() == Some(format_id))
        .cloned()
        .context("FORMAT_UNAVAILABLE")?;
    for field in ["width", "height", "filesize", "filesize_approx"] {
        video[field] = format[field].clone();
    }
    video["selected_format_id"] = format_id.into();
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
        let mut db = Connection::open(db_path).context("DATABASE_ERROR")?;
        db.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;
            CREATE TABLE IF NOT EXISTS download_tasks(id TEXT PRIMARY KEY,snapshot TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS commit_intents(task_id TEXT PRIMARY KEY,source TEXT NOT NULL,
                target TEXT NOT NULL,size INTEGER NOT NULL);
            CREATE TABLE IF NOT EXISTS task_staging(task_id TEXT PRIMARY KEY,path TEXT NOT NULL);",
        )
        .context("DATABASE_ERROR")?;
        let has_manifest = db
            .prepare("PRAGMA table_info(commit_intents)")?
            .query_map([], |r| r.get::<_, String>(1))?
            .collect::<rusqlite::Result<Vec<_>>>()?
            .iter()
            .any(|column| column == "manifest");
        if !has_manifest {
            db.execute("ALTER TABLE commit_intents ADD COLUMN manifest TEXT", [])?;
        }
        let migrate_cache = parse_cache::init(&db)?;
        transcription::init(&db)?;
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
        // Seed reusable metadata/previews from existing records where assets exist.
        let has_assets = |task: &&Task| {
            !task.files.is_empty() || task.cover_path.as_ref().is_some_and(|path| path.is_file())
        };
        let legacy = tasks.iter().filter(|_| migrate_cache);
        for task in legacy
            .clone()
            .filter(has_assets)
            .chain(legacy.filter(|task| !has_assets(task)))
        {
            let Some(video) = task.video.as_ref() else {
                continue;
            };
            let identity: Video =
                serde_json::from_value(video.clone()).context("DATABASE_ERROR")?;
            identity.validate()?;
            if parse_cache::get(
                &db,
                &identity.platform,
                &identity.video_id,
                &identity.resolver,
            )?
            .is_some()
            {
                continue;
            }
            let images = if identity.kind == "image_album" {
                let Some(path) = task.path.as_ref().filter(|_| !task.files.is_empty()) else {
                    continue;
                };
                if validate_album(path, &task.files).is_err() {
                    continue;
                }
                task.files
                    .iter()
                    .map(|f| crate::files::read_image(&path.join(&f.filename)))
                    .collect::<Result<Vec<_>>>()?
            } else {
                Vec::new()
            };
            let cover = task
                .cover_path
                .as_ref()
                .and_then(|p| crate::files::read_cover(p).ok());
            parse_cache::save(&mut db, video, images, cover, task.created_at)?;
        }
        for task in &mut tasks {
            if task.status == Status::Ready {
                if let Some(video) = task.video.as_ref() {
                    let identity: Video =
                        serde_json::from_value(video.clone()).context("DATABASE_ERROR")?;
                    if let Some(mut cached) = parse_cache::get(
                        &db,
                        &identity.platform,
                        &identity.video_id,
                        &identity.resolver,
                    )? {
                        if let Some(format_id) = video["selected_format_id"].as_str() {
                            if apply_video_format(&mut cached.video, format_id).is_err() {
                                task.status = Status::Failed;
                                task.error = Some("FORMAT_UNAVAILABLE".into());
                                task.revision += 1;
                                persist(&db, task)?;
                                continue;
                            }
                        }
                        task.video = Some(cached.video);
                        task.cache_hit = true;
                        task.cached_at = Some(cached.saved_at);
                        task.cover_cached = cached.has_cover;
                        task.revision += 1;
                        persist(&db, task)?;
                        continue;
                    }
                }
            }
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
                task.merging = false;
                task.revision += 1;
                let intent = db.query_row(
                    "SELECT source,target,size,manifest FROM commit_intents WHERE task_id=?",
                    [&task.id],
                    |r| {
                        Ok((
                            r.get::<_, String>(0)?,
                            r.get::<_, String>(1)?,
                            r.get::<_, i64>(2)? as u64,
                            r.get::<_, Option<String>>(3)?,
                        ))
                    },
                );
                match intent {
                    Ok((source, target, size, manifest))
                        if !Path::new(&source).exists()
                            && (if let Some(files) = manifest.as_ref() {
                                serde_json::from_str::<Vec<AlbumFile>>(files)
                                    .ok()
                                    .is_some_and(|files| {
                                        validate_album(Path::new(&target), &files)
                                            .is_ok_and(|n| n == size)
                                    })
                            } else {
                                std::fs::symlink_metadata(&target).is_ok_and(|m| {
                                    m.is_file()
                                        && !m.file_type().is_symlink()
                                        && m.len() == size
                                        && (task
                                            .video
                                            .as_ref()
                                            .is_none_or(|v| v["platform"] != "telegram")
                                            || crate::files::tdl_recovery_valid(
                                                Path::new(&target),
                                                size,
                                            ))
                                })
                            }) =>
                    {
                        task.status = Status::Completed;
                        task.error = None;
                        if let Some(files) = manifest {
                            task.files = serde_json::from_str(&files).context("DATABASE_ERROR")?;
                            if let Some(video) = task.video.as_mut() {
                                video["images"] = serde_json::to_value(&task.files)?;
                            }
                            task.completed_items = Some(task.files.len() as u64);
                            task.total_items = task.completed_items;
                        }
                        let target = PathBuf::from(target);
                        task.cover_path = ["jpg", "png", "webp"]
                            .iter()
                            .map(|ext| target.with_extension(format!("cover.{ext}")))
                            .find(|path| path.is_file());
                        task.path = Some(target);
                        task.bytes = size;
                        task.total_bytes = Some(size);
                        task.completed_at = Some(now());
                    }
                    Ok(_) | Err(rusqlite::Error::QueryReturnedNoRows) => {}
                    Err(_) => bail!("DATABASE_ERROR"),
                }
                persist(&db, task)?;
            }
            // Also restore the cache flag if a crash occurred between the video
            // file commit / cover cache write and publishing the completed task.
            if task.status == Status::Completed && task.cover_path.is_none() {
                if let Some(video) = task
                    .video
                    .as_ref()
                    .filter(|v| v["kind"].as_str().unwrap_or("video") == "video")
                {
                    let cached = parse_cache::cover(&db, video).is_ok();
                    if task.cover_cached != cached {
                        task.cover_cached = cached;
                        task.revision += 1;
                        persist(&db, task)?;
                    }
                }
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
            cleanup_staging(&db, &id, Path::new(&path))?;
        }
        Ok(Arc::new(Self {
            _instance_lock: instance_lock,
            inner: Mutex::new(Inner {
                db,
                tasks,
                settings,
                active: HashMap::new(),
                downloads: HashSet::new(),
                workers: Vec::new(),
                closing: false,
                dependency_cancel: None,
                transcription: None,
                transcription_unsaved: HashMap::new(),
                tdl_login_cancel: None,
                tdl_login_control: None,
            }),
            program,
            args,
            runtime_directory: db_path.parent().context("IO_ERROR")?.join("runtime"),
            tdl_session: tdl_bridge::Session::default(),
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
                t.file_missing = t.status == Status::Completed && file_missing(&t);
                t
            })
            .collect()
    }
    pub fn settings(&self) -> Settings {
        self.inner.lock().unwrap().settings.clone()
    }
    pub fn save_tdl(&self, mut config: TdlSettings) -> Result<Settings> {
        if config
            .path
            .as_ref()
            .is_some_and(|p| !p.is_absolute() || !p.is_file())
        {
            bail!("TDL_UNAVAILABLE");
        }
        let mut inner = self.inner.lock().unwrap();
        if inner.closing {
            bail!("INVALID_STATE");
        }
        if inner.tdl_login_cancel.is_some() {
            bail!("TDL_BUSY");
        }
        config.session_directory = inner.settings.tdl.session_directory.clone();
        inner.db.execute("INSERT INTO tdl_settings(id,enabled,path) VALUES(1,?1,?2) ON CONFLICT(id) DO UPDATE SET enabled=excluded.enabled,path=excluded.path",
            params![config.enabled, config.path.as_ref().map(|p| p.to_string_lossy().into_owned())]).context("DATABASE_ERROR")?;
        inner.settings.tdl = config;
        Ok(inner.settings.clone())
    }
    pub fn tdl_login_running(&self) -> bool {
        self.inner.lock().unwrap().tdl_login_cancel.is_some()
    }
    pub fn tdl_login_status(&self) -> Option<tdl_bridge::LoginStatus> {
        self.inner
            .lock()
            .unwrap()
            .tdl_login_control
            .as_ref()
            .map(|control| control.state.borrow().clone())
    }
    pub fn submit_tdl_password(
        &self,
        login_id: &str,
        password: zeroize::Zeroizing<String>,
    ) -> Result<()> {
        let inner = self.inner.lock().unwrap();
        if inner.closing {
            bail!("INVALID_STATE");
        }
        inner
            .tdl_login_control
            .as_ref()
            .context("INVALID_STATE")?
            .submit(login_id, password)
    }
    pub fn cancel_tdl_login(&self) -> Result<()> {
        if let Some(cancel) = &self.inner.lock().unwrap().tdl_login_cancel {
            let _ = cancel.send(true);
        }
        Ok(())
    }
    pub async fn login_tdl(self: &Arc<Self>, path: Option<PathBuf>) -> Result<Settings> {
        let (send, receive) = tokio::sync::oneshot::channel();
        {
            let mut inner = self.inner.lock().unwrap();
            if inner.closing {
                bail!("INVALID_STATE");
            }
            if inner.tdl_login_cancel.is_some() || inner.active.values().any(|a| a.tdl.is_some()) {
                bail!("TDL_BUSY");
            }
            let (cancel, receiver) = watch::channel(false);
            inner.tdl_login_cancel = Some(cancel);
            let (control, input) = tdl_bridge::LoginControl::new();
            let state = control.state.clone();
            inner.tdl_login_control = Some(control);
            let core = self.clone();
            let worker = tokio::spawn(async move {
                let result = tdl_bridge::login(
                    path.as_deref(),
                    &core.runtime_directory.join("tdl-sessions"),
                    receiver,
                    state,
                    input,
                )
                .await;
                let mut inner = core.inner.lock().unwrap();
                let result = result.and_then(|scratch| {
                    if inner.closing || inner.tdl_login_cancel.as_ref().is_some_and(|c| *c.borrow()) {
                        bail!("CANCELED");
                    }
                    let directory = scratch.path().join("data");
                    let transaction = inner.db.transaction().context("DATABASE_ERROR")?;
                    transaction.execute("INSERT INTO tdl_settings(id,enabled,path,session_directory) VALUES(1,0,?1,?2) ON CONFLICT(id) DO UPDATE SET path=excluded.path,session_directory=excluded.session_directory",
                        params![path.as_ref().map(|p| p.to_string_lossy().into_owned()), directory.to_string_lossy()]).context("DATABASE_ERROR")?;
                    // Metadata learned under another account must not serve as a new preview.
                    transaction.execute_batch("DELETE FROM parse_cache_images WHERE cache_key IN (SELECT cache_key FROM parse_cache WHERE platform='telegram'); DELETE FROM parse_cache WHERE platform='telegram'; DELETE FROM parse_cache_aliases WHERE platform='telegram';").context("DATABASE_ERROR")?;
                    transaction.commit().context("DATABASE_ERROR")?;
                    inner.settings.tdl.path = path;
                    inner.settings.tdl.session_directory = Some(directory);
                    let _ = scratch.keep();
                    Ok(inner.settings.clone())
                });
                inner.tdl_login_cancel = None;
                inner.tdl_login_control = None;
                let _ = send.send(result);
            });
            inner.workers.retain(|worker| !worker.is_finished());
            inner.workers.push(worker);
        }
        receive.await.context("TDL_LOGIN_FAILED")?
    }
    pub fn cookie(&self, platform: &str) -> Result<String> {
        settings::cookie(&self.inner.lock().unwrap().settings, platform)
    }
    pub fn proxy(&self, platform: &str) -> Result<Option<String>> {
        settings::proxy(&self.inner.lock().unwrap().settings, platform)
    }

    pub fn cancel_dependency(&self) -> Result<()> {
        self.inner
            .lock()
            .unwrap()
            .dependency_cancel
            .as_ref()
            .context("INVALID_STATE")?
            .send(true)
            .context("INVALID_STATE")
    }

    pub async fn dependencies(
        self: &Arc<Self>,
        tool: Option<String>,
        progress: impl FnMut(Value) + Send + 'static,
    ) -> Result<Value> {
        self.dependencies_variant(tool, None, progress).await
    }

    pub async fn dependencies_variant(
        self: &Arc<Self>,
        tool: Option<String>,
        variant: Option<String>,
        progress: impl FnMut(Value) + Send + 'static,
    ) -> Result<Value> {
        if tool
            .as_ref()
            .is_some_and(|v| !matches!(v.as_str(), "ffmpeg" | "javascript" | "fun-asr-nano"))
        {
            bail!("INVALID_INPUT");
        }
        if variant.as_ref().is_some_and(|v| !matches!(v.as_str(), "cpu" | "cuda"))
            || (variant.is_some() && tool.as_deref() != Some("fun-asr-nano"))
        {
            bail!("INVALID_INPUT");
        }
        let (cancel, mut cancellation) = watch::channel(false);
        let (send, receive) = tokio::sync::oneshot::channel();
        {
            let mut inner = self.inner.lock().unwrap();
            if inner.closing {
                bail!("INVALID_STATE");
            }
            if inner.dependency_cancel.is_some() {
                bail!("DEPENDENCY_BUSY");
            }
            if tool.is_some() && inner.transcription.is_some() {
                bail!("TRANSCRIPTION_BUSY");
            }
            inner.dependency_cancel = Some(cancel);
            let core = self.clone();
            inner.workers.retain(|worker| !worker.is_finished());
            inner.workers.push(tokio::spawn(async move {
                let result = async {
                    let mut params = json!({"runtime_directory":core.runtime_directory});
                    let mut rollback = None;
                    let stage = if let Some(tool) = tool {
                        std::fs::create_dir_all(&core.runtime_directory).context("IO_ERROR")?;
                        rollback = Some(crate::runtime_dependencies::InstallRollback::capture(
                            &core.runtime_directory, &tool, variant.as_deref(),
                        )?);
                        let stage = tempfile::Builder::new()
                            .prefix(".install-")
                            .tempdir_in(&core.runtime_directory)
                            .context("IO_ERROR")?;
                        params["tool"] = tool.into();
                        params["variant"] = json!(variant);
                        params["staging_dir"] = stage.path().to_string_lossy().into_owned().into();
                        Some(stage)
                    } else {
                        None
                    };
                    let method = if stage.is_some() {
                        "install_dependency"
                    } else {
                        "check_dependencies"
                    };
                    let result = PythonBridge::operation_cancellable(
                        &core.program,
                        &core.args,
                        "runtime-dependencies",
                        method,
                        params,
                        progress,
                        async {
                            let _ = cancellation.wait_for(|v| *v).await;
                        },
                    )
                    .await;
                    // The bridge reaps the whole process tree before staging is removed.
                    if result.is_err() {
                        if let (Some(rollback), Some(prepared_stage)) = (rollback, stage.as_ref()) {
                            if let Err(error) = rollback.restore(prepared_stage.path()) {
                                // Preserve recoverable backups rather than deleting them
                                // when an OS error prevents restoration.
                                if let Some(stage) = stage { let _ = stage.keep(); }
                                return Err(error);
                            }
                        }
                    }
                    drop(stage);
                    result.map(|value| value["result"].clone())
                }
                .await;
                core.inner.lock().unwrap().dependency_cancel = None;
                let _ = send.send(result);
            }));
        }
        receive.await.context("ENGINE_UNAVAILABLE")?
    }
    pub fn cover(&self, id: &str) -> Result<Vec<u8>> {
        let inner = self.inner.lock().unwrap();
        let task = inner
            .tasks
            .iter()
            .find(|t| t.id == id)
            .context("TASK_NOT_FOUND")?;
        if task.cover_path.is_none() && task.cover_cached {
            return parse_cache::cover(&inner.db, task.video.as_ref().context("FILE_MISSING")?);
        }
        crate::files::read_cover(task.cover_path.as_deref().context("FILE_MISSING")?)
    }
    pub fn author_avatar(&self, id: &str) -> Result<Vec<u8>> {
        let inner = self.inner.lock().unwrap();
        let video = inner.tasks.iter().find(|task| task.id == id)
            .context("TASK_NOT_FOUND")?.video.as_ref().context("FILE_MISSING")?;
        parse_cache::avatar(&inner.db, video)
    }
    pub fn export_cover(&self, id: &str) -> Result<(String, Vec<u8>)> {
        let task = self
            .list()
            .into_iter()
            .find(|t| t.id == id)
            .context("TASK_NOT_FOUND")?;
        let video = task.video.context("FILE_MISSING")?;
        let body = if video["kind"] == "image_album" {
            self.image(id, 0)?
        } else {
            self.cover(id)?
        };
        let extension = crate::files::image_extension(&body).context("INVALID_MEDIA_CONTENT")?;
        let filename = format!(
            "{}_{}.cover.{}",
            video["platform"]
                .as_str()
                .context("INVALID_ENGINE_RESULT")?,
            video["video_id"]
                .as_str()
                .context("INVALID_ENGINE_RESULT")?,
            extension
        );
        Ok((filename, body))
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
        self.resolve_task(input, None, "auto".into(), None)
    }

    pub fn resolve_with_resolver(
        self: &Arc<Self>,
        input: String,
        resolver: String,
    ) -> Result<Task> {
        if !matches!(resolver.as_str(), "auto" | "yt-dlp" | "dtk" | "tdl") {
            bail!("CONFIG_INVALID");
        }
        self.resolve_task(input, None, resolver, None)
    }

    pub fn image(&self, id: &str, index: usize) -> Result<Vec<u8>> {
        let path = {
            let inner = self.inner.lock().unwrap();
            let task = inner
                .tasks
                .iter()
                .find(|t| t.id == id)
                .context("TASK_NOT_FOUND")?;
            if index >= 64 {
                bail!("INVALID_INPUT");
            }
            if task.status == Status::Completed {
                let file = task.files.get(index).context("FILE_MISSING")?;
                task.path
                    .as_ref()
                    .context("FILE_MISSING")?
                    .join(&file.filename)
            } else {
                return parse_cache::image(
                    &inner.db,
                    task.video.as_ref().context("FILE_MISSING")?,
                    index,
                );
            }
        };
        crate::files::read_image(&path)
    }

    pub fn retry(self: &Arc<Self>, id: &str) -> Result<Task> {
        if self.list().iter().any(|task| {
            task.id == id
                && task.video.is_none()
                && task.telegram_link.is_some()
                && task.download_requested
        }) {
            self.download(id, false)?;
            return self
                .list()
                .into_iter()
                .find(|task| task.id == id)
                .context("TASK_NOT_FOUND");
        }
        let (input, resolver, format_id) = {
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
            (
                video.canonical_url,
                video.resolver,
                task.video.as_ref().unwrap()["selected_format_id"]
                    .as_str()
                    .map(str::to_owned),
            )
        };
        self.resolve_task(input, Some(id), resolver, format_id)
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
        ) || inner
            .transcription
            .as_ref()
            .is_some_and(|a| a.snapshot.task_id == id)
            || inner.active.contains_key(id)
            || inner.downloads.contains(id)
        {
            bail!("TASK_BUSY");
        }
        self.remove_history_records(&mut inner, vec![id.to_owned()])?;
        Ok(())
    }

    pub fn clear_history(&self, platform: Option<&str>) -> Result<Vec<String>> {
        if platform.is_some_and(|p| !matches!(p, "douyin" | "tiktok" | "youtube" | "telegram")) {
            bail!("CONFIG_INVALID");
        }
        let mut inner = self.inner.lock().unwrap();
        let ids = inner
            .tasks
            .iter()
            .filter(|task| {
                task.download_requested
                    && platform.is_none_or(|p| {
                        task.video
                            .as_ref()
                            .and_then(|v| v["platform"].as_str())
                            .or_else(|| task.telegram_link.as_ref().map(|_| "telegram"))
                            == Some(p)
                    })
                    && matches!(
                        task.status,
                        Status::Completed | Status::Failed | Status::Canceled
                    )
                    && !inner.active.contains_key(&task.id)
                    && !inner.downloads.contains(&task.id)
                    && !inner
                        .transcription
                        .as_ref()
                        .is_some_and(|a| a.snapshot.task_id == task.id)
            })
            .map(|task| task.id.clone())
            .collect();
        self.remove_history_records(&mut inner, ids)
    }

    fn remove_history_records(&self, inner: &mut Inner, ids: Vec<String>) -> Result<Vec<String>> {
        if ids.is_empty() {
            return Ok(ids);
        }
        let removing: HashSet<_> = ids.iter().map(String::as_str).collect();
        let retained: HashSet<_> = inner
            .tasks
            .iter()
            .filter(|t| !removing.contains(t.id.as_str()))
            .filter_map(|t| t.video.as_ref())
            .map(|v| {
                (
                    v["platform"].as_str(),
                    v["video_id"].as_str(),
                    v["resolver"].as_str(),
                )
            })
            .collect();
        let mut released = HashSet::new();
        let release_covers: Vec<_> = inner
            .tasks
            .iter()
            .filter(|t| removing.contains(t.id.as_str()))
            .filter_map(|t| t.video.as_ref())
            .filter(|v| {
                let identity = (
                    v["platform"].as_str(),
                    v["video_id"].as_str(),
                    v["resolver"].as_str(),
                );
                !retained.contains(&identity) && released.insert(identity)
            })
            .cloned()
            .collect();
        let tx = inner.db.transaction().context("DATABASE_ERROR")?;
        for id in &ids {
            tx.execute("DELETE FROM video_transcripts WHERE task_id=?", [id])
                .context("DATABASE_ERROR")?;
            tx.execute("DELETE FROM commit_intents WHERE task_id=?", [id])
                .context("DATABASE_ERROR")?;
            tx.execute("DELETE FROM download_tasks WHERE id=?", [id])
                .context("DATABASE_ERROR")?;
        }
        for video in release_covers {
            parse_cache::clear_cover(&tx, &video)?;
        }
        tx.commit().context("DATABASE_ERROR")?;
        // Staging records, if cleanup previously failed, remain available for
        // startup cleanup. Removing history never touches a completed video.
        inner.tasks.retain(|t| !removing.contains(t.id.as_str()));
        inner
            .transcription_unsaved
            .retain(|id, _| !removing.contains(id.as_str()));
        for id in &ids {
            (self.emit)(TaskEvent::Removed(id.clone()));
        }
        Ok(ids)
    }

    pub fn playback_path(&self, id: &str) -> Result<PathBuf> {
        let inner = self.inner.lock().unwrap();
        let task = inner
            .tasks
            .iter()
            .find(|t| t.id == id)
            .context("TASK_NOT_FOUND")?;
        let video = task.video.as_ref().context("INVALID_STATE")?;
        if task.status != Status::Completed
            || !task.files.is_empty()
            || video["kind"].as_str().unwrap_or("video") != "video"
        {
            bail!("INVALID_STATE");
        }
        let path = task.path.as_ref().context("FILE_MISSING")?;
        let metadata = std::fs::symlink_metadata(path).context("FILE_MISSING")?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            bail!("FILE_MISSING");
        }
        Ok(path.clone())
    }

    pub fn delete_local_file(&self, id: &str) -> Result<()> {
        let mut inner = self.inner.lock().unwrap();
        if inner.active.contains_key(id)
            || inner.downloads.contains(id)
            || inner
                .transcription
                .as_ref()
                .is_some_and(|a| a.snapshot.task_id == id)
        {
            bail!("TASK_BUSY");
        }
        let task = inner
            .tasks
            .iter()
            .find(|t| t.id == id)
            .context("TASK_NOT_FOUND")?;
        if task.status != Status::Completed {
            bail!("INVALID_STATE");
        }
        let path = task.path.as_ref().context("FILE_MISSING")?;
        match std::fs::symlink_metadata(path) {
            Ok(metadata) => {
                if !task.files.is_empty() {
                    if !metadata.is_dir() || metadata.file_type().is_symlink() {
                        bail!("IO_ERROR");
                    }
                    let owned = task
                        .files
                        .iter()
                        .map(|f| path.join(&f.filename))
                        .collect::<Vec<_>>();
                    for file in &owned {
                        match std::fs::symlink_metadata(file) {
                            Ok(m) if m.is_file() && !m.file_type().is_symlink() => {}
                            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                            _ => bail!("IO_ERROR"),
                        }
                    }
                    for file in &owned {
                        match std::fs::remove_file(file) {
                            Ok(()) => {}
                            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                            Err(_) => bail!("IO_ERROR"),
                        }
                    }
                    // Preserve unrelated files a user may have put in this directory.
                    match std::fs::remove_dir(path) {
                        Ok(()) => {}
                        Err(e) if e.kind() == std::io::ErrorKind::DirectoryNotEmpty => {}
                        Err(_) => bail!("IO_ERROR"),
                    }
                } else {
                    if !metadata.is_file() || metadata.file_type().is_symlink() {
                        bail!("IO_ERROR");
                    }
                    std::fs::remove_file(path).context("IO_ERROR")?;
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => bail!("IO_ERROR"),
        }
        // Remove the record only after file deletion succeeds. If the database
        // fails, retain the task so deletion can be retried for the missing file.
        self.remove_history_records(&mut inner, vec![id.to_owned()])?;
        Ok(())
    }

    fn resolve_task(
        self: &Arc<Self>,
        input: String,
        retry_id: Option<&str>,
        resolver: String,
        requested_format: Option<String>,
    ) -> Result<Task> {
        if input.trim().is_empty() || input.len() > 8192 {
            bail!("INVALID_INPUT");
        }
        let telegram = tdl_bridge::is_telegram(&input);
        let resolver = if telegram {
            tdl_bridge::message_link(&input)?;
            if !matches!(resolver.as_str(), "auto" | "tdl") {
                bail!("UNSUPPORTED_CONTENT");
            }
            "tdl".to_owned()
        } else {
            if resolver == "tdl" {
                bail!("INVALID_INPUT");
            }
            resolver
        };
        let mut inner = self.inner.lock().unwrap();
        if inner.closing || inner.tasks.iter().any(|t| t.status == Status::Parsing) {
            bail!("TASK_BUSY");
        }
        let previous = if let Some(id) = retry_id {
            let task = inner
                .tasks
                .iter()
                .find(|t| t.id == id)
                .context("TASK_NOT_FOUND")?;
            if !matches!(task.status, Status::Failed | Status::Canceled) {
                bail!("INVALID_STATE");
            }
            if inner.active.contains_key(id) || inner.downloads.contains(id) {
                bail!("TASK_BUSY");
            }
            Some(task.clone())
        } else {
            None
        };
        let download_after_resolve = previous.is_some();
        let mut cached = parse_cache::find(&inner.db, &input, &resolver)?;
        if let (Some(cached), Some(format_id)) = (&mut cached, &requested_format) {
            apply_video_format(&mut cached.video, format_id)?;
        }
        let tdl = if telegram {
            if inner.tdl_login_cancel.is_some() {
                bail!("TDL_BUSY");
            }
            if !inner.settings.tdl.enabled {
                bail!("TDL_EXPERIMENTAL_DISABLED");
            }
            Some(inner.settings.tdl.clone())
        } else {
            None
        };
        let profiles = if telegram {
            json!({})
        } else {
            settings::snapshot(&inner.settings)?
        };
        let mut task = Task {
            id: previous
                .as_ref()
                .map(|t| t.id.clone())
                .unwrap_or_else(|| Uuid::new_v4().to_string()),
            revision: previous.as_ref().map_or(1, |t| t.revision + 1),
            status: Status::Parsing,
            video: previous.as_ref().and_then(|t| t.video.clone()),
            bytes: 0,
            total_bytes: None,
            total_bytes_estimate: None,
            speed_bps: None,
            eta_seconds: None,
            merging: false,
            error: None,
            path: None,
            created_at: previous.as_ref().map_or_else(now, |t| t.created_at),
            completed_at: None,
            canceling: false,
            file_missing: false,
            queue_order: None,
            download_requested: download_after_resolve,
            cover_path: None,
            cover_error: None,
            files: Vec::new(),
            completed_items: None,
            total_items: None,
            cache_hit: false,
            cached_at: None,
            cover_cached: false,
            telegram_link: if telegram {
                Some(tdl_bridge::message_link(&input)?.url)
            } else {
                None
            },
        };
        let (cancel, mut cancellation) = watch::channel(false);
        let (done, done_rx) = watch::channel(cached.is_some());
        if previous.is_some() {
            let staging = inner.db.query_row(
                "SELECT path FROM task_staging WHERE task_id=?",
                [&task.id],
                |r| r.get::<_, String>(0),
            );
            match staging {
                Ok(path) => cleanup_staging(&inner.db, &task.id, Path::new(&path))?,
                Err(rusqlite::Error::QueryReturnedNoRows) => {}
                Err(_) => bail!("DATABASE_ERROR"),
            }
            // Old commit evidence must never complete a new attempt on restart.
            let tx = inner.db.transaction().context("DATABASE_ERROR")?;
            tx.execute("DELETE FROM commit_intents WHERE task_id=?", [&task.id])
                .context("DATABASE_ERROR")?;
            persist(&tx, &task)?;
            tx.commit().context("DATABASE_ERROR")?;
            *inner.tasks.iter_mut().find(|t| t.id == task.id).unwrap() = task.clone();
            (self.emit)(TaskEvent::Updated(Box::new(task.clone())));
        } else {
            publish(&mut inner, task.clone(), &*self.emit)?;
        }
        let directory = inner.settings.download_directory.clone();
        inner.active.insert(
            task.id.clone(),
            Active {
                tdl: tdl.clone(),
                download_after_resolve,
                profiles: profiles.clone(),
                directory: directory.clone(),
                cancel,
                done: done_rx,
            },
        );
        if let Some(cached) = cached {
            task.video = Some(cached.video);
            task.cache_hit = true;
            task.cached_at = Some(cached.saved_at);
            task.cover_cached = cached.has_cover;
            task.status = Status::Ready;
            task.revision += 1;
            publish(&mut inner, task.clone(), &*self.emit)?;
            drop(inner);
            if download_after_resolve {
                self.finish(&task.id, Ok(task.video.clone().unwrap()), false);
            }
            return Ok(task);
        }
        let core = self.clone();
        let id = task.id.clone();
        inner.workers.retain(|worker| !worker.is_finished());
        let worker = tokio::spawn(async move {
            let result = async {
                if let Some(config) = tdl {
                    return core.resolve_telegram(&config, &input, &id, cancellation.clone()).await;
                }
                if parse_cache::share_url(&input).is_some() {
                    let has_cache: bool = core.inner.lock().unwrap().db.query_row(
                        "SELECT EXISTS(SELECT 1 FROM parse_cache)", [], |r| r.get(0),
                    ).context("DATABASE_ERROR")?;
                    if has_cache {
                        let identified = PythonBridge::operation_cancellable(
                            &core.program, &core.args, &id, "identify_input",
                            json!({"input":input,"profiles":profiles,"resolver":resolver,"runtime_directory":core.runtime_directory}),
                            |_| {}, async { let _ = cancellation.wait_for(|v| *v).await; },
                        ).await?;
                        let identity: Video = serde_json::from_value(identified["result"].clone())
                            .context("INVALID_ENGINE_RESULT")?;
                        identity.validate()?;
                        let mut inner = core.inner.lock().unwrap();
                        if let Some(cached) = parse_cache::get(
                            &inner.db, &identity.platform, &identity.video_id, &identity.resolver,
                        )? {
                            parse_cache::remember(&inner.db, &input, &cached.video)?;
                            parse_cache::remember(&inner.db, &identity.canonical_url, &cached.video)?;
                            let task = inner.tasks.iter_mut().find(|t| t.id == id).unwrap();
                            task.cache_hit = true;
                            task.cached_at = Some(cached.saved_at);
                            task.cover_cached = cached.has_cover;
                            return Ok(cached.video);
                        }
                    }
                }
                std::fs::create_dir_all(&directory).context("IO_ERROR")?;
                let stage = tempfile::Builder::new().prefix(&format!("yingqu-{id}-"))
                    .tempdir_in(&directory).context("IO_ERROR")?;
                core.inner.lock().unwrap().db.execute(
                    "INSERT INTO task_staging VALUES(?1,?2)",
                    params![id, stage.path().to_string_lossy()],
                ).context("DATABASE_ERROR")?;
                let result = PythonBridge::operation_cancellable(
                    &core.program, &core.args, &id, "resolve_video",
                    json!({"input":input,"profiles":profiles,"resolver":resolver,"preview_dir":stage.path(),"runtime_directory":core.runtime_directory}),
                    |_| {}, async { let _ = cancellation.wait_for(|v| *v).await; },
                ).await.and_then(|result| {
                    let video: Video = serde_json::from_value(result["result"].clone())
                        .context("INVALID_ENGINE_RESULT")?;
                    video.validate()?;
                    Ok(result["result"].clone())
                });
                let result = result.and_then(|video| {
                    let images = if video["kind"] == "image_album" {
                        video["images"].as_array().context("INVALID_ENGINE_RESULT")?
                        .iter().enumerate().map(|(index, image)| {
                            let name = image["preview_filename"].as_str().context("INVALID_ENGINE_RESULT")?;
                            let extension = name.rsplit_once('.').context("INVALID_ENGINE_RESULT")?.1;
                            if name != format!("preview-{:03}.{extension}", index + 1)
                                || !matches!(extension, "jpeg"|"png"|"webp"|"gif"|"heic"|"avif") {
                                bail!("INVALID_ENGINE_RESULT");
                            }
                            crate::files::read_image(&stage.path().join(name))
                        }).collect::<Result<Vec<_>>>()?
                    } else { Vec::new() };
                    let cover = if video["cover_filename"] == "cover" {
                        Some(crate::files::read_cover(&stage.path().join("cover"))?)
                    } else { None };
                    let mut inner = core.inner.lock().unwrap();
                    let mut cached = parse_cache::save(&mut inner.db, &video, images, cover, now())?;
                    // Optional avatar failures must not fail an otherwise valid video/album preview.
                    if video["author_avatar_filename"] == "author-avatar" {
                        if let Ok(body) = crate::files::read_cover(&stage.path().join("author-avatar")) {
                            if video["author_avatar_bytes"].as_u64() == Some(body.len() as u64) {
                                let _ = parse_cache::save_avatar(&inner.db, &cached.video, &body);
                            }
                        }
                    }
                    if let Some(profile) = cached.video["author_profile"].as_object_mut() {
                        profile.insert("avatar_cached".into(), parse_cache::avatar(&inner.db, &video).is_ok().into());
                    }
                    parse_cache::remember(&inner.db, &input, &cached.video)?;
                    let task = inner.tasks.iter_mut().find(|t| t.id == id).unwrap();
                    task.cached_at = Some(cached.saved_at);
                    task.cover_cached = cached.has_cover;
                    if video["cookie_warning"] == "BROWSER_COOKIES_UNAVAILABLE" {
                        cached.video["cookie_warning"] = video["cookie_warning"].clone();
                    }
                    Ok(cached.video)
                });
                stage.close().context("IO_ERROR")?;
                core.inner.lock().unwrap().db.execute(
                    "DELETE FROM task_staging WHERE task_id=?", [&id],
                ).context("DATABASE_ERROR")?;
                result
            }.await;
            let result = result.and_then(|mut video| {
                if let Some(format_id) = requested_format {
                    apply_video_format(&mut video, &format_id)?;
                }
                Ok(video)
            });
            core.finish(&id, result, false);
            let _ = done.send(true);
        });
        inner.workers.push(worker);
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
            task.queue_order = task.queue_order.or_else(|| {
                Some(
                    inner
                        .tasks
                        .iter()
                        .filter_map(|t| t.queue_order)
                        .max()
                        .unwrap_or(0)
                        + 1,
                )
            });
        }
        task.canceling = false;
        task.speed_bps = None;
        task.eta_seconds = None;
        task.merging = false;
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
        self.dispatch(&mut inner);
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

    pub fn choose_format(&self, id: &str, format_id: &str) -> Result<Task> {
        let mut inner = self.inner.lock().unwrap();
        let mut task = inner
            .tasks
            .iter()
            .find(|t| t.id == id)
            .context("TASK_NOT_FOUND")?
            .clone();
        if inner.closing || task.status != Status::Ready {
            bail!("INVALID_STATE");
        }
        apply_video_format(task.video.as_mut().context("INVALID_STATE")?, format_id)?;
        task.revision += 1;
        publish(&mut inner, task.clone(), &*self.emit)?;
        Ok(task)
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
        let deferred = task.video.is_none()
            && task.telegram_link.is_some()
            && matches!(task.status, Status::Failed | Status::Canceled)
            && (task.error.as_deref() == Some("TDL_CHANNEL_ID_REQUIRED")
                || task.download_requested);
        if task.status != Status::Ready && !deferred {
            bail!("INVALID_STATE");
        }
        if (deferred
            || task
                .video
                .as_ref()
                .is_some_and(|v| v["platform"] == "telegram"))
            && !inner.settings.tdl.enabled
        {
            bail!("TDL_EXPERIMENTAL_DISABLED");
        }
        if !confirm_duplicate && previously_downloaded(&inner.tasks, &task) {
            bail!("ALREADY_DOWNLOADED");
        }
        if !inner.active.contains_key(id) {
            let telegram = deferred
                || task
                    .video
                    .as_ref()
                    .is_some_and(|v| v["platform"] == "telegram");
            let tdl = if telegram {
                if inner.tdl_login_cancel.is_some() {
                    bail!("TDL_BUSY");
                }
                Some(inner.settings.tdl.clone())
            } else {
                None
            };
            let profiles = if telegram {
                json!({})
            } else {
                settings::snapshot(&inner.settings)?
            };
            let (cancel, _) = watch::channel(false);
            let (_, done) = watch::channel(true);
            let directory = inner.settings.download_directory.clone();
            inner.active.insert(
                id.to_owned(),
                Active {
                    tdl,
                    download_after_resolve: deferred,
                    profiles,
                    directory,
                    cancel,
                    done,
                },
            );
        }
        task.status = Status::Queued;
        task.error = None;
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
            let waiting_channel = inner
                .tasks
                .iter()
                .filter(|t| {
                    matches!(t.status, Status::Queued | Status::Parsing)
                        && t.download_requested
                        && t.video.is_none()
                        && t.telegram_link.is_some()
                })
                .filter_map(|t| t.queue_order)
                .min();
            let telegram_busy = inner.active.iter().any(|(id, active)| {
                active.tdl.is_some()
                    && (inner.downloads.contains(id)
                        || inner
                            .tasks
                            .iter()
                            .any(|t| t.id == *id && t.status == Status::Parsing))
            });
            let Some(id) = inner
                .tasks
                .iter()
                .filter(|t| t.status == Status::Queued)
                .filter(|t| {
                    if t.video.is_none() {
                        !telegram_busy
                            && !inner
                                .tasks
                                .iter()
                                .any(|other| other.status == Status::Parsing)
                    } else {
                        // Later Telegram downloads must not keep an unknown channel
                        // waiting forever. Other platforms may use free slots.
                        !inner.active.get(&t.id).is_some_and(|a| a.tdl.is_some())
                            || waiting_channel.is_none_or(|order| t.queue_order < Some(order))
                    }
                })
                .min_by_key(|t| t.queue_order)
                .map(|t| t.id.clone())
            else {
                break;
            };
            let deferred = inner.tasks.iter().any(|t| t.id == id && t.video.is_none());
            let started = if deferred {
                self.start_deferred(inner, &id)
            } else {
                self.start_download(inner, &id)
            };
            if started.is_err() {
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

    async fn resolve_telegram(
        self: &Arc<Self>,
        config: &TdlSettings,
        input: &str,
        id: &str,
        cancel: watch::Receiver<bool>,
    ) -> Result<Value> {
        let channel = parse_cache::tdl_channel(&self.inner.lock().unwrap().db, input)?;
        let video = self
            .tdl_session
            .operation(config, input, None, None, |_| {}, cancel, channel)
            .await?;
        let identity: Video =
            serde_json::from_value(video.clone()).context("INVALID_ENGINE_RESULT")?;
        identity.validate()?;
        let mut inner = self.inner.lock().unwrap();
        let cached = parse_cache::save(&mut inner.db, &video, Vec::new(), None, now())?;
        parse_cache::remember(&inner.db, input, &cached.video)?;
        let task = inner.tasks.iter_mut().find(|t| t.id == id).unwrap();
        task.cached_at = Some(cached.saved_at);
        Ok(cached.video)
    }

    fn start_deferred(self: &Arc<Self>, inner: &mut Inner, id: &str) -> Result<()> {
        let mut task = inner.tasks.iter().find(|t| t.id == id).unwrap().clone();
        let input = task.telegram_link.clone().context("INVALID_STATE")?;
        let config = inner
            .active
            .get(id)
            .unwrap()
            .tdl
            .clone()
            .context("INVALID_STATE")?;
        let (cancel, cancellation) = watch::channel(false);
        let (done, done_rx) = watch::channel(false);
        task.status = Status::Parsing;
        task.revision += 1;
        publish(inner, task, &*self.emit)?;
        inner.downloads.insert(id.to_owned());
        let active = inner.active.get_mut(id).unwrap();
        active.cancel = cancel;
        active.done = done_rx;
        let core = self.clone();
        let id = id.to_owned();
        inner.workers.retain(|worker| !worker.is_finished());
        inner.workers.push(tokio::spawn(async move {
            let result = core
                .resolve_telegram(&config, &input, &id, cancellation)
                .await;
            core.inner.lock().unwrap().downloads.remove(&id);
            core.finish(&id, result, false);
            let _ = done.send(true);
        }));
        Ok(())
    }

    fn start_download(self: &Arc<Self>, inner: &mut Inner, id: &str) -> Result<()> {
        let mut task = inner.tasks.iter().find(|t| t.id == id).unwrap().clone();
        let active = inner
            .active
            .get(id)
            .expect("queued task has configuration snapshot");
        let profiles = active.profiles.clone();
        let tdl = active.tdl.clone();
        let directory = active.directory.clone();
        let video = task.video.clone().context("INVALID_STATE")?;
        let tdl_input = if tdl.is_some() {
            Some(parse_cache::tdl_source(&inner.db, &video)?)
        } else {
            None
        };
        let (cancel, mut cancellation) = watch::channel(false);
        let (done, done_rx) = watch::channel(false);
        task.status = Status::Downloading;
        task.total_bytes_estimate = None;
        task.revision += 1;
        publish(inner, task, &*self.emit)?;
        inner.downloads.insert(id.to_owned());
        let active = inner.active.get_mut(id).unwrap();
        active.cancel = cancel;
        active.done = done_rx;
        let core = self.clone();
        let id = id.to_owned();
        inner.workers.retain(|worker| !worker.is_finished());
        let worker = tokio::spawn(async move {
            let result = async {
                std::fs::create_dir_all(&directory).context("IO_ERROR")?;
                let stage = tempfile::Builder::new().prefix(&format!("yingqu-{id}-")).tempdir_in(&directory).context("IO_ERROR")?;
                let media = stage.path().join("media");
                std::fs::create_dir(&media).context("IO_ERROR")?;
                core.inner.lock().unwrap().db.execute("INSERT OR REPLACE INTO task_staging VALUES(?1,?2)",
                    params![id,stage.path().to_string_lossy()]).context("DATABASE_ERROR")?;
                let mut last = Instant::now() - Duration::from_secs(1);
                let mut transfer_rate = YoutubeTransferRate::default();
                let youtube = video["platform"] == "youtube";
                let progress = |mut event: Value| {
                        if youtube { transfer_rate.update(&mut event, Instant::now()); }
                        if last.elapsed() < Duration::from_millis(250) && event["phase"] != "merging" { return; }
                        last = Instant::now();
                        let mut inner = core.inner.lock().unwrap();
                        let task = inner.tasks.iter_mut().find(|t| t.id == id).unwrap();
                        if task.canceling { return; }
                        task.bytes = event["bytes"].as_u64().unwrap_or(0);
                        task.total_bytes = event["total_bytes"].as_u64();
                        task.total_bytes_estimate = event["total_bytes_estimate"].as_u64();
                        task.speed_bps = event["speed_bps"].as_f64();
                        task.eta_seconds = event["eta_seconds"].as_f64();
                        task.merging |= event["phase"] == "merging";
                        task.completed_items = event["completed_items"].as_u64();
                        task.total_items = event["total_items"].as_u64();
                        task.revision += 1;
                        (core.emit)(TaskEvent::Updated(Box::new(task.clone())));
                    };
                let result = if let Some(config) = tdl {
                    match core.tdl_session.operation(&config, tdl_input.as_deref().context("INVALID_ENGINE_RESULT")?, Some(&video), Some(&media), progress, cancellation.clone(), None).await {
                        Ok(mut result) => {
                            let mut cover_cancel = cancellation.clone();
                            let cover = PythonBridge::operation_cancellable(
                                &core.program, &core.args, &id, "extract_cover",
                                json!({"runtime_directory":core.runtime_directory,
                                    "staging_dir":media,"filename":result["filename"]}),
                                |_| {}, async { let _ = cover_cancel.wait_for(|v| *v).await; },
                            ).await;
                            match cover {
                                Err(error) if error.to_string() == "CANCELED" => Err(error),
                                cover => {
                                    if let Ok(cover) = cover {
                                        result["cover_filename"] = cover["cover_filename"].clone();
                                        result["cover_bytes"] = cover["cover_bytes"].clone();
                                    }
                                    Ok(result)
                                }
                            }
                        }
                        Err(error) => Err(error),
                    }
                } else {
                    PythonBridge::operation_cancellable(&core.program, &core.args, &id, "download_video",
                    json!({"input":video["canonical_url"],"profiles":profiles,"runtime_directory":core.runtime_directory,
                        "resolver":video["resolver"].as_str().unwrap_or("yt-dlp"),
                        "format_id":video["selected_format_id"],
                        "expected":{"platform":video["platform"],"video_id":video["video_id"],"kind":video["kind"].as_str().unwrap_or("video"),
                            "image_count":video["images"].as_array().map(Vec::len)},"staging_dir":media}),
                    progress, async { let _ = cancellation.wait_for(|v| *v).await; }).await
                };
                if result.as_ref().is_err_and(|error| error.root_cause().to_string() == tdl_bridge::ALIAS_IDENTITY_MISMATCH) {
                    // A freshly resolved public bootstrap proved this exact alias
                    // stale. Do not retry or remove a concurrently updated mapping.
                    parse_cache::forget_tdl_channel(&core.inner.lock().unwrap().db,
                        tdl_input.as_deref().context("INVALID_ENGINE_RESULT")?, &video)?;
                }
                if result.as_ref().is_err_and(|error| error.to_string() == "TDL_MEDIA_CHANGED") {
                    parse_cache::invalidate_tdl(&mut core.inner.lock().unwrap().db, &video)?;
                }
                let committed = result.and_then(|result| {
                    let mut inner = core.inner.lock().unwrap();
                    let mut task = inner.tasks.iter().find(|t| t.id == id).unwrap().clone();
                    if task.canceling { bail!("CANCELED"); }
                    if video["selected_format_id"].is_string() && result["format_id"] != video["selected_format_id"] {
                        bail!("INVALID_ENGINE_RESULT");
                    }
                    let identity: Video = serde_json::from_value(video).context("INVALID_ENGINE_RESULT")?;
                    // A retry keeps the task ID but must not collide with files
                    // committed by an earlier attempt whose database write failed.
                    let attempt_id = Uuid::new_v4();
                    let file = if identity.kind == "image_album" {
                        let target = directory.join(format!("{}_{}_{}", identity.platform, identity.video_id, attempt_id));
                        let file = finalize_album(&identity, &result, &media, &target, |source,size,files| {
                            inner.db.execute("INSERT INTO commit_intents(task_id,source,target,size,manifest) VALUES(?1,?2,?3,?4,?5)", params![
                                id,source.to_string_lossy(),target.to_string_lossy(),i64::try_from(size).context("IO_ERROR")?,serde_json::to_string(files)?]).context("DATABASE_ERROR")?;
                            Ok(())
                        })?;
                        task.files = serde_json::from_value(result["files"].clone()).context("INVALID_ENGINE_RESULT")?;
                        if let Some(video) = task.video.as_mut() { video["images"] = result["files"].clone(); }
                        task.completed_items = Some(task.files.len() as u64);
                        task.total_items = task.completed_items;
                        file
                    } else {
                    let extension = result["container"].as_str().context("INVALID_ENGINE_RESULT")?;
                    if !matches!(extension, "mp4" | "webm") && !(identity.platform == "telegram" && matches!(extension, "mov" | "mkv")) { bail!("INVALID_MEDIA_CONTENT"); }
                    let target = directory.join(format!("{}_{}_{}.{}",identity.platform,identity.video_id,attempt_id,extension));
                    let file = finalize_to(&identity, &result, &media, &target, |source,size| {
                        inner.db.execute("INSERT INTO commit_intents(task_id,source,target,size) VALUES(?1,?2,?3,?4)", params![
                            id,source.to_string_lossy(),target.to_string_lossy(),i64::try_from(size).context("IO_ERROR")?]).context("DATABASE_ERROR")?;
                        Ok(())
                    })?;
                    let saved = crate::files::staged_cover(&result, &media).and_then(|body| {
                        parse_cache::save_cover(&inner.db, task.video.as_ref().context("INVALID_ENGINE_RESULT")?, &body)
                    }).unwrap_or(false);
                    task.cover_path = None;
                    if saved {
                        task.cover_cached = true;
                        task.cover_error = None;
                    } else {
                        task.cover_error = Some("COVER_UNAVAILABLE".into());
                    }
                    file
                    };
                    if identity.kind == "video" {
                        if let Some(video) = task.video.as_mut() {
                            for field in ["width", "height"] {
                                if result[field].is_number() { video[field] = result[field].clone(); }
                            }
                            video["filesize"] = file.bytes.into();
                            video["filesize_approx"] = Value::Null;
                        }
                    }
                    task.path = Some(file.path);
                    task.bytes = file.bytes;
                    task.total_bytes = Some(file.bytes);
                    task.total_bytes_estimate = None;
                    task.completed_at = Some(now());
                    task.status = Status::Completed;
                    task.speed_bps = None;
                    task.eta_seconds = None;
                task.merging = false;
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
        inner.workers.push(worker);
        Ok(())
    }

    pub async fn cancel(self: &Arc<Self>, id: &str) -> Result<()> {
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
                self.dispatch(&mut inner);
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
        let (active, workers) = {
            let mut inner = self.inner.lock().unwrap();
            inner.closing = true;
            if let Some(active) = &inner.transcription {
                let _ = active.cancel.send(true);
            }
            if let Some(cancel) = &inner.dependency_cancel {
                let _ = cancel.send(true);
            }
            if let Some(cancel) = &inner.tdl_login_cancel {
                let _ = cancel.send(true);
            }
            let active = inner
                .active
                .iter()
                .filter(|(id, _)| {
                    inner
                        .tasks
                        .iter()
                        .any(|t| t.id == **id && t.status != Status::Ready)
                })
                .map(|(id, a)| (id.clone(), a.done.clone()))
                .collect::<Vec<_>>();
            (active, std::mem::take(&mut inner.workers))
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
        // Active entries and done signals precede worker destruction. Join even
        // retired workers so their Core references cannot retain the database lock.
        for worker in workers {
            let _ = worker.await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn youtube_speed_samples_aggregate_bytes_instead_of_resetting_hook_rates() {
        let mut rate = YoutubeTransferRate::default();
        let start = Instant::now();
        let update = |rate: &mut YoutubeTransferRate, millis, count, speed| {
            let mut event = json!({"bytes":count,"total_bytes":10000,"speed_bps":speed,"eta_seconds":1});
            rate.update(&mut event, start + Duration::from_millis(millis));
            event
        };
        assert!(update(&mut rate, 0, 100, json!(1000000))["speed_bps"].is_null());
        assert!(update(&mut rate, 999, 1099, Value::Null)["speed_bps"].is_null());
        let sampled = update(&mut rate, 1000, 1100, Value::Null);
        assert_eq!(sampled["speed_bps"], 1000.0);
        assert_eq!(sampled["eta_seconds"], 8.9);
        // Finished/chunk/next-stream hooks may omit or reset their speed while
        // the operation's aggregate byte count continues increasing.
        assert_eq!(update(&mut rate, 1100, 1100, Value::Null)["speed_bps"], 1000.0);
        assert_eq!(update(&mut rate, 1250, 1350, json!(999999999))["speed_bps"], 1000.0);
        assert_eq!(update(&mut rate, 1500, 1600, Value::Null)["speed_bps"], 1000.0);
        assert_eq!(update(&mut rate, 2000, 2100, json!(1))["speed_bps"], 1000.0);
        assert_eq!(update(&mut rate, 3500, 2850, Value::Null)["speed_bps"], 500.0);
        let stalled = update(&mut rate, 4500, 2850, Value::Null);
        assert_eq!(stalled["speed_bps"], 0.0);
        assert!(stalled["eta_seconds"].is_null());
    }

    #[test]
    fn youtube_speed_clears_for_merging_and_new_attempts() {
        let start = Instant::now();
        let mut rate = YoutubeTransferRate::default();
        let mut event = json!({"bytes":0,"total_bytes":null,"total_bytes_estimate":5000});
        rate.update(&mut event, start);
        event["bytes"] = 1000.into();
        rate.update(&mut event, start + Duration::from_secs(1));
        assert_eq!(event["speed_bps"], 1000.0);
        assert_eq!(event["eta_seconds"], 4.0);
        event["total_bytes_estimate"] = Value::Null;
        rate.update(&mut event, start + Duration::from_millis(1250));
        assert_eq!(event["speed_bps"], 1000.0);
        assert!(event["eta_seconds"].is_null());
        event["phase"] = "merging".into();
        rate.update(&mut event, start + Duration::from_millis(1500));
        assert!(event["speed_bps"].is_null());
        assert!(event["eta_seconds"].is_null());
        let mut retry = json!({"bytes":0,"total_bytes":5000});
        rate.update(&mut retry, start + Duration::from_secs(2));
        assert!(retry["speed_bps"].is_null());
        let mut fresh = YoutubeTransferRate::default();
        fresh.update(&mut retry, start + Duration::from_secs(3));
        assert!(retry["speed_bps"].is_null());
    }

    #[cfg(windows)]
    #[test]
    fn staging_cleanup_waits_for_transient_exclusive_handles_and_preserves_persistent_errors() {
        use std::os::windows::fs::OpenOptionsExt;
        for transient in [true, false] {
            let stage = tempfile::Builder::new().prefix("yingqu-lock-test-").tempdir().unwrap();
            let db = Connection::open_in_memory().unwrap();
            db.execute_batch("CREATE TABLE task_staging(task_id TEXT PRIMARY KEY,path TEXT NOT NULL);").unwrap();
            db.execute("INSERT INTO task_staging VALUES(?1,?2)", params!["lock-test", stage.path().to_string_lossy()]).unwrap();
            let file = std::fs::OpenOptions::new().write(true).create_new(true).share_mode(0)
                .open(stage.path().join("audio.wav")).unwrap();
            if transient {
                let release = std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_millis(60));
                    drop(file);
                });
                cleanup_staging(&db, "lock-test", stage.path()).unwrap();
                release.join().unwrap();
            } else {
                assert_eq!(cleanup_staging(&db, "lock-test", stage.path()).unwrap_err().to_string(), "IO_ERROR");
                assert_eq!(db.query_row("SELECT count(*) FROM task_staging", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
                drop(file);
                cleanup_staging(&db, "lock-test", stage.path()).unwrap();
            }
            assert!(!stage.path().exists());
            assert_eq!(db.query_row("SELECT count(*) FROM task_staging", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
        }
    }
    async fn transcript_done(app: &Core, id: &str) -> transcription::Snapshot {
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let snapshot = app.transcript(id).unwrap().unwrap();
                if snapshot.state != "running" && app.inner.lock().unwrap().transcription.is_none()
                {
                    return snapshot;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap()
    }
    async fn transcript_video(app: &Arc<Core>, name: &str) -> Task {
        let task = app.resolve("valid".into()).unwrap();
        wait(app, &task.id, Status::Ready).await;
        app.download(&task.id, true).unwrap();
        let mut task = wait(app, &task.id, Status::Completed).await;
        let path = task.path.as_ref().unwrap().parent().unwrap().join(name);
        std::fs::rename(task.path.as_ref().unwrap(), &path).unwrap();
        task.path = Some(path);
        let mut inner = app.inner.lock().unwrap();
        persist(&inner.db, &task).unwrap();
        *inner.tasks.iter_mut().find(|t| t.id == task.id).unwrap() = task.clone();
        task
    }
    #[tokio::test]
    async fn transcription_result_copy_export_restart_and_history_cleanup() {
        let root = tempfile::tempdir().unwrap();
        let app = core(root.path());
        let task = transcript_video(&app, "speech.mp4").await;
        let original = std::fs::read(task.path.as_ref().unwrap()).unwrap();
        app.start_transcription(&task.id, false).unwrap();
        let result = transcript_done(&app, &task.id).await;
        assert_eq!(result.state, "completed", "{:?}", result.error_code);
        assert_eq!(
            app.transcript_text(&task.id).unwrap().text,
            "欢迎体验。\nEnglish text."
        );
        assert_eq!(
            std::fs::read(task.path.as_ref().unwrap()).unwrap(),
            original
        );
        assert!(app.transcripts().unwrap()[0].result.is_none());
        assert!(app.transcripts().unwrap()[0].has_result);
        let exported = root.path().join("文案.txt");
        transcription::export_txt(&app.transcript_text(&task.id).unwrap(), &exported).unwrap();
        assert!(std::fs::read(&exported)
            .unwrap()
            .starts_with(b"\xef\xbb\xbf"));
        let alias = root.path().join("video-alias.txt");
        std::fs::hard_link(task.path.as_ref().unwrap(), &alias).unwrap();
        assert_eq!(
            transcription::export_txt(&app.transcript_text(&task.id).unwrap(), &alias)
                .unwrap_err()
                .to_string(),
            "TRANSCRIPTION_EXPORT_SOURCE"
        );
        app.shutdown().await;
        drop(app);
        let app = core(root.path());
        assert_eq!(
            app.transcript(&task.id).unwrap().unwrap().state,
            "completed"
        );
        app.remove_after_transcription(&task.id, false)
            .await
            .unwrap();
        assert!(app.transcripts().unwrap().is_empty());
        assert!(exported.is_file() && task.path.unwrap().is_file());
        app.shutdown().await;
    }
    #[test]
    fn transcription_export_cannot_recreate_a_missing_video_as_text() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().canonicalize().unwrap().join("video.mp4");
        let result = transcription::TranscriptResult {
            text: "文案".into(),
            source: transcription::Source {
                path: source.clone(),
                size: 123,
                mtime_ns: "1".into(),
            },
            model_revision: "fixture".into(),
            model_name: None,
            pipeline_revision: "fixture".into(),
            backend: None,
            completed_at: 1,
        };
        assert_eq!(
            transcription::export_txt(&result, &root.path().join("video.mp4"))
                .unwrap_err()
                .to_string(),
            "TRANSCRIPTION_EXPORT_SOURCE"
        );
        assert!(!source.exists());
    }
    #[tokio::test]
    async fn transcription_cuda_vad_progress_and_model_identity_survive_restart() {
        let root = tempfile::tempdir().unwrap();
        let app = core(root.path());
        for (tool, variant) in [(None, Some("cuda")), (Some("ffmpeg"), Some("cuda")), (Some("fun-asr-nano"), Some("metal"))] {
            assert_eq!(app.dependencies_variant(tool.map(str::to_owned), variant.map(str::to_owned), |_| {}).await.unwrap_err().to_string(), "INVALID_INPUT");
        }
        let task = transcript_video(&app, "cuda-speech.mp4").await;
        app.start_transcription(&task.id, false).unwrap();
        let value = transcript_done(&app, &task.id).await;
        assert_eq!(value.state, "completed", "{:?}", value.error_code);
        let result = value.result.unwrap();
        assert_eq!(result.backend.as_deref(), Some("cuda"));
        assert_eq!(result.model_revision, "ce72677f84900f0dc57f498ace253bfb3c9155b6");
        assert_eq!(result.pipeline_revision, "fun-asr-nano-audiocpp-50749561-fsmn-v1");
        app.shutdown().await;
        drop(app);
        let app = core(root.path());
        assert_eq!(app.transcript(&task.id).unwrap().unwrap().result.unwrap().backend.as_deref(), Some("cuda"));
        app.shutdown().await;
    }
    #[tokio::test]
    async fn transcription_segment_progress_is_queryable_and_protocol_regressions_fail() {
        let root = tempfile::tempdir().unwrap();
        let app = core(root.path());
        let task = transcript_video(&app, "hold.mp4").await;
        let started = app.start_transcription(&task.id, false).unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let progress = app.transcript(&task.id).unwrap().unwrap();
                if progress.completed_segments == Some(1) {
                    assert_eq!(progress.total_segments, Some(2));
                    assert_eq!(progress.phase.as_deref(), Some("recognizing"));
                    assert_eq!(app.transcripts().unwrap()[0].completed_segments, Some(1));
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }).await.unwrap();
        app.cancel_transcription(&task.id, &started.operation_id).await.unwrap();
        let canceled = transcript_done(&app, &task.id).await;
        assert!(canceled.completed_segments.is_none() && canceled.total_segments.is_none());
        for mode in ["backwards", "total", "overflow", "missing", "type"] {
            let task = transcript_video(&app, &format!("bad-progress-{mode}.mp4")).await;
            app.start_transcription(&task.id, false).unwrap();
            let result = transcript_done(&app, &task.id).await;
            assert_eq!(result.state, "failed", "{mode}");
            assert_eq!(result.error_code.as_deref(), Some("ENGINE_PROTOCOL_ERROR"));
            assert!(!result.has_result && result.completed_segments.is_none());
        }
        app.shutdown().await;
    }
    #[tokio::test]
    async fn transcription_nano_identity_is_persisted_and_legacy_results_remain_readable() {
        let root = tempfile::tempdir().unwrap();
        let app = core(root.path());
        let task = transcript_video(&app, "speech.mp4").await;
        app.start_transcription(&task.id, false).unwrap();
        let value = transcript_done(&app, &task.id).await;
        assert_eq!(value.state, "completed", "{:?}", value.error_code);
        assert_eq!(value.backend.as_deref(), Some("cpu"));
        let result = value.result.as_ref().unwrap();
        assert_eq!(result.backend.as_deref(), Some("cpu"));
        assert_eq!(result.model_name.as_deref(), Some("Fun-ASR-Nano"));
        assert_eq!(result.model_revision, "46e849502a867080d66d351b8dfb1018b607e509");
        assert_eq!(result.pipeline_revision, "fun-asr-nano-cpu-fbbfe018-v1");
        app.shutdown().await;
        drop(app);
        let app = core(root.path());
        let value = app.transcript(&task.id).unwrap().unwrap();
        assert_eq!(value.backend.as_deref(), Some("cpu"));
        let mut old = serde_json::to_value(value).unwrap();
        old["backend"] = json!("cuda");
        old["result"]["backend"] = json!("cuda");
        old["result"]["model_revision"] = json!("90c1c61912018b70ada0fcc024ea24aca62f2e63");
        old["result"]["pipeline_revision"] = json!("sensevoice-cuda-fbbfe018-v1");
        old["result"].as_object_mut().unwrap().remove("model_name");
        app.inner.lock().unwrap().db.execute("UPDATE video_transcripts SET snapshot=? WHERE task_id=?",
            params![serde_json::to_string(&old).unwrap(), task.id]).unwrap();
        let legacy = app.transcript(&task.id).unwrap().unwrap().result.unwrap();
        assert_eq!(legacy.backend.as_deref(), Some("cuda"));
        assert!(legacy.model_name.is_none());
        assert_eq!(legacy.text, "欢迎体验。\nEnglish text.");
        old.as_object_mut().unwrap().remove("backend");
        old["result"].as_object_mut().unwrap().remove("backend");
        let old: transcription::Snapshot = serde_json::from_value(old).unwrap();
        assert!(old.backend.is_none() && old.result.unwrap().backend.is_none());
        app.shutdown().await;
    }
    #[tokio::test]
    async fn transcription_failure_empty_and_cancel_preserve_previous_success() {
        let root = tempfile::tempdir().unwrap();
        let app = core(root.path());
        let mut task = transcript_video(&app, "speech.mp4").await;
        app.start_transcription(&task.id, false).unwrap();
        transcript_done(&app, &task.id).await;
        for (name, expected) in [
            ("fail.mp4", "failed"),
            ("silent.mp4", "no_speech"),
            ("hold.mp4", "canceled"),
        ] {
            let path = task.path.as_ref().unwrap().with_file_name(name);
            std::fs::rename(task.path.as_ref().unwrap(), &path).unwrap();
            task.path = Some(path);
            {
                let mut inner = app.inner.lock().unwrap();
                *inner.tasks.iter_mut().find(|t| t.id == task.id).unwrap() = task.clone();
            }
            let started = app.start_transcription(&task.id, true).unwrap();
            if expected == "canceled" {
                app.cancel_transcription(&task.id, &started.operation_id)
                    .await
                    .unwrap();
            }
            let result = transcript_done(&app, &task.id).await;
            assert_eq!(result.state, expected);
            assert_eq!(
                app.transcript_text(&task.id).unwrap().text,
                "欢迎体验。\nEnglish text."
            );
        }
        assert!(
            app.inner
                .lock()
                .unwrap()
                .db
                .query_row("SELECT count(*) FROM task_staging", [], |r| r
                    .get::<_, i64>(0))
                .unwrap()
                == 0
        );
        app.shutdown().await;
    }
    #[tokio::test]
    async fn transcription_exclusion_delete_and_shutdown_reap_workers() {
        let root = tempfile::tempdir().unwrap();
        let app = core(root.path());
        let one = transcript_video(&app, "hold.mp4").await;
        let two = transcript_video(&app, "other.mp4").await;
        app.start_transcription(&one.id, false).unwrap();
        assert_eq!(
            app.start_transcription(&two.id, false)
                .err()
                .unwrap()
                .to_string(),
            "TRANSCRIPTION_BUSY"
        );
        assert_eq!(
            app.dependencies(Some("fun-asr-nano".into()), |_| {})
                .await
                .unwrap_err()
                .to_string(),
            "TRANSCRIPTION_BUSY"
        );
        assert!(!app.clear_history(None).unwrap().contains(&one.id));
        app.remove_after_transcription(&one.id, true).await.unwrap();
        assert!(!one.path.unwrap().exists());
        assert!(app.transcripts().unwrap().is_empty());
        let task = transcript_video(&app, "hold-again.mp4").await;
        app.start_transcription(&task.id, false).unwrap();
        tokio::time::timeout(Duration::from_secs(10), app.shutdown())
            .await
            .unwrap();
        assert_eq!(app.transcript(&task.id).unwrap().unwrap().state, "canceled");
    }
    #[tokio::test]
    async fn transcription_database_failure_remains_queryable_without_unsaved_result() {
        let root = tempfile::tempdir().unwrap();
        let app = core(root.path());
        let task = transcript_video(&app, "speech.mp4").await;
        app.inner.lock().unwrap().db.execute_batch("CREATE TRIGGER reject_transcript_end BEFORE UPDATE ON video_transcripts WHEN json_extract(NEW.snapshot,'$.state') != 'running' BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
        app.start_transcription(&task.id, false).unwrap();
        let result = transcript_done(&app, &task.id).await;
        assert_eq!(result.state, "failed");
        assert_eq!(result.error_code.as_deref(), Some("DATABASE_ERROR"));
        assert!(!result.has_result && result.result.is_none());
        assert_eq!(app.transcripts().unwrap()[0].state, "failed");
        assert!(app.transcript_text(&task.id).is_err());
        app.inner
            .lock()
            .unwrap()
            .db
            .execute_batch("DROP TRIGGER reject_transcript_end;")
            .unwrap();
        app.start_transcription(&task.id, true).unwrap();
        assert_eq!(transcript_done(&app, &task.id).await.state, "completed");
        app.inner.lock().unwrap().db.execute_batch("CREATE TRIGGER reject_transcript_end BEFORE UPDATE ON video_transcripts WHEN json_extract(NEW.snapshot,'$.state') != 'running' BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
        app.start_transcription(&task.id, true).unwrap();
        let result = transcript_done(&app, &task.id).await;
        assert_eq!(result.state, "failed");
        assert_eq!(result.result.unwrap().text, "欢迎体验。\nEnglish text.");
        app.shutdown().await;
    }
    #[tokio::test]
    async fn cover_export_preserves_cached_legacy_and_album_bytes_and_real_format() {
        let root = tempfile::tempdir().unwrap();
        let app = core(root.path());
        assert_eq!(
            app.export_cover("missing").unwrap_err().to_string(),
            "TASK_NOT_FOUND"
        );
        let task = app.resolve("valid".into()).unwrap();
        let ready = wait(&app, &task.id, Status::Ready).await;
        let before = serde_json::to_value(&ready).unwrap();
        let (name, bytes) = app.export_cover(&task.id).unwrap();
        assert_eq!(name, "douyin_123.cover.png");
        assert_eq!(bytes, app.cover(&task.id).unwrap());
        assert_eq!(
            serde_json::to_value(app.list().into_iter().find(|t| t.id == task.id).unwrap())
                .unwrap(),
            before
        );
        let path = root.path().join("legacy.cover.jpg");
        for (body, extension) in [
            (b"\xff\xd8\xfffixture".to_vec(), "jpeg"),
            (b"RIFF\x10\0\0\0WEBPfixture".to_vec(), "webp"),
        ] {
            std::fs::write(&path, &body).unwrap();
            {
                let mut inner = app.inner.lock().unwrap();
                let task = inner.tasks.iter_mut().find(|t| t.id == task.id).unwrap();
                task.cover_path = Some(path.clone());
                task.cover_cached = false;
            }
            let (name, bytes) = app.export_cover(&task.id).unwrap();
            assert_eq!(name, format!("douyin_123.cover.{extension}"));
            assert_eq!(bytes, body);
            assert_eq!(std::fs::read(&path).unwrap(), body);
        }
        std::fs::remove_file(&path).unwrap();
        assert!(app.export_cover(&task.id).is_err());
        let album = app.resolve("album".into()).unwrap();
        wait(&app, &album.id, Status::Ready).await;
        let (name, bytes) = app.export_cover(&album.id).unwrap();
        assert!(name.starts_with("douyin_601.cover."));
        assert_eq!(bytes, app.image(&album.id, 0).unwrap());
        app.shutdown().await;
    }

    #[tokio::test]
    async fn author_avatar_survives_restart_and_reads_only_task_owned_cache() {
        let root = tempfile::tempdir().unwrap();
        let app = core(root.path());
        let task = app.resolve("valid".into()).unwrap();
        let ready = wait(&app, &task.id, Status::Ready).await;
        let video = ready.video.as_ref().unwrap();
        assert_eq!(video["author_profile"]["id"], "fixture-author");
        assert_eq!(video["author_profile"]["nickname"], "作者昵称");
        assert_eq!(video["author_profile"]["avatar_cached"], true);
        assert!(video.get("author_avatar_filename").is_none());
        let body = app.author_avatar(&task.id).unwrap();
        assert_eq!(body, b"\x89PNG\r\n\x1a\nfixture");
        assert!(app.author_avatar("not-a-task").is_err());
        app.shutdown().await;
        drop(app);
        let app = core(root.path());
        assert_eq!(app.author_avatar(&task.id).unwrap(), body);
        app.download(&task.id, false).unwrap();
        wait(&app, &task.id, Status::Completed).await;
        app.remove_history(&task.id).unwrap();
        assert!(app.author_avatar(&task.id).is_err());
        assert!(parse_cache::avatar(&app.inner.lock().unwrap().db, video).is_err());
        app.shutdown().await;
    }

    #[tokio::test]
    async fn album_author_avatar_is_released_without_deleting_cached_album_images() {
        let root = tempfile::tempdir().unwrap();
        let app = core(root.path());
        let task = app.resolve("album".into()).unwrap();
        let ready = wait(&app, &task.id, Status::Ready).await;
        let video = ready.video.unwrap();
        let body = b"\x89PNG\r\n\x1a\nfixture";
        parse_cache::save_avatar(&app.inner.lock().unwrap().db, &video, body).unwrap();
        assert_eq!(app.author_avatar(&task.id).unwrap(), body);
        app.download(&task.id, false).unwrap();
        wait(&app, &task.id, Status::Completed).await;
        app.remove_history(&task.id).unwrap();
        {
            let inner = app.inner.lock().unwrap();
            assert!(parse_cache::avatar(&inner.db, &video).is_err());
            assert!(parse_cache::image(&inner.db, &video, 0).is_ok());
        }
        app.shutdown().await;
    }

    #[tokio::test]
    async fn playback_uses_only_completed_video_records_and_preserves_files() {
        let root = tempfile::tempdir().unwrap();
        let app = core(root.path());
        assert_eq!(
            app.playback_path("missing").unwrap_err().to_string(),
            "TASK_NOT_FOUND"
        );
        let task = app.resolve("valid".into()).unwrap();
        wait(&app, &task.id, Status::Ready).await;
        assert_eq!(
            app.playback_path(&task.id).unwrap_err().to_string(),
            "INVALID_STATE"
        );
        app.download(&task.id, false).unwrap();
        let done = wait(&app, &task.id, Status::Completed).await;
        let path = done.path.unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(app.playback_path(&task.id).unwrap(), path);
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        let album = app.resolve("album".into()).unwrap();
        wait(&app, &album.id, Status::Ready).await;
        app.download(&album.id, false).unwrap();
        wait(&app, &album.id, Status::Completed).await;
        assert_eq!(
            app.playback_path(&album.id).unwrap_err().to_string(),
            "INVALID_STATE"
        );
        app.shutdown().await;
        drop(app);
        let app = core(root.path());
        assert_eq!(app.playback_path(&task.id).unwrap(), path);
        std::fs::remove_file(&path).unwrap();
        assert_eq!(
            app.playback_path(&task.id).unwrap_err().to_string(),
            "FILE_MISSING"
        );
        std::fs::create_dir(&path).unwrap();
        assert_eq!(
            app.playback_path(&task.id).unwrap_err().to_string(),
            "FILE_MISSING"
        );
        assert!(path.is_dir());
        app.shutdown().await;
    }

    #[tokio::test]
    async fn bulk_history_clear_is_atomic_and_preserves_live_tasks_previews_and_files() {
        let root = tempfile::tempdir().unwrap();
        let app = core(root.path());
        let sample = app.resolve("valid".into()).unwrap();
        let sample = wait(&app, &sample.id, Status::Ready).await;
        app.cancel(&sample.id).await.unwrap();
        app.remove_history(&sample.id).unwrap();
        let downloads = root.path().join("downloads");
        std::fs::create_dir_all(&downloads).unwrap();
        let video_path = downloads.join("saved.mp4");
        let cover_path = downloads.join("saved.cover.jpg");
        let album_path = downloads.join("album");
        std::fs::create_dir(&album_path).unwrap();
        std::fs::write(&video_path, b"original video").unwrap();
        std::fs::write(&cover_path, b"original cover").unwrap();
        std::fs::write(album_path.join("001.jpg"), b"original image").unwrap();
        let telegram = json!({"platform":"telegram","video_id":"100_9","resolver":"tdl","kind":"video",
            "canonical_url":"https://t.me/c/100/9","selected_format_id":"original",
            "formats":[{"format_id":"original","container":"mp4","filesize":4096}]});
        let mut retained_video = telegram.clone();
        retained_video["video_id"] = "101_9".into();
        retained_video["canonical_url"] = "https://t.me/c/101/9".into();
        let cover = b"\x89PNG\r\n\x1a\nfixture".to_vec();
        {
            let mut inner = app.inner.lock().unwrap();
            for (id, status, requested) in [
                ("completed", Status::Completed, true),
                ("failed", Status::Failed, true),
                ("canceled", Status::Canceled, true),
                ("tg-one", Status::Completed, true),
                ("tg-two", Status::Completed, true),
                ("tg-shared", Status::Completed, true),
                ("running", Status::Downloading, true),
                ("queued", Status::Queued, true),
                ("parsing", Status::Parsing, true),
                ("preview", Status::Ready, false),
                ("canceled-preview", Status::Canceled, false),
                ("finishing", Status::Completed, true),
            ] {
                let mut task = sample.clone();
                task.id = id.into();
                task.status = status;
                task.download_requested = requested;
                if id.starts_with("tg-") {
                    task.video = Some(if id == "tg-shared" {
                        retained_video.clone()
                    } else {
                        telegram.clone()
                    });
                    task.cover_cached = true;
                }
                if id == "preview" {
                    task.video = Some(retained_video.clone());
                    task.cover_cached = true;
                }
                if id == "completed" {
                    task.path = Some(video_path.clone());
                    task.cover_path = Some(cover_path.clone());
                }
                if id == "failed" {
                    task.path = Some(album_path.clone());
                }
                persist(&inner.db, &task).unwrap();
                inner.tasks.push(task);
            }
            parse_cache::save(&mut inner.db, &telegram, vec![], Some(cover.clone()), 100).unwrap();
            parse_cache::save(
                &mut inner.db,
                &retained_video,
                vec![],
                Some(cover.clone()),
                100,
            )
            .unwrap();
            // A terminal snapshot can precede worker destruction; it remains ineligible.
            inner.downloads.insert("finishing".into());
            inner
                .db
                .execute_batch(
                    "CREATE TRIGGER block_history_clear BEFORE DELETE ON download_tasks
                WHEN OLD.id='failed' BEGIN SELECT RAISE(ABORT, 'test clear error'); END;",
                )
                .unwrap();
        }
        assert_eq!(
            app.clear_history(None).unwrap_err().to_string(),
            "DATABASE_ERROR"
        );
        assert_eq!(app.list().len(), 12);
        {
            let inner = app.inner.lock().unwrap();
            assert_eq!(
                inner
                    .db
                    .query_row("SELECT count(*) FROM download_tasks", [], |r| r
                        .get::<_, i64>(0))
                    .unwrap(),
                12
            );
            assert_eq!(parse_cache::cover(&inner.db, &telegram).unwrap(), cover);
            inner
                .db
                .execute_batch("DROP TRIGGER block_history_clear")
                .unwrap();
        }
        let removed: HashSet<_> = app.clear_history(None).unwrap().into_iter().collect();
        assert_eq!(
            removed,
            [
                "completed",
                "failed",
                "canceled",
                "tg-one",
                "tg-two",
                "tg-shared"
            ]
            .map(String::from)
            .into_iter()
            .collect()
        );
        assert!(app.clear_history(None).unwrap().is_empty());
        assert_eq!(app.list().len(), 6);
        assert_eq!(std::fs::read(&video_path).unwrap(), b"original video");
        assert_eq!(std::fs::read(&cover_path).unwrap(), b"original cover");
        assert_eq!(
            std::fs::read(album_path.join("001.jpg")).unwrap(),
            b"original image"
        );
        {
            let inner = app.inner.lock().unwrap();
            assert_eq!(
                inner
                    .db
                    .query_row("SELECT count(*) FROM download_tasks", [], |r| r
                        .get::<_, i64>(0))
                    .unwrap(),
                6
            );
            assert!(parse_cache::cover(&inner.db, &telegram).is_err());
            assert_eq!(
                parse_cache::cover(&inner.db, &retained_video).unwrap(),
                cover
            );
            assert!(parse_cache::find(&inner.db, "https://t.me/c/100/9", "tdl")
                .unwrap()
                .is_some());
        }
        app.shutdown().await;
        drop(app);
        let app = core(root.path());
        assert!(app.list().iter().all(|t| !removed.contains(&t.id)));
        assert!(app.cover("preview").is_ok());
        app.shutdown().await;
    }

    #[tokio::test]
    async fn history_clear_scopes_platforms_and_preserves_other_tabs_and_live_tasks() {
        let root = tempfile::tempdir().unwrap();
        let app = core(root.path());
        let sample = app.resolve("valid".into()).unwrap();
        let sample = wait(&app, &sample.id, Status::Ready).await;
        app.cancel(&sample.id).await.unwrap();
        app.remove_history(&sample.id).unwrap();
        {
            let mut inner = app.inner.lock().unwrap();
            for (id, platform, status, requested) in [
                ("dy", Some("douyin"), Status::Completed, true),
                ("tt", Some("tiktok"), Status::Failed, true),
                ("yt", Some("youtube"), Status::Canceled, true),
                ("tg", Some("telegram"), Status::Completed, true),
                ("tg-link", None, Status::Failed, true),
                ("unknown", None, Status::Failed, true),
                ("live", Some("douyin"), Status::Downloading, true),
                ("queue", Some("douyin"), Status::Queued, true),
                ("preview", Some("douyin"), Status::Canceled, false),
                ("busy", Some("douyin"), Status::Completed, true),
            ] {
                let mut task = sample.clone();
                task.id = id.into();
                task.status = status;
                task.download_requested = requested;
                task.video = platform.map(|platform| {
                    let mut video = sample.video.clone().unwrap();
                    video["platform"] = platform.into();
                    video["canonical_url"] = match platform {
                        "tiktok" => "https://www.tiktok.com/@fixture/video/123",
                        "youtube" => "https://www.youtube.com/watch?v=fixture0001",
                        "telegram" => "https://t.me/c/100/9",
                        _ => "https://www.douyin.com/video/123",
                    }
                    .into();
                    if platform == "youtube" {
                        video["video_id"] = "fixture0001".into();
                    }
                    if platform == "telegram" {
                        video["video_id"] = "100_9".into();
                        video["resolver"] = "tdl".into();
                        video["selected_format_id"] = "original".into();
                        video["formats"] =
                            json!([{"format_id":"original","container":"mp4","filesize":4096}]);
                    }
                    video
                });
                task.telegram_link = (id == "tg-link").then(|| "https://t.me/c/100/9".into());
                persist(&inner.db, &task).unwrap();
                inner.tasks.push(task);
            }
            inner.downloads.insert("busy".into());
        }
        assert_eq!(
            app.clear_history(Some("invalid")).unwrap_err().to_string(),
            "CONFIG_INVALID"
        );
        assert_eq!(app.list().len(), 10);
        assert_eq!(app.clear_history(Some("douyin")).unwrap(), ["dy"]);
        assert_eq!(app.clear_history(Some("tiktok")).unwrap(), ["tt"]);
        assert_eq!(app.clear_history(Some("youtube")).unwrap(), ["yt"]);
        let telegram: HashSet<_> = app
            .clear_history(Some("telegram"))
            .unwrap()
            .into_iter()
            .collect();
        assert_eq!(
            telegram,
            ["tg".to_owned(), "tg-link".to_owned()]
                .into_iter()
                .collect()
        );
        assert!(app.clear_history(Some("douyin")).unwrap().is_empty());
        assert_eq!(app.clear_history(None).unwrap(), ["unknown"]);
        let remaining: HashSet<_> = app.list().into_iter().map(|t| t.id).collect();
        assert_eq!(
            remaining,
            ["live", "queue", "preview", "busy"]
                .map(String::from)
                .into_iter()
                .collect()
        );
        let inner = app.inner.lock().unwrap();
        assert_eq!(
            inner
                .db
                .query_row("SELECT count(*) FROM download_tasks", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            4
        );
    }

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
    async fn runtime_dependencies_check_install_and_cancel_share_worker_lifecycle() {
        let root = tempfile::tempdir().unwrap();
        let app = core(root.path());
        let status = app.dependencies(None, |_| {}).await.unwrap();
        for tool in ["sensevoice", "sensevoice-cuda", "arbitrary-url"] {
            assert_eq!(app.dependencies(Some(tool.into()), |_| {}).await.unwrap_err().to_string(), "INVALID_INPUT");
        }
        assert_eq!(
            status["directory"],
            root.path().join("runtime").to_string_lossy().as_ref()
        );
        assert!(app
            .dependencies(Some("arbitrary-url".into()), |_| {})
            .await
            .is_err());
        let progress = Arc::new(Mutex::new(Vec::new()));
        let observed = progress.clone();
        app.dependencies(Some("ffmpeg".into()), move |value| {
            observed.lock().unwrap().push(value)
        })
        .await
        .unwrap();
        assert_eq!(progress.lock().unwrap().len(), 2);
        assert_eq!(
            std::fs::read_dir(root.path().join("runtime"))
                .unwrap()
                .count(),
            0
        );
        let (send, receive) = tokio::sync::oneshot::channel();
        let running = app.clone();
        let mut send = Some(send);
        let worker = tokio::spawn(async move {
            running
                .dependencies(Some("javascript".into()), move |_| {
                    if let Some(send) = send.take() {
                        let _ = send.send(());
                    }
                })
                .await
        });
        tokio::time::timeout(Duration::from_secs(10), receive)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            app.dependencies(None, |_| {})
                .await
                .unwrap_err()
                .to_string(),
            "DEPENDENCY_BUSY"
        );
        app.cancel_dependency().unwrap();
        assert_eq!(worker.await.unwrap().unwrap_err().to_string(), "CANCELED");
        assert_eq!(
            std::fs::read_dir(root.path().join("runtime"))
                .unwrap()
                .count(),
            0
        );
        app.dependencies(None, |_| {}).await.unwrap();
        app.shutdown().await;
    }

    #[tokio::test]
    async fn canceled_dependency_commit_restores_directories_and_backend_choice() {
        for variant in ["cpu", "cuda"] {
            for had_target in [false, true] {
                for phase in ["between-renames", "after-choice"] {
                    let root = tempfile::tempdir().unwrap();
                    let app = core(root.path());
                    let runtime = root.path().join("runtime");
                    std::fs::create_dir_all(&runtime).unwrap();
                    let name = if variant == "cuda" { "fun-asr-nano-cuda" } else { "fun-asr-nano" };
                    let target = runtime.join(name);
                    if had_target {
                        std::fs::create_dir(&target).unwrap();
                        std::fs::write(target.join("old-model"), b"original").unwrap();
                    }
                    let selection = runtime.join("fun-asr-nano-backend.json");
                    let original = (phase == "after-choice").then_some(b"{\"variant\":\"cpu\"}".to_vec());
                    if let Some(bytes) = &original { std::fs::write(&selection, bytes).unwrap(); }
                    std::fs::write(root.path().join("dependency-commit-mode"), phase).unwrap();
                    let running = app.clone();
                    let worker = tokio::spawn(async move {
                        running.dependencies_variant(Some("fun-asr-nano".into()), Some(variant.into()), |_| {}).await
                    });
                    tokio::time::timeout(Duration::from_secs(10), async {
                        while !root.path().join("dependency-commit-started").is_file() {
                            tokio::time::sleep(Duration::from_millis(10)).await;
                        }
                    }).await.unwrap();
                    if phase == "after-choice" && variant == "cuda" && had_target {
                        app.shutdown().await;
                    } else { app.cancel_dependency().unwrap(); }
                    assert_eq!(worker.await.unwrap().unwrap_err().to_string(), "CANCELED");
                    assert_eq!(std::fs::read(&selection).ok(), original);
                    if had_target {
                        assert_eq!(std::fs::read(target.join("old-model")).unwrap(), b"original");
                        assert!(!target.join("new-model").exists());
                    } else { assert!(!target.exists()); }
                    assert!(!std::fs::read_dir(&runtime).unwrap().any(|entry|
                        entry.unwrap().file_name().to_string_lossy().starts_with(".install-")));
                    app.shutdown().await;
                }
            }
        }
    }

    #[tokio::test]
    async fn shutdown_cancels_and_reaps_dependency_download_before_releasing_core() {
        let root = tempfile::tempdir().unwrap();
        let app = core(root.path());
        let (send, receive) = tokio::sync::oneshot::channel();
        let running = app.clone();
        let mut send = Some(send);
        let worker = tokio::spawn(async move {
            running
                .dependencies(Some("javascript".into()), move |_| {
                    if let Some(send) = send.take() {
                        let _ = send.send(());
                    }
                })
                .await
        });
        tokio::time::timeout(Duration::from_secs(10), receive)
            .await
            .unwrap()
            .unwrap();
        tokio::time::timeout(Duration::from_secs(10), app.shutdown())
            .await
            .unwrap();
        assert_eq!(worker.await.unwrap().unwrap_err().to_string(), "CANCELED");
        assert_eq!(
            std::fs::read_dir(root.path().join("runtime"))
                .unwrap()
                .count(),
            0
        );
        assert!(app.dependencies(None, |_| {}).await.is_err());
    }

    #[tokio::test]
    async fn live_cookie_warning_reaches_ui_but_is_not_replayed_from_cache() {
        let root = tempfile::tempdir().unwrap();
        let app = core(root.path());
        let task = app.resolve("cookie-warning".into()).unwrap();
        let ready = wait(&app, &task.id, Status::Ready).await;
        assert_eq!(
            ready.video.as_ref().unwrap()["cookie_warning"],
            "BROWSER_COOKIES_UNAVAILABLE"
        );
        app.cancel(&task.id).await.unwrap();
        let cached = app
            .resolve("https://www.douyin.com/video/123".into())
            .unwrap();
        let cached = wait(&app, &cached.id, Status::Ready).await;
        assert!(cached.cache_hit);
        assert!(cached.video.as_ref().unwrap()["cookie_warning"].is_null());
        app.shutdown().await;
    }
    #[tokio::test]
    async fn selected_format_survives_restart_and_reaches_fresh_download() {
        let root = tempfile::tempdir().unwrap();
        let app = core(root.path());
        let task = app.resolve("valid".into()).unwrap();
        wait(&app, &task.id, Status::Ready).await;
        assert!(app
            .choose_format(&task.id, "https://attacker.invalid/video")
            .is_err());
        let chosen = app.choose_format(&task.id, &"2".repeat(24)).unwrap();
        assert_eq!(chosen.video.as_ref().unwrap()["width"], 720);
        assert_eq!(chosen.video.as_ref().unwrap()["filesize"], 20);
        app.shutdown().await;
        drop(app);
        let restarted = core(root.path());
        let restored = &restarted.list()[0];
        assert_eq!(restored.status, Status::Ready);
        assert_eq!(
            restored.video.as_ref().unwrap()["selected_format_id"],
            "2".repeat(24)
        );
        assert_eq!(restored.video.as_ref().unwrap()["width"], 720);
        restarted.download(&task.id, false).unwrap();
        assert!(restarted.choose_format(&task.id, &"1".repeat(24)).is_err());
        let completed = wait(&restarted, &task.id, Status::Completed).await;
        let video = completed.video.as_ref().unwrap();
        assert_eq!(video["selected_format_id"], "2".repeat(24));
        assert_eq!(video["width"], 720);
        assert_eq!(video["height"], 1280);
        assert_eq!(video["filesize"], completed.bytes);
    }

    #[tokio::test]
    async fn retry_preserves_the_selected_format_instead_of_resetting_to_default() {
        let root = tempfile::tempdir().unwrap();
        let app = core(root.path());
        let task = app.resolve("valid".into()).unwrap();
        wait(&app, &task.id, Status::Ready).await;
        app.choose_format(&task.id, &"2".repeat(24)).unwrap();
        app.cancel(&task.id).await.unwrap();
        let retried = app.retry(&task.id).unwrap();
        assert_eq!(retried.id, task.id);
        let completed = wait(&app, &retried.id, Status::Completed).await;
        assert_eq!(completed.video.as_ref().unwrap()["width"], 720);
        assert_eq!(
            completed.video.as_ref().unwrap()["selected_format_id"],
            "2".repeat(24)
        );
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
    async fn album_preview_commit_restart_and_owned_file_deletion() {
        let root = tempfile::tempdir().unwrap();
        let app = core(root.path());
        let task = app
            .resolve_with_resolver("album".into(), "dtk".into())
            .unwrap();
        let ready = wait(&app, &task.id, Status::Ready).await;
        assert_eq!(
            ready.video.as_ref().unwrap()["images"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        assert!(app
            .image(&task.id, 0)
            .unwrap()
            .starts_with(&[0xff, 0xd8, 0xff]));
        assert!(app.image(&task.id, 64).is_err());
        app.download(&task.id, false).unwrap();
        let completed = wait(&app, &task.id, Status::Completed).await;
        assert_eq!(completed.files.len(), 2);
        assert_eq!(completed.completed_items, Some(2));
        let path = completed.path.clone().unwrap();
        assert_eq!(
            validate_album(&path, &completed.files).unwrap(),
            completed.bytes
        );
        app.shutdown().await;
        drop(app);
        let restored = core(root.path());
        assert_eq!(restored.list()[0].status, Status::Completed);
        assert!(!restored.list()[0].file_missing);
        assert!(restored.image(&task.id, 1).unwrap().starts_with(b"RIFF"));
        // Simulate a crash after the directory move, before publishing completed.
        let mut interrupted = completed.clone();
        interrupted.status = Status::Downloading;
        interrupted.files.clear();
        interrupted.bytes = 0;
        interrupted.path = None;
        {
            let inner = restored.inner.lock().unwrap();
            persist(&inner.db, &interrupted).unwrap();
        }
        drop(restored);
        let restored = core(root.path());
        let recovered = restored.list().remove(0);
        assert_eq!(recovered.status, Status::Completed);
        assert_eq!(recovered.files.len(), 2);
        assert_eq!(recovered.bytes, completed.bytes);
        assert_eq!(recovered.completed_items, Some(2));
        assert!(restored.image(&task.id, 1).unwrap().starts_with(b"RIFF"));
        std::fs::write(path.join("user-note.txt"), "preserve").unwrap();
        restored.delete_local_file(&task.id).unwrap();
        assert_eq!(
            std::fs::read(path.join("user-note.txt")).unwrap(),
            b"preserve"
        );
        for file in &completed.files {
            assert!(!path.join(&file.filename).exists());
        }
        assert!(restored.list().is_empty());
        drop(restored);
        assert!(core(root.path()).list().is_empty());
    }

    #[tokio::test]
    async fn sqlite_previews_survive_restart_without_an_engine_but_download_requires_one() {
        let root = tempfile::tempdir().unwrap();
        let app = core(root.path());
        let video = app.resolve("valid".into()).unwrap();
        let video = wait(&app, &video.id, Status::Ready).await;
        assert!(!video.cache_hit && video.cover_cached);
        let album = app.resolve("album".into()).unwrap();
        let album = wait(&app, &album.id, Status::Ready).await;
        assert!(!album.cache_hit);
        app.shutdown().await;
        drop(app);
        assert_eq!(
            std::fs::read_dir(root.path().join("downloads"))
                .unwrap()
                .count(),
            0
        );
        let restarted = Core::open(
            &root.path().join("tasks.sqlite"),
            root.path().join("downloads"),
            root.path().join("no-engine").to_string_lossy().into_owned(),
            vec![],
            |_| {},
        )
        .unwrap();
        assert!(restarted
            .list()
            .iter()
            .all(|t| t.status == Status::Ready && t.cache_hit));
        assert!(restarted.cover(&video.id).unwrap().starts_with(b"\x89PNG"));
        assert!(restarted.image(&album.id, 1).unwrap().starts_with(b"RIFF"));
        let cached = restarted
            .resolve("private copy https://www.douyin.com/note/601/?tracking=secret".into())
            .unwrap();
        assert_eq!(cached.status, Status::Ready);
        assert!(cached.cache_hit);
        assert!(restarted
            .image(&cached.id, 0)
            .unwrap()
            .starts_with(&[0xff, 0xd8, 0xff]));
        let other = restarted
            .resolve_with_resolver("https://www.douyin.com/video/123".into(), "dtk".into())
            .unwrap();
        let failed = wait(&restarted, &other.id, Status::Failed).await;
        assert_eq!(failed.error.as_deref(), Some("ENGINE_UNAVAILABLE"));
        restarted.download(&cached.id, false).unwrap();
        let failed = wait(&restarted, &cached.id, Status::Failed).await;
        assert_eq!(failed.error.as_deref(), Some("ENGINE_UNAVAILABLE"));
        assert!(failed.path.is_none());
    }

    #[tokio::test]
    async fn new_short_alias_reuses_identity_cache_and_persists_for_offline_reuse() {
        let root = tempfile::tempdir().unwrap();
        let app = core(root.path());
        let album = app.resolve("album".into()).unwrap();
        wait(&app, &album.id, Status::Ready).await;
        let short = app
            .resolve("https://v.douyin.com/share_album/".into())
            .unwrap();
        let ready = wait(&app, &short.id, Status::Ready).await;
        assert!(ready.cache_hit);
        assert_eq!(ready.video.as_ref().unwrap()["video_id"], "601");
        app.shutdown().await;
        drop(app);
        let restarted = Core::open(
            &root.path().join("tasks.sqlite"),
            root.path().join("downloads"),
            "missing-engine".into(),
            vec![],
            |_| {},
        )
        .unwrap();
        let cached = restarted
            .resolve("7.10 share https://v.douyin.com/share_album/ 复制".into())
            .unwrap();
        assert_eq!(cached.status, Status::Ready);
        assert!(cached.cache_hit);
    }

    #[tokio::test]
    async fn legacy_completed_album_seeds_sqlite_assets_for_an_existing_preview() {
        let root = tempfile::tempdir().unwrap();
        let app = core(root.path());
        let album = app.resolve("album".into()).unwrap();
        wait(&app, &album.id, Status::Ready).await;
        app.download(&album.id, false).unwrap();
        let completed = wait(&app, &album.id, Status::Completed).await;
        let preview = app.resolve("album".into()).unwrap();
        wait(&app, &preview.id, Status::Ready).await;
        app.shutdown().await;
        app.inner.lock().unwrap().db.execute_batch(
            "DROP TABLE parse_cache_aliases;DROP TABLE parse_cache_images;DROP TABLE parse_cache;").unwrap();
        drop(app);
        let restored = Core::open(
            &root.path().join("tasks.sqlite"),
            root.path().join("downloads"),
            "missing-engine".into(),
            vec![],
            |_| {},
        )
        .unwrap();
        std::fs::remove_dir_all(completed.path.unwrap()).unwrap();
        assert!(restored.image(&preview.id, 1).unwrap().starts_with(b"RIFF"));
        let cached = restored
            .resolve("https://www.douyin.com/note/601".into())
            .unwrap();
        assert!(cached.cache_hit);
        assert_eq!(cached.status, Status::Ready);
    }
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn shutdown_allows_immediate_reopen_after_completion() {
        let root = tempfile::tempdir().unwrap();
        for _ in 0..16 {
            let app = core(root.path());
            let task = app.resolve("valid".into()).unwrap();
            wait(&app, &task.id, Status::Ready).await;
            app.download(&task.id, true).unwrap();
            wait(&app, &task.id, Status::Completed).await;
            app.shutdown().await;
            drop(app);
            // No retry or sleep: shutdown must release every worker reference.
            let reopened = core(root.path());
            assert!(reopened.list().iter().any(|t| t.id == task.id));
        }
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
    async fn ready_restart_restores_preview_and_input_is_not_stored() {
        let root = tempfile::tempdir().unwrap();
        let app = core(root.path());
        let task = app.resolve("private share text".into()).unwrap();
        wait(&app, &task.id, Status::Ready).await;
        // Ready is published before the worker releases its Core reference.
        // Preserve the Ready snapshot for recovery, but await actual process exit.
        let worker = app.inner.lock().unwrap().workers.pop().unwrap();
        worker.await.unwrap();
        drop(app);
        let restarted = core(root.path());
        assert_eq!(restarted.list()[0].status, Status::Ready);
        assert!(restarted.list()[0].cache_hit);
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
            total_bytes_estimate: None,
            speed_bps: None,
            eta_seconds: None,
            merging: false,
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
            files: Vec::new(),
            completed_items: None,
            total_items: None,
            cache_hit: false,
            cached_at: None,
            cover_cached: false,
            telegram_link: None,
        };
        {
            let inner = app.inner.lock().unwrap();
            persist(&inner.db, &task).unwrap();
            inner
                .db
                .execute(
                    "INSERT INTO commit_intents(task_id,source,target,size) VALUES(?1,?2,?3,8)",
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
            kind: "video".into(),
            resolver: "yt-dlp".into(),
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
        app.save_profile(settings::ProfileUpdate {
            platform: "youtube".into(),
            user_agent: "youtube-fixture".into(),
            cookie: None,
            proxy: None,
            http_chunk_size: Some(Some(10_485_760)),
        })
        .unwrap();
        let a = enqueue(&app, "8201").await;
        let old = app.settings().profiles[0].user_agent.clone();
        let b = enqueue(&app, "8202").await;
        app.save_profile(settings::ProfileUpdate {
            http_chunk_size: None,
            platform: "douyin".into(),
            user_agent: "new-agent".into(),
            cookie: None,
            proxy: None,
        })
        .unwrap();
        app.save_profile(settings::ProfileUpdate {
            platform: "youtube".into(),
            user_agent: "youtube-fixture".into(),
            cookie: None,
            proxy: None,
            http_chunk_size: Some(Some(0)),
        })
        .unwrap();
        app.cancel(&a.id).await.unwrap();
        started(root.path(), "8202").await;
        let captured: Value = serde_json::from_str(
            &std::fs::read_to_string(root.path().join("started-8202")).unwrap(),
        )
        .unwrap();
        assert_eq!(captured["douyin"]["user_agent"], old);
        assert_eq!(captured["youtube"]["http_chunk_size"], 10_485_760);
        assert_eq!(app.settings().profiles[2].http_chunk_size, Some(0));
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
    async fn retry_reuses_record_with_new_settings_and_survives_restart() {
        let root = tempfile::tempdir().unwrap();
        let app = core(root.path());
        let old = enqueue(&app, "8301").await;
        started(root.path(), "8301").await;
        assert!(app.remove_history(&old.id).is_err());
        assert!(app.delete_local_file(&old.id).is_err());
        app.cancel(&old.id).await.unwrap();
        let canceled = app.list().into_iter().find(|t| t.id == old.id).unwrap();
        std::fs::remove_file(root.path().join("started-8301")).unwrap();
        app.save_profile(settings::ProfileUpdate {
            http_chunk_size: None,
            platform: "douyin".into(),
            user_agent: "retry-agent".into(),
            cookie: None,
            proxy: None,
        })
        .unwrap();
        let retry = app.retry(&old.id).unwrap();
        assert_eq!(retry.id, old.id);
        assert_eq!(retry.created_at, old.created_at);
        assert!(retry.revision > canceled.revision);
        assert_eq!(retry.bytes, 0);
        assert!(retry.error.is_none());
        assert_eq!(app.list().len(), 1);
        assert!(retry.download_requested);
        started(root.path(), "8301").await;
        let profile: Value = serde_json::from_str(
            &std::fs::read_to_string(root.path().join("started-8301")).unwrap(),
        )
        .unwrap();
        assert_eq!(profile["douyin"]["user_agent"], "retry-agent");
        assert_eq!(
            app.list().iter().find(|t| t.id == old.id).unwrap().status,
            Status::Downloading
        );
        release(root.path(), "8301");
        let completed = wait(&app, &retry.id, Status::Completed).await;
        app.shutdown().await;
        assert!(app.retry(&retry.id).is_err());
        drop(app);
        let app = core(root.path());
        assert_eq!(app.list().len(), 1);
        let restored = &app.list()[0];
        assert_eq!(restored.id, old.id);
        assert_eq!(restored.created_at, old.created_at);
        assert_eq!(restored.status, Status::Completed);
        assert_eq!(restored.path, completed.path);
        app.shutdown().await;
    }

    #[tokio::test]
    async fn failed_retry_reuses_progress_record_and_reparse_failure_keeps_identity() {
        let root = tempfile::tempdir().unwrap();
        let app = core(root.path());
        let task = enqueue(&app, "7301").await;
        started(root.path(), "7301").await;
        release(root.path(), "7301");
        let failed = wait(&app, &task.id, Status::Failed).await;
        assert_eq!(failed.bytes, 7);
        assert_eq!(failed.error.as_deref(), Some("NETWORK_ERROR"));
        // Wait for the completed attempt's cleanup, without shutting down the core.
        let workers = std::mem::take(&mut app.inner.lock().unwrap().workers);
        for worker in workers {
            worker.await.unwrap();
        }
        {
            let inner = app.inner.lock().unwrap();
            inner
                .db
                .execute_batch(
                    "DELETE FROM parse_cache;
                CREATE TRIGGER reject_retry_preview BEFORE INSERT ON parse_cache
                BEGIN SELECT RAISE(ABORT, 'test reparse failure'); END;",
                )
                .unwrap();
        }
        let retry = app.retry(&task.id).unwrap();
        assert_eq!(retry.id, task.id);
        assert_eq!(retry.bytes, 0);
        assert!(retry.total_bytes.is_none());
        assert!(retry.error.is_none());
        assert!(retry.queue_order.is_none());
        assert!(retry.completed_at.is_none());
        assert!(retry.revision > failed.revision);
        let failed_again = wait(&app, &task.id, Status::Failed).await;
        assert_eq!(failed_again.error.as_deref(), Some("DATABASE_ERROR"));
        assert_eq!(failed_again.video.as_ref().unwrap()["video_id"], "7301");
        assert_eq!(app.list().len(), 1);
        app.inner
            .lock()
            .unwrap()
            .db
            .execute_batch("DROP TRIGGER reject_retry_preview")
            .unwrap();
        let retry = app.retry(&task.id).unwrap();
        assert_eq!(retry.id, task.id);
        assert!(retry.revision > failed_again.revision);
        let failed_again = wait(&app, &task.id, Status::Failed).await;
        assert_eq!(failed_again.error.as_deref(), Some("NETWORK_ERROR"));
        assert_eq!(app.list().len(), 1);
        app.shutdown().await;
    }

    #[tokio::test]
    async fn retry_is_atomic_cleans_old_attempt_and_never_overwrites_committed_files() {
        let root = tempfile::tempdir().unwrap();
        let app = core(root.path());
        let task = app.resolve("valid".into()).unwrap();
        wait(&app, &task.id, Status::Ready).await;
        app.cancel(&task.id).await.unwrap();
        let old = app.list()[0].clone();
        let target = root
            .path()
            .join("downloads")
            .join(format!("douyin_123_{}.mp4", task.id));
        std::fs::write(&target, b"previous committed file").unwrap();
        let staging = root
            .path()
            .join("downloads")
            .join(format!("yingqu-{}-old", task.id));
        std::fs::create_dir(&staging).unwrap();
        std::fs::write(staging.join("video.mp4.part"), b"partial").unwrap();
        {
            let mut inner = app.inner.lock().unwrap();
            inner.downloads.insert(task.id.clone());
        }
        assert_eq!(app.retry(&task.id).err().unwrap().to_string(), "TASK_BUSY");
        {
            let mut inner = app.inner.lock().unwrap();
            inner.downloads.remove(&task.id);
            inner
                .db
                .execute(
                    "INSERT INTO task_staging VALUES(?1,?2)",
                    params![task.id, staging.to_string_lossy()],
                )
                .unwrap();
            inner
                .db
                .execute(
                    "INSERT INTO commit_intents(task_id,source,target,size) VALUES(?1,?2,?3,23)",
                    params![
                        task.id,
                        staging.join("video.mp4").to_string_lossy(),
                        target.to_string_lossy()
                    ],
                )
                .unwrap();
            inner
                .db
                .execute_batch(
                    "CREATE TRIGGER reject_retry_reset BEFORE UPDATE ON download_tasks
                BEGIN SELECT RAISE(ABORT, 'test reset failure'); END;",
                )
                .unwrap();
        }
        assert_eq!(
            app.retry(&task.id).err().unwrap().to_string(),
            "DATABASE_ERROR"
        );
        assert_eq!(app.list()[0].revision, old.revision);
        assert_eq!(app.list()[0].status, Status::Canceled);
        {
            let inner = app.inner.lock().unwrap();
            assert!(inner.active.is_empty());
            let stored: String = inner
                .db
                .query_row(
                    "SELECT snapshot FROM download_tasks WHERE id=?",
                    [&task.id],
                    |r| r.get(0),
                )
                .unwrap();
            let stored: Task = serde_json::from_str(&stored).unwrap();
            assert_eq!(stored.revision, old.revision);
            assert_eq!(stored.status, Status::Canceled);
            assert_eq!(
                inner
                    .db
                    .query_row("SELECT count(*) FROM commit_intents", [], |r| r
                        .get::<_, i64>(0))
                    .unwrap(),
                1
            );
            inner
                .db
                .execute_batch("DROP TRIGGER reject_retry_reset")
                .unwrap();
        }
        let retry = app.retry(&task.id).unwrap();
        assert_eq!(retry.id, task.id);
        assert!(!staging.exists());
        let completed = wait(&app, &task.id, Status::Completed).await;
        assert_ne!(completed.path.as_ref().unwrap(), &target);
        assert_eq!(std::fs::read(&target).unwrap(), b"previous committed file");
        app.shutdown().await;
        drop(app);
        let app = core(root.path());
        assert_eq!(app.list().len(), 1);
        assert_eq!(app.list()[0].id, task.id);
        assert_eq!(app.list()[0].path, completed.path);
        app.shutdown().await;
    }

    #[tokio::test]
    async fn retry_joins_fifo_tail_and_rejects_duplicate_submissions() {
        let root = tempfile::tempdir().unwrap();
        let app = core(root.path());
        app.set_max_downloads(1).unwrap();
        let task = enqueue(&app, "8302").await;
        started(root.path(), "8302").await;
        app.cancel(&task.id).await.unwrap();
        std::fs::remove_file(root.path().join("started-8302")).unwrap();
        let blocker = enqueue(&app, "8401").await;
        started(root.path(), "8401").await;
        let queued = enqueue(&app, "8402").await;
        let retry = app.retry(&task.id).unwrap();
        assert_eq!(retry.id, task.id);
        assert_eq!(app.list().len(), 3);
        let retried = app.list().into_iter().find(|t| t.id == task.id).unwrap();
        let ahead = app.list().into_iter().find(|t| t.id == queued.id).unwrap();
        assert_eq!(retried.status, Status::Queued);
        assert!(retried.queue_order > ahead.queue_order);
        assert!(app.retry(&task.id).is_err());
        release(root.path(), "8401");
        wait(&app, &blocker.id, Status::Completed).await;
        started(root.path(), "8402").await;
        assert!(!root.path().join("started-8302").exists());
        release(root.path(), "8402");
        wait(&app, &queued.id, Status::Completed).await;
        started(root.path(), "8302").await;
        release(root.path(), "8302");
        wait(&app, &task.id, Status::Completed).await;
        assert_eq!(app.list().len(), 3);
        app.shutdown().await;
    }

    #[tokio::test]
    async fn history_removal_preserves_file_and_deletion_removes_history() {
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
        assert!(app.list().iter().any(|t| t.id == second.id));
        std::fs::remove_dir(&second_path).unwrap();
        std::fs::write(&second_path, b"video").unwrap();
        app.inner.lock().unwrap().db.execute_batch("CREATE TRIGGER reject_deletion BEFORE DELETE ON download_tasks BEGIN SELECT RAISE(ABORT, 'test'); END;").unwrap();
        assert_eq!(
            app.delete_local_file(&second.id).unwrap_err().to_string(),
            "DATABASE_ERROR"
        );
        assert!(!second_path.exists());
        assert!(app
            .list()
            .iter()
            .any(|t| t.id == second.id && t.file_missing));
        {
            let inner = app.inner.lock().unwrap();
            assert_eq!(
                inner
                    .db
                    .query_row(
                        "SELECT count(*) FROM download_tasks WHERE id=?",
                        [&second.id],
                        |r| r.get::<_, i64>(0)
                    )
                    .unwrap(),
                1
            );
            assert_eq!(
                inner
                    .db
                    .query_row(
                        "SELECT count(*) FROM commit_intents WHERE task_id=?",
                        [&second.id],
                        |r| r.get::<_, i64>(0)
                    )
                    .unwrap(),
                1
            );
            inner
                .db
                .execute_batch("DROP TRIGGER reject_deletion")
                .unwrap();
        }
        // Retrying deletion of an already missing file must still clear its record.
        app.delete_local_file(&second.id).unwrap();
        assert!(!second_path.exists());
        assert!(path.exists());
        {
            let inner = app.inner.lock().unwrap();
            for table in ["download_tasks", "commit_intents"] {
                assert_eq!(
                    inner
                        .db
                        .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r
                            .get::<_, i64>(0))
                        .unwrap(),
                    0
                );
            }
        }
        assert!(app.list().is_empty());
        drop(app);
        let app = core(root.path());
        assert!(app.list().is_empty());
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
    const COVER_LINKS: [&str; 3] = [
        "https://www.douyin.com/video/456",
        "https://www.tiktok.com/@fixture/video/456",
        "https://www.youtube.com/watch?v=fixture0001",
    ];
    #[tokio::test]
    async fn downloaded_video_covers_survive_restart_recovery_and_are_released_with_deleted_task() {
        for input in COVER_LINKS {
            let root = tempfile::tempdir().unwrap();
            let app = core(root.path());
            let task = app.resolve(input.into()).unwrap();
            wait(&app, &task.id, Status::Ready).await;
            app.download(&task.id, false).unwrap();
            let done = wait(&app, &task.id, Status::Completed).await;
            assert!(done.cover_path.is_none() && done.cover_cached);
            assert!(done.cover_error.is_none());
            assert_eq!(
                app.cover(&task.id).unwrap(),
                b"\x89PNG\r\n\x1a\ndownloaded-fixture"
            );
            app.shutdown().await;
            assert_eq!(
                std::fs::read_dir(root.path().join("downloads"))
                    .unwrap()
                    .count(),
                1
            );
            drop(app);
            let app = core(root.path());
            assert_eq!(
                app.cover(&task.id).unwrap(),
                b"\x89PNG\r\n\x1a\ndownloaded-fixture"
            );
            app.shutdown().await;
            drop(app);
            let mut interrupted = done.clone();
            interrupted.status = Status::Downloading;
            interrupted.cover_cached = false;
            let db = Connection::open(root.path().join("tasks.sqlite")).unwrap();
            db.execute(
                "UPDATE download_tasks SET snapshot=? WHERE id=?",
                params![serde_json::to_string(&interrupted).unwrap(), task.id],
            )
            .unwrap();
            let app = core(root.path());
            let recovered = app.list().into_iter().find(|t| t.id == task.id).unwrap();
            assert_eq!(recovered.status, Status::Completed);
            assert!(recovered.cover_cached && recovered.cover_path.is_none());
            assert!(app.cover(&task.id).is_ok());
            app.delete_local_file(&task.id).unwrap();
            assert!(app.list().is_empty());
            assert!(app.cover(&task.id).is_err());
            let video = done.video.unwrap();
            assert!(parse_cache::cover(&db, &video).is_err());
            assert!(parse_cache::get(
                &db,
                video["platform"].as_str().unwrap(),
                video["video_id"].as_str().unwrap(),
                video["resolver"].as_str().unwrap()
            )
            .unwrap()
            .is_some());
            app.shutdown().await;
        }
    }
    #[tokio::test]
    async fn shared_video_covers_are_released_atomically_only_after_last_reference() {
        for input in COVER_LINKS {
            let root = tempfile::tempdir().unwrap();
            let app = core(root.path());
            let first = app.resolve(input.into()).unwrap();
            wait(&app, &first.id, Status::Ready).await;
            app.download(&first.id, false).unwrap();
            let done = wait(&app, &first.id, Status::Completed).await;
            let preview = app.resolve(input.into()).unwrap();
            wait(&app, &preview.id, Status::Ready).await;
            app.shutdown().await;
            app.delete_local_file(&first.id).unwrap();
            assert!(app.cover(&preview.id).is_ok());
            assert!(!done.path.as_ref().unwrap().exists());
            app.cancel(&preview.id).await.unwrap();
            let db = Connection::open(root.path().join("tasks.sqlite")).unwrap();
            db.execute_batch(
                "CREATE TRIGGER block_cover_cleanup BEFORE UPDATE OF cover ON parse_cache
                WHEN NEW.cover IS NULL BEGIN SELECT RAISE(ABORT, 'fixture cleanup failure'); END;",
            )
            .unwrap();
            assert_eq!(
                app.remove_history(&preview.id).unwrap_err().to_string(),
                "DATABASE_ERROR"
            );
            assert!(app.list().iter().any(|t| t.id == preview.id));
            assert!(app.cover(&preview.id).is_ok());
            assert_eq!(
                db.query_row(
                    "SELECT count(*) FROM download_tasks WHERE id=?",
                    [&preview.id],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
                1
            );
            db.execute_batch("DROP TRIGGER block_cover_cleanup")
                .unwrap();
            let legacy = root.path().join("downloads/legacy.cover.jpg");
            std::fs::write(&legacy, b"legacy cover").unwrap();
            app.remove_history(&preview.id).unwrap();
            assert!(parse_cache::cover(&db, done.video.as_ref().unwrap()).is_err());
            assert!(!done.path.unwrap().exists());
            assert_eq!(std::fs::read(legacy).unwrap(), b"legacy cover");
            app.shutdown().await;
        }
    }
    #[tokio::test]
    async fn video_cover_cache_failure_keeps_completed_video_and_existing_preview() {
        for input in COVER_LINKS {
            let root = tempfile::tempdir().unwrap();
            let app = core(root.path());
            let task = app.resolve(input.into()).unwrap();
            wait(&app, &task.id, Status::Ready).await;
            let before = app.cover(&task.id).unwrap();
            let db = Connection::open(root.path().join("tasks.sqlite")).unwrap();
            db.execute_batch(
                "CREATE TRIGGER block_cover_save BEFORE UPDATE OF cover ON parse_cache
                BEGIN SELECT RAISE(ABORT, 'fixture cache failure'); END;",
            )
            .unwrap();
            app.download(&task.id, false).unwrap();
            let done = wait(&app, &task.id, Status::Completed).await;
            assert!(done.path.unwrap().is_file());
            assert!(done.cover_path.is_none() && done.cover_cached);
            assert_eq!(done.cover_error.as_deref(), Some("COVER_UNAVAILABLE"));
            assert_eq!(app.cover(&task.id).unwrap(), before);
            app.shutdown().await;
            assert_eq!(
                std::fs::read_dir(root.path().join("downloads"))
                    .unwrap()
                    .count(),
                1
            );
        }
    }
    #[tokio::test]
    async fn duplicate_variants_files_legacy_and_concurrent_submissions() {
        let root = tempfile::tempdir().unwrap();
        let app = core(root.path());
        let first = app.resolve("valid".into()).unwrap();
        wait(&app, &first.id, Status::Ready).await;
        app.download(&first.id, false).unwrap();
        let original = wait(&app, &first.id, Status::Completed).await;
        let second = app.resolve("valid".into()).unwrap();
        let ready = wait(&app, &second.id, Status::Ready).await;
        assert!(previously_downloaded(&app.list(), &ready));
        let different = app.choose_format(&second.id, &"2".repeat(24)).unwrap();
        assert!(!previously_downloaded(&app.list(), &different));
        let mut legacy = original.clone();
        legacy.video.as_mut().unwrap()["selected_format_id"] = Value::Null;
        assert!(previously_downloaded(&[legacy], &different));
        for status in [
            Status::Failed,
            Status::Canceled,
            Status::Queued,
            Status::Downloading,
        ] {
            let mut old = original.clone();
            old.status = status;
            assert!(!previously_downloaded(&[old], &ready));
        }
        app.shutdown().await;
        drop(app);
        let app = core(root.path());
        let restored = app.list().into_iter().find(|t| t.id == second.id).unwrap();
        assert!(!previously_downloaded(&app.list(), &restored));
        let one = app.clone();
        let two = app.clone();
        let id_one = second.id.clone();
        let id_two = second.id.clone();
        let (one, two) = tokio::join!(
            tokio::spawn(async move { one.download(&id_one, false) }),
            tokio::spawn(async move { two.download(&id_two, false) })
        );
        assert_eq!(
            usize::from(one.unwrap().is_ok()) + usize::from(two.unwrap().is_ok()),
            1
        );
        wait(&app, &second.id, Status::Completed).await;
        let third = app.resolve("valid".into()).unwrap();
        wait(&app, &third.id, Status::Ready).await;
        assert_eq!(
            app.download(&third.id, false).unwrap_err().to_string(),
            "ALREADY_DOWNLOADED"
        );
        std::fs::remove_file(original.path.unwrap()).unwrap();
        // The other completed format must not block this exact, now-missing variant.
        app.download(&third.id, false).unwrap();
        wait(&app, &third.id, Status::Completed).await;
        app.shutdown().await;
    }

    #[test]
    fn album_telegram_and_legacy_variants_have_explicit_identity_rules() {
        let video =
            json!({"platform":"douyin","video_id":"123","kind":"video","selected_format_id":"a"});
        let mut other = video.clone();
        other["selected_format_id"] = "b".into();
        assert!(!same_download_variant(&video, &other));
        other["kind"] = "image_album".into();
        assert!(!same_download_variant(&video, &other));
        let mut album = other.clone();
        album["selected_format_id"] = Value::Null;
        assert!(same_download_variant(&album, &other));
        other["platform"] = "telegram".into();
        other["kind"] = "video".into();
        let mut original = other.clone();
        original["selected_format_id"] = Value::Null;
        assert!(same_download_variant(&original, &other));
        original["video_id"] = "124".into();
        assert!(!same_download_variant(&original, &other));
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
