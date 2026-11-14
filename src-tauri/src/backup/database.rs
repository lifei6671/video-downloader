//! Encrypted, point-in-time database snapshots and offline application on startup.
use super::{
    library::LibraryMetadata,
    model::Space,
    provider::{ProviderType, TransferControl},
    repository, schema_migration,
    settings::{SystemVault, Vault},
    spaces::{self, Config},
    store::{ObjectInfo, ObjectStore, Write},
};
use crate::desktop_core::Core;
use age::{secrecy::ExposeSecret, x25519::Identity};
use anyhow::{bail, ensure, Context, Result};
use bech32::{ToBase32, Variant};
use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{self, Read, Write as _},
    path::Path,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use tempfile::NamedTempFile;
use tokio::{io::AsyncReadExt, sync::watch};
use uuid::Uuid;
use zeroize::Zeroizing;

const DEFAULT_INTERVAL_HOURS: u32 = 24;
const SECONDS_PER_HOUR: i64 = 60 * 60;
const RETAIN: usize = 3;
// S3/R2 support an encrypted snapshot of this size in one atomic PUT.
const MAX_BYTES: u64 = 4 * 1024 * 1024 * 1024;
const POLL: Duration = Duration::from_secs(30);

#[derive(Serialize)]
pub struct Status {
    pub enabled: bool,
    pub generated_password: bool,
    pub queued: bool,
    pub interval_hours: u32,
    pub next_attempt_at: Option<i64>,
    pub last_success_at: Option<i64>,
    pub last_message_link: Option<String>,
    pub last_error: Option<String>,
    pub last_canceled: bool,
    pub last_speed_bps: Option<i64>,
    pub progress: Option<Progress>,
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum Phase {
    Preparing,
    Encrypting,
    Uploading,
    Confirming,
    Finishing,
    Canceling,
}

#[derive(Serialize)]
pub struct Progress {
    phase: Phase,
    uploaded_bytes: u64,
    total_bytes: Option<u64>,
    speed_bps: Option<i64>,
}

pub(crate) struct ActiveBackup {
    id: String,
    stop: watch::Sender<bool>,
    phase: Phase,
    bytes: Arc<AtomicU64>,
    upload: Option<(u64, Instant)>,
    sample: Option<(Instant, u64)>,
    speed_bps: Option<i64>,
}

pub(super) struct BackupTransfer {
    pub(super) stop: watch::Sender<bool>,
    progress: Arc<AtomicU64>,
}

impl ActiveBackup {
    fn new(id: &str) -> Self {
        Self {
            id: id.into(),
            stop: watch::channel(false).0,
            phase: Phase::Preparing,
            bytes: Default::default(),
            upload: None,
            sample: None,
            speed_bps: None,
        }
    }
    fn progress(&self) -> Progress {
        let uploaded_bytes = self.bytes.load(Ordering::Relaxed);
        let total_bytes = self.upload.map(|(total, _)| total);
        let phase = if *self.stop.borrow() && !matches!(self.phase, Phase::Finishing) {
            Phase::Canceling
        } else if matches!(self.phase, Phase::Uploading) && total_bytes == Some(uploaded_bytes) {
            Phase::Confirming
        } else {
            self.phase
        };
        Progress {
            phase,
            uploaded_bytes,
            total_bytes,
            speed_bps: self.speed_bps.filter(|_| uploaded_bytes > 0),
        }
    }
    fn average_speed(&self, bytes: u64) -> Option<i64> {
        self.upload.filter(|_| bytes > 0).map(|(_, start)| {
            (bytes as f64 / start.elapsed().as_secs_f64().max(0.001)).round() as i64
        })
    }
    fn sample_speed(&mut self) {
        let Some((previous, previous_bytes)) = self.sample else {
            return;
        };
        let now = Instant::now();
        let elapsed = now.duration_since(previous);
        if elapsed >= Duration::from_secs(1) {
            let bytes = self.bytes.load(Ordering::Relaxed);
            self.speed_bps = Some(
                (bytes.saturating_sub(previous_bytes) as f64 / elapsed.as_secs_f64()).round()
                    as i64,
            );
            self.sample = Some((now, bytes));
        }
    }
}

pub(crate) fn init_schema(db: &Connection) -> Result<()> {
    let current: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info('backup_databases') WHERE name='generated_password')",
        [], |row| row.get(0),
    )?;
    if !current {
        // Reset only pre-release snapshot settings. Video data and cloud objects are independent.
        db.execute_batch("DROP TABLE IF EXISTS backup_databases;")?;
    }
    db.execute_batch(include_str!("database-schema.sql"))?;
    Ok(())
}

#[derive(Serialize)]
pub struct Snapshot {
    pub id: String,
    pub created_at: i64,
    pub bytes: u64,
}

