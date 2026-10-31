//! Saved provider spaces, capability gates and explicit selection; credentials stay in the vault.
use super::{
    model::{Role, Space},
    provider::*,
    repository,
    s3_client::S3Store,
    s3_provider::{self, S3Provider},
    settings::{self, Credentials, SystemVault, Vault},
    store::{ObjectStore, Write},
    library::LibraryMetadata,
};
use crate::desktop_core::Core;
use anyhow::{bail, Context, Result};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use zeroize::Zeroizing;

fn default_retention_days() -> u32 {
    30
}
fn default_region() -> String {
    "auto".into()
}
fn default_profile() -> String {
    "r2".into()
}
fn yes() -> bool {
    true
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Update {
    #[serde(default)]
    pub display_name: Option<String>,
    pub provider_type: ProviderType,
    pub profile: String,
    pub endpoint: String,
    pub bucket: String,
    pub prefix: String,
    pub region: String,
    pub path_style: bool,
    pub enabled: bool,
    /// Existing clients retain save-and-activate; the settings editor saves independently.
    #[serde(default = "yes")]
    pub activate: bool,
    pub credentials: Option<Credentials>,
    #[serde(default)]
    pub cleanup_deleted_records: bool,
    #[serde(default = "default_retention_days")]
    pub deleted_record_retention_days: u32,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Config {
    pub provider_type: ProviderType,
    #[serde(default = "default_profile")]
    pub profile: String,
    #[serde(default = "default_region")]
    pub region: String,
    #[serde(default = "yes")]
    pub path_style: bool,
    pub protocol: u32,
    pub capabilities: Option<ProviderCapabilities>,
}
#[derive(Clone, Debug, Serialize)]
pub struct View {
    pub display_name: String,
    pub space_id: String,
    pub library_id: String,
    pub provider_type: ProviderType,
    pub profile: String,
    pub endpoint: String,
    pub bucket: String,
    pub prefix: String,
    pub region: String,
    pub path_style: bool,
    pub enabled: bool,
    pub credentials_configured: bool,
    #[serde(skip_serializing)]
    pub protocol: u32,
    pub last_error: Option<String>,
    pub connection_pending: bool,
    pub cleanup_deleted_records: bool,
    pub deleted_record_retention_days: u32,
}
pub fn config(core: &Core, space: &str) -> Result<Config> {
    core.backup_db(|db|{
        let (provider,profile,region,path_style,protocol,caps):(String,String,String,bool,u32,Option<String>)=db.query_row("SELECT provider_type,profile,region,path_style,protocol,capabilities_json FROM backup_provider_config WHERE space_id=?",[space],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?)))?;
        Ok(Config{provider_type:serde_json::from_value(provider.into())?,profile,region,path_style,protocol,capabilities:caps.map(|v|serde_json::from_str(&v)).transpose()?})
    })
}
pub fn list(core: &Core) -> Result<Vec<View>> {
    let removed: Vec<String> = core.backup_db(|db| {
        let mut q = db.prepare("SELECT space_id FROM backup_removed_spaces")?;
        let rows = q.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    })?;
    core.backup_db(repository::spaces)?
        .into_iter()
        .filter(|s| !removed.contains(&s.space_id))
        .map(|s| {
            let c = config(core, &s.space_id)?;
            let (last_error, display_name) = core.backup_db(|db| {
                let last_error = db
                    .query_row(
                        "SELECT last_error FROM backup_sync_checkpoints WHERE space_id=?",
                        [&s.space_id],
                        |r| r.get(0),
                    )
                    .optional()?
                    .flatten();
                let display_name = db.query_row("SELECT display_name FROM backup_space_names WHERE space_id=?", [&s.space_id], |r| r.get(0)).optional()?.unwrap_or_default();
                Ok((last_error, display_name))
            })?;
            let (cleanup_deleted_records,deleted_record_retention_days,cleanup_error)=retention(core,&s.space_id)?;
            Ok(View {
                display_name,
                space_id: s.space_id,
                library_id: s.library_id,
                provider_type: c.provider_type,
                profile: c.profile,
                endpoint: s.endpoint,
                bucket: s.bucket,
                prefix: s.prefix,
                region: c.region,
                path_style: c.path_style,
                enabled: s.enabled,
                credentials_configured: s.credential_ref.is_some(),
                protocol: c.protocol,
                connection_pending: c.protocol != 2 || c.capabilities.is_none(),
                last_error: last_error.or(cleanup_error),
                cleanup_deleted_records,
                deleted_record_retention_days,
            })
        })
        .collect()
}
pub fn retention(core: &Core, id: &str) -> Result<(bool, u32, Option<String>)> {
    core.backup_db(|db| {
        Ok(db
            .query_row(
                "SELECT enabled,days,last_error FROM backup_record_retention WHERE space_id=?",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?
            .unwrap_or((false, 30, None)))
    })
}
fn write_retention(
    db: &rusqlite::Connection,
    id: &str,
    update: &Update,
    verified: bool,
) -> Result<()> {
    db.execute("INSERT INTO backup_record_retention(space_id,enabled,days,delete_verified) VALUES(?1,?2,?3,?4) ON CONFLICT(space_id) DO UPDATE SET enabled=excluded.enabled,days=excluded.days,next_check_at=CASE WHEN enabled<>excluded.enabled OR days<>excluded.days THEN 0 ELSE next_check_at END,last_error=NULL,delete_verified=MAX(backup_record_retention.delete_verified,excluded.delete_verified)",params![id,update.cleanup_deleted_records,update.deleted_record_retention_days,verified && update.cleanup_deleted_records])?;
    Ok(())
}

pub fn current_id(core: &Core) -> Result<Option<String>> {
    core.backup_db(|db| {
        Ok(db
            .query_row(
                "SELECT space_id FROM backup_current_space WHERE id=1",
                [],
                |r| r.get(0),
            )
            .optional()?)
    })
}
fn normalized_name(name: &str) -> Result<String> {
    let name = name.trim();
    if name.chars().count() > 64 || name.chars().any(char::is_control) { bail!("BACKUP_INVALID_SPACE_NAME"); }
    Ok(name.into())
}
fn write_name(db: &rusqlite::Connection, id: &str, name: &str) -> Result<()> {
    db.execute("INSERT INTO backup_space_names(space_id,display_name) VALUES(?1,?2) ON CONFLICT(space_id) DO UPDATE SET display_name=excluded.display_name", params![id,name])?;
    Ok(())
}
/// Local display metadata only: no vault, provider, queue or remote calls.
pub fn rename(core: &Core, id: &str, name: &str) -> Result<View> {
    let name = normalized_name(name)?;
    saved_space(core, id)?;
    core.backup_db(|db| write_name(db, id, &name))?;
    list(core)?.into_iter().find(|s| s.space_id == id).context("BACKUP_SPACE_NOT_FOUND")
}
pub fn get(core: &Core) -> Result<Option<View>> {
    let current = current_id(core)?;
    Ok(list(core)?
        .into_iter()
        .find(|s| Some(&s.space_id) == current.as_ref()))
}
fn saved_space(core: &Core, id: &str) -> Result<Space> {
    if removed(core, id)? { bail!("BACKUP_SPACE_NOT_FOUND"); }
    core.backup_db(repository::spaces)?
        .into_iter()
        .find(|s| s.space_id == id)
        .context("BACKUP_SPACE_NOT_FOUND")
}

fn removed(core: &Core, id: &str) -> Result<bool> {
    core.backup_db(|db| {
        Ok(db.query_row(
            "SELECT EXISTS(SELECT 1 FROM backup_removed_spaces WHERE space_id=?)",
            [id],
            |r| r.get(0),
        )?)
    })
}
/// Before the first explicit toggle, preserve the existing active-space setting.
pub fn enabled(core: &Core) -> Result<bool> {
    core.backup_db(enabled_in)
}
fn enabled_in(db: &rusqlite::Connection) -> Result<bool> {
    Ok(db.query_row("SELECT COALESCE((SELECT enabled FROM backup_sync_preference WHERE id=1),(SELECT s.enabled FROM backup_current_space c JOIN backup_spaces s USING(space_id) WHERE c.id=1),0)", [], |r| r.get(0))?)
}
/// Desired state is a local transaction; network preparation belongs to the Rust worker.
pub async fn set_enabled(core: &Arc<Core>, enabled: bool) -> Result<bool> {
    core.backup_db(|db| {
        let tx = db.unchecked_transaction()?;
        let current: Option<(String,String,bool)> = tx.query_row("SELECT s.space_id,p.provider_type,p.protocol=2 AND p.capabilities_json IS NOT NULL FROM backup_current_space c JOIN backup_spaces s USING(space_id) JOIN backup_provider_config p USING(space_id) WHERE c.id=1", [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
        if enabled && current.as_ref().is_some_and(|(_,provider,_)|provider!="s3") { bail!("BACKUP_PROVIDER_NOT_IMPLEMENTED"); }
        tx.execute("INSERT INTO backup_sync_preference VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET enabled=excluded.enabled", [enabled])?;
        if let Some((id,_,ready))=current {
            select_in_transaction(&tx,&id,enabled && ready)?;
            if enabled { tx.execute("UPDATE backup_sync_checkpoints SET last_error=NULL WHERE space_id=?",[id])?; }
        }
        tx.commit()?; Ok(())
    })?;
    core.wake_backup();
    Ok(enabled)
}
pub async fn prepare_current(core: &Arc<Core>) -> Result<()> {
    prepare_current_with(core, &SystemVault, |s, c, region, path_style| {
        S3Store::configured(
            s,
            &c.access_key_id,
            &c.secret_access_key,
            region,
            path_style,
        )
    })
    .await
}
/// Uses the existing queue worker; stopping backup drops this future and its network requests.
pub async fn prepare_current_with<S: ObjectStore>(
    core: &Arc<Core>,
    vault: &impl Vault,
    make: impl FnOnce(&Space, &Credentials, &str, bool) -> Result<S>,
) -> Result<()> {
    let Some(id) = current_id(core)? else {
        return Ok(());
    };
    if !enabled(core)? {
        return Ok(());
    }
    let mut shutdown = core.backup_cancellation();
    let work = async {
        let _guard = core.backup_config_lock.lock().await;
        if !enabled(core)? || current_id(core)?.as_deref() != Some(&id) {
            bail!("CANCELED");
        }
        let cfg = config(core, &id)?;
        if cfg.protocol == 2 && cfg.capabilities.is_some() {
            core.backup_db(|db| select_local(db, &id, true))?;
            return Ok(());
        }
        let mut update = saved_update(core, &id, true)?;
        update.activate = false;
        connect_locked(core, update, vault, make, true)
            .await?
            .context("BACKUP_SPACE_NOT_FOUND")?;
        Ok(())
    };
    tokio::pin!(work);
    let deadline = tokio::time::sleep(std::time::Duration::from_secs(120));
    tokio::pin!(deadline);
    let result = loop {
        tokio::select! {
            result=&mut work=>break result,
            _=&mut deadline=>break Err(anyhow::anyhow!("BACKUP_TIMEOUT")),
            _=shutdown.changed()=>break Err(anyhow::anyhow!("CANCELED")),
            _=tokio::time::sleep(std::time::Duration::from_millis(100))=>{
                if !enabled(core)? || current_id(core)?.as_deref()!=Some(&id) {break Err(anyhow::anyhow!("CANCELED"));}
            }
        }
    };
    if let Err(error) = &result {
        if error.to_string() != "CANCELED"
            && enabled(core)?
            && current_id(core)?.as_deref() == Some(&id)
        {
            core.backup_db(|db|{db.execute("INSERT INTO backup_sync_checkpoints(space_id,last_error) VALUES(?1,?2) ON CONFLICT(space_id) DO UPDATE SET last_error=excluded.last_error",params![id,error.to_string()])?;Ok(())})?;
        }
    }
    result
}

fn saved_update(core: &Core, id: &str, enabled: bool) -> Result<Update> {
    let value = list(core)?
        .into_iter()
        .find(|s| s.space_id == id)
        .context("BACKUP_SPACE_NOT_FOUND")?;
    Ok(Update {
        display_name: None,
        provider_type: value.provider_type,
        profile: value.profile,
        endpoint: value.endpoint,
        bucket: value.bucket,
        prefix: value.prefix,
        region: value.region,
        path_style: value.path_style,
        enabled,
        activate: true,
        cleanup_deleted_records: value.cleanup_deleted_records,
        deleted_record_retention_days: value.deleted_record_retention_days,
        credentials: None,
    })
}

#[derive(Serialize)]
pub struct Usage {
    pub assets: i64,
    pub operations: i64,
}
pub fn usage(core: &Core, id: &str) -> Result<Usage> {
    saved_space(core, id)?;
    core.backup_db(|db| usage_in(db, id))
}
fn usage_in(db: &rusqlite::Connection, id: &str) -> Result<Usage> {
    let assets = db.query_row("SELECT COUNT(*) FROM (SELECT local_asset_id FROM backup_space_assets WHERE space_id=?1 UNION SELECT asset_id FROM asset_remote_objects WHERE space_id=?1)", [id], |r| r.get(0))?;
    let operations = db.query_row("SELECT COUNT(*) FROM backup_operations WHERE space_id=? AND state NOT IN ('completed','canceled')", [id], |r| r.get(0))?;
    Ok(Usage { assets, operations })
}
/// Forget configuration locally, retaining media, pending edits and scoped resume evidence.
pub async fn remove(core: &Arc<Core>, id: &str, confirmed: bool) -> Result<()> {
    remove_with(core, id, confirmed, &SystemVault).await
}
pub async fn remove_with(
    core: &Arc<Core>,
    id: &str,
    confirmed: bool,
    vault: &impl Vault,
) -> Result<()> {
    let _guard = tokio::time::timeout(std::time::Duration::from_secs(30), core.backup_config_lock.lock()).await.context("BACKUP_BUSY")?;
    let space = saved_space(core, id)?;
    let sync_enabled = enabled(core)?;
    core.backup_db(|db| {
        let tx = db.unchecked_transaction()?;
        let refs = usage_in(&tx, id)?;
        if !confirmed && (refs.assets > 0 || refs.operations > 0) { bail!("BACKUP_SPACE_REFERENCED"); }
        tx.execute("INSERT OR IGNORE INTO backup_sync_preference VALUES(1,?)", [sync_enabled])?;
        tx.execute("INSERT OR IGNORE INTO backup_removed_spaces VALUES(?)", [id])?;
        tx.execute("UPDATE backup_spaces SET enabled=0,credential_ref=NULL WHERE space_id=?", [id])?;
        tx.execute("DELETE FROM backup_current_space WHERE space_id=?", [id])?;
        tx.execute("UPDATE backup_operations SET state='blocked',last_error='BACKUP_SPACE_INACTIVE' WHERE space_id=? AND state IN ('pending','running','retry_wait')", [id])?;
        if let Some(reference) = &space.credential_ref {
            tx.execute("INSERT OR IGNORE INTO backup_credential_cleanup VALUES(?)", [reference])?;
        }
        tx.commit()?; Ok(())
    })?;
    core.wake_backup();
    settings::cleanup_credentials(core, vault)
}

pub fn provider(core: &Core, id: &str) -> Result<Arc<dyn BackupProvider>> {
    let space = saved_space(core, id)?;
    let cfg = config(core, id)?;
    cfg.provider_type.require_implemented()?;
    if cfg.protocol != 2 {
        bail!("BACKUP_STORAGE_NOT_VALIDATED");
    }
    if !space.enabled || current_id(core)?.as_deref() != Some(id) {
        bail!("BACKUP_SPACE_INACTIVE");
    }
    let credentials = settings::credentials(core, id, &SystemVault)?;
    let store = S3Store::configured(
        &space,
        &credentials.access_key_id,
        &credentials.secret_access_key,
        &cfg.region,
        cfg.path_style,
    )?;
    let caps = cfg.capabilities.context("BACKUP_CAPABILITIES_UNVERIFIED")?;
    caps.require_multi_writer()?;
    Ok(Arc::new(S3Provider::new(
        store,
        space,
        caps,
        core.backup_db(repository::device_id)?,
    )))
}

/// Freeze old work without changing IDs, payloads, credentials or asset memberships.
pub fn select_local(db: &rusqlite::Connection, id: &str, enabled: bool) -> Result<()> {
    let tx = db.unchecked_transaction()?;
    select_in_transaction(&tx, id, enabled)?;
    tx.commit()?;
    Ok(())
}
fn select_in_transaction(tx: &rusqlite::Connection, id: &str, enabled: bool) -> Result<()> {
    let exists: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM backup_spaces WHERE space_id=?)",
        [id],
        |r| r.get(0),
    )?;
    if !exists {
        bail!("BACKUP_SPACE_NOT_FOUND");
    }
    tx.execute("UPDATE backup_operations SET state='blocked',last_error='BACKUP_SPACE_INACTIVE' WHERE (space_id<>?1 OR NOT ?2) AND state IN ('pending','running','retry_wait')",params![id,enabled])?;
    tx.execute(
        "UPDATE backup_spaces SET enabled=CASE WHEN space_id=?1 THEN ?2 ELSE 0 END",
        params![id, enabled],
    )?;
    tx.execute("INSERT INTO backup_current_space VALUES(1,?) ON CONFLICT(id) DO UPDATE SET space_id=excluded.space_id",[id])?;
    if enabled {
        tx.execute("UPDATE backup_operations SET state='pending',last_error=NULL,next_retry_at=NULL WHERE space_id=? AND last_error='BACKUP_SPACE_INACTIVE' AND kind<>'restore'",[id])?;
    }
    Ok(())
}
pub async fn select(core: &Arc<Core>, id: &str) -> Result<View> {
    select_with(core, id, &SystemVault, |s, c, region, path_style| {
        S3Store::configured(
            s,
            &c.access_key_id,
            &c.secret_access_key,
            region,
            path_style,
        )
    })
    .await
}
pub async fn select_with<S: ObjectStore>(
    core: &Arc<Core>,
    id: &str,
    vault: &impl Vault,
    make: impl FnOnce(&Space, &Credentials, &str, bool) -> Result<S>,
) -> Result<View> {
    let _guard = tokio::time::timeout(std::time::Duration::from_secs(30), core.backup_config_lock.lock()).await.context("BACKUP_BUSY")?;
    if !enabled(core)? {
        bail!("BACKUP_SYNC_DISABLED");
    }
    let mut shutdown = core.backup_cancellation();
    let mut space = saved_space(core, id)?;
    let cfg = config(core, id)?;
    cfg.provider_type.require_implemented()?;
    if cfg.protocol != 2 || cfg.capabilities.is_none() {
        return connect_locked(core, saved_update(core, id, true)?, vault, make, true)
            .await?
            .context("BACKUP_SPACE_NOT_FOUND");
    }
    let credentials = settings::credentials(core, id, vault)?;
    let store = make(&space, &credentials, &cfg.region, cfg.path_style)?;
    space.enabled = true;
    let provider = S3Provider::new(
        store,
        space,
        cfg.capabilities.context("BACKUP_CAPABILITIES_UNVERIFIED")?,
        core.backup_db(repository::device_id)?,
    );
    provider.capabilities().require_multi_writer()?;
    tokio::select! {_=shutdown.changed()=>bail!("CANCELED"),result=provider.open_library(false)=>{result?;}}
    core.backup_db(|db| select_local(db, id, true))?;
    core.wake_backup();
    get(core)?.context("BACKUP_SPACE_NOT_FOUND")
}


fn normalize(update: &mut Update) -> Result<()> {
    if !(1..=3650).contains(&update.deleted_record_retention_days) {bail!("BACKUP_INVALID_RETENTION_DAYS");}
    update.display_name = update.display_name.as_deref().map(normalized_name).transpose()?;
    update.provider_type.require_implemented()?;
    if !matches!(update.profile.as_str(), "r2" | "s3") {
        bail!("BACKUP_PROVIDER_NOT_IMPLEMENTED");
    }
    update.endpoint = update.endpoint.trim().trim_end_matches('/').into();
    update.bucket = update.bucket.trim().into();
    update.prefix = format!("{}/", update.prefix.trim().trim_end_matches('/'));
    if update.profile == "r2" {
        super::s3_client::validate_target(&update.endpoint, &update.bucket, &update.prefix)?;
    } else {
        super::s3_client::validate_s3_target(&update.endpoint, &update.bucket, &update.prefix)?;
    }
    if update.region.is_empty()
        || update.region.len() > 128
        || !update
            .region
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        bail!("BACKUP_INVALID_REGION");
    }
    Ok(())
}

