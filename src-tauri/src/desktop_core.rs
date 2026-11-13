use crate::{
    files::{
        finalize_album, finalize_album_with, finalize_to, finalize_to_with, validate_album,
        AlbumFile, Video,
    },
    parse_cache,
    python_bridge::PythonBridge,
    resolve_progress::{ResolvePhase, ResolveProgress},
    settings::{self, Settings},
    telegram_download,
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
#[path = "authors.rs"]
mod authors;
#[path = "douyin_author.rs"]
pub mod douyin_author;
#[path = "redownload.rs"]
mod redownload;
#[path = "task_actions.rs"]
pub mod task_actions;
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
    #[serde(default)]
    pub insertion_id: i64,
    #[serde(default)]
    pub asset_id: Option<String>,
    pub revision: u64,
    pub status: Status,
    #[serde(default)]
    pub resolve_progress: Option<ResolveProgress>,
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
    #[serde(default)]
    pub download_requested_at: Option<u64>,
    pub completed_at: Option<u64>,
    pub canceling: bool,
    #[serde(default)]
    pub file_missing: bool,
    #[serde(default)]
    pub queue_order: Option<u64>,
    #[serde(default)]
    pub download_requested: bool,
    #[serde(default)]
    pub duplicate_confirmed: bool,
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
struct DownloadProgressCadence {
    emitted_at: Option<Instant>,
    merging: bool,
}
impl DownloadProgressCadence {
    fn should_emit(&mut self, event: &Value, at: Instant) -> bool {
        let merging = event["phase"] == "merging";
        let phase_changed = merging && !self.merging;
        if !phase_changed
            && self
                .emitted_at
                .is_some_and(|last| at.duration_since(last) < Duration::from_secs(1))
        {
            return false;
        }
        self.emitted_at = Some(at);
        self.merging |= merging;
        true
    }
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
            event["total_bytes"]
                .as_u64()
                .or_else(|| event["total_bytes_estimate"].as_u64())
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
                    }) || file.video.as_ref().is_some_and(|video| {
                        !std::fs::symlink_metadata(path.join(&video.filename)).is_ok_and(|m| {
                            m.is_file() && !m.file_type().is_symlink() && m.len() == video.bytes
                        })
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

fn same_replacement_variant(old: &Value, video: &Value) -> bool {
    same_download_variant(old, video)
        && (video["kind"] == "image_album"
            || video["platform"] == "telegram"
            || old["selected_format_id"]
                .as_str()
                .zip(video["selected_format_id"].as_str())
                .is_some_and(|(old, new)| old == new))
}

fn previously_downloaded(tasks: &[Task], task: &Task) -> bool {
    task.video.as_ref().is_some_and(|video| {
        tasks.iter().any(|old| {
            old.id != task.id
                && (download_in_progress(old)
                    || (old.status == Status::Completed && !file_missing(old)))
                && old
                    .video
                    .as_ref()
                    .is_some_and(|v| same_download_variant(v, video))
        })
    })
}

fn download_in_progress(task: &Task) -> bool {
    matches!(task.status, Status::Queued | Status::Downloading)
        || (task.status == Status::Parsing && task.download_requested)
}

#[derive(Clone, serde::Serialize)]
pub struct DeleteActivity {
    pub operation_id: String,
    pub companion: Option<String>,
    pub step: String,
}

pub enum TaskEvent {
    DeleteActivity(Option<DeleteActivity>),
    BackupAssetDeleted(String),
    Updated(Box<Task>),
    Removed(String),
    RemovedFromSpace { id: String, space_id: String },
    Transcription(Box<transcription::Snapshot>),
}

struct Active {
    telegram: bool,
    download_after_resolve: bool,
    profiles: Arc<Value>,
    directory: PathBuf,
    filename_template: String,
    cancel: watch::Sender<bool>,
    done: watch::Receiver<bool>,
}
fn running_downloads(inner: &Inner) -> usize {
    inner
        .downloads
        .iter()
        .filter(|id| {
            inner.tasks.iter().any(|task| {
                &task.id == *id && task.error.as_deref() != Some("TELEGRAM_RECONNECTING")
            })
        })
        .count()
}

struct ResolveInput {
    original: String,
    verified_url: Option<String>,
}

impl From<String> for ResolveInput {
    fn from(original: String) -> Self {
        Self {
            original,
            verified_url: None,
        }
    }
}

struct Inner {
    author_scan: Option<douyin_author::ActiveScan>,
    author_draft: Option<douyin_author::ScanDraft>,
    author_paused: HashSet<String>,
    backup_cancel: watch::Sender<bool>,
    backup_started: bool,
    telegram_recovery_epoch: u64,
    backup_wake: Arc<tokio::sync::Notify>,
    backup_restores: HashMap<String, crate::backup::restore::Shared>,
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
}
pub struct Core {
    author_download_finished: Mutex<Option<Instant>>,
    backup_display_db: Mutex<Connection>,
    pub(crate) backup_file_presence: Mutex<crate::backup::reconcile::FilePresence>,
    task_actions: Mutex<HashMap<String, task_actions::PendingAction>>,
    delete_activity: Mutex<Option<DeleteActivity>>,
    pub telegram_accounts: crate::telegram_accounts::TelegramAccounts,
    pub(crate) backup_wake: Arc<tokio::sync::Notify>,
    pub(crate) backup_window_wake: tokio::sync::Notify,
    pub(crate) delete_queue_wake: tokio::sync::Notify,
    pub(crate) deferred_pulls: crate::backup::sync_engine::DeferredPulls,
    pub(crate) backup_foreground: std::sync::atomic::AtomicBool,
    pub(crate) backup_active_probes: Mutex<HashSet<String>>,
    pub(crate) backup_config_lock: tokio::sync::Mutex<()>,
    pub(crate) backup_restore_lock: tokio::sync::Mutex<()>,
    restore_file_locks: Mutex<HashMap<String, std::sync::Weak<tokio::sync::Mutex<()>>>>,
    backup_history_registered: Mutex<HashMap<String, (i64, Option<tokio::time::Instant>)>>,
    // Transfers and per-asset deletion share this lease; global maintenance is exclusive.
    pub(crate) backup_write_lock: tokio::sync::RwLock<()>,
    pub(crate) telegram_map_refresh: tokio::sync::Mutex<()>,
    _instance_lock: std::fs::File,
    inner: Mutex<Inner>,
    program: String,
    args: Vec<String>,
    runtime_directory: PathBuf,
    #[cfg(test)]
    pub(crate) telegram_source: Mutex<Option<telegram_download::Factory>>,
    #[cfg(test)]
    pub(crate) local_media_fixture: Mutex<Option<crate::local_media::Fixture>>,
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
fn cleanup_task_staging(db: &Connection, id: &str) -> Result<()> {
    match db.query_row(
        "SELECT path FROM task_staging WHERE task_id=?",
        [id],
        |row| row.get::<_, String>(0),
    ) {
        Ok(path) => cleanup_staging(db, id, Path::new(&path)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(()),
        Err(error) => Err(error).context("DATABASE_ERROR"),
    }
}
fn persist(db: &Connection, task: &Task) -> Result<()> {
    let transaction = db
        .is_autocommit()
        .then(|| db.unchecked_transaction())
        .transpose()?;
    let mut stored = task.clone();
    // Covers may carry short-lived query tokens. Keep them only in memory.
    if let Some(video) = stored.video.as_mut() {
        video["cover_url"] = Value::Null;
        authors::strip_public_details(video);
    }
    db.execute(
        "INSERT INTO download_tasks(id,snapshot) VALUES (?1,?2)
        ON CONFLICT(id) DO UPDATE SET snapshot=excluded.snapshot",
        params![task.id, serde_json::to_string(&stored)?],
    )
    .context("DATABASE_ERROR")?;
    authors::link_task(db, task)?;
    if let Some(transaction) = transaction {
        transaction.commit()?;
    }
    Ok(())
}
fn persist_new_attempt(db: &Connection, task: &Task) -> Result<()> {
    let tx = db.unchecked_transaction().context("DATABASE_ERROR")?;
    tx.execute("DELETE FROM commit_intents WHERE task_id=?", [&task.id])
        .context("DATABASE_ERROR")?;
    persist(&tx, task)?;
    tx.commit().context("DATABASE_ERROR")?;
    Ok(())
}
fn publish(inner: &mut Inner, mut task: Task, emit: &dyn Fn(TaskEvent)) -> Result<()> {
    let tx = inner.db.transaction().context("DATABASE_ERROR")?;
    persist_published(&tx, &mut task)?;
    tx.commit().context("DATABASE_ERROR")?;
    emit_published(inner, task, emit);
    Ok(())
}
fn persist_published(tx: &Connection, task: &mut Task) -> Result<()> {
    crate::backup::repository::associate(tx, task)?;
    persist(tx, task)?;
    let insertion_id = tx
        .query_row(
            "SELECT rowid FROM download_tasks WHERE id=?",
            [&task.id],
            |row| row.get(0),
        )
        .context("DATABASE_ERROR")?;
    if task.insertion_id != insertion_id {
        task.insertion_id = insertion_id;
        persist(tx, task)?;
    }
    Ok(())
}
fn emit_published(inner: &mut Inner, task: Task, emit: &dyn Fn(TaskEvent)) {
    if task.status == Status::Completed {
        inner.backup_wake.notify_waiters();
    }
    if let Some(old) = inner.tasks.iter_mut().find(|t| t.id == task.id) {
        *old = task.clone();
    } else {
        inner.tasks.insert(0, task.clone());
    }
    emit_task(&inner.db, task, emit);
}

fn emit_task(db: &Connection, task: Task, emit: &dyn Fn(TaskEvent)) {
    emit(TaskEvent::Updated(Box::new(authors::display_task(
        db, task,
    ))));
}

fn restore_original_task(
    inner: &mut Inner,
    attempt: &Task,
    emit: &dyn Fn(TaskEvent),
) -> Result<bool> {
    let Some(mut original) = redownload::original(&inner.db, &attempt.id)? else {
        return Ok(false);
    };
    original.revision = attempt.revision;
    original.error = attempt.error.clone().or_else(|| Some("CANCELED".into()));
    let tx = inner.db.transaction().context("DATABASE_ERROR")?;
    redownload::forget(&tx, &attempt.id)?;
    persist(&tx, &original)?;
    tx.commit().context("DATABASE_ERROR")?;
    emit_published(inner, original, emit);
    Ok(true)
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
        db.pragma_update(None, "foreign_keys", true)?;
        // Snapshot the master database before the single, atomic schema upgrade.
        let backup_migration = crate::backup::schema_migration::prepare(&db)?;
        let author_migration = authors::prepare(&db, backup_migration.snapshot_created)?;
        crate::cover_store::prepare(
            &db,
            backup_migration.snapshot_created || !matches!(author_migration, authors::Migration::Current),
        )?;
        db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;")?;
        let schema_tx = db.unchecked_transaction()?;
        schema_tx.execute_batch(
            "CREATE TABLE IF NOT EXISTS download_tasks(id TEXT PRIMARY KEY,snapshot TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS commit_intents(task_id TEXT PRIMARY KEY,source TEXT NOT NULL,
                target TEXT NOT NULL,size INTEGER NOT NULL);
            CREATE TABLE IF NOT EXISTS task_staging(task_id TEXT PRIMARY KEY,path TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS download_replacements(task_id TEXT PRIMARY KEY,original TEXT NOT NULL);",
        )
        .context("DATABASE_ERROR")?;
        let has_manifest = schema_tx
            .prepare("PRAGMA table_info(commit_intents)")?
            .query_map([], |r| r.get::<_, String>(1))?
            .collect::<rusqlite::Result<Vec<_>>>()?
            .iter()
            .any(|column| column == "manifest");
        if !has_manifest {
            schema_tx.execute("ALTER TABLE commit_intents ADD COLUMN manifest TEXT", [])?;
        }
        let migrate_cache = parse_cache::init(&schema_tx)?;
        transcription::init(&schema_tx)?;
        settings::init(&schema_tx)?;
        douyin_author::init(&schema_tx)?;
        authors::apply_migration(&schema_tx, author_migration)?;
        crate::backup::repository::init_in_transaction(&schema_tx, backup_migration)?;
        schema_tx.commit()?;
        transcription::recover(&db)?;
        let settings = settings::load(&db, directory)?;
        let telegram_accounts = crate::telegram_accounts::TelegramAccounts::load(&db)?;
        telegram_accounts.configure_proxy(
            settings
                .profiles
                .iter()
                .find(|p| p.platform == "telegram")
                .cloned(),
        );
        let mut tasks = {
            let mut query =
                db.prepare("SELECT rowid,snapshot FROM download_tasks ORDER BY rowid DESC")?;
            let strings = query
                .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            strings
                .iter()
                .map(|(insertion_id, snapshot)| {
                    let mut task = serde_json::from_str::<Task>(snapshot)?;
                    task.insertion_id = *insertion_id;
                    Ok(task)
                })
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
            if let Some(mut original) = redownload::original(&db, &task.id)? {
                if redownload::rollback(&db, &original).is_err() {
                    task.status = Status::Failed;
                    task.error = Some("REDOWNLOAD_RESTORE_FAILED".into());
                    task.canceling = false;
                    task.revision += 1;
                    persist(&db, task)?;
                    continue;
                }
                if task.canceling || task.error.as_deref() == Some("REDOWNLOAD_RESTORE_FAILED") {
                    original.revision = task.revision + 1;
                    let tx = db.transaction().context("DATABASE_ERROR")?;
                    redownload::forget(&tx, &original.id)?;
                    persist(&tx, &original)?;
                    tx.commit().context("DATABASE_ERROR")?;
                    *task = original;
                }
            }
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
                        if identity.platform == "weixin" {
                            cached.video["canonical_url"] = identity.canonical_url.into();
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
                                            || crate::files::video_recovery_valid(
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
                let tx = db.transaction()?;
                crate::backup::repository::associate(&tx, task)?;
                persist(&tx, task)?;
                tx.commit()?;
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
            if tasks.iter().any(|task| {
                task.id == id && task.error.as_deref() == Some("REDOWNLOAD_RESTORE_FAILED")
            }) {
                continue;
            }
            cleanup_staging(&db, &id, Path::new(&path))?;
        }
        let backup_wake = Arc::new(tokio::sync::Notify::new());
        let author_paused = douyin_author::paused(&db)?;
        Ok(Arc::new(Self {
            author_download_finished: Default::default(),
            telegram_accounts,
            task_actions: Mutex::default(),
            delete_activity: Default::default(),
            backup_wake: backup_wake.clone(),
            backup_window_wake: Default::default(),
            delete_queue_wake: tokio::sync::Notify::new(),
            deferred_pulls: Default::default(),
            backup_foreground: std::sync::atomic::AtomicBool::new(true),
            backup_active_probes: Mutex::new(HashSet::new()),
            backup_config_lock: tokio::sync::Mutex::new(()),
            backup_restore_lock: tokio::sync::Mutex::new(()),
            restore_file_locks: Mutex::default(),
            backup_history_registered: Default::default(),
            backup_write_lock: tokio::sync::RwLock::new(()),
            telegram_map_refresh: tokio::sync::Mutex::new(()),
            _instance_lock: instance_lock,
            inner: Mutex::new(Inner {
                author_scan: None,
                author_draft: None,
                author_paused,
                backup_cancel: watch::channel(false).0,
                backup_started: false,
                telegram_recovery_epoch: 0,
                backup_wake,
                backup_restores: HashMap::new(),
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
            }),
            program,
            args,
            runtime_directory: db_path.parent().context("IO_ERROR")?.join("runtime"),
            backup_file_presence: Mutex::default(),
            backup_display_db: Mutex::new(Connection::open_with_flags(
                db_path,
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
            )?),
            #[cfg(test)]
            telegram_source: Mutex::new(None),
            #[cfg(test)]
            local_media_fixture: Mutex::new(None),
            emit: Arc::new(emit),
        }))
    }

    pub fn list(&self) -> Vec<Task> {
        let inner = self.inner.lock().unwrap();
        let mut profiles = authors::display_profiles(&inner.db).unwrap_or_default();
        inner
            .tasks
            .iter()
            .cloned()
            .map(|mut t| {
                t.file_missing = t.status == Status::Completed && file_missing(&t);
                if let (Some(profile), Some(video)) = (profiles.remove(&t.id), t.video.as_mut()) {
                    video["author_profile"] = profile;
                }
                t
            })
            .collect()
    }

    pub fn backup_asset(&self, id: &str) -> Result<Option<crate::backup::model::Asset>> {
        crate::backup::repository::asset(&self.inner.lock().unwrap().db, id)
    }

    pub fn backup_assets(&self) -> Result<Vec<crate::backup::model::Asset>> {
        crate::backup::repository::list_assets(&self.inner.lock().unwrap().db)
    }
    /// Register local history only while the selected backup worker is ready.
    /// Existing space membership, cancellation and deletion evidence always win.
    pub(crate) fn register_backup_history(&self, space: &str) -> Result<usize> {
        use rusqlite::OptionalExtension;
        let mut inner = self.inner.lock().unwrap();
        if inner.closing {
            return Ok(0);
        }
        let enabled:bool=inner.db.query_row("SELECT EXISTS(SELECT 1 FROM backup_current_space c JOIN backup_spaces s USING(space_id) JOIN backup_provider_config p USING(space_id) WHERE c.id=1 AND s.space_id=?1 AND s.enabled=1 AND p.protocol=2 AND p.capabilities_json IS NOT NULL AND p.provider_type IN ('s3','telegram') AND COALESCE((SELECT enabled FROM backup_sync_preference WHERE id=1),s.enabled)=1)",[space],|r|r.get(0))?;
        if !enabled {
            return Ok(0);
        }
        let generation: i64 = inner.db.query_row(
            "SELECT generation FROM backup_history_generation WHERE id=1",
            [],
            |r| r.get(0),
        )?;
        let mut registered = self.backup_history_registered.lock().unwrap();
        if registered.get(space).is_some_and(|(saved, retry)| {
            *saved == generation
                && retry.is_none_or(|deadline| tokio::time::Instant::now() < deadline)
        }) {
            return Ok(0);
        }
        let detached = {
            let mut q=inner.db.prepare("SELECT a.asset_id FROM media_assets a WHERE a.asset_state='active' AND NOT EXISTS(SELECT 1 FROM backup_space_assets m WHERE m.local_asset_id=a.asset_id) AND NOT EXISTS(SELECT 1 FROM backup_operations o WHERE o.asset_id=a.asset_id) AND NOT EXISTS(SELECT 1 FROM asset_hidden_history h WHERE h.asset_id=a.asset_id)")?;
            let rows = q
                .query_map([], |r| r.get::<_, String>(0))?
                .collect::<rusqlite::Result<HashSet<_>>>()?;
            rows
        };
        let tasks = inner
            .tasks
            .iter()
            .filter(|task| {
                task.status == Status::Completed
                    && task.files.is_empty()
                    && task
                        .video
                        .as_ref()
                        .is_some_and(|v| v["kind"].as_str().unwrap_or("video") == "video")
                    && task
                        .asset_id
                        .as_ref()
                        .is_none_or(|id| detached.contains(id))
            })
            .cloned()
            .collect::<Vec<_>>();
        if tasks.is_empty() {
            registered.insert(space.to_owned(), (generation, None));
            return Ok(0);
        }
        let tx = inner.db.unchecked_transaction()?;
        let mut updated = vec![];
        let mut missing_file = false;
        for mut task in tasks {
            let known:Option<(String,String,bool)>=tx.query_row("SELECT a.asset_id,a.asset_state,EXISTS(SELECT 1 FROM backup_space_assets m WHERE m.local_asset_id=a.asset_id) OR EXISTS(SELECT 1 FROM backup_operations o WHERE o.asset_id=a.asset_id) OR EXISTS(SELECT 1 FROM asset_hidden_history h WHERE h.asset_id=a.asset_id) FROM media_assets a WHERE (?1 IS NOT NULL AND a.asset_id=?1) OR (?1 IS NULL AND a.task_id=?2)",params![task.asset_id,task.id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
            if let Some((id, state, protected)) = known {
                if task.asset_id.is_none() {
                    task.asset_id = Some(id);
                    task.revision += 1;
                    persist(&tx, &task)?;
                    updated.push(task.clone());
                }
                if state != "active" || protected {
                    continue;
                }
            } else if task.asset_id.is_some() {
                continue;
            }
            let Some(path) = task.path.as_ref().filter(|p| p.is_file()) else {
                missing_file = true;
                continue;
            };
            let source = transcription::Source::read(path)?;
            crate::backup::repository::associate(&tx, &mut task)?;
            let Some(id) = task.asset_id.as_ref() else {
                continue;
            };
            if crate::backup::repository::transcript(&tx, id)?.is_none() {
                if let Some(snapshot) = transcription::load(&tx, &task.id)? {
                    if snapshot.result.as_ref().is_some_and(|r| r.source == source) {
                        crate::backup::repository::preserve_transcript(&tx, &snapshot)?;
                    }
                }
            }
            crate::backup::sync_engine::register_download(&tx, id)?;
            if !updated.iter().any(|t| t.id == task.id) {
                task.revision += 1;
            }
            persist(&tx, &task)?;
            if let Some(old) = updated.iter_mut().find(|t| t.id == task.id) {
                *old = task;
            } else {
                updated.push(task);
            }
        }
        tx.commit()?;
        for task in &updated {
            if let Some(old) = inner.tasks.iter_mut().find(|t| t.id == task.id) {
                *old = task.clone();
            }
        }
        let generation = inner.db.query_row(
            "SELECT generation FROM backup_history_generation WHERE id=1",
            [],
            |r| r.get(0),
        )?;
        registered.insert(
            space.to_owned(),
            (
                generation,
                missing_file
                    .then(|| tokio::time::Instant::now() + std::time::Duration::from_secs(30)),
            ),
        );
        drop(registered);
        let displayed: Vec<_> = updated
            .iter()
            .cloned()
            .map(|task| authors::display_task(&inner.db, task))
            .collect();
        drop(inner);
        for task in displayed {
            (self.emit)(TaskEvent::Updated(Box::new(task)));
        }
        if !updated.is_empty() {
            self.backup_wake.notify_waiters();
        }
        Ok(updated.len())
    }
    pub fn telegram_storage_device_id(&self) -> Result<String> {
        self.backup_db(crate::backup::repository::device_id)
    }

    pub(crate) fn backup_db<T>(&self, action: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
        action(&self.inner.lock().unwrap().db)
    }

    pub(crate) fn backup_display_db<T>(
        &self,
        action: impl FnOnce(&Connection) -> Result<T>,
    ) -> Result<T> {
        action(&self.backup_display_db.lock().unwrap())
    }

    pub fn asset_transcript(
        &self,
        id: &str,
    ) -> Result<Option<crate::backup::model::AssetTranscript>> {
        crate::backup::repository::transcript(&self.inner.lock().unwrap().db, id)
    }

    pub async fn verify_backup_asset(&self, id: &str) -> Result<()> {
        let (_sender, cancel) = watch::channel(false);
        self.verify_backup_asset_cancelable(id, cancel).await
    }
    pub(crate) async fn verify_backup_asset_cancelable(
        &self,
        id: &str,
        extra: watch::Receiver<bool>,
    ) -> Result<()> {
        let (path, cancel) = {
            let inner = self.inner.lock().unwrap();
            let asset = crate::backup::repository::asset(&inner.db, id)?
                .context("BACKUP_ASSET_NOT_FOUND")?;
            (
                asset.path.context("FILE_MISSING")?,
                inner.backup_cancel.subscribe(),
            )
        };
        let (hash, source) = tokio::task::spawn_blocking(move || {
            crate::backup::repository::fingerprint_with_cancel(&path, || {
                *cancel.borrow() || *extra.borrow() || extra.has_changed().is_err()
            })
        })
        .await??;
        crate::backup::repository::save_fingerprint(
            &self.inner.lock().unwrap().db,
            id,
            &hash,
            &source,
        )
    }

    pub fn start_backup_service(self: &Arc<Self>) {
        let mut inner = self.inner.lock().unwrap();
        if inner.closing || inner.backup_started {
            return;
        }
        inner.backup_started = true;
        self.restore_interrupted_downloads(&mut inner);
        let core = Arc::clone(self);
        let cancel = inner.backup_cancel.subscribe();
        inner
            .workers
            .push(tokio::spawn(crate::backup::sync_engine::run(core, cancel)));
        let recovery_core = self.clone();
        let mut recovery_stop = inner.backup_cancel.subscribe();
        let recovery_epoch = inner.telegram_recovery_epoch;
        let recovery_binding = self
            .telegram_accounts
            .binding(crate::telegram_accounts::Purpose::Download);
        inner.workers.push(tokio::spawn(async move {
            loop {
                let current = recovery_core
                    .telegram_accounts
                    .binding(crate::telegram_accounts::Purpose::Download);
                if !Arc::ptr_eq(&recovery_binding.session, &current.session)
                    || recovery_core.inner.lock().unwrap().telegram_recovery_epoch != recovery_epoch
                {
                    break;
                }
                if *recovery_stop.borrow() {
                    break;
                }
                let candidates: Vec<_> = recovery_core
                    .inner
                    .lock()
                    .unwrap()
                    .tasks
                    .iter()
                    .filter(|task| {
                        task.status == Status::Failed
                            && task.telegram_link.is_some()
                            && task
                                .error
                                .as_deref()
                                .is_some_and(crate::telegram_reconnect::network_error)
                    })
                    .cloned()
                    .collect();
                for task in candidates {
                    if *recovery_stop.borrow() {
                        break;
                    }
                    let format = task
                        .video
                        .as_ref()
                        .and_then(|v| v["selected_format_id"].as_str())
                        .map(str::to_owned);
                    let _ = recovery_core.resolve_task(
                        task.telegram_link.unwrap().into(),
                        Some(&task.id),
                        "grammers".into(),
                        format,
                        task.download_requested,
                        Some((task.revision, recovery_epoch)),
                    );
                }
                tokio::select! { biased;
                    _ = recovery_stop.wait_for(|v| *v) => break,
                    _ = tokio::time::sleep(Duration::from_secs(2)) => {},
                }
            }
        }));
        let repairs: Vec<_> = inner
            .tasks
            .iter()
            .filter(|task| {
                task.status == Status::Completed
                    && task.path.is_some()
                    && task.video.as_ref().is_some_and(|video| {
                        video["platform"] == "telegram"
                            && ["width", "height", "duration_ms"]
                                .iter()
                                .any(|field| video[*field].as_u64().is_none_or(|n| n == 0))
                    })
            })
            .cloned()
            .collect();
        if !repairs.is_empty() {
            let core = self.clone();
            inner.workers.push(tokio::spawn(async move {
                core.repair_telegram_metadata(repairs).await;
            }));
        }
    }
    async fn repair_telegram_metadata(self: &Arc<Self>, tasks: Vec<Task>) {
        for expected in tasks {
            if self.inner.lock().unwrap().closing {
                break;
            }
            let Some(path) = expected.path.as_ref().filter(|path| {
                std::fs::symlink_metadata(path).is_ok_and(|m| {
                    m.is_file() && !m.file_type().is_symlink() && m.len() == expected.bytes
                })
            }) else {
                continue;
            };
            let metadata = self
                .probe_video_metadata(path, &format!("telegram-metadata-{}", expected.id), async {
                    loop {
                        if self.inner.lock().unwrap().closing {
                            return;
                        }
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
                })
                .await;
            let metadata = match metadata {
                Ok(metadata) => metadata,
                Err(error) => {
                    if error.to_string() != "CANCELED" {
                        eprintln!("Telegram metadata probe: {error}");
                    }
                    continue;
                }
            };
            let result = (|| -> Result<()> {
                let mut inner = self.inner.lock().unwrap();
                if inner.closing {
                    return Ok(());
                }
                let Some(mut task) = inner
                    .tasks
                    .iter()
                    .find(|task| {
                        task.id == expected.id
                            && task.status == Status::Completed
                            && task.path == expected.path
                            && task.completed_at == expected.completed_at
                    })
                    .cloned()
                else {
                    return Ok(());
                };
                let Some(video) = task.video.as_mut() else {
                    return Ok(());
                };
                if video["platform"] != "telegram" {
                    return Ok(());
                }
                let before = video.clone();
                telegram_download::fill_missing_attributes(video, &metadata);
                if *video == before {
                    return Ok(());
                }
                parse_cache::update_telegram_metadata(&inner.db, video)?;
                task.revision += 1;
                publish(&mut inner, task, &*self.emit)
            })();
            if let Err(error) = result {
                eprintln!("Telegram metadata repair: {error}");
            }
        }
    }

    pub(crate) fn stop_telegram_recovery(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.telegram_recovery_epoch = inner.telegram_recovery_epoch.wrapping_add(1);
    }

    pub fn wake_backup(&self) {
        self.backup_wake.notify_waiters();
    }
    pub fn current_delete_activity(&self) -> Option<DeleteActivity> {
        self.delete_activity
            .lock()
            .expect("delete activity lock poisoned")
            .clone()
    }

    pub(crate) fn replace_delete_activity(
        &self,
        activity: Option<DeleteActivity>,
    ) -> Option<DeleteActivity> {
        let previous = std::mem::replace(
            &mut *self
                .delete_activity
                .lock()
                .expect("delete activity lock poisoned"),
            activity.clone(),
        );
        (self.emit)(TaskEvent::DeleteActivity(activity));
        previous
    }

    pub fn set_backup_foreground(&self, foreground: bool) {
        self.backup_foreground
            .store(foreground, std::sync::atomic::Ordering::Relaxed);
        if foreground {
            self.backup_window_wake.notify_waiters();
        }
    }
    pub(crate) fn backup_cancellation(&self) -> watch::Receiver<bool> {
        self.inner.lock().unwrap().backup_cancel.subscribe()
    }

    pub(crate) async fn probe_video_metadata(
        &self,
        source: &std::path::Path,
        operation: &str,
        cancel: impl std::future::Future<Output = ()>,
    ) -> Result<serde_json::Value> {
        let _ = operation;
        #[cfg(test)]
        if let Some(fixture) = self.local_media_fixture.lock().unwrap().clone() {
            return fixture("probe", source);
        }
        crate::local_media::probe(source, &self.runtime_directory, cancel).await
    }
    async fn extract_telegram_cover(
        &self,
        source: &Path,
        cancel: impl std::future::Future<Output = ()>,
    ) -> Result<Value> {
        #[cfg(test)]
        if let Some(fixture) = self.local_media_fixture.lock().unwrap().clone() {
            return fixture("cover", source);
        }
        crate::local_media::extract(source, &self.runtime_directory, cancel).await
    }

    pub async fn ensure_local_video(self: &Arc<Self>, asset_id: &str) -> Result<PathBuf> {
        let mut receiver = {
            let mut inner = self.inner.lock().unwrap();
            if inner.closing {
                bail!("CANCELED");
            }
            if let Some(shared) = inner.backup_restores.get(asset_id) {
                shared.result.clone()
            } else {
                let (sender, receiver) = watch::channel(None);
                let (cancel, mut cancellation) = watch::channel(false);
                inner.backup_restores.insert(
                    asset_id.into(),
                    crate::backup::restore::Shared {
                        result: receiver.clone(),
                        cancel,
                    },
                );
                let core = self.clone();
                let id = asset_id.to_owned();
                inner.workers.push(tokio::spawn(async move {
                    let outcome=tokio::select!{_=cancellation.changed()=>Err(anyhow::anyhow!("CANCELED")),result=crate::backup::restore::ensure_current(&core,&id)=>result}.map_err(|e|e.to_string());
                    if let Err(error)=&outcome {
                        let _=core.backup_db(|db|{db.execute("UPDATE backup_operations SET state='blocked',last_error=?2 WHERE asset_id=?1 AND kind='restore' AND state='running'",params![id,error])?;db.execute("UPDATE asset_local_files SET local_state='missing' WHERE asset_id=? AND local_state='restoring'",[&id])?;Ok(())});
                    }
                    sender.send_replace(Some(outcome));
                    core.inner.lock().unwrap().backup_restores.remove(&id);
                }));
                receiver
            }
        };
        let result = receiver
            .wait_for(|r| r.is_some())
            .await
            .context("CANCELED")?
            .clone()
            .context("CANCELED")?;
        result.map_err(anyhow::Error::msg)
    }

    /// A foreground delete waits only for file work on the same asset.
    pub(crate) async fn restore_file_guard(&self, asset: &str) -> tokio::sync::OwnedMutexGuard<()> {
        let lock = {
            let mut locks = self.restore_file_locks.lock().unwrap();
            locks.retain(|_, lock| lock.strong_count() > 0);
            if let Some(lock) = locks.get(asset).and_then(std::sync::Weak::upgrade) {
                lock
            } else {
                let lock = Arc::new(tokio::sync::Mutex::new(()));
                locks.insert(asset.into(), Arc::downgrade(&lock));
                lock
            }
        };
        lock.lock_owned().await
    }

    pub fn cancel_asset_restore(&self, id: &str) -> Result<()> {
        crate::backup::restore::cancel(self, id)?;
        if let Some(shared) = self.inner.lock().unwrap().backup_restores.get(id) {
            shared.cancel.send_replace(true);
        }
        Ok(())
    }
    pub async fn cancel_asset_transcription(&self, id: &str) -> Result<()> {
        let operation = {
            let inner = self.inner.lock().unwrap();
            inner
                .transcription
                .as_ref()
                .filter(|a| {
                    a.snapshot.task_id == id
                        || inner.tasks.iter().any(|t| {
                            t.id == a.snapshot.task_id && t.asset_id.as_deref() == Some(id)
                        })
                })
                .map(|a| (a.snapshot.task_id.clone(), a.snapshot.operation_id.clone()))
        };
        if let Some((task, operation)) = operation {
            self.cancel_transcription(&task, &operation).await?;
        }
        Ok(())
    }
    pub(crate) fn mark_asset_missing(&self, id: &str) -> Result<()> {
        let mut inner = self.inner.lock().unwrap();
        let mut tasks: Vec<Task> = inner
            .tasks
            .iter()
            .filter(|t| t.asset_id.as_deref() == Some(id))
            .cloned()
            .collect();
        let tx = inner.db.unchecked_transaction()?;
        tx.execute(
            "UPDATE asset_local_files SET local_state='missing',path=NULL WHERE asset_id=?",
            [id],
        )?;
        for task in &mut tasks {
            task.path = None;
            task.file_missing = true;
            task.revision += 1;
            persist(&tx, task)?;
        }
        tx.commit()?;
        for task in tasks {
            if let Some(old) = inner.tasks.iter_mut().find(|t| t.id == task.id) {
                *old = task.clone();
            }
            emit_task(&inner.db, task, &*self.emit);
        }
        Ok(())
    }
    pub(crate) fn commit_asset_file(
        &self,
        id: &str,
        path: &Path,
        hash: &str,
        source: &Value,
    ) -> Result<()> {
        let mut inner = self.inner.lock().unwrap();
        let mut tasks: Vec<Task> = inner
            .tasks
            .iter()
            .filter(|t| t.asset_id.as_deref() == Some(id))
            .cloned()
            .collect();
        let tx = inner.db.unchecked_transaction()?;
        tx.execute("UPDATE asset_local_files SET path=?2,bytes=?3,sha256=?4,source_json=?5,local_state='present',verified_at=unixepoch() WHERE asset_id=?1",params![id,path.to_string_lossy(),source["size"].as_i64().context("BACKUP_INVALID_SIZE")?,hash,source.to_string()])?;
        for task in &mut tasks {
            task.path = Some(path.to_owned());
            task.file_missing = false;
            task.revision += 1;
            persist(&tx, task)?;
        }
        let transcripts = transcription::rebind_asset_source(&tx, id, hash, source)?;
        tx.commit()?;
        for snapshot in transcripts {
            (self.emit)(TaskEvent::Transcription(Box::new(snapshot.summary())));
        }
        for task in tasks {
            if let Some(old) = inner.tasks.iter_mut().find(|t| t.id == task.id) {
                *old = task.clone();
            }
            emit_task(&inner.db, task, &*self.emit);
        }
        Ok(())
    }
    pub fn settings(&self) -> Settings {
        self.inner.lock().unwrap().settings.clone()
    }
    pub fn cookie(&self, platform: &str) -> Result<String> {
        settings::cookie(&self.inner.lock().unwrap().settings, platform)
    }
    /// Updates the selected WeChat Channels authentication method and its credential.
    pub fn update_account_auth(
        &self,
        platform: &str,
        mode: settings::AccountAuth,
        credential: Option<String>,
    ) -> Result<Settings> {
        let mut inner = self.inner.lock().unwrap();
        if inner.closing {
            bail!("CANCELED");
        }
        let Inner { db, settings, .. } = &mut *inner;
        settings::update_account_auth(db, settings, platform, mode, credential)?;
        Ok(settings.clone())
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
        if variant
            .as_ref()
            .is_some_and(|v| !matches!(v.as_str(), "cpu" | "cuda"))
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
                            &core.runtime_directory,
                            &tool,
                            variant.as_deref(),
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
                                if let Some(stage) = stage {
                                    let _ = stage.keep();
                                }
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
        let video = inner
            .tasks
            .iter()
            .find(|task| task.id == id)
            .context("TASK_NOT_FOUND")?
            .video
            .as_ref()
            .context("FILE_MISSING")?;
        authors::avatar(&inner.db, id, video)
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
    pub async fn save_profile_with_network(
        &self,
        update: settings::ProfileUpdate,
    ) -> Result<Settings> {
        settings::validate_profile_update(&self.settings(), &update)?;
        let network = if update.platform == "telegram"
            && (update.proxy.is_some() || update.bypass_proxy.is_some())
        {
            Some(self.telegram_accounts.network_sessions()?)
        } else {
            None
        };
        let changed = if network.is_some() {
            let settings = self.settings();
            let current = settings
                .profiles
                .iter()
                .find(|p| p.platform == "telegram")
                .context("CONFIG_INVALID")?;
            let normalized = |value: &str| -> Result<Option<url::Url>> {
                if value.is_empty() {
                    return Ok(None);
                }
                let (mut proxy, _) = settings::parse_proxy("telegram", value)?;
                proxy.set_path("/");
                Ok(Some(proxy))
            };
            let requested = if update.bypass_proxy.unwrap_or(current.bypass_proxy) {
                None
            } else if let Some(proxy) = &update.proxy {
                normalized(proxy)?
            } else {
                normalized(settings::profile_proxy(current)?.as_deref().unwrap_or(""))?
            };
            // An unreadable old credential must not prevent replacing or clearing it.
            match settings::effective_proxy(current)
                .and_then(|current| normalized(current.as_deref().unwrap_or("")))
            {
                Ok(current) => requested != current,
                Err(_) => true,
            }
        } else {
            false
        };
        if !changed {
            return self.save_profile(update);
        }
        let network = network
            .as_ref()
            .expect("changed Telegram proxy owns its snapshot");
        let mut paused = Vec::with_capacity(network.sessions.len());
        for session in &network.sessions {
            paused.push(session.pause_network().await?);
        }
        // Guards keep connection admission and state locked through persistence.
        self.save_profile(update)
    }
    pub fn save_profile(&self, update: settings::ProfileUpdate) -> Result<Settings> {
        let mut inner = self.inner.lock().unwrap();
        let Inner { db, settings, .. } = &mut *inner;
        let result = settings::save_profile(db, settings, update);
        self.telegram_accounts.configure_proxy(
            settings
                .profiles
                .iter()
                .find(|p| p.platform == "telegram")
                .cloned(),
        );
        result?;
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

    pub fn set_filename_template(&self, template: String) -> Result<Settings> {
        crate::naming::validate(&template)?;
        let mut inner = self.inner.lock().unwrap();
        inner
            .db
            .execute(
                "INSERT INTO app_settings(id,directory,filename_template) VALUES(1,?1,?2)
            ON CONFLICT(id) DO UPDATE SET filename_template=excluded.filename_template",
                params![
                    inner.settings.download_directory.to_string_lossy(),
                    template
                ],
            )
            .context("DATABASE_ERROR")?;
        inner.settings.filename_template = template;
        Ok(inner.settings.clone())
    }

    pub fn resolve(self: &Arc<Self>, input: String) -> Result<Task> {
        self.resolve_task(input.into(), None, "auto".into(), None, false, None)
    }

    pub(crate) fn resolve_progress(&self, id: &str, phase: ResolvePhase) {
        let mut inner = self.inner.lock().unwrap();
        if inner.closing {
            return;
        }
        let Some(task) = inner.tasks.iter_mut().find(|task| task.id == id) else {
            return;
        };
        if task.status != Status::Parsing || task.canceling {
            return;
        }
        if task
            .resolve_progress
            .as_ref()
            .is_some_and(|progress| progress.phase == phase)
        {
            return;
        }
        task.resolve_progress
            .get_or_insert_with(|| ResolveProgress::new(phase))
            .advance(phase);
        task.revision += 1;
        let displayed = task.clone();
        emit_task(&inner.db, displayed, &*self.emit);
    }

    fn resolve_event(&self, id: &str, event: &Value) {
        if let Ok(phase) = serde_json::from_value(event["resolve_phase"].clone()) {
            self.resolve_progress(id, phase);
        }
    }

    pub fn resolve_with_resolver(
        self: &Arc<Self>,
        input: String,
        resolver: String,
    ) -> Result<Task> {
        self.resolve_work(input.into(), resolver)
    }

    fn resolve_work(self: &Arc<Self>, input: ResolveInput, resolver: String) -> Result<Task> {
        if !matches!(
            resolver.as_str(),
            "auto" | "yt-dlp" | "dtk" | "grammers" | "weixin"
        ) {
            bail!("CONFIG_INVALID");
        }
        self.resolve_task(input, None, resolver, None, false, None)
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
        if let Some(task) = self.retry_author_download(id)? {
            return Ok(task);
        }
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
            if task.error.as_deref() == Some("REDOWNLOAD_RESTORE_FAILED") {
                bail!("REDOWNLOAD_RESTORE_FAILED");
            }
            if redownload::original(&inner.db, id)?.is_some() {
                bail!("TASK_BUSY");
            }
            if !matches!(task.status, Status::Failed | Status::Canceled) {
                bail!("INVALID_STATE");
            }
            let video: Video =
                serde_json::from_value(task.video.clone().context("REPARSE_REQUIRED")?)
                    .context("INVALID_ENGINE_RESULT")?;
            video.validate()?;
            (
                if video.platform == "telegram" {
                    task.telegram_link
                        .clone()
                        .unwrap_or(parse_cache::telegram_source(
                            &inner.db,
                            task.video.as_ref().unwrap(),
                        )?)
                } else {
                    video.canonical_url
                },
                if video.platform == "telegram" {
                    "grammers".into()
                } else {
                    video.resolver
                },
                task.video.as_ref().unwrap()["selected_format_id"]
                    .as_str()
                    .map(str::to_owned),
            )
        };
        self.resolve_task(input.into(), Some(id), resolver, format_id, true, None)
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
        ) || task.error.as_deref() == Some("REDOWNLOAD_RESTORE_FAILED")
            || inner
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
        if platform.is_some_and(|p| {
            !matches!(
                p,
                "douyin" | "xiaohongshu" | "tiktok" | "youtube" | "telegram" | "weixin"
            )
        }) {
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
                    && task.error.as_deref() != Some("REDOWNLOAD_RESTORE_FAILED")
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
        self.remove_history_records_with(inner, ids, None, |_| Ok(()))
    }

    pub(crate) fn finish_asset_deletion(&self, asset: &str, operation: &str) -> Result<()> {
        let mut inner = self.inner.lock().unwrap();
        let tasks: Vec<_> = inner
            .tasks
            .iter()
            .filter(|task| task.asset_id.as_deref() == Some(asset))
            .collect();
        if tasks.iter().any(|task| {
            !matches!(
                task.status,
                Status::Completed | Status::Failed | Status::Canceled
            ) || inner.active.contains_key(&task.id)
                || inner.downloads.contains(&task.id)
                || inner
                    .transcription
                    .as_ref()
                    .is_some_and(|active| active.snapshot.task_id == task.id)
        }) {
            bail!("TASK_BUSY");
        }
        let ids = tasks.into_iter().map(|task| task.id.clone()).collect();
        let space: String = inner.db.query_row("SELECT space_id FROM backup_operations WHERE operation_id=?1 AND asset_id=?2 AND kind='delete'",params![operation,asset],|row|row.get(0))?;
        self.remove_history_records_with(&mut inner, ids, Some(&space), |tx| {
            tx.execute("DELETE FROM asset_hidden_history WHERE asset_id=?",[asset])?;
            tx.execute("INSERT OR IGNORE INTO asset_deletion_hidden_history(space_id,asset_id) VALUES(?1,?2)",params![space,asset])?;
            tx.execute("UPDATE asset_remote_objects SET remote_state='deleted' WHERE space_id=?1 AND asset_id=?2",params![space,asset])?;
            tx.execute("UPDATE backup_operations SET payload_json=json_set(payload_json,'$.local_cleanup_done',json('true'),'$.local_cleanup_ms',CAST(unixepoch('subsec')*1000 AS INTEGER)) WHERE operation_id=?",[operation])?;
            Ok(())
        })?;
        drop(inner);
        (self.emit)(TaskEvent::BackupAssetDeleted(operation.to_owned()));
        Ok(())
    }

    fn remove_history_records_with(
        &self,
        inner: &mut Inner,
        ids: Vec<String>,
        space: Option<&str>,
        update: impl FnOnce(&rusqlite::Transaction<'_>) -> Result<()>,
    ) -> Result<Vec<String>> {
        let removing: HashSet<_> = ids.iter().map(String::as_str).collect();
        let removed_authors: HashSet<_> = inner
            .tasks
            .iter()
            .filter(|task| removing.contains(task.id.as_str()))
            .filter_map(|task| task.video.as_ref().and_then(authors::key))
            .collect();
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
            if tx
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM download_replacements WHERE task_id=?)",
                    [id],
                    |row| row.get::<_, bool>(0),
                )
                .context("DATABASE_ERROR")?
            {
                bail!("TASK_BUSY");
            }
            tx.execute("INSERT OR IGNORE INTO asset_hidden_history SELECT asset_id FROM media_assets WHERE task_id=?",[id]).context("DATABASE_ERROR")?;
            tx.execute("UPDATE media_assets SET task_id=NULL WHERE task_id=?", [id])
                .context("DATABASE_ERROR")?;
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
        for author in removed_authors {
            authors::release_avatar(&tx, author)?;
        }
        update(&tx)?;
        tx.commit().context("DATABASE_ERROR")?;
        // Staging records, if cleanup previously failed, remain available for
        // startup cleanup. Removing history never touches a completed video.
        inner.tasks.retain(|t| !removing.contains(t.id.as_str()));
        inner
            .transcription_unsaved
            .retain(|id, _| !removing.contains(id.as_str()));
        for id in &ids {
            (self.emit)(match space {
                Some(space) => TaskEvent::RemovedFromSpace {
                    id: id.clone(),
                    space_id: space.to_owned(),
                },
                None => TaskEvent::Removed(id.clone()),
            });
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

    pub fn album_video_path(&self, id: &str, index: usize) -> Result<PathBuf> {
        let inner = self.inner.lock().unwrap();
        let task = inner
            .tasks
            .iter()
            .find(|t| t.id == id)
            .context("TASK_NOT_FOUND")?;
        if task.status != Status::Completed
            || task
                .video
                .as_ref()
                .is_none_or(|video| video["kind"] != "image_album")
        {
            bail!("INVALID_STATE");
        }
        let video = task
            .files
            .get(index)
            .and_then(|file| file.video.as_ref())
            .context("FILE_MISSING")?;
        if video.filename != format!("{:03}.mp4", index + 1) {
            bail!("INVALID_ENGINE_RESULT");
        }
        let directory = task.path.as_ref().context("FILE_MISSING")?;
        let metadata = std::fs::symlink_metadata(directory).context("FILE_MISSING")?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            bail!("FILE_MISSING");
        }
        let path = directory.join(&video.filename);
        let metadata = std::fs::symlink_metadata(&path).context("FILE_MISSING")?;
        if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() != video.bytes
        {
            bail!("FILE_MISSING");
        }
        Ok(path)
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
                        .flat_map(|f| {
                            std::iter::once(path.join(&f.filename))
                                .chain(f.video.iter().map(|v| path.join(&v.filename)))
                        })
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
        source: ResolveInput,
        retry_id: Option<&str>,
        resolver: String,
        requested_format: Option<String>,
        download_after_resolve: bool,
        recovery_revision: Option<(u64, u64)>,
    ) -> Result<Task> {
        let ResolveInput {
            original: input,
            verified_url,
        } = source;
        if input.trim().is_empty() || input.len() > 8192 {
            bail!("INVALID_INPUT");
        }
        let telegram = telegram_download::is_telegram(&input);
        let resolver = if telegram {
            telegram_download::message_link(&input)?;
            if !matches!(resolver.as_str(), "auto" | "grammers") {
                bail!("UNSUPPORTED_CONTENT");
            }
            "grammers".to_owned()
        } else {
            if resolver == "grammers" {
                bail!("INVALID_INPUT");
            }
            resolver
        };
        let mut inner = self.inner.lock().unwrap();
        if inner.closing || inner.tasks.iter().any(|t| t.status == Status::Parsing) {
            bail!("TASK_BUSY");
        }
        if recovery_revision.is_some_and(|(_, epoch)| inner.telegram_recovery_epoch != epoch) {
            bail!("TELEGRAM_ACCOUNT_MISMATCH");
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
            if recovery_revision.is_some_and(|(revision, _)| {
                task.status != Status::Failed
                    || task.revision != revision
                    || !task
                        .error
                        .as_deref()
                        .is_some_and(crate::telegram_reconnect::network_error)
            }) {
                bail!("INVALID_STATE");
            }
            if inner.active.contains_key(id) || inner.downloads.contains(id) {
                bail!("TASK_BUSY");
            }
            Some(task.clone())
        } else {
            None
        };
        let mut cached = if telegram {
            None
        } else {
            parse_cache::find(
                &inner.db,
                verified_url.as_deref().unwrap_or(&input),
                &resolver,
            )?
        };
        let expected_weixin_id = previous
            .as_ref()
            .and_then(|task| task.video.as_ref())
            .filter(|video| video["platform"] == "weixin")
            .and_then(|video| video["video_id"].as_str())
            .map(str::to_owned);
        if let (Some(cached), Some(expected)) = (&cached, &expected_weixin_id) {
            if cached.video["video_id"] != expected.as_str() {
                bail!("UPSTREAM_CHANGED");
            }
        }
        if let (Some(cached), Some(format_id)) = (&mut cached, &requested_format) {
            apply_video_format(&mut cached.video, format_id)?;
        }
        if let (Some(cached), Some(_)) = (&cached, &verified_url) {
            parse_cache::remember(&inner.db, &input, &cached.video)?;
        }
        let profiles = if telegram {
            json!({})
        } else {
            settings::snapshot(&inner.settings)?
        };
        let mut task = Task {
            insertion_id: previous.as_ref().map_or(0, |task| task.insertion_id),
            asset_id: previous.as_ref().and_then(|t| t.asset_id.clone()),
            id: previous
                .as_ref()
                .map(|t| t.id.clone())
                .unwrap_or_else(|| Uuid::new_v4().to_string()),
            revision: previous.as_ref().map_or(1, |t| t.revision + 1),
            status: Status::Parsing,
            resolve_progress: Some({
                let mut progress = ResolveProgress::new(ResolvePhase::CheckingLink);
                if !telegram {
                    progress.advance(ResolvePhase::CheckingCache);
                }
                progress
            }),
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
            download_requested_at: download_after_resolve
                .then(|| chrono::Utc::now().timestamp_millis().max(0) as u64),
            completed_at: None,
            canceling: false,
            file_missing: false,
            queue_order: None,
            download_requested: download_after_resolve,
            duplicate_confirmed: previous
                .as_ref()
                .is_some_and(|task| task.duplicate_confirmed),
            cover_path: None,
            cover_error: None,
            files: Vec::new(),
            completed_items: None,
            total_items: None,
            cache_hit: false,
            cached_at: None,
            cover_cached: false,
            telegram_link: if telegram {
                Some(telegram_download::message_link(&input)?.url)
            } else {
                None
            },
        };
        let (cancel, mut cancellation) = watch::channel(false);
        let (done, done_rx) = watch::channel(cached.is_some());
        if previous.is_some() {
            cleanup_task_staging(&inner.db, &task.id)?;
            // Old commit evidence must never complete a new attempt on restart.
            persist_new_attempt(&inner.db, &task)?;
            *inner.tasks.iter_mut().find(|t| t.id == task.id).unwrap() = task.clone();
            let displayed = task.clone();
            emit_task(&inner.db, displayed, &*self.emit);
        } else {
            publish(&mut inner, task.clone(), &*self.emit)?;
            task.insertion_id = inner
                .tasks
                .iter()
                .find(|stored| stored.id == task.id)
                .unwrap()
                .insertion_id;
        }
        let directory = inner.settings.download_directory.clone();
        let filename_template = inner.settings.filename_template.clone();
        inner.active.insert(
            task.id.clone(),
            Active {
                telegram,
                download_after_resolve,
                profiles: Arc::new(profiles.clone()),
                directory: directory.clone(),
                filename_template,
                cancel,
                done: done_rx,
            },
        );
        if let Some(cached) = cached {
            let progress = task.resolve_progress.as_mut().unwrap();
            progress.advance(ResolvePhase::CacheHit);
            progress.advance(ResolvePhase::Completed);
            task.video = Some(cached.video);
            task.cache_hit = true;
            task.cached_at = Some(cached.saved_at);
            task.cover_cached = cached.has_cover;
            task.status = Status::Ready;
            task.revision += 1;
            publish(&mut inner, task.clone(), &*self.emit)?;
            let displayed = authors::display_task(&inner.db, task.clone());
            drop(inner);
            if download_after_resolve {
                self.finish(&task.id, Ok(task.video.clone().unwrap()), false);
            }
            return Ok(displayed);
        }
        let core = self.clone();
        let id = task.id.clone();
        inner.workers.retain(|worker| !worker.is_finished());
        let worker = tokio::spawn(async move {
            let result = async {
                if telegram {
                    return core.resolve_telegram(&input, &id, cancellation.clone()).await;
                }
                let engine_input = verified_url.as_deref().unwrap_or(&input);
                let verified_identity = verified_url.as_deref().map(|url| url.trim_end_matches('/'));
                if parse_cache::share_url(engine_input).is_some() {
                    let has_cache: bool = core.inner.lock().unwrap().db.query_row(
                        "SELECT EXISTS(SELECT 1 FROM parse_cache)", [], |r| r.get(0),
                    ).context("DATABASE_ERROR")?;
                    if has_cache {
                        core.resolve_progress(&id, ResolvePhase::StartingEngine);
                        let identified = PythonBridge::operation_cancellable(
                            &core.program, &core.args, &id, "identify_input",
                            json!({"input":engine_input,"profiles":profiles,"resolver":resolver,"runtime_directory":core.runtime_directory,"report_resolve_progress":true}),
                            |event| core.resolve_event(&id, &event), async { let _ = cancellation.wait_for(|v| *v).await; },
                        ).await?;
                        let identity: Video = serde_json::from_value(identified["result"].clone())
                            .context("INVALID_ENGINE_RESULT")?;
                        identity.validate()?;
                        if expected_weixin_id.as_ref().is_some_and(|expected| &identity.video_id != expected) {
                            bail!("UPSTREAM_CHANGED");
                        }
                        if verified_identity.is_some_and(|url| identity.canonical_url.trim_end_matches('/') != url) {
                            bail!("INVALID_ENGINE_RESULT");
                        }
                        core.resolve_progress(&id, ResolvePhase::CheckingCache);
                        let mut inner = core.inner.lock().unwrap();
                        if inner.closing || *cancellation.borrow() { bail!("CANCELED"); }
                        if let Some(mut cached) = parse_cache::get(
                            &inner.db, &identity.platform, &identity.video_id, &identity.resolver,
                        )? {
                            parse_cache::remember(&inner.db, &input, &cached.video)?;
                            parse_cache::remember(&inner.db, &identity.canonical_url, &cached.video)?;
                            if identity.platform == "weixin" {
                                cached.video["canonical_url"] = identity.canonical_url.into();
                            }
                            let task = inner.tasks.iter_mut().find(|t| t.id == id).unwrap();
                            task.cache_hit = true;
                            task.cached_at = Some(cached.saved_at);
                            task.cover_cached = cached.has_cover;
                            task.resolve_progress.as_mut().unwrap().advance(ResolvePhase::CacheHit);
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
                core.resolve_progress(&id, ResolvePhase::StartingEngine);
                let result = PythonBridge::operation_cancellable(
                    &core.program, &core.args, &id, "resolve_video",
                    json!({"input":engine_input,"profiles":profiles,"resolver":resolver,"format_id":requested_format,"preview_dir":stage.path(),"runtime_directory":core.runtime_directory,"report_resolve_progress":true}),
                    |event| core.resolve_event(&id, &event), async { let _ = cancellation.wait_for(|v| *v).await; },
                ).await.and_then(|result| {
                    core.resolve_progress(&id, ResolvePhase::Finalizing);
                    let video: Video = serde_json::from_value(result["result"].clone())
                        .context("INVALID_ENGINE_RESULT")?;
                    video.validate()?;
                    if expected_weixin_id.as_ref().is_some_and(|expected| &video.video_id != expected) {
                        bail!("UPSTREAM_CHANGED");
                    }
                    if verified_identity.is_some_and(|url| video.canonical_url.trim_end_matches('/') != url) {
                        bail!("INVALID_ENGINE_RESULT");
                    }
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
                    let _ = authors::save_staged_avatar(&inner.db, &cached.video, &video, stage.path(), cached.saved_at);
                    if let Some(profile) = cached.video["author_profile"].as_object_mut() {
                        profile.insert("avatar_cached".into(), authors::has_avatar(&inner.db, &video).into());
                    }
                    parse_cache::remember(&inner.db, &input, &cached.video)?;
                    let task = inner.tasks.iter_mut().find(|t| t.id == id).unwrap();
                    task.cached_at = Some(cached.saved_at);
                    task.cover_cached = cached.has_cover;
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
        Ok(authors::display_task(&inner.db, task))
    }

    fn finish(self: &Arc<Self>, id: &str, result: Result<Value>, downloading: bool) {
        let mut inner = self.inner.lock().unwrap();
        let mut task = inner
            .tasks
            .iter()
            .find(|t| t.id == id)
            .expect("active task exists")
            .clone();
        if result
            .as_ref()
            .is_err_and(|error| error.to_string() == "REDOWNLOAD_RESTORE_FAILED")
        {
            task.status = Status::Failed;
            task.error = Some("REDOWNLOAD_RESTORE_FAILED".into());
        } else if task.canceling {
            task.status = Status::Canceled;
            task.error = None;
        } else if inner.closing && (!downloading || result.is_err()) {
            task.status = Status::Failed;
            task.error = Some("INTERRUPTED".into());
        } else {
            match result {
                Ok(video) if !downloading => {
                    if let Some(progress) = &mut task.resolve_progress {
                        progress.advance(ResolvePhase::Completed);
                    }
                    task.error = None;
                    task.video = Some(video);
                    task.status = Status::Ready;
                }
                Ok(_) => {
                    task.error = None;
                    task.status = Status::Completed;
                }
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
            && (task.duplicate_confirmed || !previously_downloaded(&inner.tasks, &task))
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
        let saved = if downloading
            && !inner.closing
            && matches!(task.status, Status::Failed | Status::Canceled)
            && task.error.as_deref() != Some("REDOWNLOAD_RESTORE_FAILED")
        {
            restore_original_task(&mut inner, &task, &*self.emit).and_then(|restored| {
                if restored {
                    Ok(())
                } else {
                    publish(&mut inner, task.clone(), &*self.emit)
                }
            })
        } else {
            publish(&mut inner, task.clone(), &*self.emit)
        };
        if saved.is_err() {
            task.status = Status::Failed;
            task.error = Some("DATABASE_ERROR".into());
            let previous = inner.tasks.iter_mut().find(|t| t.id == id).unwrap();
            task.resolve_progress = previous.resolve_progress.clone();
            *previous = task.clone();
            let displayed = task.clone();
            emit_task(&inner.db, displayed, &*self.emit);
        }
        if !matches!(task.status, Status::Ready | Status::Queued) {
            inner.active.remove(id);
        }
        self.dispatch(&mut inner);
    }

    pub fn set_max_downloads(self: &Arc<Self>, value: u8) -> Result<Settings> {
        if !(1..=10).contains(&value) {
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
        task.duplicate_confirmed = false;
        task.revision += 1;
        publish(&mut inner, task.clone(), &*self.emit)?;
        Ok(authors::display_task(&inner.db, task))
    }

    pub fn download(self: &Arc<Self>, id: &str, confirm_duplicate: bool) -> Result<()> {
        self.submit_download(id, confirm_duplicate, None)
            .map(|_| ())
    }

    pub fn submit_download(
        self: &Arc<Self>,
        id: &str,
        confirm_duplicate: bool,
        replace_task_id: Option<&str>,
    ) -> Result<String> {
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
        let matches = |old: &&Task| {
            old.id != task.id
                && old
                    .video
                    .as_ref()
                    .zip(task.video.as_ref())
                    .is_some_and(|(old, video)| same_download_variant(old, video))
        };
        let in_progress = inner.tasks.iter().filter(matches).any(download_in_progress);
        let completed = |old: &&Task| old.status == Status::Completed && !file_missing(old);
        let replaceable = |old: &&Task| {
            old.video
                .as_ref()
                .zip(task.video.as_ref())
                .is_some_and(|(old, video)| same_replacement_variant(old, video))
        };
        let original = if let Some(replace_id) = replace_task_id {
            if !confirm_duplicate {
                bail!("ALREADY_DOWNLOADED");
            }
            let original = inner
                .tasks
                .iter()
                .filter(completed)
                .filter(replaceable)
                .find(|old| old.id == replace_id)
                .context("ALREADY_DOWNLOADED")?;
            if inner.active.contains_key(replace_id)
                || inner.transcription.as_ref().is_some_and(|active| {
                    active.snapshot.task_id == replace_id
                        || original.asset_id.as_deref() == Some(active.snapshot.task_id.as_str())
                })
            {
                bail!("TASK_BUSY");
            }
            Some(original.clone())
        } else {
            let mut completed = inner.tasks.iter().filter(matches).filter(completed);
            if completed.clone().any(|old| replaceable(&old))
                || (!confirm_duplicate && (in_progress || completed.next().is_some()))
            {
                bail!("ALREADY_DOWNLOADED");
            }
            None
        };
        if let Some(original) = &original {
            task.id.clone_from(&original.id);
            task.insertion_id = original.insertion_id;
            task.created_at = original.created_at;
            task.revision = original.revision;
            task.asset_id.clone_from(&original.asset_id);
        }
        task.duplicate_confirmed |= confirm_duplicate;
        let queued_id = task.id.clone();
        self.queue_download(
            &mut inner,
            task,
            true,
            original.as_ref().map(|original| (original, id)),
        )?;
        self.dispatch(&mut inner);
        Ok(queued_id)
    }

    fn queue_download(
        self: &Arc<Self>,
        inner: &mut Inner,
        mut task: Task,
        new_request: bool,
        replacing: Option<(&Task, &str)>,
    ) -> Result<()> {
        let deferred = task.video.is_none() && task.telegram_link.is_some();
        if !inner.active.contains_key(&task.id) {
            let telegram = deferred
                || task
                    .video
                    .as_ref()
                    .is_some_and(|v| v["platform"] == "telegram");
            let profiles = if telegram {
                json!({})
            } else if douyin_author::batch_id(&task).is_some() {
                settings::douyin_snapshot(&inner.settings)?
            } else {
                settings::snapshot(&inner.settings)?
            };
            let (cancel, _) = watch::channel(false);
            let (_, done) = watch::channel(true);
            let replacement = if let Some((original, _)) = replacing {
                Some(original.clone())
            } else {
                redownload::original(&inner.db, &task.id)?
            };
            let directory = replacement
                .as_ref()
                .map(redownload::directory)
                .transpose()?
                .unwrap_or_else(|| inner.settings.download_directory.clone());
            let filename_template = inner.settings.filename_template.clone();
            inner.active.insert(
                task.id.clone(),
                Active {
                    telegram,
                    download_after_resolve: deferred,
                    profiles: Arc::new(profiles),
                    directory,
                    filename_template,
                    cancel,
                    done,
                },
            );
        }
        let filename_template = inner.settings.filename_template.clone();
        let active = inner.active.get_mut(&task.id).context("INVALID_STATE")?;
        active.filename_template = filename_template;
        if let Some(platform) = task.video.as_ref().and_then(|video| video["platform"].as_str()) {
            if new_request && platform != "telegram" {
                settings::refresh_account_auth(&inner.settings, platform, Arc::make_mut(&mut active.profiles));
            }
        }
        task.status = Status::Queued;
        task.error = None;
        task.bytes = 0;
        task.speed_bps = None;
        task.eta_seconds = None;
        task.merging = false;
        task.completed_items = None;
        task.download_requested = true;
        if new_request {
            task.download_requested_at = Some(chrono::Utc::now().timestamp_millis().max(0) as u64);
        }
        if new_request || task.queue_order.is_none() {
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
        task.revision += 1;
        if let Some((original, preview_id)) = replacing {
            let saved = (|| {
                let tx = inner.db.transaction().context("DATABASE_ERROR")?;
                redownload::remember(&tx, original, Some(preview_id))?;
                persist(&tx, &task)?;
                tx.commit().context("DATABASE_ERROR")
            })();
            if let Err(error) = saved {
                inner.active.remove(&task.id);
                return Err(error);
            }
            inner.active.remove(preview_id);
            inner.tasks.retain(|stored| stored.id != preview_id);
            (self.emit)(TaskEvent::Removed(preview_id.to_owned()));
        } else {
            persist_new_attempt(&inner.db, &task)?;
        }
        *inner
            .tasks
            .iter_mut()
            .find(|stored| stored.id == task.id)
            .context("TASK_NOT_FOUND")? = task.clone();
        emit_task(&inner.db, task, &*self.emit);
        Ok(())
    }

    fn restore_interrupted_downloads(self: &Arc<Self>, inner: &mut Inner) {
        let mut pending: Vec<_> = inner
            .tasks
            .iter()
            .filter(|task| {
                task.download_requested
                    && task.status == Status::Failed
                    && task.error.as_deref() == Some("INTERRUPTED")
                    && (task.video.is_some() || task.telegram_link.is_some())
            })
            .cloned()
            .collect();
        pending.sort_by_key(|task| (task.queue_order.unwrap_or(u64::MAX), task.insertion_id));
        for mut task in pending {
            if let Err(error) = self.queue_download(inner, task.clone(), false, None) {
                inner.active.remove(&task.id);
                task.status = Status::Failed;
                task.error = Some(error.to_string());
                task.revision += 1;
                let _ = restore_original_task(inner, &task, &*self.emit).and_then(|restored| {
                    if restored {
                        Ok(())
                    } else {
                        publish(inner, task, &*self.emit)
                    }
                });
                eprintln!("startup_download event=queue_failed");
            }
        }
        self.dispatch(inner);
    }

    // This lock covers slot reservation and FIFO selection, including concurrent
    // submissions, settings changes, cancellation, and process completion.
    fn dispatch(self: &Arc<Self>, inner: &mut Inner) {
        while !inner.closing && running_downloads(inner) < usize::from(inner.settings.max_downloads)
        {
            let author_busy = inner.author_scan.is_some()
                || inner.tasks.iter().any(|task| {
                    douyin_author::batch_id(task).is_some() && inner.downloads.contains(&task.id)
                });
            let Some(id) = inner
                .tasks
                .iter()
                .filter(|t| t.status == Status::Queued && !inner.downloads.contains(&t.id))
                .filter(|t| {
                    douyin_author::batch_id(t)
                        .is_none_or(|id| !author_busy && !inner.author_paused.contains(id))
                })
                .filter(|t| {
                    !(t.telegram_link.is_some()
                        || t.video
                            .as_ref()
                            .is_some_and(|v| v["platform"] == "telegram"))
                        || !self
                            .telegram_accounts
                            .binding(crate::telegram_accounts::Purpose::Download)
                            .session
                            .reconnect
                            .waiting()
                        || !inner.tasks.iter().any(|other| {
                            inner.active.contains_key(&other.id)
                                && other.error.as_deref() == Some("TELEGRAM_RECONNECTING")
                        })
                })
                .filter(|t| {
                    t.video.is_some()
                        || !inner
                            .tasks
                            .iter()
                            .any(|other| other.status == Status::Parsing)
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
                let saved = restore_original_task(inner, &task, &*self.emit).and_then(|restored| {
                    if restored {
                        Ok(())
                    } else {
                        publish(inner, task.clone(), &*self.emit)
                    }
                });
                if saved.is_err() {
                    *inner.tasks.iter_mut().find(|t| t.id == id).unwrap() = task.clone();
                    emit_task(&inner.db, task, &*self.emit);
                }
                inner.active.remove(&id);
            }
        }
    }

    fn telegram_connection_progress(self: &Arc<Self>, id: &str, event: &Value) {
        let Some(waiting) = event["reconnecting"].as_bool() else {
            return;
        };
        let mut inner = self.inner.lock().unwrap();
        let Some(task) = inner.tasks.iter_mut().find(|t| t.id == id) else {
            return;
        };
        if task.canceling {
            return;
        }
        let error = waiting.then(|| "TELEGRAM_RECONNECTING".to_owned());
        if task.error == error {
            return;
        }
        task.error = error;
        if !waiting {
            task.bytes = 0;
        }
        task.speed_bps = None;
        task.eta_seconds = None;
        task.revision += 1;
        let displayed = task.clone();
        emit_task(&inner.db, displayed, &*self.emit);
        if waiting {
            self.dispatch(&mut inner);
        }
    }

    pub(crate) async fn wait_telegram_slot(
        self: &Arc<Self>,
        id: &str,
        mut cancel: watch::Receiver<bool>,
    ) -> Result<()> {
        loop {
            {
                let mut inner = self.inner.lock().unwrap();
                if inner.closing || *cancel.borrow() {
                    bail!("CANCELED");
                }
                let task = inner
                    .tasks
                    .iter()
                    .find(|t| t.id == id)
                    .context("TASK_NOT_FOUND")?;
                if task.canceling {
                    bail!("CANCELED");
                }
                if task.error.as_deref() != Some("TELEGRAM_RECONNECTING") {
                    return Ok(());
                }
                if !inner.downloads.contains(id)
                    || running_downloads(&inner) < inner.settings.max_downloads as usize
                {
                    let task = inner.tasks.iter_mut().find(|t| t.id == id).unwrap();
                    task.error = None;
                    task.bytes = 0;
                    task.speed_bps = None;
                    task.eta_seconds = None;
                    task.revision += 1;
                    let displayed = task.clone();
                    emit_task(&inner.db, displayed, &*self.emit);
                    self.dispatch(&mut inner);
                    return Ok(());
                }
            }
            tokio::select! { biased;
                _ = cancel.wait_for(|v|*v) => bail!("CANCELED"),
                _ = tokio::time::sleep(Duration::from_millis(100)) => {},
            }
        }
    }

    async fn resolve_telegram(
        self: &Arc<Self>,
        input: &str,
        id: &str,
        cancel: watch::Receiver<bool>,
    ) -> Result<Value> {
        let previous = self
            .inner
            .lock()
            .unwrap()
            .tasks
            .iter()
            .find(|task| task.id == id)
            .and_then(|task| task.video.clone());
        let outcome = telegram_download::operation(
            self,
            id,
            input,
            None,
            None,
            |event| {
                if event["resolve_phase"].is_string() {
                    self.resolve_event(id, &event);
                } else {
                    self.telegram_connection_progress(id, &event);
                }
            },
            cancel,
        )
        .await?;
        self.resolve_progress(id, ResolvePhase::Finalizing);
        let video = outcome.value;
        if let Some(previous) = &previous {
            if previous["platform"] == "telegram" && previous["video_id"] != video["video_id"] {
                parse_cache::forget_telegram_channel(
                    &self.inner.lock().unwrap().db,
                    input,
                    previous,
                )?;
                bail!(telegram_download::ALIAS_IDENTITY_MISMATCH);
            }
        }
        let identity: Video =
            serde_json::from_value(video.clone()).context("INVALID_ENGINE_RESULT")?;
        identity.validate()?;
        let mut inner = self.inner.lock().unwrap();
        let mut cached =
            parse_cache::save(&mut inner.db, &video, Vec::new(), outcome.cover, now())?;
        if let Some(body) = outcome.avatar {
            let _ = authors::save_avatar(&inner.db, &cached.video, &body, cached.saved_at);
        }
        cached.video["author_profile"]["avatar_cached"] =
            authors::has_avatar(&inner.db, &cached.video).into();
        parse_cache::remember(&inner.db, input, &cached.video)?;
        let task = inner.tasks.iter_mut().find(|t| t.id == id).unwrap();
        task.cached_at = Some(cached.saved_at);
        task.cover_cached = cached.has_cover;
        Ok(cached.video)
    }

    fn start_deferred(self: &Arc<Self>, inner: &mut Inner, id: &str) -> Result<()> {
        let mut task = inner.tasks.iter().find(|t| t.id == id).unwrap().clone();
        let input = task.telegram_link.clone().context("INVALID_STATE")?;
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
            let result = core.resolve_telegram(&input, &id, cancellation).await;
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
        let telegram = active.telegram;
        let directory = active.directory.clone();
        let filename_template = active.filename_template.clone();
        let replacement = redownload::original(&inner.db, id)?;
        let naming_time = crate::naming::current_time();
        let video = task.video.clone().context("INVALID_STATE")?;
        let author_batch = douyin_author::batch_id(&task).map(str::to_owned);
        let fetch_author_avatar = author_batch.is_some() && !authors::has_avatar(&inner.db, &video);
        let telegram_input = if telegram {
            Some(parse_cache::telegram_source(&inner.db, &video)?)
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
                if author_batch.is_some() {
                    tokio::select! {
                        _ = core.wait_author_detail() => {},
                        _ = cancellation.wait_for(|v| *v) => bail!("CANCELED"),
                    }
                }
                std::fs::create_dir_all(&directory).context("IO_ERROR")?;
                let stage = tempfile::Builder::new().prefix(&format!("yingqu-{id}-")).tempdir_in(&directory).context("IO_ERROR")?;
                let media = stage.path().join("media");
                std::fs::create_dir(&media).context("IO_ERROR")?;
                core.inner.lock().unwrap().db.execute("INSERT OR REPLACE INTO task_staging VALUES(?1,?2)",
                    params![id,stage.path().to_string_lossy()]).context("DATABASE_ERROR")?;
                let mut cadence = DownloadProgressCadence::default();
                let mut transfer_rate = YoutubeTransferRate::default();
                let youtube = video["platform"] == "youtube";
                let progress = |mut event: Value| {
                        if event["reconnecting"].is_boolean() { core.telegram_connection_progress(&id, &event); return; }
                        if youtube { transfer_rate.update(&mut event, Instant::now()); }
                        if !cadence.should_emit(&event, Instant::now()) { return; }
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
                        let displayed = task.clone();
                        emit_task(&inner.db, displayed, &*core.emit);
                    };
                let result = if telegram {
                    match telegram_download::operation(&core, &id, telegram_input.as_deref().context("INVALID_ENGINE_RESULT")?, Some(&video), Some(&media), progress, cancellation.clone()).await {
                        Ok(outcome) => {
                            let mut result = outcome.value;
                            if let Some(body) = outcome.cover {
                                let saved:Result<()> = async {
                                    let mut cover_file = tokio::fs::OpenOptions::new().write(true).create_new(true).open(media.join("cover")).await.context("IO_ERROR")?;
                                    tokio::io::AsyncWriteExt::write_all(&mut cover_file, &body).await.context("IO_ERROR")?;
                                    tokio::io::AsyncWriteExt::flush(&mut cover_file).await.context("IO_ERROR")?;
                                    Ok(())
                                }.await;
                                if saved.is_ok() {
                                    result["cover_filename"] = "cover".into();
                                    result["cover_bytes"] = (body.len() as u64).into();
                                }
                            }
                            if result["width"].as_u64().is_none() || result["height"].as_u64().is_none() || result["duration_ms"].as_u64().is_none() {
                                let mut metadata_cancel = cancellation.clone();
                                match core.probe_video_metadata(&media.join(result["filename"].as_str().context("INVALID_ENGINE_RESULT")?), &id, async { let _ = metadata_cancel.wait_for(|v|*v).await; }).await {
                                    Ok(metadata) => telegram_download::fill_missing_attributes(&mut result, &metadata),
                                    Err(error) if error.to_string()=="CANCELED" => return Err(error),
                                    Err(_) => {},
                                }
                            }
                            if result["cover_filename"].is_string() { Ok(result) } else {
                            let mut cover_cancel = cancellation.clone();
                            let cover = core.extract_telegram_cover(&media.join(result["filename"].as_str().context("INVALID_ENGINE_RESULT")?), async { let _ = cover_cancel.wait_for(|v| *v).await; }).await;
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
                        }
                        Err(error) => Err(error),
                    }
                } else {
                    PythonBridge::operation_cancellable(&core.program, &core.args, &id, "download_video",
                    json!({"input":video["canonical_url"],"profiles":profiles.as_ref(),"runtime_directory":core.runtime_directory,
                        "resolver":video["resolver"].as_str().unwrap_or("yt-dlp"),
                        "fetch_author_avatar":fetch_author_avatar,
                        "format_id":if author_batch.is_some() { &Value::Null } else { &video["selected_format_id"] },
                        "expected":{"platform":video["platform"],"video_id":video["video_id"],"kind":video["kind"].as_str().unwrap_or("video"),
                            "image_count":video["images"].as_array().map(Vec::len),
                            "author_id":video["batch_author_id"]},"staging_dir":media}),
                    progress, async { let _ = cancellation.wait_for(|v| *v).await; }).await
                };
                if result.as_ref().is_err_and(|error| error.root_cause().to_string() == telegram_download::ALIAS_IDENTITY_MISMATCH) {
                    // A freshly resolved public bootstrap proved this exact alias
                    // stale. Do not retry or remove a concurrently updated mapping.
                    parse_cache::forget_telegram_channel(&core.inner.lock().unwrap().db,
                        telegram_input.as_deref().context("INVALID_ENGINE_RESULT")?, &video)?;
                }
                if result.as_ref().is_err_and(|error| error.to_string() == "TELEGRAM_MEDIA_CHANGED") {
                    parse_cache::invalidate_telegram(&mut core.inner.lock().unwrap().db, &video)?;
                }
                // Release, upload and restore may still be using the old local
                // asset. Serialize their file work with the replacement commit.
                let asset = replacement.as_ref().and_then(|original| original.asset_id.as_deref());
                let (result, _backup_guards) = match (result, asset) {
                    (Ok(result), Some(asset)) => tokio::select! {
                        _ = cancellation.wait_for(|value| *value) => (Err(anyhow::anyhow!("CANCELED")), None),
                        guards = async {
                            let write = core.backup_write_lock.read().await;
                            let file = core.restore_file_guard(asset).await;
                            (write, file)
                        } => (Ok(result), Some(guards)),
                    },
                    (result, _) => (result, None),
                };
                let committed = result.and_then(|result| {
                    let mut inner = core.inner.lock().unwrap();
                    let mut task = inner.tasks.iter().find(|t| t.id == id).unwrap().clone();
                    if task.canceling { bail!("CANCELED"); }
                    let mut video = video;
                    if author_batch.is_some() {
                        video = douyin_author::downloaded_metadata(&video, &result)?;
                        task.video = Some(video.clone());
                    }
                    if telegram {
                        telegram_download::merge_attributes(&mut video, &result);
                        parse_cache::update_telegram_metadata(&inner.db, &video)?;
                        task.video = Some(video.clone());
                    }
                    if video["selected_format_id"].is_string() && result["format_id"] != video["selected_format_id"] {
                        bail!("INVALID_ENGINE_RESULT");
                    }
                    let relative = if replacement.is_none() {
                        Some(crate::naming::render(&filename_template, &video, &naming_time)?)
                    } else {
                        None
                    };
                    let previous_path = replacement.as_ref().and_then(|task| task.path.as_deref());
                    let identity: Video = serde_json::from_value(video).context("INVALID_ENGINE_RESULT")?;
                    let record_intent = |source: &Path, target: &Path, size: u64, files: Option<&[AlbumFile]>| -> Result<()> {
                        let manifest = files.map(serde_json::to_string).transpose()?;
                        let size = i64::try_from(size).context("IO_ERROR")?;
                        let insert = |db: &Connection| {
                            db.execute("INSERT INTO commit_intents(task_id,source,target,size,manifest) VALUES(?1,?2,?3,?4,?5)",
                                params![id, source.to_string_lossy(), target.to_string_lossy(), size, manifest])
                                .context("DATABASE_ERROR")
                        };
                        if author_batch.is_some() {
                            // Recovery needs the deferred metadata if the move succeeds before the final snapshot.
                            let tx = inner.db.unchecked_transaction().context("DATABASE_ERROR")?;
                            persist(&tx, &task)?;
                            insert(&tx)?;
                            tx.commit().context("DATABASE_ERROR")?;
                        } else {
                            insert(&inner.db)?;
                        }
                        Ok(())
                    };
                    let file = if identity.kind == "image_album" {
                        let target = if let Some(previous_path) = previous_path {
                            previous_path.to_owned()
                        } else {
                            crate::naming::destination(&directory, relative.as_deref().context("NAMING_INVALID")?, None)?
                        };
                        let record = |source: &Path, size, files: &[AlbumFile]|
                            record_intent(source, &target, size, Some(files));
                        let file = if let Some(original) = replacement.as_ref() {
                            finalize_album_with(&identity, &result, &media, &target, |source, size, files| {
                                record(source, size, files)?;
                                redownload::replace(source, &target, original)
                            })?
                        } else {
                            finalize_album(&identity, &result, &media, &target, record)?
                        };
                        task.files = serde_json::from_value(result["files"].clone()).context("INVALID_ENGINE_RESULT")?;
                        if let Some(video) = task.video.as_mut() { video["images"] = result["files"].clone(); }
                        task.completed_items = Some(task.files.len() as u64);
                        task.total_items = task.completed_items;
                        file
                    } else {
                    let extension = result["container"].as_str().context("INVALID_ENGINE_RESULT")?;
                    if !matches!(extension, "mp4" | "webm") && !(identity.platform == "telegram" && matches!(extension, "mov" | "mkv")) { bail!("INVALID_MEDIA_CONTENT"); }
                    let target = if let Some(previous_path) = previous_path {
                        previous_path.with_extension(extension)
                    } else {
                        crate::naming::destination(&directory, relative.as_deref().context("NAMING_INVALID")?, Some(extension))?
                    };
                    let record = |source: &Path, size| record_intent(source, &target, size, None);
                    let file = if let Some(original) = replacement.as_ref() {
                        finalize_to_with(&identity, &result, &media, &target, |source, size| {
                            record(source, size)?;
                            redownload::replace(source, &target, original)
                        })?
                    } else {
                        finalize_to(&identity, &result, &media, &target, record)?
                    };
                    let saved = crate::files::staged_cover(&result, &media).and_then(|body| {
                        let video = task.video.as_ref().context("INVALID_ENGINE_RESULT")?;
                        let updated = parse_cache::save_cover(&inner.db, video, &body)?;
                        if updated || author_batch.is_none() {
                            return Ok(updated);
                        }
                        // Author downloads get their first full metadata while downloading.
                        Ok(parse_cache::save(&mut inner.db, video, Vec::new(), Some(body), now())?.has_cover)
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
                    if author_batch.is_some() {
                        if let Some(video) = task.video.as_mut() {
                            // The validated detail supplies the author ID for both videos and albums.
                            let _ = authors::save_staged_avatar(&inner.db, video, &result, &media, now());
                            video["author_profile"]["avatar_cached"] = authors::has_avatar(&inner.db, video).into();
                        }
                    }
                    task.path = Some(file.path);
                    task.bytes = file.bytes;
                    task.total_bytes = Some(file.bytes);
                    task.total_bytes_estimate = None;
                    let completed_at = now();
                    task.completed_at = Some(completed_at);
                    task.status = Status::Completed;
                    task.speed_bps = None;
                    task.eta_seconds = None;
                    task.merging = false;
                    task.revision += 1;
                    // Cancellation and file/DB commit share this lock.
                    let tx = inner.db.transaction().context("DATABASE_ERROR")?;
                    let downloaded_profile = task.video.as_mut().and_then(|video| authors::downloaded_profile(&tx, video, &result));
                    if let Some(original) = replacement.as_ref() {
                        redownload::retire_local_asset(&tx, original)?;
                        if original.asset_id.is_some() {
                            task.asset_id = Some(Uuid::new_v4().to_string());
                        }
                    }
                    persist_published(&tx, &mut task)?;
                    if let Some(profile) = downloaded_profile {
                        authors::save_downloaded_profile(&tx, profile, completed_at)?;
                    }
                    if replacement.is_some() {
                        redownload::forget(&tx, &id)?;
                    }
                    tx.commit().context("DATABASE_ERROR")?;
                    emit_published(&mut inner, task, &*core.emit);
                    Ok(Value::Null)
                });
                if let Some(original) = replacement.as_ref().filter(|_| committed.is_err()) {
                    if redownload::rollback(&core.inner.lock().unwrap().db, original).is_err() {
                        let _ = stage.keep();
                        bail!("REDOWNLOAD_RESTORE_FAILED");
                    }
                }
                stage.close().context("IO_ERROR")?;
                core.inner.lock().unwrap().db.execute("DELETE FROM task_staging WHERE task_id=?",[&id]).context("DATABASE_ERROR")?;
                committed
            }.await;
            if let Some(batch) = author_batch.as_deref() {
                *core.author_download_finished.lock().unwrap() = Some(Instant::now());
                if let Err(error) = &result {
                    core.pause_author_on_error(batch, error);
                }
            }
            // Preserve an already committed success, including when staging cleanup fails.
            let completed = core
                .inner
                .lock()
                .unwrap()
                .tasks
                .iter()
                .any(|task| task.id == id && task.status == Status::Completed);
            if !completed {
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
        self.cancel_waiting_task_action(id);
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
                if !restore_original_task(&mut inner, &task, &*self.emit)? {
                    publish(&mut inner, task, &*self.emit)?;
                }
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
        self.close_task_action_admission();
        let (active, workers) = {
            let mut inner = self.inner.lock().unwrap();
            inner.closing = true;
            inner.backup_cancel.send_replace(true);
            if let Some(active) = &inner.transcription {
                let _ = active.cancel.send(true);
            }
            if let Some(cancel) = &inner.dependency_cancel {
                let _ = cancel.send(true);
            }
            if let Some(scan) = &inner.author_scan {
                scan.request_stop();
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
                .map(|(id, a)| {
                    let _ = a.cancel.send(true);
                    (id.clone(), a.done.clone())
                })
                .collect::<Vec<_>>();
            let queued = inner
                .tasks
                .iter()
                .filter(|task| task.status == Status::Queued)
                .cloned()
                .collect::<Vec<_>>();
            for mut task in queued {
                task.status = Status::Failed;
                task.error = Some("INTERRUPTED".into());
                task.canceling = false;
                task.speed_bps = None;
                task.eta_seconds = None;
                task.merging = false;
                task.revision += 1;
                if publish(&mut inner, task.clone(), &*self.emit).is_err() {
                    task.error = Some("DATABASE_ERROR".into());
                    *inner.tasks.iter_mut().find(|t| t.id == task.id).unwrap() = task.clone();
                    let displayed = task.clone();
                    emit_task(&inner.db, displayed, &*self.emit);
                }
                inner.active.remove(&task.id);
            }
            (active, std::mem::take(&mut inner.workers))
        };
        self.telegram_accounts.shutdown().await;
        for (_, mut done) in active {
            let _ = done.wait_for(|v| *v).await;
        }
        // Active entries and done signals precede worker destruction. Join even
        // retired workers so their Core references cannot retain the database lock.
        for worker in workers {
            let _ = worker.await;
        }
        self.inner.lock().unwrap().author_draft = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    mod task_action_tests {
        include!("task_action_tests.rs");
    }
    mod download_duplicate_tests {
        include!("download_duplicate_tests.rs");
    }

    #[tokio::test]
    async fn telegram_proxy_can_be_replaced_when_its_old_secret_is_missing() {
        for replacement in ["", "socks5://127.0.0.1:9"] {
            let root = tempfile::tempdir().unwrap();
            let app = core(root.path());
            {
                let mut inner = app.inner.lock().unwrap();
                let profile = inner
                    .settings
                    .profiles
                    .iter_mut()
                    .find(|p| p.platform == "telegram")
                    .unwrap();
                profile.proxy = Some("socks5://127.0.0.1:1080/".into());
                // A new random, nonexistent proxy entry, never a real Telegram session.
                profile.credential_ref =
                    Some(format!("yingqu-transfer-test-{}", uuid::Uuid::new_v4()));
            }
            assert!(app.proxy("telegram").is_err());
            let result = app
                .save_profile_with_network(settings::ProfileUpdate {
                    platform: "telegram".into(),
                    user_agent: "test".into(),
                    cookie: None,
                    proxy: Some(replacement.into()),
                    bypass_proxy: None,
                    http_chunk_size: None,
                })
                .await;
            if let Err(error) = result {
                // The existing writer reports old-secret cleanup failures after persistence.
                assert!(matches!(
                    error.to_string().as_str(),
                    "CREDENTIAL_CLEANUP_ERROR" | "CREDENTIAL_ERROR"
                ));
            }
            assert_eq!(
                app.proxy("telegram").unwrap(),
                (!replacement.is_empty()).then(|| format!("{replacement}/"))
            );
            assert!(app
                .settings()
                .profiles
                .iter()
                .find(|p| p.platform == "telegram")
                .unwrap()
                .credential_ref
                .is_none());
            drop(app.telegram_accounts.network_sessions().unwrap());
            app.shutdown().await;
        }
    }

    #[tokio::test]
    async fn telegram_proxy_database_failure_keeps_old_profile_and_releases_paused_sessions() {
        let root = tempfile::tempdir().unwrap();
        let app = core(root.path());
        let update = |proxy: &str| settings::ProfileUpdate {
            platform: "telegram".into(),
            user_agent: "test".into(),
            cookie: None,
            proxy: Some(proxy.into()),
            bypass_proxy: None,
            http_chunk_size: None,
        };
        app.save_profile(update("socks5://127.0.0.1:9")).unwrap();
        let session = app
            .telegram_accounts
            .binding(crate::telegram_accounts::Purpose::Backup)
            .session;
        session.set_test_view(crate::backup::telegram_session::Phase::Ready, "123");
        app.backup_db(|db| {
            db.execute_batch("CREATE TEMP TRIGGER reject_proxy_save BEFORE INSERT ON platform_settings WHEN NEW.platform='telegram' BEGIN SELECT RAISE(ABORT,'synthetic settings failure'); END;")?;
            Ok(())
        }).unwrap();
        assert_eq!(
            app.save_profile_with_network(update("socks5://127.0.0.1:10"))
                .await
                .err()
                .unwrap()
                .to_string(),
            "DATABASE_ERROR"
        );
        assert_eq!(
            app.proxy("telegram").unwrap().as_deref(),
            Some("socks5://127.0.0.1:9/")
        );
        assert!(matches!(
            session.status().phase,
            crate::backup::telegram_session::Phase::Ready
        ));
        drop(app.telegram_accounts.network_sessions().unwrap());
        let pause = tokio::time::timeout(Duration::from_secs(1), session.pause_network())
            .await
            .unwrap()
            .unwrap();
        drop(pause);
        app.shutdown().await;
    }

    #[tokio::test]
    async fn shutdown_cancels_an_in_progress_asset_fingerprint() {
        let root = tempfile::tempdir().unwrap();
        let app = core(root.path());
        let path = root.path().join("fingerprint.mp4");
        std::fs::write(&path, vec![0_u8; 128 * 1024]).unwrap();
        let cancel = app.inner.lock().unwrap().backup_cancel.subscribe();
        let (started, ready) = tokio::sync::oneshot::channel();
        let (result, finished) = tokio::sync::oneshot::channel();
        let worker = tokio::spawn(async move {
            let outcome = tokio::task::spawn_blocking(move || {
                let started = Mutex::new(Some(started));
                let checks = std::sync::atomic::AtomicUsize::new(0);
                crate::backup::repository::fingerprint_with_cancel(&path, || {
                    // Pause at the second block boundary to make shutdown deterministic.
                    if checks.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 2 {
                        started.lock().unwrap().take().unwrap().send(()).unwrap();
                        while !*cancel.borrow() {
                            std::thread::yield_now();
                        }
                    }
                    *cancel.borrow()
                })
            })
            .await
            .unwrap();
            let _ = result.send(outcome.map(|_| ()).map_err(|e| e.to_string()));
        });
        app.inner.lock().unwrap().workers.push(worker);
        ready.await.unwrap();
        tokio::time::timeout(Duration::from_secs(2), app.shutdown())
            .await
            .unwrap();
        assert_eq!(finished.await.unwrap().unwrap_err(), "CANCELED");
        drop(app);
        let reopened = core(root.path());
        reopened.shutdown().await;
    }

    #[test]
    fn download_progress_emits_at_one_second_intervals_and_merging_immediately() {
        let mut cadence = DownloadProgressCadence::default();
        let start = Instant::now();
        let progress = json!({"bytes":10,"total_bytes":100});
        assert!(cadence.should_emit(&progress, start));
        for millis in [1, 250, 500, 750, 999] {
            assert!(!cadence.should_emit(&progress, start + Duration::from_millis(millis)));
        }
        assert!(cadence.should_emit(&progress, start + Duration::from_secs(1)));
        assert!(!cadence.should_emit(&progress, start + Duration::from_millis(1999)));
        assert!(cadence.should_emit(&progress, start + Duration::from_secs(2)));
        let merging = json!({"bytes":100,"phase":"merging"});
        assert!(cadence.should_emit(&merging, start + Duration::from_millis(2100)));
        assert!(!cadence.should_emit(&merging, start + Duration::from_millis(2200)));
        assert!(cadence.should_emit(&merging, start + Duration::from_millis(3100)));
        assert!(
            DownloadProgressCadence::default().should_emit(&progress, start),
            "each task and retry starts independently"
        );
    }

    #[test]
    fn youtube_speed_samples_aggregate_bytes_instead_of_resetting_hook_rates() {
        let mut rate = YoutubeTransferRate::default();
        let start = Instant::now();
        let update = |rate: &mut YoutubeTransferRate, millis, count, speed| {
            let mut event =
                json!({"bytes":count,"total_bytes":10000,"speed_bps":speed,"eta_seconds":1});
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
        assert_eq!(
            update(&mut rate, 1100, 1100, Value::Null)["speed_bps"],
            1000.0
        );
        assert_eq!(
            update(&mut rate, 1250, 1350, json!(999999999))["speed_bps"],
            1000.0
        );
        assert_eq!(
            update(&mut rate, 1500, 1600, Value::Null)["speed_bps"],
            1000.0
        );
        assert_eq!(update(&mut rate, 2000, 2100, json!(1))["speed_bps"], 1000.0);
        assert_eq!(
            update(&mut rate, 3500, 2850, Value::Null)["speed_bps"],
            500.0
        );
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
            let stage = tempfile::Builder::new()
                .prefix("yingqu-lock-test-")
                .tempdir()
                .unwrap();
            let db = Connection::open_in_memory().unwrap();
            db.execute_batch(
                "CREATE TABLE task_staging(task_id TEXT PRIMARY KEY,path TEXT NOT NULL);",
            )
            .unwrap();
            db.execute(
                "INSERT INTO task_staging VALUES(?1,?2)",
                params!["lock-test", stage.path().to_string_lossy()],
            )
            .unwrap();
            let file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .share_mode(0)
                .open(stage.path().join("audio.wav"))
                .unwrap();
            if transient {
                let release = std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_millis(60));
                    drop(file);
                });
                cleanup_staging(&db, "lock-test", stage.path()).unwrap();
                release.join().unwrap();
            } else {
                assert_eq!(
                    cleanup_staging(&db, "lock-test", stage.path())
                        .unwrap_err()
                        .to_string(),
                    "IO_ERROR"
                );
                assert_eq!(
                    db.query_row("SELECT count(*) FROM task_staging", [], |r| r
                        .get::<_, i64>(0))
                        .unwrap(),
                    1
                );
                drop(file);
                cleanup_staging(&db, "lock-test", stage.path()).unwrap();
            }
            assert!(!stage.path().exists());
            assert_eq!(
                db.query_row("SELECT count(*) FROM task_staging", [], |r| r
                    .get::<_, i64>(0))
                    .unwrap(),
                0
            );
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
        let task = app.resolve(format!("hold:6{}", app.list().len())).unwrap();
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
        for (tool, variant) in [
            (None, Some("cuda")),
            (Some("ffmpeg"), Some("cuda")),
            (Some("fun-asr-nano"), Some("metal")),
        ] {
            assert_eq!(
                app.dependencies_variant(
                    tool.map(str::to_owned),
                    variant.map(str::to_owned),
                    |_| {}
                )
                .await
                .unwrap_err()
                .to_string(),
                "INVALID_INPUT"
            );
        }
        let task = transcript_video(&app, "cuda-speech.mp4").await;
        app.start_transcription(&task.id, false).unwrap();
        let value = transcript_done(&app, &task.id).await;
        assert_eq!(value.state, "completed", "{:?}", value.error_code);
        let result = value.result.unwrap();
        assert_eq!(result.backend.as_deref(), Some("cuda"));
        assert_eq!(
            result.model_revision,
            "ce72677f84900f0dc57f498ace253bfb3c9155b6"
        );
        assert_eq!(
            result.pipeline_revision,
            "fun-asr-nano-audiocpp-50749561-fsmn-v1"
        );
        app.shutdown().await;
        drop(app);
        let app = core(root.path());
        assert_eq!(
            app.transcript(&task.id)
                .unwrap()
                .unwrap()
                .result
                .unwrap()
                .backend
                .as_deref(),
            Some("cuda")
        );
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
        })
        .await
        .unwrap();
        app.cancel_transcription(&task.id, &started.operation_id)
            .await
            .unwrap();
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
        assert_eq!(
            result.model_revision,
            "46e849502a867080d66d351b8dfb1018b607e509"
        );
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
        app.inner
            .lock()
            .unwrap()
            .db
            .execute(
                "UPDATE video_transcripts SET snapshot=? WHERE task_id=?",
                params![serde_json::to_string(&old).unwrap(), task.id],
            )
            .unwrap();
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
    async fn author_upgrade_adopts_tasks_written_by_an_older_build() {
        let root = tempfile::tempdir().unwrap();
        let app = core(root.path());
        let preview = app.resolve("valid".into()).unwrap();
        let template = wait(&app, &preview.id, Status::Ready).await;
        app.shutdown().await;
        drop(app);

        let mut db = Connection::open(root.path().join("tasks.sqlite")).unwrap();
        let shared = b"\x89PNG\r\n\x1a\nshared";
        let legacy = b"\x89PNG\r\n\x1a\nlegacy";
        let mut old_tasks = Vec::new();
        for (id, video_id, author_id) in [
            ("older-write-existing-author", "11", "101001"),
            ("older-write-new-author", "12", "201002"),
            ("older-write-linked-author", "13", "301003"),
        ] {
            let mut task = template.clone();
            task.id = id.into();
            task.status = Status::Completed;
            task.download_requested = true;
            let video = json!({
                "platform":"tiktok", "video_id":video_id, "kind":"video", "resolver":"dtk",
                "canonical_url":format!("https://www.tiktok.com/@fixture/video/{video_id}"),
                "author_profile":{"id":author_id,"nickname":"Fixture","avatar_cached":true},
            });
            if author_id == "101001" {
                authors::save_avatar(&db, &video, shared, task.created_at).unwrap();
            }
            parse_cache::save(&mut db, &video, Vec::new(), None, task.created_at).unwrap();
            assert!(parse_cache::save_avatar(&db, &video, legacy).unwrap());
            db.execute("UPDATE parse_cache SET revision=0 WHERE video_id=?1", [video_id])
                .unwrap();
            task.video = Some(video);
            if author_id == "301003" {
                // A task can already be linked when the old build fills its legacy cache.
                persist(&db, &task).unwrap();
            } else {
                // The older writer persists new snapshots without author links.
                db.execute(
                    "INSERT INTO download_tasks(id,snapshot) VALUES(?1,?2)",
                    params![task.id, serde_json::to_string(&task).unwrap()],
                )
                .unwrap();
            }
            old_tasks.push(task.id);
        }
        drop(db);

        let reopened = core(root.path());
        for (id, body) in old_tasks.iter().zip([shared.as_slice(), legacy, legacy]) {
            let task = reopened
                .list()
                .into_iter()
                .find(|task| &task.id == id)
                .unwrap();
            assert_eq!(task.video.unwrap()["author_profile"]["avatar_cached"], true);
            assert_eq!(reopened.author_avatar(id).unwrap(), body);
        }
        reopened.shutdown().await;
    }

    #[tokio::test]
    async fn old_author_database_copy_backs_up_and_migrates_shared_avatars() {
        let root = tempfile::tempdir().unwrap();
        let app = core(root.path());
        let preview = app.resolve("valid".into()).unwrap();
        let template = wait(&app, &preview.id, Status::Ready).await;
        app.shutdown().await;
        drop(app);

        let old = Connection::open(root.path().join("tasks.sqlite")).unwrap();
        let first = b"\x89PNG\r\n\x1a\nfirst";
        let newest = b"\x89PNG\r\n\x1a\nnewest";
        let shared = b"\x89PNG\r\n\x1a\nshared";
        let rows = [
            (
                "older",
                "tiktok",
                "11",
                "101001",
                "https://www.tiktok.com/@fixture/video/11",
                Some(first.as_slice()),
            ),
            (
                "newer",
                "tiktok",
                "12",
                "101001",
                "https://www.tiktok.com/@fixture/video/12",
                Some(newest.as_slice()),
            ),
            (
                "same-name",
                "tiktok",
                "13",
                "201002",
                "https://www.tiktok.com/@other/video/13",
                None,
            ),
            (
                "telegram-with-profile",
                "telegram",
                "100_8",
                "100",
                "https://t.me/c/100/8",
                Some(shared.as_slice()),
            ),
            (
                "telegram-old",
                "telegram",
                "100_9",
                "",
                "https://t.me/c/100/9",
                None,
            ),
            (
                "douyin-batch",
                "douyin",
                "20",
                "fixture-author",
                "https://www.douyin.com/video/20",
                Some(first.as_slice()),
            ),
        ];
        for (offset, (id, platform, video_id, author_id, url, avatar)) in
            rows.into_iter().enumerate()
        {
            let mut task = template.clone();
            task.id = id.into();
            task.status = Status::Canceled;
            task.created_at += offset as u64;
            task.cached_at = Some(task.created_at);
            let resolver = if platform == "telegram" {
                "grammers"
            } else {
                "dtk"
            };
            let mut video = json!({
                "platform":platform, "video_id":video_id, "canonical_url":url,
                "kind":"video", "resolver":resolver, "title":"迁移样本", "author":"同名作者",
            });
            if !author_id.is_empty() {
                video["author_profile"] = json!({"id":author_id, "nickname":"同名作者"});
            }
            if id == "older" {
                video["author_profile"]["bio"] = "旧库保留的简介".into();
            }
            if id == "newer" {
                video["author_profile"]["handle"] = "new_handle".into();
            }
            if id == "douyin-batch" {
                video["batch_author_id"] = author_id.into();
            }
            if let Some(body) = avatar {
                let mut metadata = video.clone();
                // The old parse cache did not keep batch metadata.
                metadata.as_object_mut().unwrap().remove("batch_author_id");
                parse_cache::save(
                    &mut Connection::open(root.path().join("tasks.sqlite")).unwrap(),
                    &metadata,
                    Vec::new(),
                    None,
                    task.created_at,
                )
                .unwrap();
                parse_cache::save_avatar(&old, &metadata, body).unwrap();
            }
            task.video = Some(video);
            persist(&old, &task).unwrap();
        }
        old.execute_batch(
            "DROP TABLE task_authors; DROP TABLE authors;
             CREATE TABLE douyin_author_avatars(author_id TEXT PRIMARY KEY, body BLOB NOT NULL);",
        )
        .unwrap();
        old.execute(
            "INSERT INTO douyin_author_avatars VALUES('fixture-author',?1)",
            [shared.as_slice()],
        )
        .unwrap();
        let copy = root.path().join("old-copy.sqlite");
        old.backup(rusqlite::MAIN_DB, &copy, None).unwrap();
        drop(old);

        let open = || {
            Core::open(
                &copy,
                root.path().join("downloads"),
                "missing-engine".into(),
                Vec::new(),
                |_| {},
            )
            .unwrap()
        };
        let app = open();
        for id in ["older", "newer"] {
            assert_eq!(app.author_avatar(id).unwrap(), newest);
        }
        for id in ["telegram-old", "telegram-with-profile", "douyin-batch"] {
            assert_eq!(app.author_avatar(id).unwrap(), shared);
        }
        let legacy_telegram = app
            .list()
            .into_iter()
            .find(|task| task.id == "telegram-old")
            .unwrap();
        let profile = &legacy_telegram.video.unwrap()["author_profile"];
        assert_eq!(profile["id"], "100");
        assert_eq!(profile["kind"], "channel");
        assert_eq!(profile["avatar_cached"], true);
        let updated_author = app
            .list()
            .into_iter()
            .find(|task| task.id == "older")
            .unwrap();
        let profile = &updated_author.video.unwrap()["author_profile"];
        assert_eq!(profile["bio"], "旧库保留的简介");
        assert_eq!(profile["handle"], "new_handle");
        {
            let inner = app.inner.lock().unwrap();
            let count = |sql: &str| {
                inner
                    .db
                    .query_row(sql, [], |row| row.get::<_, i64>(0))
                    .unwrap()
            };
            assert_eq!(count("SELECT count(*) FROM authors"), 4);
            assert_eq!(
                count("SELECT count(*) FROM authors WHERE avatar IS NOT NULL"),
                3
            );
            assert_eq!(count("SELECT count(*) FROM task_authors"), 6);
            assert_eq!(count("SELECT count(*) FROM pragma_foreign_key_check"), 0);
        }
        let backups = || {
            std::fs::read_dir(root.path())
                .unwrap()
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| {
                    path.extension().is_some_and(|ext| ext == "sqlite")
                        && path
                            .file_name()
                            .unwrap()
                            .to_string_lossy()
                            .starts_with("old-copy.sqlite.before-authors-v1-")
                })
                .collect::<Vec<_>>()
        };
        let before = backups();
        assert_eq!(before.len(), 1);
        let backup = Connection::open(&before[0]).unwrap();
        assert_eq!(
            backup
                .query_row("SELECT count(*) FROM download_tasks", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            7
        );
        assert!(backup.prepare("SELECT * FROM authors").is_err());
        drop(backup);
        app.shutdown().await;
        drop(app);

        let reopened = open();
        assert_eq!(backups(), before);
        assert_eq!(reopened.author_avatar("telegram-old").unwrap(), shared);
        assert!(reopened.author_avatar("same-name").is_err());
        reopened.remove_history("older").unwrap();
        assert_eq!(reopened.author_avatar("newer").unwrap(), newest);
        reopened.shutdown().await;
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
    async fn album_playback_only_exposes_committed_companions() {
        let root = tempfile::tempdir().unwrap();
        let app = core(root.path());
        let task = app.resolve("album".into()).unwrap();
        wait(&app, &task.id, Status::Ready).await;
        assert_eq!(
            app.album_video_path(&task.id, 0).unwrap_err().to_string(),
            "INVALID_STATE"
        );
        app.download(&task.id, false).unwrap();
        let completed = wait(&app, &task.id, Status::Completed).await;
        let path = completed.path.unwrap().join("001.mp4");
        let body = std::fs::read(&path).unwrap();
        assert_eq!(app.album_video_path(&task.id, 0).unwrap(), path);
        assert_eq!(
            app.album_video_path(&task.id, 1).unwrap_err().to_string(),
            "FILE_MISSING"
        );
        assert_eq!(
            app.album_video_path(&task.id, usize::MAX)
                .unwrap_err()
                .to_string(),
            "FILE_MISSING"
        );
        assert_eq!(
            app.album_video_path("missing", 0).unwrap_err().to_string(),
            "TASK_NOT_FOUND"
        );
        app.shutdown().await;
        drop(app);
        let app = core(root.path());
        assert_eq!(app.album_video_path(&task.id, 0).unwrap(), path);
        assert_eq!(std::fs::read(&path).unwrap(), body);
        std::fs::write(&path, []).unwrap();
        assert_eq!(
            app.album_video_path(&task.id, 0).unwrap_err().to_string(),
            "FILE_MISSING"
        );
        std::fs::remove_file(&path).unwrap();
        assert_eq!(
            app.album_video_path(&task.id, 0).unwrap_err().to_string(),
            "FILE_MISSING"
        );
        std::fs::create_dir(&path).unwrap();
        assert_eq!(
            app.album_video_path(&task.id, 0).unwrap_err().to_string(),
            "FILE_MISSING"
        );
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
        let telegram = json!({"platform":"telegram","video_id":"100_9","resolver":"grammers","kind":"video",
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
            assert!(
                parse_cache::find(&inner.db, "https://t.me/c/100/9", "grammers")
                    .unwrap()
                    .is_some()
            );
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
                        video["resolver"] = "grammers".into();
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

    #[tokio::test]
    async fn stale_telegram_recovery_is_rejected_before_task_admission() {
        let root = tempfile::tempdir().unwrap();
        let app = core(root.path());
        let sample = app.resolve("valid".into()).unwrap();
        let mut task = wait(&app, &sample.id, Status::Ready).await;
        task.status = Status::Failed;
        task.error = Some("TELEGRAM_CONNECTION_ERROR".into());
        task.telegram_link = Some("https://t.me/c/100/9".into());
        task.video = None;
        let epoch = {
            let mut inner = app.inner.lock().unwrap();
            inner.active.remove(&task.id);
            publish(&mut inner, task.clone(), &*app.emit).unwrap();
            inner.telegram_recovery_epoch
        };
        app.stop_telegram_recovery(); // Account switch/logout begins before taking its task snapshot.
        let error = app
            .resolve_task(
                task.telegram_link.clone().unwrap().into(),
                Some(&task.id),
                "grammers".into(),
                None,
                false,
                Some((task.revision, epoch)),
            )
            .err()
            .unwrap();
        assert_eq!(error.to_string(), "TELEGRAM_ACCOUNT_MISMATCH");
        let current = app.list().into_iter().find(|t| t.id == task.id).unwrap();
        assert_eq!(current.status, Status::Failed);
        assert_eq!(current.revision, task.revision);
        assert!(!app.inner.lock().unwrap().active.contains_key(&task.id));
        app.shutdown().await;
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
    async fn resolve_progress_tracks_real_phases_cache_and_cancellation() {
        use ResolvePhase::*;
        let root = tempfile::tempdir().unwrap();
        let app = core(root.path());
        let first = app.resolve("valid".into()).unwrap();
        let ready = wait(&app, &first.id, Status::Ready).await;
        let progress = ready.resolve_progress.as_ref().unwrap();
        assert_eq!(progress.phase, Completed);
        assert_eq!(
            progress.steps,
            [
                CheckingLink,
                CheckingCache,
                StartingEngine,
                LoadingConfiguration,
                FetchingMetadata,
                SelectingFormats,
                LoadingPreview,
                LoadingAuthor,
                Finalizing,
                Completed
            ]
        );
        assert!(ready.revision > first.revision + 1);
        app.resolve_progress(&first.id, LoadingAuthor);
        assert_eq!(
            app.list()
                .iter()
                .find(|task| task.id == first.id)
                .unwrap()
                .revision,
            ready.revision
        );

        let cached = app
            .resolve("https://www.douyin.com/video/123".into())
            .unwrap();
        assert_eq!(cached.status, Status::Ready);
        assert!(cached.cache_hit);
        assert_eq!(
            cached.resolve_progress.unwrap().steps,
            [CheckingLink, CheckingCache, CacheHit, Completed]
        );

        let blocked = app.resolve("progress-block".into()).unwrap();
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let task = app
                    .list()
                    .into_iter()
                    .find(|task| task.id == blocked.id)
                    .unwrap();
                assert_eq!(task.status, Status::Parsing);
                if task
                    .resolve_progress
                    .is_some_and(|progress| progress.phase == FetchingMetadata)
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        app.cancel(&blocked.id).await.unwrap();
        let canceled = wait(&app, &blocked.id, Status::Canceled).await;
        app.resolve_progress(&blocked.id, LoadingPreview);
        let current = app
            .list()
            .into_iter()
            .find(|task| task.id == blocked.id)
            .unwrap();
        assert_eq!(current.revision, canceled.revision);
        assert_eq!(current.resolve_progress.unwrap().phase, FetchingMetadata);
        app.shutdown().await;
    }

    #[tokio::test]
    async fn resolve_progress_does_not_complete_when_task_publication_fails() {
        use ResolvePhase::*;
        let root = tempfile::tempdir().unwrap();
        let app = core(root.path());
        app.inner
            .lock()
            .unwrap()
            .db
            .execute_batch(
                "CREATE TRIGGER reject_ready BEFORE UPDATE ON download_tasks
             WHEN json_extract(NEW.snapshot,'$.status') = 'ready'
             BEGIN SELECT RAISE(ABORT, 'test'); END;",
            )
            .unwrap();
        let task = app.resolve("valid".into()).unwrap();
        let failed = wait(&app, &task.id, Status::Failed).await;
        assert_eq!(failed.error.as_deref(), Some("DATABASE_ERROR"));
        let progress = failed.resolve_progress.unwrap();
        assert_eq!(progress.phase, Finalizing);
        assert!(!progress.steps.contains(&Completed));
        app.shutdown().await;
    }

    #[tokio::test]
    async fn resolve_progress_protocol_rejects_unknown_and_regressing_phases() {
        let root = tempfile::tempdir().unwrap();
        let app = core(root.path());
        let mut phases = Vec::new();
        let result = PythonBridge::operation(
            &app.program,
            &app.args,
            "phase-probe",
            "resolve_video",
            json!({"input":"valid","report_resolve_progress":true}),
            |event| phases.push(event["resolve_phase"].clone()),
        )
        .await
        .unwrap();
        assert_eq!(result["type"], "resolved");
        assert_eq!(
            phases,
            [
                json!("loading_configuration"),
                json!("fetching_metadata"),
                json!("selecting_formats")
            ]
        );
        for input in [
            "progress-unknown",
            "progress-core-phase",
            "progress-duplicate",
            "progress-backwards",
        ] {
            let result = PythonBridge::operation(
                &app.program,
                &app.args,
                "phase-probe",
                "resolve_video",
                json!({"input":input,"report_resolve_progress":true}),
                |_| {},
            )
            .await;
            assert_eq!(
                result.unwrap_err().to_string(),
                "ENGINE_PROTOCOL_ERROR",
                "{input}"
            );
        }
        app.shutdown().await;
    }
    #[tokio::test]
    async fn runtime_dependencies_check_install_and_cancel_share_worker_lifecycle() {
        let root = tempfile::tempdir().unwrap();
        let app = core(root.path());
        let status = app.dependencies(None, |_| {}).await.unwrap();
        for tool in ["sensevoice", "sensevoice-cuda", "arbitrary-url"] {
            assert_eq!(
                app.dependencies(Some(tool.into()), |_| {})
                    .await
                    .unwrap_err()
                    .to_string(),
                "INVALID_INPUT"
            );
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
                    let name = if variant == "cuda" {
                        "fun-asr-nano-cuda"
                    } else {
                        "fun-asr-nano"
                    };
                    let target = runtime.join(name);
                    if had_target {
                        std::fs::create_dir(&target).unwrap();
                        std::fs::write(target.join("old-model"), b"original").unwrap();
                    }
                    let selection = runtime.join("fun-asr-nano-backend.json");
                    let original =
                        (phase == "after-choice").then_some(b"{\"variant\":\"cpu\"}".to_vec());
                    if let Some(bytes) = &original {
                        std::fs::write(&selection, bytes).unwrap();
                    }
                    std::fs::write(root.path().join("dependency-commit-mode"), phase).unwrap();
                    let running = app.clone();
                    let worker = tokio::spawn(async move {
                        running
                            .dependencies_variant(
                                Some("fun-asr-nano".into()),
                                Some(variant.into()),
                                |_| {},
                            )
                            .await
                    });
                    tokio::time::timeout(Duration::from_secs(10), async {
                        while !root.path().join("dependency-commit-started").is_file() {
                            tokio::time::sleep(Duration::from_millis(10)).await;
                        }
                    })
                    .await
                    .unwrap();
                    if phase == "after-choice" && variant == "cuda" && had_target {
                        app.shutdown().await;
                    } else {
                        app.cancel_dependency().unwrap();
                    }
                    assert_eq!(worker.await.unwrap().unwrap_err().to_string(), "CANCELED");
                    assert_eq!(std::fs::read(&selection).ok(), original);
                    if had_target {
                        assert_eq!(
                            std::fs::read(target.join("old-model")).unwrap(),
                            b"original"
                        );
                        assert!(!target.join("new-model").exists());
                    } else {
                        assert!(!target.exists());
                    }
                    assert!(!std::fs::read_dir(&runtime).unwrap().any(|entry| entry
                        .unwrap()
                        .file_name()
                        .to_string_lossy()
                        .starts_with(".install-")));
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
    async fn weixin_download_submission_refreshes_auth_from_the_current_settings() {
        use settings::AccountAuth;

        let root = tempfile::tempdir().unwrap();
        let app = core(root.path());
        app.update_account_auth("weixin", AccountAuth::Manual, Some("auth=manual".into())).unwrap();
        let task = app.resolve("https://weixin.qq.com/sph/First".into()).unwrap();
        let ready = wait(&app, &task.id, Status::Ready).await;
        app.update_account_auth("weixin", AccountAuth::Login, Some("auth=login".into())).unwrap();
        {
            let mut inner = app.inner.lock().unwrap();
            assert_eq!(inner.active[&task.id].profiles["weixin"]["cookie"], "auth=manual");
            app.queue_download(&mut inner, ready, true, None).unwrap();
            let profiles = &inner.active[&task.id].profiles;
            assert_eq!(profiles["weixin"]["cookie"], "auth=login");
        }
        app.shutdown().await;
    }

    #[tokio::test]
    async fn weixin_ready_alias_and_selected_format_survive_restart() {
        let root = tempfile::tempdir().unwrap();
        let app = core(root.path());
        let first = app.resolve("https://weixin.qq.com/sph/First".into()).unwrap();
        wait(&app, &first.id, Status::Ready).await;
        let second_url = "https://weixin.qq.com/sph/Second";
        let second = app.resolve(second_url.into()).unwrap();
        let ready = wait(&app, &second.id, Status::Ready).await;
        assert!(ready.cache_hit);
        assert_eq!(ready.video.unwrap()["canonical_url"], second_url);
        assert_eq!(app.author_avatar(&second.id).unwrap(), b"\xff\xd8\xfffixture-avatar");
        let selected = "2".repeat(24);
        app.choose_format(&second.id, &selected).unwrap();
        app.shutdown().await;
        drop(app);
        let restarted = core(root.path());
        let ready = wait(&restarted, &second.id, Status::Ready).await;
        let video = ready.video.unwrap();
        assert_eq!(video["canonical_url"], second_url);
        assert_eq!(video["selected_format_id"], selected);
        assert_eq!(video["width"], 720);
        assert_eq!(video["height"], 1280);
        assert_eq!(video["author_profile"]["avatar_cached"], true);
        assert_eq!(restarted.author_avatar(&second.id).unwrap(), b"\xff\xd8\xfffixture-avatar");
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
            std::fs::read(path.join("001.mp4")).unwrap(),
            b"\0\0\0\x18ftypmp42live-fixture"
        );
        assert!(completed.files[0].video.is_some());
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
        assert_eq!(
            recovered.files[0].video.as_ref().unwrap().filename,
            "001.mp4"
        );
        std::fs::rename(path.join("001.mp4"), path.join("missing.mp4")).unwrap();
        assert!(restored.list()[0].file_missing);
        std::fs::rename(path.join("missing.mp4"), path.join("001.mp4")).unwrap();
        assert!(restored.image(&task.id, 1).unwrap().starts_with(b"RIFF"));
        std::fs::write(path.join("user-note.txt"), "preserve").unwrap();
        restored.delete_local_file(&task.id).unwrap();
        assert_eq!(
            std::fs::read(path.join("user-note.txt")).unwrap(),
            b"preserve"
        );
        for file in &completed.files {
            assert!(!path.join(&file.filename).exists());
            if let Some(video) = &file.video {
                assert!(!path.join(&video.filename).exists());
            }
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

        let douyin_short = "https://v.douyin.com/fixture-work/";
        let douyin_alias = "https://v.douyin.com/fixture-work-alias/";
        let douyin_author::DownloadLink::Work(work) = app
            .resolve_download_link(douyin_short.into(), "auto".into())
            .await
            .unwrap()
        else {
            panic!("expected a single work");
        };
        let ready = wait(&app, &work.id, Status::Ready).await;
        assert!(!ready.cache_hit);
        let douyin_author::DownloadLink::Work(alias) = app
            .resolve_download_link(douyin_alias.into(), "auto".into())
            .await
            .unwrap()
        else {
            panic!("expected a single work");
        };
        assert!(alias.cache_hit);

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
        for (url, id) in [
            ("7.10 share https://v.douyin.com/share_album/ 复制", "601"),
            (douyin_short, "123"),
            (douyin_alias, "123"),
        ] {
            let douyin_author::DownloadLink::Work(cached) = restarted
                .resolve_download_link(url.into(), "auto".into())
                .await
                .unwrap()
            else {
                panic!("expected a single work");
            };
            assert_eq!(cached.status, Status::Ready);
            assert!(cached.cache_hit);
            assert_eq!(cached.video.as_ref().unwrap()["video_id"], id);
        }
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
        for index in 0..16 {
            let app = core(root.path());
            let task = app.resolve(format!("hold:6{index:02}")).unwrap();
            wait(&app, &task.id, Status::Ready).await;
            app.download(&task.id, true).unwrap();
            wait(&app, &task.id, Status::Completed).await;
            app.shutdown().await;
            drop(app);
            // No retry or sleep: shutdown must release every worker reference.
            let reopened = core(root.path());
            let restored = reopened
                .list()
                .into_iter()
                .find(|t| t.id == task.id)
                .unwrap();
            assert_eq!(restored.status, Status::Completed);
            assert!(restored.path.unwrap().is_file());
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
            insertion_id: 0,
            asset_id: None,
            revision: 1,
            status: Status::Downloading,
            resolve_progress: None,
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
            download_requested_at: None,
            completed_at: None,
            canceling: false,
            file_missing: false,
            queue_order: None,
            download_requested: false,
            duplicate_confirmed: false,
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
    #[tokio::test]
    async fn filename_rules_persist_snapshot_queue_and_suffix_collisions() {
        let root = tempfile::tempdir().unwrap();
        let app = core(root.path());
        assert_eq!(
            app.settings().filename_template,
            crate::naming::DEFAULT_TEMPLATE
        );
        app.set_max_downloads(1).unwrap();
        app.set_filename_template("old/{link_id}-{quality}".into())
            .unwrap();
        let a = enqueue(&app, "9101").await;
        let b = enqueue(&app, "9102").await;
        app.set_filename_template("new/{link_id}".into()).unwrap();
        assert!(app.set_filename_template("../escape".into()).is_err());
        release(root.path(), "9101");
        let a = wait(&app, &a.id, Status::Completed).await;
        assert!(a.path.as_ref().unwrap().ends_with("old/9101-1080P.mp4"));
        release(root.path(), "9102");
        let b = wait(&app, &b.id, Status::Completed).await;
        assert!(b.path.as_ref().unwrap().ends_with("old/9102-1080P.mp4"));
        let c = enqueue(&app, "9103").await;
        release(root.path(), "9103");
        let c = wait(&app, &c.id, Status::Completed).await;
        assert!(c.path.as_ref().unwrap().ends_with("new/9103.mp4"));
        let original = std::fs::read(c.path.as_ref().unwrap()).unwrap();
        let repeat = app.resolve("hold:9103".into()).unwrap();
        wait(&app, &repeat.id, Status::Ready).await;
        assert_eq!(
            app.submit_download(&repeat.id, true, Some(&c.id)).unwrap(),
            c.id
        );
        let repeat = wait(&app, &c.id, Status::Completed).await;
        assert_eq!(repeat.path, c.path);
        assert_eq!(std::fs::read(c.path.unwrap()).unwrap(), original);
        app.set_directory(root.path().to_owned()).unwrap();
        app.set_max_downloads(3).unwrap();
        app.shutdown().await;
        drop(app);
        let app = core(root.path());
        assert_eq!(app.settings().filename_template, "new/{link_id}");
        assert_eq!(app.settings().max_downloads, 3);
        assert_eq!(
            app.settings().download_directory,
            root.path().canonicalize().unwrap()
        );
        app.shutdown().await;
    }

    #[tokio::test]
    async fn filename_rules_preserve_literal_braces_in_settings_and_downloads() {
        let root = tempfile::tempdir().unwrap();
        let app = core(root.path());
        let rule = "literal{unknown}/{user_id}-{link_id}-{quality";
        app.set_filename_template(rule.into()).unwrap();
        app.shutdown().await;
        drop(app);
        let app = core(root.path());
        assert_eq!(app.settings().filename_template, rule);
        let task = enqueue(&app, "9120").await;
        release(root.path(), "9120");
        let completed = wait(&app, &task.id, Status::Completed).await;
        let path = completed.path.unwrap();
        assert!(path.ends_with("literal{unknown}/fixture-author-9120-{quality.mp4"));
        assert!(path.is_file());
        app.shutdown().await;
    }

    #[tokio::test]
    async fn filename_rules_name_album_directories_and_preserve_internal_filenames() {
        let root = tempfile::tempdir().unwrap();
        let app = core(root.path());
        app.set_filename_template("{platform}/{content}-{quality}".into())
            .unwrap();
        let task = app.resolve("album".into()).unwrap();
        wait(&app, &task.id, Status::Ready).await;
        app.download(&task.id, false).unwrap();
        let completed = wait(&app, &task.id, Status::Completed).await;
        let path = completed.path.unwrap();
        assert!(path.ends_with("douyin/离线图集-图集"));
        assert!(path.join("001.jpeg").is_file() && path.join("002.webp").is_file());
        app.shutdown().await;
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
    async fn startup_clears_unmoved_commit_intent_before_downloading_again() {
        let root = tempfile::tempdir().unwrap();
        let app = core(root.path());
        let task = enqueue(&app, "8609").await;
        started(root.path(), "8609").await;
        app.shutdown().await;
        drop(app);
        let db = Connection::open(root.path().join("tasks.sqlite")).unwrap();
        let source = root.path().join(format!("yingqu-{}-old-staging", task.id));
        std::fs::create_dir(&source).unwrap();
        let staged_file = source.join("video.mp4");
        std::fs::write(&staged_file, b"old bytes").unwrap();
        db.execute(
            "INSERT INTO task_staging(task_id,path) VALUES(?1,?2)",
            params![task.id, source.to_string_lossy()],
        )
        .unwrap();
        db.execute(
            "INSERT INTO commit_intents(task_id,source,target,size) VALUES(?1,?2,?3,9)",
            params![
                task.id,
                staged_file.to_string_lossy(),
                root.path().join("not-moved.mp4").to_string_lossy()
            ],
        )
        .unwrap();
        drop(db);
        let app = core(root.path());
        assert!(!source.exists());
        assert_eq!(app.list()[0].error.as_deref(), Some("INTERRUPTED"));
        app.start_backup_service();
        assert_eq!(
            app.inner
                .lock()
                .unwrap()
                .db
                .query_row(
                    "SELECT count(*) FROM commit_intents WHERE task_id=?",
                    [&task.id],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            0
        );
        release(root.path(), "8609");
        let done = wait(&app, &task.id, Status::Completed).await;
        assert!(done.path.unwrap().is_file());
        assert_eq!(app.list().len(), 1);
        app.shutdown().await;
    }

    #[tokio::test]
    async fn startup_restores_interrupted_downloads_in_original_fifo_order() {
        let root = tempfile::tempdir().unwrap();
        let app = core(root.path());
        app.set_max_downloads(1).unwrap();
        let first = enqueue(&app, "8601").await;
        started(root.path(), "8601").await;
        let second = enqueue(&app, "8602").await;
        let original = app.list();
        app.shutdown().await;
        drop(app);
        std::fs::remove_file(root.path().join("started-8601")).unwrap();
        let app = core(root.path());
        assert!(app
            .list()
            .iter()
            .all(|task| task.error.as_deref() == Some("INTERRUPTED")));
        app.start_backup_service();
        started(root.path(), "8601").await;
        assert!(!root.path().join("started-8602").exists());
        let second_waiting = app
            .list()
            .into_iter()
            .find(|task| task.id == second.id)
            .unwrap();
        assert_eq!(second_waiting.status, Status::Queued);
        assert_eq!(second_waiting.bytes, 0);
        for task in app.list() {
            let before = original
                .iter()
                .find(|previous| previous.id == task.id)
                .unwrap();
            assert_eq!(task.insertion_id, before.insertion_id);
            assert_eq!(task.queue_order, before.queue_order);
            assert_eq!(task.download_requested_at, before.download_requested_at);
            assert_eq!(
                task.video.as_ref().unwrap()["selected_format_id"],
                before.video.as_ref().unwrap()["selected_format_id"]
            );
        }
        let workers = app.inner.lock().unwrap().workers.len();
        app.start_backup_service();
        assert_eq!(app.inner.lock().unwrap().workers.len(), workers);
        release(root.path(), "8601");
        wait(&app, &first.id, Status::Completed).await;
        started(root.path(), "8602").await;
        release(root.path(), "8602");
        wait(&app, &second.id, Status::Completed).await;
        assert_eq!(app.list().len(), 2);
        app.shutdown().await;
    }

    #[tokio::test]
    async fn startup_skips_canceled_business_failures_and_unsubmitted_previews() {
        let root = tempfile::tempdir().unwrap();
        let app = core(root.path());
        let done = enqueue(&app, "9105").await;
        wait(&app, &done.id, Status::Completed).await;
        let failed = enqueue(&app, "7305").await;
        started(root.path(), "7305").await;
        release(root.path(), "7305");
        wait(&app, &failed.id, Status::Failed).await;
        let canceled = enqueue(&app, "8305").await;
        started(root.path(), "8305").await;
        app.cancel(&canceled.id).await.unwrap();
        let draft = app.resolve("valid".into()).unwrap();
        wait(&app, &draft.id, Status::Ready).await;
        app.shutdown().await;
        drop(app);
        let app = core(root.path());
        let before: Vec<_> = app
            .list()
            .into_iter()
            .map(|task| (task.id, task.status, task.error, task.revision))
            .collect();
        app.start_backup_service();
        let after: Vec<_> = app
            .list()
            .into_iter()
            .map(|task| (task.id, task.status, task.error, task.revision))
            .collect();
        assert_eq!(before, after);
        assert!(app.inner.lock().unwrap().downloads.is_empty());
        assert!(app.inner.lock().unwrap().active.is_empty());
        app.shutdown().await;
    }

    #[tokio::test]
    async fn startup_resumes_an_accepted_replacement_without_requiring_new_confirmation() {
        let root = tempfile::tempdir().unwrap();
        let app = core(root.path());
        app.set_filename_template("{link_id}".into()).unwrap();
        let first = enqueue(&app, "8606").await;
        started(root.path(), "8606").await;
        release(root.path(), "8606");
        let first = wait(&app, &first.id, Status::Completed).await;
        let original = std::fs::read(first.path.as_ref().unwrap()).unwrap();
        std::fs::remove_file(root.path().join("release-8606")).unwrap();
        std::fs::remove_file(root.path().join("started-8606")).unwrap();
        let repeat = app.resolve("hold:8606".into()).unwrap();
        wait(&app, &repeat.id, Status::Ready).await;
        app.submit_download(&repeat.id, true, Some(&first.id))
            .unwrap();
        started(root.path(), "8606").await;
        app.shutdown().await;
        drop(app);
        std::fs::remove_file(root.path().join("started-8606")).unwrap();
        let app = core(root.path());
        app.start_backup_service();
        started(root.path(), "8606").await;
        release(root.path(), "8606");
        let repeat = wait(&app, &first.id, Status::Completed).await;
        assert_eq!(first.path, repeat.path);
        assert_eq!(std::fs::read(first.path.unwrap()).unwrap(), original);
        assert_eq!(app.list().len(), 1);
        app.shutdown().await;
    }

    #[tokio::test]
    async fn fifo_completion_failure_and_queued_cancellation_release_slots() {
        let root = tempfile::tempdir().unwrap();
        let app = core(root.path());
        assert_eq!(app.settings().max_downloads, 5);
        app.set_max_downloads(2).unwrap();
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
            std::fs::read_dir(root.path().join("downloads/douyin"))
                .unwrap()
                .count(),
            2
        );
    }

    #[tokio::test]
    async fn ten_download_slots_dispatch_and_reduce_without_interrupting() {
        let root = tempfile::tempdir().unwrap();
        let app = core(root.path());
        assert_eq!(app.settings().max_downloads, 5);
        app.set_max_downloads(10).unwrap();
        let mut tasks = Vec::new();
        for id in 8151..=8161 {
            tasks.push(enqueue(&app, &id.to_string()).await);
        }
        for id in 8151..=8160 {
            started(root.path(), &id.to_string()).await;
        }
        assert!(!root.path().join("started-8161").exists());
        assert_eq!(
            app.list()
                .iter()
                .filter(|task| task.status == Status::Downloading)
                .count(),
            10
        );
        app.set_max_downloads(9).unwrap();
        assert_eq!(
            app.list()
                .iter()
                .filter(|task| task.status == Status::Downloading)
                .count(),
            10
        );
        app.cancel(&tasks[0].id).await.unwrap();
        assert_eq!(
            app.list()
                .iter()
                .find(|task| task.id == tasks[10].id)
                .unwrap()
                .status,
            Status::Queued
        );
        app.cancel(&tasks[1].id).await.unwrap();
        started(root.path(), "8161").await;
        assert_eq!(
            app.list()
                .iter()
                .filter(|task| task.status == Status::Downloading)
                .count(),
            9
        );
        app.set_max_downloads(10).unwrap();
        app.shutdown().await;
        drop(app);
        let app = core(root.path());
        assert_eq!(app.settings().max_downloads, 10);
        app.shutdown().await;
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
        assert!(app.set_max_downloads(11).is_err());
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
        for id in [&a.id, &b.id, &c.id] {
            assert_eq!(
                app.list().iter().find(|t| &t.id == id).unwrap().status,
                Status::Canceled
            );
        }
        let interrupted = app.list().into_iter().find(|t| t.id == d.id).unwrap();
        assert_eq!(interrupted.status, Status::Failed);
        assert_eq!(interrupted.error.as_deref(), Some("INTERRUPTED"));
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
            bypass_proxy: None,
            http_chunk_size: Some(Some(10_485_760)),
        })
        .unwrap();
        let a = enqueue(&app, "8201").await;
        let old = app.settings().profiles[0].user_agent.clone();
        let b = enqueue(&app, "8202").await;
        app.save_profile(settings::ProfileUpdate {
            bypass_proxy: None,
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
            bypass_proxy: None,
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
        assert_eq!(
            app.settings()
                .profiles
                .iter()
                .find(|profile| profile.platform == "youtube")
                .unwrap()
                .http_chunk_size,
            Some(0)
        );
        let c = enqueue(&app, "8203").await;
        let parsing = app.resolve("block".into()).unwrap();
        app.shutdown().await;
        for id in [&b.id, &c.id, &parsing.id] {
            let task = app.list().into_iter().find(|t| &t.id == id).unwrap();
            assert_eq!(task.status, Status::Failed);
            assert_eq!(task.error.as_deref(), Some("INTERRUPTED"));
            assert!(!task.canceling);
        }
        assert!(app.inner.lock().unwrap().active.is_empty());
        assert!(!root.path().join("started-8203").exists());
        assert_eq!(
            std::fs::read_dir(root.path().join("downloads"))
                .unwrap()
                .count(),
            0
        );
        drop(app);
        let reopened = core(root.path());
        for id in [&b.id, &c.id, &parsing.id] {
            let task = reopened.list().into_iter().find(|t| &t.id == id).unwrap();
            assert_eq!(task.status, Status::Failed);
            assert_eq!(task.error.as_deref(), Some("INTERRUPTED"));
        }
        let canceled = reopened.list().into_iter().find(|t| t.id == a.id).unwrap();
        assert_eq!(canceled.status, Status::Canceled);
        assert!(canceled.error.is_none());
        reopened.shutdown().await;
    }

    #[tokio::test]
    async fn restart_interrupts_persisted_active_tasks_without_starting_engine() {
        for status in [Status::Parsing, Status::Queued, Status::Downloading] {
            let root = tempfile::tempdir().unwrap();
            let app = core(root.path());
            let task = app.resolve("valid".into()).unwrap();
            let mut ready = wait(&app, &task.id, Status::Ready).await;
            ready.status = status;
            ready.queue_order = Some(1);
            persist(&app.inner.lock().unwrap().db, &ready).unwrap();
            drop(app);
            let app = core(root.path());
            assert_eq!(app.list()[0].status, Status::Failed);
            assert_eq!(app.list()[0].error.as_deref(), Some("INTERRUPTED"));
            assert!(app.inner.lock().unwrap().active.is_empty());
            app.shutdown().await;
        }
    }

    #[tokio::test]
    async fn shutdown_preserves_an_in_flight_user_cancellation() {
        let root = tempfile::tempdir().unwrap();
        let app = core(root.path());
        let task = enqueue(&app, "8401").await;
        started(root.path(), "8401").await;
        {
            let mut inner = app.inner.lock().unwrap();
            let mut canceling = inner
                .tasks
                .iter()
                .find(|t| t.id == task.id)
                .unwrap()
                .clone();
            // User intent is already recorded; exit may now stop the worker.
            canceling.canceling = true;
            canceling.revision += 1;
            publish(&mut inner, canceling, &*app.emit).unwrap();
        }
        app.shutdown().await;
        drop(app);
        let reopened = core(root.path());
        let canceled = reopened
            .list()
            .into_iter()
            .find(|t| t.id == task.id)
            .unwrap();
        assert_eq!(canceled.status, Status::Canceled);
        assert!(canceled.error.is_none());
        assert!(!canceled.canceling);
        assert!(!root.path().join("downloads/douyin").exists());
        reopened.shutdown().await;
    }
    #[tokio::test]
    async fn insertion_order_uses_original_database_rowids_for_legacy_snapshots() {
        let root = tempfile::tempdir().unwrap();
        let app = core(root.path());
        let first = app.resolve("valid".into()).unwrap();
        wait(&app, &first.id, Status::Ready).await;
        let second = app.resolve("valid".into()).unwrap();
        wait(&app, &second.id, Status::Ready).await;
        assert!(first.insertion_id > 0);
        assert!(second.insertion_id > first.insertion_id);
        app.shutdown().await;
        drop(app);
        let app = core(root.path());
        app.backup_db(|db| {
            db.execute("UPDATE download_tasks SET snapshot=json_remove(json_set(snapshot,'$.created_at',CASE WHEN id=? THEN 9999999999 ELSE 1 END),'$.insertion_id')", [&first.id])?;
            Ok(())
        }).unwrap();
        app.shutdown().await;
        drop(app);
        let app = core(root.path());
        let tasks = app.list();
        assert_eq!(
            tasks
                .iter()
                .map(|task| task.id.as_str())
                .collect::<Vec<_>>(),
            vec![second.id.as_str(), first.id.as_str()]
        );
        assert_eq!(tasks[0].insertion_id, second.insertion_id);
        assert_eq!(tasks[1].insertion_id, first.insertion_id);
        app.shutdown().await;
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
            bypass_proxy: None,
            http_chunk_size: None,
            platform: "douyin".into(),
            user_agent: "retry-agent".into(),
            cookie: None,
            proxy: None,
        })
        .unwrap();
        let retry = app.retry(&old.id).unwrap();
        assert_eq!(retry.id, old.id);
        assert!(old.insertion_id > 0);
        assert_eq!(retry.insertion_id, old.insertion_id);
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
        assert_eq!(restored.insertion_id, old.insertion_id);
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
                "CREATE TRIGGER block_cover_cleanup BEFORE DELETE ON cover_refs
                WHEN OLD.owner='cache' BEGIN SELECT RAISE(ABORT, 'fixture cleanup failure'); END;",
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
                "CREATE TRIGGER block_cover_save BEFORE INSERT ON cover_refs
                WHEN NEW.owner='cache' BEGIN SELECT RAISE(ABORT, 'fixture cache failure'); END;",
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
        for status in [Status::Failed, Status::Canceled, Status::Ready] {
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
        assert_eq!(
            app.download(&retry.id, true).unwrap_err().to_string(),
            "ALREADY_DOWNLOADED"
        );
        app.submit_download(&retry.id, true, Some(&original.id))
            .unwrap();
        let repeated = wait(&app, &original.id, Status::Completed).await;
        assert_eq!(original.path, repeated.path);
        assert_eq!(original.insertion_id, repeated.insertion_id);
        assert_eq!(app.list().len(), 1);
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
    #[test]
    fn asset_deletion_finalization_retains_the_existing_busy_guard() {
        let root = tempfile::tempdir().unwrap();
        let core = Core::open(
            &root.path().join("busy-delete.sqlite"),
            root.path().join("downloads"),
            "unused".into(),
            vec![],
            |_| {},
        )
        .unwrap();
        let task: Task = serde_json::from_value(json!({"id":"busy-task","asset_id":"busy-asset","revision":1,"status":"queued","video":null,"bytes":0,"total_bytes":null,"speed_bps":null,"error":null,"path":null,"created_at":1,"completed_at":null,"canceling":false})).unwrap();
        core.inner.lock().unwrap().tasks.push(task);
        assert_eq!(
            core.finish_asset_deletion("busy-asset", "unstarted-operation")
                .unwrap_err()
                .to_string(),
            "TASK_BUSY"
        );
        assert_eq!(core.list().len(), 1);
    }

    #[test]
    fn asset_deletion_event_keeps_operation_space_after_switching_spaces() {
        let root = tempfile::tempdir().unwrap();
        let events = Arc::new(Mutex::new(Vec::new()));
        let captured = events.clone();
        let core = Core::open(
            &root.path().join("scoped-delete.sqlite"),
            root.path().join("downloads"),
            "unused".into(),
            vec![],
            move |event| captured.lock().unwrap().push(event),
        )
        .unwrap();
        let task: Task = serde_json::from_value(json!({"id":"task","asset_id":"asset","revision":1,"status":"completed","video":null,"bytes":0,"total_bytes":null,"speed_bps":null,"error":null,"path":null,"created_at":1,"completed_at":2,"canceling":false})).unwrap();
        {
            let mut inner = core.inner.lock().unwrap();
            persist(&inner.db, &task).unwrap();
            inner.db.execute_batch("
                INSERT INTO backup_spaces VALUES('A','a','','','','',0),('B','b','','','','',1);
                INSERT INTO backup_current_space VALUES(1,'B');
                INSERT INTO media_assets VALUES('asset','task','{}','video.mp4',NULL,24,'active');
                INSERT INTO backup_operations(operation_id,space_id,asset_id,kind,state,phase,payload_json) VALUES('delete','A','asset','delete','running','finalizing','{}');
            ").unwrap();
            inner.tasks.push(task);
        }
        core.finish_asset_deletion("asset", "delete").unwrap();
        let captured = events.lock().unwrap();
        assert!(captured.iter().any(|event| matches!(event, TaskEvent::RemovedFromSpace { id, space_id } if id == "task" && space_id == "A")));
        assert!(!captured
            .iter()
            .any(|event| matches!(event, TaskEvent::Removed(_))));
        assert!(core.list().is_empty());
    }

    #[tokio::test]
    async fn download_request_time_survives_progress_cancel_and_restart_with_legacy_fallback() {
        let root = tempfile::tempdir().unwrap();
        let app = core(root.path());
        let parsed = app.resolve("hold:78001".into()).unwrap();
        let ready = wait(&app, &parsed.id, Status::Ready).await;
        assert!(ready.download_requested_at.is_none());
        let before = chrono::Utc::now().timestamp_millis().max(0) as u64;
        app.download(&ready.id, false).unwrap();
        let running = wait(&app, &ready.id, Status::Downloading).await;
        let requested = running.download_requested_at.unwrap();
        assert!(requested >= before);
        assert_eq!(
            app.list()
                .iter()
                .find(|t| t.id == ready.id)
                .unwrap()
                .download_requested_at,
            Some(requested)
        );
        app.cancel(&ready.id).await.unwrap();
        let canceled = app.list().into_iter().find(|t| t.id == ready.id).unwrap();
        assert_eq!(canceled.status, Status::Canceled);
        assert_eq!(canceled.download_requested_at, Some(requested));
        let mut legacy = serde_json::to_value(&canceled).unwrap();
        legacy
            .as_object_mut()
            .unwrap()
            .remove("download_requested_at");
        assert!(serde_json::from_value::<Task>(legacy)
            .unwrap()
            .download_requested_at
            .is_none());
        app.shutdown().await;
        drop(app);
        let reopened = core(root.path());
        assert_eq!(
            reopened
                .list()
                .iter()
                .find(|t| t.id == ready.id)
                .unwrap()
                .download_requested_at,
            Some(requested)
        );
        reopened.shutdown().await;
    }
}