pub fn status(core: &Core, id: &str) -> Result<Status> {
    let active = core.database_backup_active.lock().unwrap();
    let progress = active
        .as_ref()
        .filter(|work| work.id == id)
        .map(ActiveBackup::progress);
    core.backup_db(|db| {
        let mut status = db
            .query_row(
                "SELECT d.enabled,d.last_success_at,d.last_error,d.enabled AND d.next_attempt_at<=unixepoch(),
                        d.interval_hours,NULLIF(d.next_attempt_at,0),t.channel_id,d.last_telegram_message_id,
                        d.last_canceled,d.last_speed_bps,d.generated_password
                 FROM backup_databases d LEFT JOIN telegram_spaces t USING(space_id) WHERE d.space_id=?",
                [id],
                |r| {
                    let channel: Option<String> = r.get(6)?;
                    let message: Option<i32> = r.get(7)?;
                    Ok(Status {
                        enabled: r.get(0)?,
                        generated_password: r.get(10)?,
                        last_success_at: r.get(1)?,
                        last_error: r.get(2)?,
                        queued: r.get(3)?,
                        interval_hours: r.get(4)?,
                        next_attempt_at: r.get(5)?,
                        last_message_link: channel.zip(message).map(|(id, message)| format!("https://t.me/c/{id}/{message}")),
                        last_canceled: r.get(8)?, last_speed_bps: r.get(9)?, progress: None,
                    })
                },
            )
            .optional()?
            .unwrap_or(Status { enabled: false, generated_password: false, queued: false, interval_hours: DEFAULT_INTERVAL_HOURS,
                next_attempt_at: None, last_success_at: None, last_message_link: None, last_error: None,
                last_canceled: false, last_speed_bps: None, progress: None });
        status.progress = progress;
        Ok(status)
    })
}

fn target(core: &Core, id: &str) -> Result<(Space, Config)> {
    let space = spaces::saved_space(core, id)?;
    let config = spaces::config(core, id)?;
    ensure!(
        config.capabilities.is_some(),
        "BACKUP_STORAGE_NOT_VALIDATED"
    );
    Ok((space, config))
}

fn key_ref(core: &Core, id: &str) -> Result<Option<String>> {
    core.backup_db(|db| {
        Ok(db
            .query_row(
                "SELECT key_ref FROM backup_databases WHERE space_id=?",
                [id],
                |r| r.get(0),
            )
            .optional()?)
    })
}

#[derive(Serialize, Deserialize)]
struct StoredSecret<'a> {
    key: &'a str,
    generated_password: Option<&'a str>,
}

fn save_secret(
    vault: &impl Vault,
    reference: &str,
    key: &Identity,
    password: Option<&str>,
) -> Result<()> {
    let key = key.to_string();
    let secret = Zeroizing::new(serde_json::to_string(&StoredSecret {
        key: key.expose_secret(),
        generated_password: password,
    })?);
    vault.write(reference, &secret)
}

fn stored_identity(secret: &str) -> Result<Identity> {
    let stored: StoredSecret<'_> = serde_json::from_str(secret).context("CREDENTIAL_ERROR")?;
    identity(stored.key)
}

pub async fn set_enabled(
    core: &Core,
    id: &str,
    enabled: bool,
    password: Option<Zeroizing<String>>,
    generated: bool,
) -> Result<Status> {
    set_enabled_with(core, id, enabled, password, generated, &SystemVault).await
}

pub(super) async fn set_enabled_with(
    core: &Core,
    id: &str,
    enabled: bool,
    password: Option<Zeroizing<String>>,
    generated: bool,
    vault: &impl Vault,
) -> Result<Status> {
    let _guard = core.backup_config_lock.lock().await;
    let (space, _) = target(core, id)?;
    if enabled {
        let password = password.context("DATABASE_BACKUP_PASSWORD_REQUIRED")?;
        ensure!(
            !status(core, id)?.enabled,
            "DATABASE_BACKUP_ALREADY_ENABLED"
        );
        if generated {
            ensure!(
                password.len() == 24
                    && password
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_')),
                "DATABASE_BACKUP_INVALID_PASSWORD"
            );
        }
        let generated_password = generated.then(|| password.clone());
        let key = password_identity(password, space.library_id).await?;
        let previous_ref = key_ref(core, id)?;
        let same_key = previous_ref
            .as_ref()
            .and_then(|reference| vault.read(reference).ok())
            .and_then(|secret| stored_identity(&secret).ok())
            .is_some_and(|previous| {
                previous.to_public().to_string() == key.to_public().to_string()
            });
        let active = core.database_backup_active.lock().unwrap();
        ensure!(
            !active.as_ref().is_some_and(|work| work.id == id),
            "DATABASE_BACKUP_IN_PROGRESS"
        );
        let reference = format!("database-{}", Uuid::new_v4());
        save_secret(
            vault,
            &reference,
            &key,
            generated_password.as_deref().map(String::as_str),
        )?;
        let saved = core.backup_db(|db| {
            let tx = db.unchecked_transaction()?;
            tx.execute(
                "INSERT INTO backup_databases(space_id,key_ref,enabled,generated_password)
                 VALUES(?1,?2,1,?3)
                 ON CONFLICT(space_id) DO UPDATE SET key_ref=excluded.key_ref,enabled=1,
                    generated_password=excluded.generated_password,last_error=NULL,last_canceled=0,
                    last_success_at=CASE WHEN ?4 THEN last_success_at END,
                    last_telegram_message_id=CASE WHEN ?4 THEN last_telegram_message_id END",
                params![id, reference, generated, same_key],
            )?;
            tx.execute("UPDATE backup_databases SET next_attempt_at=unixepoch()+interval_hours*?2 WHERE space_id=?1", params![id, SECONDS_PER_HOUR])?;
            if let Some(previous) = previous_ref {
                tx.execute("INSERT OR IGNORE INTO backup_credential_cleanup(reference) VALUES(?)", [previous])?;
            }
            tx.commit()?;
            Ok(())
        });
        if saved.is_err() {
            vault.delete(&reference)?;
        }
        saved?;
        drop(active);
        if let Err(error) = super::settings::cleanup_credentials(core, vault) {
            eprintln!("Database backup credential cleanup: {error}");
        }
    } else {
        core.backup_db(|db| {
            db.execute(
                "UPDATE backup_databases SET enabled=0,last_error=NULL WHERE space_id=?",
                [id],
            )?;
            Ok(())
        })?;
    }
    core.database_backup_wake.notify_one();
    status(core, id)
}