pub async fn save(core: &Arc<Core>, update: Update) -> Result<View> {
    save_with(core, update, &SystemVault, |s, c, region, path_style| {
        S3Store::configured(
            s,
            &c.access_key_id,
            &c.secret_access_key,
            region,
            path_style,
        )
    })
    .await
}
pub async fn test_draft(core: &Arc<Core>, update: Update) -> Result<()> {
    test_with(core, update, &SystemVault, |s, c, region, path_style| {
        S3Store::configured(
            s,
            &c.access_key_id,
            &c.secret_access_key,
            region,
            path_style,
        )
    })
    .await
}
pub async fn save_with<S: ObjectStore>(
    core: &Arc<Core>,
    update: Update,
    vault: &impl Vault,
    make: impl FnOnce(&Space, &Credentials, &str, bool) -> Result<S>,
) -> Result<View> {
    connect(core, update, vault, make, true)
        .await?
        .context("BACKUP_SPACE_NOT_FOUND")
}
pub async fn test_with<S: ObjectStore>(
    core: &Arc<Core>,
    update: Update,
    vault: &impl Vault,
    make: impl FnOnce(&Space, &Credentials, &str, bool) -> Result<S>,
) -> Result<()> {
    connect(core, update, vault, make, false).await?;
    Ok(())
}
async fn connect<S: ObjectStore>(
    core: &Arc<Core>,
    update: Update,
    vault: &impl Vault,
    make: impl FnOnce(&Space, &Credentials, &str, bool) -> Result<S>,
    save: bool,
) -> Result<Option<View>> {
    let _guard = tokio::time::timeout(std::time::Duration::from_secs(30), core.backup_config_lock.lock()).await.context("BACKUP_BUSY")?;
    connect_locked(core, update, vault, make, save).await
}
async fn connect_locked<S: ObjectStore>(
    core: &Arc<Core>,
    mut update: Update,
    vault: &impl Vault,
    make: impl FnOnce(&Space, &Credentials, &str, bool) -> Result<S>,
    save: bool,
) -> Result<Option<View>> {
    let mut shutdown = core.backup_cancellation();
    normalize(&mut update)?;
    let old = core.backup_db(repository::spaces)?.into_iter().find(|s| {
        s.endpoint == update.endpoint && s.bucket == update.bucket && s.prefix == update.prefix
    });
    if save
        && !update.enabled
        && update.credentials.is_none()
        && (update.activate || !enabled(core)?)
    {
        if let Some(old) = &old {
            let cfg = config(core, &old.space_id)?;
            if cfg.profile == update.profile
                && cfg.region == update.region
                && cfg.path_style == update.path_style
            {
                if update.activate || current_id(core)?.as_deref() == Some(&old.space_id) {
                    core.backup_db(|db| {
                        let tx = db.unchecked_transaction()?;
                        select_in_transaction(&tx, &old.space_id, false)?;
                        if update.activate { tx.execute("INSERT INTO backup_sync_preference VALUES(1,0) ON CONFLICT(id) DO UPDATE SET enabled=0", [])?; }
                        tx.commit()?; Ok(())
                    })?;
                    core.wake_backup();
                }
                core.backup_db(|db|write_retention(db,&old.space_id,&update,false))?;
                if let Some(name) = &update.display_name {
                    core.backup_db(|db| write_name(db, &old.space_id, name))?;
                }
                return Ok(list(core)?.into_iter().find(|s| s.space_id == old.space_id));
            }
        }
    }
    let credentials = match update.credentials.take() {
        Some(c) => c,
        None => settings::credentials(
            core,
            &old.as_ref().context("BACKUP_CREDENTIALS_MISSING")?.space_id,
            vault,
        )?,
    };
    let device = core.backup_db(repository::device_id)?;
    let mut space = Space {
        space_id: old
            .as_ref()
            .map(|s| s.space_id.clone())
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
        library_id: old
            .as_ref()
            .map(|s| s.library_id.clone())
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
        endpoint: update.endpoint.clone(),
        bucket: update.bucket.clone(),
        prefix: update.prefix.clone(),
        role: Role::Member,
        writer_device_id: device.clone(),
        writer_epoch: 1,
        credential_ref: None,
        enabled: true,
    };
    let store = make(&space, &credentials, &update.region, update.path_style)?;
    let library_key = format!("{}library.json", space.prefix);
    let existing = tokio::select! {_=shutdown.changed()=>bail!("CANCELED"),result=store.get(&library_key)=>result?};
    let mut initialize = false;
    let mut initialization_id = None;
    if let Some(object) = &existing {
        let raw: serde_json::Value =
            serde_json::from_slice(&object.body).context("BACKUP_INVALID_LIBRARY")?;
        let library_id = if raw["schema_version"] == 1 {
            if let Some(old) = &old {
                if config(core, &old.space_id)?.protocol == 2 {
                    bail!("BACKUP_INVALID_LIBRARY");
                }
            }
            let library: super::model::Library =
                serde_json::from_value(raw).context("BACKUP_INVALID_LIBRARY")?;
            library.validate()?;
            initialize = true;
            library.library_id
        } else {
            let library: LibraryMetadata =
                serde_json::from_value(raw).context("BACKUP_INVALID_LIBRARY")?;
            if library.schema_version == 2 && library.mode == "migrating" {
                initialize = true;
                initialization_id = Some(
                    library
                        .migration
                        .as_ref()
                        .context("BACKUP_INVALID_LIBRARY")?
                        .operation_id
                        .clone(),
                );
            } else {
                library.validate()?;
            }
            library.library_id
        };
        uuid::Uuid::parse_str(&library_id).context("BACKUP_INVALID_LIBRARY")?;
        if old.as_ref().is_some_and(|s| s.library_id != library_id) {
            bail!("BACKUP_LIBRARY_MISMATCH");
        }
        space.library_id = library_id;
    }
    let reference = uuid::Uuid::new_v4().to_string();
    vault.write(
        &reference,
        &Zeroizing::new(serde_json::to_string(&credentials)?),
    )?;
    let operation = uuid::Uuid::new_v4().to_string();
    core.backup_active_probes
        .lock()
        .expect("backup probe registry poisoned")
        .insert(operation.clone());
    let _active = ActiveProbe {
        core,
        operation: &operation,
    };
    let target = ProbeTarget {
        space: space.clone(),
        region: update.region.clone(),
        path_style: update.path_style,
    };
    let journal = core.backup_db(|db| {
        db.execute(
            "INSERT INTO backup_probe_cleanup VALUES(?1,?2,?3,NULL)",
            params![operation, serde_json::to_string(&target)?, reference],
        )?;
        Ok(())
    });
    if let Err(error) = journal {
        vault.delete(&reference)?;
        return Err(error);
    }
    let probe = tokio::select! {_=shutdown.changed()=>Err(anyhow::anyhow!("CANCELED")),result=probe_with_retention_timeout(&store,&space.prefix,&operation,std::time::Duration::from_secs(30),update.cleanup_deleted_records)=>result};
    let capabilities = match probe {
        Ok(c) => c,
        Err(error) => {
            if !matches!(
                error.to_string().as_str(),
                "BACKUP_PROBE_CLEANUP_FAILED" | "CANCELED" | "BACKUP_TIMEOUT"
            ) {
                finish_probe(core, &operation, &reference, vault, false)?;
            } else {
                core.backup_db(|db| {
                    db.execute(
                        "UPDATE backup_probe_cleanup SET last_error=?2 WHERE operation_id=?1",
                        params![operation, error.to_string()],
                    )?;
                    Ok(())
                })?;
            }
            return Err(error);
        }
    };
    if !save {
        finish_probe(core, &operation, &reference, vault, false)?;
        return Ok(None);
    }
    let local_unprepared = old
        .as_ref()
        .map(|s| config(core, &s.space_id))
        .transpose()?
        .is_some_and(|c| c.protocol != 2);
    if existing.is_some() && (initialize || local_unprepared) {
        let id = initialization_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        if let Some(old) = &old {
            let sync_enabled = enabled(core)?;
            core.backup_db(|db| { let tx = db.unchecked_transaction()?;
                // Stop scoped work while connecting development data, preserving every resume token.
                tx.execute("INSERT OR IGNORE INTO backup_sync_preference VALUES(1,?)", [sync_enabled])?;
                tx.execute("UPDATE backup_spaces SET enabled=0 WHERE space_id=?", [&old.space_id])?;
                tx.execute("UPDATE backup_operations SET state='blocked',last_error='BACKUP_SPACE_INACTIVE' WHERE space_id=? AND state IN ('pending','running','retry_wait')", [&old.space_id])?;
                tx.commit()?; Ok(())
            })?;
        }
        let prepared = tokio::select! {_=shutdown.changed()=>Err(anyhow::anyhow!("CANCELED")), result=super::library::initialize(&store,&space.prefix,&space.library_id,&id)=>result};
        if let Err(error) = prepared {
            finish_probe(core, &operation, &reference, vault, false)?;
            return Err(error);
        }
        if let Some(old) = &old {
            if let Err(error) = super::library::reconcile_local(core, &store, old).await {
                finish_probe(core, &operation, &reference, vault, false)?;
                return Err(error);
            }
        }
    }
    let provider = S3Provider::new(store, space.clone(), capabilities.clone(), device);
    let opened = tokio::select! {_=shutdown.changed()=>Err(anyhow::anyhow!("CANCELED")),result=provider.open_library(existing.is_none())=>result};
    if let Err(error) = opened {
        finish_probe(core, &operation, &reference, vault, false)?;
        return Err(error);
    }
    space.credential_ref = Some(reference.clone());
    let result=core.backup_db(|db|{
        let tx=db.unchecked_transaction()?;
        let current:Option<String>=tx.query_row("SELECT space_id FROM backup_current_space WHERE id=1",[],|r|r.get(0)).optional()?;
        let select_saved=update.activate || current.as_deref()==Some(&space.space_id);
        let desired=if update.activate {update.enabled} else {enabled_in(&tx)?};
        space.enabled=select_saved && desired;
        repository::save_space(&tx,&space)?;
        write_retention(&tx,&space.space_id,&update,true)?;
        tx.execute("DELETE FROM backup_removed_spaces WHERE space_id=?",[&space.space_id])?;
        if update.activate {tx.execute("INSERT INTO backup_sync_preference VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET enabled=excluded.enabled",[update.enabled])?;}
        if let Some(name)=&update.display_name {write_name(&tx,&space.space_id,name)?;}
        tx.execute("UPDATE backup_provider_config SET provider_type='s3',profile=?2,region=?3,path_style=?4,capabilities_json=?5,protocol=2,migration_json=NULL WHERE space_id=?1",params![space.space_id,update.profile,update.region,update.path_style,serde_json::to_string(&capabilities)?])?;
        tx.execute("DELETE FROM backup_probe_cleanup WHERE operation_id=?",[&operation])?;
        tx.execute("UPDATE backup_sync_checkpoints SET last_error=NULL WHERE space_id=?",[&space.space_id])?;
        if let Some(old)=old.as_ref().and_then(|s|s.credential_ref.as_ref()) {tx.execute("INSERT OR IGNORE INTO backup_credential_cleanup VALUES(?)",[old])?;}
        if select_saved {select_in_transaction(&tx,&space.space_id,space.enabled)?;}
        tx.commit()?;Ok(())
    });
    if let Err(error) = result {
        finish_probe(core, &operation, &reference, vault, false)?;
        return Err(error);
    }
    settings::cleanup_credentials(core, vault)?;
    core.wake_backup();
    Ok(list(core)?
        .into_iter()
        .find(|s| s.space_id == space.space_id))
}