pub fn generated_recovery_password(core: &Core, id: &str) -> Result<Zeroizing<String>> {
    generated_recovery_password_with(core, id, &SystemVault)
}

pub(super) fn generated_recovery_password_with(
    core: &Core,
    id: &str,
    vault: &impl Vault,
) -> Result<Zeroizing<String>> {
    target(core, id)?;
    let reference = core
        .backup_db(|db| {
            Ok(db
                .query_row(
                    "SELECT key_ref FROM backup_databases WHERE space_id=? AND generated_password=1",
                    [id],
                    |r| r.get::<_, String>(0),
                )
                .optional()?)
        })?
        .context("DATABASE_BACKUP_PASSWORD_NOT_COPYABLE")?;
    let secret = vault.read(&reference)?;
    let stored: StoredSecret<'_> = serde_json::from_str(&secret).context("CREDENTIAL_ERROR")?;
    Ok(Zeroizing::new(
        stored
            .generated_password
            .context("DATABASE_BACKUP_PASSWORD_NOT_COPYABLE")?
            .into(),
    ))
}

pub fn set_interval(core: &Core, id: &str, hours: u32) -> Result<Status> {
    ensure!(
        matches!(hours, 1 | 3 | 6 | 24),
        "DATABASE_BACKUP_INVALID_INTERVAL"
    );
    target(core, id)?;
    core.backup_db(|db| {
        let changed = db.execute(
            "UPDATE backup_databases SET interval_hours=?2,
             next_attempt_at=CASE WHEN next_attempt_at=0 THEN 0
                                  ELSE next_attempt_at+(?2-interval_hours)*?3 END
             WHERE space_id=?1 AND enabled=1",
            params![id, hours, SECONDS_PER_HOUR],
        )?;
        ensure!(changed == 1, "DATABASE_BACKUP_DISABLED");
        Ok(())
    })?;
    core.database_backup_wake.notify_one();
    status(core, id)
}

pub fn request_backup(core: &Core, id: &str) -> Result<Status> {
    target(core, id)?;
    ensure!(spaces::enabled(core)?, "BACKUP_SYNC_DISABLED");
    let active = core.database_backup_active.lock().unwrap();
    ensure!(
        !active.as_ref().is_some_and(|work| work.id == id),
        "DATABASE_BACKUP_IN_PROGRESS"
    );
    core.backup_db(|db| {
        let changed = db.execute(
            "UPDATE backup_databases SET next_attempt_at=0,last_error=NULL,last_canceled=0
             WHERE space_id=? AND enabled=1 AND next_attempt_at>unixepoch()",
            [id],
        )?;
        if changed == 0 {
            let enabled: bool = db
                .query_row(
                    "SELECT enabled FROM backup_databases WHERE space_id=?",
                    [id],
                    |row| row.get(0),
                )
                .optional()?
                .unwrap_or(false);
            ensure!(enabled, "DATABASE_BACKUP_DISABLED");
            bail!("DATABASE_BACKUP_IN_PROGRESS");
        }
        Ok(())
    })?;
    drop(active);
    core.database_backup_wake.notify_one();
    status(core, id)
}

pub fn cancel_backup(core: &Core, id: &str) -> Result<Status> {
    let active = core.database_backup_active.lock().unwrap();
    if let Some(work) = active.as_ref().filter(|work| work.id == id) {
        if !matches!(work.phase, Phase::Finishing) {
            work.stop.send_replace(true);
        }
    } else {
        core.backup_db(|db| {
            db.execute("UPDATE backup_databases SET last_canceled=1,last_error=NULL,
                        next_attempt_at=unixepoch()+interval_hours*?2 WHERE space_id=?1 AND enabled=1 AND next_attempt_at<=unixepoch()",
                params![id, SECONDS_PER_HOUR])?;
            Ok(())
        })?;
    }
    drop(active);
    status(core, id)
}

pub(crate) async fn run(core: Arc<Core>, mut cancel: watch::Receiver<bool>) {
    while !*cancel.borrow() {
        if let Err(error) = automatic(&core, &mut cancel).await {
            eprintln!("Database backup scheduler: {error}");
        }
        tokio::select! {
            _ = cancel.wait_for(|value| *value) => break,
            _ = core.database_backup_wake.notified() => {},
            _ = tokio::time::sleep(POLL) => {},
        }
    }
}

async fn automatic(core: &Arc<Core>, cancel: &mut watch::Receiver<bool>) -> Result<()> {
    if !spaces::enabled(core)? {
        return Ok(());
    }
    let due = core.backup_db(|db| {
        let mut query = db.prepare(
            "SELECT d.space_id FROM backup_databases d
            JOIN backup_provider_config p USING(space_id)
            WHERE d.enabled=1 AND p.capabilities_json IS NOT NULL
              AND d.next_attempt_at<=unixepoch() ORDER BY d.next_attempt_at",
        )?;
        let ids = query
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(ids)
    })?;
    for id in due {
        let _guard = tokio::select! {
            _ = cancel.wait_for(|value| *value) => return Ok(()),
            guard = core.database_backup_lock.lock() => guard,
        };
        if *cancel.borrow() {
            break;
        }
        let Some(BackupTransfer { stop, progress }) = claim(core, &id)? else {
            continue;
        };
        let mut upload = Box::pin(upload(core, &id, stop.subscribe(), progress));
        let result = loop {
            tokio::select! {
                result = &mut upload => break result,
                _ = tokio::time::sleep(Duration::from_millis(250)), if !*stop.borrow() => {
                    if *cancel.borrow() || !spaces::enabled(core).unwrap_or(false)
                        || !status(core, &id).is_ok_and(|status| status.enabled) {
                        stop.send_replace(true);
                    }
                    if let Some(work) = core.database_backup_active.lock().unwrap().as_mut() {
                        work.sample_speed();
                    }
                }
            }
        };
        finish_attempt(core, &id, result, *stop.borrow() || *cancel.borrow())?;
    }
    Ok(())
}

pub(super) fn claim(core: &Core, id: &str) -> Result<Option<BackupTransfer>> {
    let mut active = core.database_backup_active.lock().unwrap();
    if active.is_some() || !spaces::enabled(core)? {
        return Ok(None);
    }
    let due = core.backup_db(|db| {
        Ok(db.query_row(
            "SELECT EXISTS(SELECT 1 FROM backup_databases WHERE space_id=? AND enabled=1 AND next_attempt_at<=unixepoch())",
            [id], |row| row.get::<_, bool>(0),
        )?)
    })?;
    if !due {
        return Ok(None);
    }
    let backup = ActiveBackup::new(id);
    let transfer = BackupTransfer {
        stop: backup.stop.clone(),
        progress: backup.bytes.clone(),
    };
    *active = Some(backup);
    Ok(Some(transfer))
}

pub(super) fn finish_attempt(
    core: &Core,
    id: &str,
    result: Result<()>,
    canceled: bool,
) -> Result<()> {
    let mut active = core.database_backup_active.lock().unwrap();
    let recorded = match result {
        Err(error) => core.backup_db(|db| {
            let failure = (!canceled).then(|| error.to_string());
            db.execute("UPDATE backup_databases SET last_error=?2,last_canceled=?3,
                        next_attempt_at=unixepoch()+interval_hours*?4 WHERE space_id=?1 AND enabled=1",
                params![id,failure,canceled,SECONDS_PER_HOUR])?;
            Ok(())
        }),
        Ok(()) => Ok(()),
    };
    *active = None;
    recorded
}

#[cfg(test)]
pub(super) fn record_success(db: &Connection, id: &str, message: Option<i32>) -> Result<()> {
    write_success(db, id, message, None)
}

fn write_success(
    db: &Connection,
    id: &str,
    message: Option<i32>,
    speed: Option<i64>,
) -> Result<()> {
    db.execute("UPDATE backup_databases SET last_success_at=unixepoch(),last_error=NULL,last_canceled=0,
                last_telegram_message_id=?2,last_speed_bps=?3,next_attempt_at=unixepoch()+interval_hours*?4 WHERE space_id=?1",
        params![id, message, speed, SECONDS_PER_HOUR])?;
    Ok(())
}

pub(super) fn complete(core: &Core, id: &str, message: Option<i32>, bytes: u64) -> Result<()> {
    let mut active = core.database_backup_active.lock().unwrap();
    let work = active
        .as_mut()
        .filter(|work| work.id == id)
        .context("DATABASE_BACKUP_FAILED")?;
    let speed = work.average_speed(bytes);
    core.backup_db(|db| write_success(db, id, message, speed))?;
    work.phase = Phase::Finishing;
    Ok(())
}

fn record_restored_snapshot(path: &Path, receipt: (i64, Option<i32>)) -> Result<()> {
    let db = Connection::open(path)?;
    db.execute(
        "UPDATE backup_databases SET last_success_at=?1,last_telegram_message_id=?2
                WHERE space_id=(SELECT space_id FROM backup_current_space WHERE id=1)",
        params![receipt.0, receipt.1],
    )?;
    Ok(())
}