pub(super) async fn verify_record_cleanup(core: &Arc<Core>, id: &str) -> Result<()> {
    let mut update = saved_update(core, id, true)?;
    update.activate = false;
    update.cleanup_deleted_records = true;
    test_draft(core, update).await
}
async fn probe_with_retention_timeout<S: ObjectStore>(
    store: &S,
    prefix: &str,
    operation: &str,
    timeout: std::time::Duration,
    cleanup: bool,
) -> Result<ProviderCapabilities> {
    if !cleanup {
        return probe_with_timeout(store, prefix, operation, timeout).await;
    }
    tokio::time::timeout(
        timeout,
        s3_provider::probe_with_cleanup(store, prefix, operation, true),
    )
    .await
    .context("BACKUP_TIMEOUT")?
}

pub(crate) async fn probe_with_timeout<S: ObjectStore>(
    store: &S,
    prefix: &str,
    operation: &str,
    timeout: std::time::Duration,
) -> Result<ProviderCapabilities> {
    tokio::time::timeout(timeout, s3_provider::probe(store, prefix, operation))
        .await
        .context("BACKUP_TIMEOUT")?
}

fn finish_probe(
    core: &Core,
    operation: &str,
    reference: &str,
    vault: &impl Vault,
    keep: bool,
) -> Result<()> {
    core.backup_db(|db| {
        let tx = db.unchecked_transaction()?;
        tx.execute(
            "DELETE FROM backup_probe_cleanup WHERE operation_id=?",
            [operation],
        )?;
        if !keep {
            tx.execute(
                "INSERT OR IGNORE INTO backup_credential_cleanup VALUES(?)",
                [reference],
            )?;
        }
        tx.commit()?;
        Ok(())
    })?;
    settings::cleanup_credentials(core, vault)
}
#[derive(Serialize, Deserialize)]
struct ProbeTarget {
    space: Space,
    region: String,
    path_style: bool,
}
struct ActiveProbe<'a> {
    core: &'a Core,
    operation: &'a str,
}
impl Drop for ActiveProbe<'_> {
    fn drop(&mut self) {
        self.core
            .backup_active_probes
            .lock()
            .expect("backup probe registry poisoned")
            .remove(self.operation);
    }
}
pub async fn cleanup_probes(core: &Core) -> Result<()> {
    cleanup_probes_with(
        core,
        &SystemVault,
        |space, credentials, region, path_style| {
            S3Store::configured(
                space,
                &credentials.access_key_id,
                &credentials.secret_access_key,
                region,
                path_style,
            )
        },
    )
    .await
}
pub async fn cleanup_probes_with<S: ObjectStore>(
    core: &Core,
    vault: &impl Vault,
    make: impl Fn(&Space, &Credentials, &str, bool) -> Result<S>,
) -> Result<()> {
    let rows: Vec<(String, String, String)> = core.backup_db(|db| {
        let mut q =
            db.prepare("SELECT operation_id,target_json,credential_ref FROM backup_probe_cleanup")?;
        let rows = q.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    })?;
    for (operation, raw, reference) in rows {
        if core
            .backup_active_probes
            .lock()
            .expect("backup probe registry poisoned")
            .contains(&operation)
        {
            continue;
        }
        let result = async {
            let target: ProbeTarget = serde_json::from_str(&raw)?;
            let credentials: Credentials = serde_json::from_str(&vault.read(&reference)?)?;
            let store = make(
                &target.space,
                &credentials,
                &target.region,
                target.path_style,
            )?;
            for name in ["conditional", "multipart"] {
                let key = format!("{}probes/{operation}/{name}", target.space.prefix);
                for upload_id in store.uploads(&key).await? {
                    store
                        .write(Write::AbortMultipart {
                            key: key.clone(),
                            upload_id,
                        })
                        .await?;
                }
                store.write(Write::Delete { key: key.clone() }).await?;
                if store.head(&key).await?.is_some() {
                    bail!("BACKUP_CLEANUP_PENDING");
                }
            }
            finish_probe(core, &operation, &reference, vault, false)?;
            Ok::<_, anyhow::Error>(())
        }
        .await;
        if let Err(error) = result {
            core.backup_db(|db| {
                db.execute(
                    "UPDATE backup_probe_cleanup SET last_error=?2 WHERE operation_id=?1",
                    params![operation, error.to_string()],
                )?;
                Ok(())
            })?;
        }
    }
    Ok(())
}