async fn upload(
    core: &Arc<Core>,
    id: &str,
    mut cancel: watch::Receiver<bool>,
    progress: Arc<AtomicU64>,
) -> Result<()> {
    let (space, config) = target(core, id)?;
    let reference = key_ref(core, id)?.context("DATABASE_BACKUP_KEY_MISSING")?;
    let key = stored_identity(&SystemVault.read(&reference)?)?;
    let device = core.backup_db(repository::device_id)?;
    let prefix = format!("{}{device}/", prefix(&space, &key));
    let recipient = key.to_public().to_string();
    let path = core.database_path()?;
    let stop = cancel.clone();
    let worker = Arc::downgrade(core);
    let (file, bytes, sha256) = tokio::task::spawn_blocking(move || {
        encrypt_with_phase(
            &path,
            &key,
            || *stop.borrow(),
            || {
                if let Some(core) = worker.upgrade() {
                    if let Some(active) = core.database_backup_active.lock().unwrap().as_mut() {
                        active.phase = Phase::Encrypting;
                    }
                }
            },
        )
        .map_err(|error| match error.to_string().as_str() {
            "CANCELED" | "DATABASE_BACKUP_TOO_LARGE" => error,
            _ => error.context("DATABASE_BACKUP_FAILED"),
        })
    })
    .await
    .context("DATABASE_BACKUP_FAILED")??;
    ensure!(!*cancel.borrow(), "CANCELED");
    {
        let mut active = core.database_backup_active.lock().unwrap();
        let work = active.as_mut().context("DATABASE_BACKUP_FAILED")?;
        work.phase = Phase::Uploading;
        let started = Instant::now();
        work.upload = Some((bytes, started));
        work.sample = Some((started, 0));
    }
    if config.provider_type == ProviderType::Telegram {
        let provider = super::telegram_spaces::telegram_provider(core, &space, config)?;
        let stop = cancel.clone();
        let control = Arc::new(TransferControl {
            check: Box::new(move || {
                ensure!(!*stop.borrow(), "CANCELED");
                Ok(())
            }),
            checkpoint: Box::new(|_| Ok(())),
            progress,
        });
        return super::telegram_database::publish(
            &provider,
            recipient,
            device,
            super::telegram_database::SnapshotFile {
                path: file.path().into(),
                bytes,
                sha256,
            },
            control,
            cancel,
        )
        .await;
    }
    let store = spaces::s3_store(core, &space, &config)?;
    let time = chrono::Utc::now().timestamp();
    let object = format!("{prefix}{time:012}-{}.sqlite.age", Uuid::new_v4());
    tokio::select! {
        biased;
        result = publish(&store, &space, &object, file.path(), bytes, sha256, Some(progress)) => result?,
        _ = cancel.wait_for(|value| *value) => bail!("CANCELED"),
    }
    complete(core, id, None, bytes)?;
    tokio::select! {
        result = prune(&store, &prefix, &device, &object) => {
            if let Err(error) = result { eprintln!("Database snapshot retention: {error}"); }
        },
        _ = cancel.wait_for(|value| *value) => {},
    }
    Ok(())
}

pub(super) async fn publish(
    store: &impl ObjectStore,
    space: &Space,
    object: &str,
    path: &Path,
    bytes: u64,
    sha256: String,
    progress: Option<Arc<AtomicU64>>,
) -> Result<()> {
    verify_library(store, space).await?;
    store
        .write(Write::PutFile {
            key: object.into(),
            path: path.into(),
            bytes,
            sha256: sha256.clone(),
            progress,
        })
        .await?;
    let stored = store
        .head(object)
        .await?
        .context("DATABASE_BACKUP_FAILED")?;
    ensure!(
        stored.bytes == bytes && stored.sha256.as_deref() == Some(&sha256),
        "BACKUP_CHECKSUM_MISMATCH"
    );
    Ok(())
}

pub(super) async fn prune(
    store: &impl ObjectStore,
    prefix: &str,
    device: &str,
    current: &str,
) -> Result<()> {
    let mut entries = list_objects(store, prefix, &format!("{device}/")).await?;
    entries.sort_unstable_by(|a, b| b.key.cmp(&a.key));
    // Retain the upload just verified, even when the local clock moved back.
    for old in entries
        .into_iter()
        .filter(|entry| entry.key != current)
        .skip(RETAIN - 1)
    {
        store.write(Write::Delete { key: old.key }).await?;
    }
    Ok(())
}

fn identity(key: &str) -> Result<Identity> {
    key.trim()
        .parse()
        .map_err(|_| anyhow::anyhow!("CREDENTIAL_ERROR"))
}

fn validate_password(password: &str) -> Result<()> {
    ensure!(
        (12..=128).contains(&password.chars().count()) && !password.trim().is_empty(),
        "DATABASE_BACKUP_INVALID_PASSWORD"
    );
    Ok(())
}

pub(super) async fn password_identity(
    password: Zeroizing<String>,
    library_id: String,
) -> Result<Identity> {
    validate_password(&password)?;
    tokio::task::spawn_blocking(move || {
        // The same remote library ID is available on a new device before decryption.
        // The salt label and scrypt parameters are part of the backup format.
        let salt = format!("yingqu/database-password/v1/{library_id}");
        let params = scrypt::Params::new(17, 8, 1, 32).map_err(|error| {
            anyhow::anyhow!(error.to_string()).context("DATABASE_BACKUP_FAILED")
        })?;
        let mut key = Zeroizing::new([0u8; 32]);
        scrypt::scrypt(password.as_bytes(), salt.as_bytes(), &params, key.as_mut()).map_err(
            |error| anyhow::anyhow!(error.to_string()).context("DATABASE_BACKUP_FAILED"),
        )?;
        let encoded = Zeroizing::new(
            bech32::encode("age-secret-key-", key.to_base32(), Variant::Bech32)
                .context("DATABASE_BACKUP_FAILED")?,
        );
        identity(&encoded)
    })
    .await
    .context("DATABASE_BACKUP_FAILED")?
}

pub(super) fn prefix(space: &Space, key: &Identity) -> String {
    let fingerprint = Sha256::digest(key.to_public().to_string().as_bytes());
    format!("{}database/v1/{fingerprint:x}/", space.prefix)
}

async fn verify_library(store: &impl ObjectStore, space: &Space) -> Result<()> {
    let object = store.get(&format!("{}library.json", space.prefix)).await?;
    let Some(object) = object else {
        ensure!(
            space.library_id
                == super::library::target_id(&[
                    "s3",
                    &space.endpoint,
                    &space.bucket,
                    &space.prefix,
                ]),
            "BACKUP_LIBRARY_NOT_FOUND"
        );
        return Ok(());
    };
    let library: LibraryMetadata =
        serde_json::from_slice(&object.body).context("BACKUP_INVALID_LIBRARY")?;
    library.validate()?;
    ensure!(
        library.library_id == space.library_id,
        "BACKUP_LIBRARY_MISMATCH"
    );
    Ok(())
}

fn snapshot_time(id: &str) -> Option<i64> {
    let (device, filename) = id.split_once('/')?;
    let (time, unique) = filename.strip_suffix(".sqlite.age")?.split_once('-')?;
    if time.len() != 12 || !time.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Uuid::parse_str(device).ok()?;
    Uuid::parse_str(unique).ok()?;
    time.parse().ok()
}

async fn list_objects(
    store: &impl ObjectStore,
    prefix: &str,
    parent: &str,
) -> Result<Vec<ObjectInfo>> {
    let mut objects = Vec::new();
    let mut token = None;
    loop {
        let page = store.list(prefix, token).await?;
        objects.extend(page.objects.into_iter().filter(|obj| {
            obj.key
                .strip_prefix(prefix)
                .is_some_and(|name| snapshot_time(&format!("{parent}{name}")).is_some())
        }));
        token = page.next;
        if token.is_none() {
            return Ok(objects);
        }
    }
}

pub async fn list(core: &Arc<Core>, id: &str, key: Zeroizing<String>) -> Result<Vec<Snapshot>> {
    validate_password(&key)?;
    let (space, config) = target(core, id)?;
    ensure!(
        config.provider_type == ProviderType::S3,
        "DATABASE_BACKUP_INVALID_MESSAGE_LINK"
    );
    let store = spaces::s3_store(core, &space, &config)?;
    verify_library(&store, &space).await?;
    let key = password_identity(key, space.library_id.clone()).await?;
    let prefix = prefix(&space, &key);
    let mut snapshots = list_objects(&store, &prefix, "")
        .await?
        .into_iter()
        .filter_map(|obj| {
            let id = obj.key.strip_prefix(&prefix)?;
            Some(Snapshot {
                id: id.into(),
                created_at: snapshot_time(id)?,
                bytes: obj.bytes,
            })
        })
        .collect::<Vec<_>>();
    snapshots.sort_unstable_by_key(|snapshot| std::cmp::Reverse(snapshot.created_at));
    Ok(snapshots)
}

pub async fn inspect_telegram_snapshot(
    core: &Arc<Core>,
    binding: spaces::TelegramConfig,
    link: &str,
    key: Zeroizing<String>,
) -> Result<Snapshot> {
    validate_password(&key)?;
    Ok(
        super::telegram_database::selected(core, binding, key, link, None, MAX_BYTES)
            .await?
            .snapshot,
    )
}

#[cfg(test)]
pub(super) fn encrypt(
    path: &Path,
    key: &Identity,
    stopped: impl Fn() -> bool,
) -> Result<(NamedTempFile, u64, String)> {
    encrypt_with_phase(path, key, stopped, || {})
}

fn encrypt_with_phase(
    path: &Path,
    key: &Identity,
    stopped: impl Fn() -> bool,
    encrypting: impl FnOnce(),
) -> Result<(NamedTempFile, u64, String)> {
    let directory = path.parent().context("DATABASE_BACKUP_FAILED")?;
    let plain = NamedTempFile::new_in(directory)?.into_temp_path();
    {
        let source = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let read = source.unchecked_transaction()?;
        // Pin committed WAL pages while SQLite makes a consistent copy.
        read.query_row("SELECT count(*) FROM sqlite_schema", [], |row| {
            row.get::<_, i64>(0)
        })?;
        let mut destination = Connection::open(&plain)?;
        let backup = rusqlite::backup::Backup::new(&read, &mut destination)?;
        loop {
            ensure!(!stopped(), "CANCELED");
            match backup.step(256)? {
                rusqlite::backup::StepResult::Done => break,
                rusqlite::backup::StepResult::More => {}
                _ => bail!("DATABASE_BACKUP_FAILED"),
            }
        }
    }
    encrypting();
    let mut input = fs::File::open(&plain)?;
    ensure!(
        input.metadata()?.len() < MAX_BYTES,
        "DATABASE_BACKUP_TOO_LARGE"
    );
    let mut encrypted = NamedTempFile::new_in(directory)?;
    let recipient = key.to_public();
    let encryptor =
        age::Encryptor::with_recipients(std::iter::once(&recipient as &dyn age::Recipient))?;
    let mut writer = encryptor.wrap_output(encrypted.as_file_mut())?;
    let mut buffer = vec![0; 64 * 1024];
    loop {
        ensure!(!stopped(), "CANCELED");
        let bytes = input.read(&mut buffer)?;
        if bytes == 0 {
            break;
        }
        writer.write_all(&buffer[..bytes])?;
    }
    writer.finish()?;
    let bytes = encrypted.as_file().metadata()?.len();
    ensure!(bytes <= MAX_BYTES, "DATABASE_BACKUP_TOO_LARGE");
    let (sha256, _) = repository::fingerprint_with_cancel(encrypted.path(), stopped)?;
    Ok((encrypted, bytes, sha256))
}

pub async fn stage_restore(
    core: &Arc<Core>,
    id: &str,
    snapshot: &str,
    key: Zeroizing<String>,
    telegram_binding: Option<spaces::TelegramConfig>,
) -> Result<()> {
    validate_password(&key)?;
    let mut cancel = core.backup_cancellation();
    let _guard = core.database_backup_lock.lock().await;
    ensure!(!*cancel.borrow(), "CANCELED");
    let session = telegram_binding.as_ref().map(|_| {
        core.telegram_accounts
            .binding(crate::telegram_accounts::Purpose::Backup)
            .namespace
    });
    let path = core.database_path()?;
    let directory = path.parent().context("DATABASE_BACKUP_FAILED")?;
    ensure!(!pending(&path).try_exists()?, "DATABASE_RESTORE_PENDING");
    let temporary = tempfile::tempdir_in(directory)?;
    let file = temporary.path().join("snapshot.age");
    let (space, config, telegram, receipt, key) = tokio::select! {
        _ = cancel.wait_for(|value| *value) => bail!("CANCELED"),
        result = async {
            if let Some(binding) = telegram_binding {
                let selected = super::telegram_database::selected(core, binding, key, snapshot, Some(&file), MAX_BYTES).await?;
                Ok((selected.space, selected.config, Some(selected.binding), (selected.snapshot.created_at, Some(selected.message_id)), selected.key))
            } else {
                let (space, config) = target(core, id)?;
                ensure!(config.provider_type == ProviderType::S3, "DATABASE_BACKUP_INVALID_MESSAGE_LINK");
                let created_at = snapshot_time(snapshot).context("DATABASE_BACKUP_INVALID_SNAPSHOT")?;
                let store = spaces::s3_store(core, &space, &config)?;
                verify_library(&store, &space).await?;
                let key = password_identity(key, space.library_id.clone()).await?;
                let object = format!("{}{snapshot}", prefix(&space, &key));
                download(&store, &object, &file).await?;
                Ok::<_, anyhow::Error>((space, config, None, (created_at, None), key))
            }
        } => result?,
    };
    let local_device = core.backup_db(repository::device_id)?;
    let local_directory = core.settings().download_directory;
    let snapshot_space = space.clone();
    let reference = format!("database-{}", Uuid::new_v4());
    let key_ref = reference.clone();
    let secret = key.clone();
    let prepared_telegram = telegram.zip(session.clone());
    let directory = directory.to_owned();
    let staged = tokio::task::spawn_blocking(move || {
        let _encrypted_directory = temporary;
        let temporary = NamedTempFile::new_in(directory)?.into_temp_path();
        decrypt(&file, &temporary, &key)?;
        prepare_restore(
            &temporary,
            &snapshot_space,
            &config,
            &local_device,
            &local_directory,
            &key_ref,
            prepared_telegram
                .as_ref()
                .map(|(binding, session)| (binding, session.as_str())),
        )
        .map_err(|error| match error.downcast_ref::<io::Error>() {
            Some(_) => error.context("DATABASE_RESTORE_FAILED"),
            None => error.context("DATABASE_BACKUP_INVALID_SNAPSHOT"),
        })?;
        record_restored_snapshot(&temporary, receipt)?;
        Ok::<_, anyhow::Error>(temporary)
    })
    .await
    .context("DATABASE_BACKUP_FAILED")??;
    let _configuration = core.backup_config_lock.lock().await;
    ensure!(!*cancel.borrow(), "CANCELED");
    if let Some(session) = session {
        ensure!(
            core.telegram_accounts
                .binding(crate::telegram_accounts::Purpose::Backup)
                .namespace
                == session,
            "BACKUP_SPACE_CHANGED"
        );
    } else {
        let current = spaces::saved_space(core, id)?;
        ensure!(
            current.library_id == space.library_id
                && current.credential_ref == space.credential_ref,
            "BACKUP_SPACE_CHANGED"
        );
    }
    let destination = pending(&path);
    ensure!(!destination.try_exists()?, "DATABASE_RESTORE_PENDING");
    save_secret(&SystemVault, &reference, &secret, None)?;
    if let Err(error) = staged.persist_noclobber(destination) {
        SystemVault.delete(&reference)?;
        return Err(error.error).context("DATABASE_RESTORE_FAILED");
    }
    Ok(())
}

pub(super) async fn download(store: &impl ObjectStore, object: &str, path: &Path) -> Result<()> {
    let stream = store.stream(object, 0, None).await?;
    ensure!(
        stream.info.bytes > 0 && stream.info.bytes <= MAX_BYTES,
        "DATABASE_BACKUP_TOO_LARGE"
    );
    let expected = stream.info.bytes;
    let mut input = stream.reader.take(expected + 1);
    let mut file = tokio::fs::File::create(path).await?;
    let copied = tokio::time::timeout(
        Duration::from_secs(30 * 60),
        tokio::io::copy(&mut input, &mut file),
    )
    .await
    .context("BACKUP_NETWORK_ERROR")??;
    ensure!(copied == expected, "BACKUP_CHECKSUM_MISMATCH");
    file.sync_all().await?;
    Ok(())
}

pub(super) fn decrypt(source: &Path, output: &Path, key: &Identity) -> Result<()> {
    let decryptor = age::Decryptor::new_buffered(io::BufReader::new(fs::File::open(source)?))
        .context("DATABASE_BACKUP_INVALID_SNAPSHOT")?;
    let mut reader = decryptor
        .decrypt(std::iter::once(key as &dyn age::Identity))
        .context("DATABASE_BACKUP_DECRYPT_FAILED")?;
    let mut output = fs::File::create(output)?;
    io::copy(&mut reader, &mut output).context("DATABASE_BACKUP_DECRYPT_FAILED")?;
    output.sync_all()?;
    Ok(())
}

pub(super) fn prepare_restore(
    path: &Path,
    space: &Space,
    config: &Config,
    device: &str,
    directory: &Path,
    key_ref: &str,
    telegram: Option<(&spaces::TelegramConfig, &str)>,
) -> Result<()> {
    let db = Connection::open(path)?;
    db.pragma_update(None, "foreign_keys", true)?;
    db.pragma_update(None, "trusted_schema", false)?;
    db.pragma_update(None, "journal_mode", "DELETE")?;
    let integrity: String = db.query_row("PRAGMA integrity_check(1)", [], |row| row.get(0))?;
    ensure!(integrity == "ok", "DATABASE_RESTORE_FAILED");
    schema_migration::prepare(&db)?;
    let tx = db.unchecked_transaction()?;
    tx.execute_batch(include_str!("telegram-schema.sql"))?;
    init_schema(&tx)?;
    let restored_id: String = if let Some((binding, _)) = telegram {
        tx.query_row(
            "SELECT s.space_id FROM backup_spaces s JOIN telegram_spaces t USING(space_id)
             WHERE s.library_id=?1 AND t.account_id=?2 AND t.channel_id=?3",
            params![space.library_id, binding.account_id, binding.channel_id],
            |row| row.get(0),
        )?
    } else {
        tx.query_row(
        "SELECT s.space_id FROM backup_spaces s JOIN backup_provider_config p USING(space_id)
         WHERE s.library_id=?1 AND s.endpoint=?2 AND s.bucket=?3 AND s.prefix=?4 AND p.provider_type='s3'",
        params![space.library_id, space.endpoint, space.bucket, space.prefix],
        |row| row.get(0),
    )?
    };
    tx.execute_batch(include_str!("database-restore.sql"))?;
    if let Some((_, session)) = telegram {
        tx.execute(
            "INSERT INTO telegram_account_bindings VALUES('backup',?)",
            [session],
        )?;
    }
    tx.execute("UPDATE backup_device SET device_id=? WHERE id=1", [device])?;
    tx.execute(
        "UPDATE app_settings SET directory=? WHERE id=1",
        [directory.to_string_lossy()],
    )?;
    tx.execute(
        "UPDATE backup_spaces SET credential_ref=?2,enabled=1 WHERE space_id=?1",
        params![restored_id, space.credential_ref],
    )?;
    tx.execute(
        "UPDATE backup_provider_config SET profile=?2,region=?3,path_style=?4,capabilities_json=?5 WHERE space_id=?1",
        params![restored_id,config.profile,config.region,config.path_style,serde_json::to_string(&config.capabilities)?],
    )?;
    tx.execute(
        "DELETE FROM backup_removed_spaces WHERE space_id=?",
        [&restored_id],
    )?;
    tx.execute(
        "INSERT INTO backup_current_space VALUES(1,?)",
        [&restored_id],
    )?;
    tx.execute("INSERT INTO backup_sync_preference VALUES(1,1)", [])?;
    tx.execute(
        "INSERT INTO backup_databases(space_id,key_ref,enabled) VALUES(?,?,0)",
        params![restored_id, key_ref],
    )?;
    tx.execute(
        "INSERT OR REPLACE INTO backup_database_restore VALUES(1,?,?,1)",
        params![key_ref, restored_id],
    )?;
    ensure!(
        tx.prepare("PRAGMA foreign_key_check")?
            .query([])?
            .next()?
            .is_none(),
        "DATABASE_RESTORE_FAILED"
    );
    tx.commit()?;
    fs::OpenOptions::new().write(true).open(path)?.sync_all()?;
    Ok(())
}

fn restore_receipt(db: &Connection) -> Result<Option<String>> {
    let exists: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name='backup_database_restore')",
        [],
        |r| r.get(0),
    )?;
    if exists {
        return Ok(db
            .query_row(
                "SELECT key_ref FROM backup_database_restore WHERE id=1",
                [],
                |r| r.get(0),
            )
            .optional()?);
    }
    Ok(None)
}

pub(super) fn discovery_paused(core: &Core, space: &str) -> Result<bool> {
    core.backup_db(|db| {
        Ok(db
            .query_row(
                "SELECT discovery_paused FROM backup_database_restore WHERE space_id=?",
                [space],
                |r| r.get(0),
            )
            .optional()?
            .unwrap_or(false))
    })
}

pub(super) fn resume_discovery(db: &Connection, space: &str) -> Result<()> {
    db.execute(
        "UPDATE backup_database_restore SET discovery_paused=0 WHERE space_id=?",
        [space],
    )?;
    Ok(())
}

pub(super) fn pending(path: &Path) -> std::path::PathBuf {
    path.with_extension("restore-pending.sqlite")
}

pub(crate) fn apply_pending(path: &Path) -> Result<()> {
    let staged = pending(path);
    if !staged.try_exists()? {
        return Ok(());
    }
    let receipt = restore_receipt(&Connection::open_with_flags(
        &staged,
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?)?
    .context("DATABASE_RESTORE_FAILED")?;
    let mut db = Connection::open(path).context("DATABASE_ERROR")?;
    if restore_receipt(&db)?.as_deref() != Some(&receipt) {
        schema_migration::snapshot(&db, "before-database-restore")?;
        db.restore(
            rusqlite::MAIN_DB,
            &staged,
            None::<fn(rusqlite::backup::Progress)>,
        )
        .context("DATABASE_RESTORE_FAILED")?;
    }
    if let Err(error) = fs::remove_file(staged) {
        eprintln!(
            "Unable to remove applied database restore: {}",
            error.kind()
        );
    }
    Ok(())
}
