//! Backup configuration and credential rotation with a redacted frontend view.
use super::{
    model::{Library, Role, Space},
    repository,
    s3_client::{validate_target, S3Store},
    store::{Client, ObjectStore},
};
use crate::desktop_core::Core;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use zeroize::{Zeroize, Zeroizing};

#[derive(Deserialize, Serialize)]
pub struct Credentials {
    pub access_key_id: String,
    pub secret_access_key: String,
}
impl Drop for Credentials {
    fn drop(&mut self) {
        self.access_key_id.zeroize();
        self.secret_access_key.zeroize();
    }
}

#[derive(Deserialize)]
pub struct Update {
    pub endpoint: String,
    pub bucket: String,
    pub prefix: String,
    pub role: Role,
    pub enabled: bool,
    pub credentials: Option<Credentials>,
}

#[derive(Serialize)]
pub struct View {
    pub space_id: String,
    pub library_id: String,
    pub endpoint: String,
    pub bucket: String,
    pub prefix: String,
    pub role: Role,
    pub enabled: bool,
    pub credentials_configured: bool,
}
impl From<Space> for View {
    fn from(s: Space) -> Self {
        Self {
            space_id: s.space_id,
            library_id: s.library_id,
            endpoint: s.endpoint,
            bucket: s.bucket,
            prefix: s.prefix,
            role: s.role,
            enabled: s.enabled,
            credentials_configured: s.credential_ref.is_some(),
        }
    }
}

pub trait Vault {
    fn read(&self, reference: &str) -> Result<Zeroizing<String>>;
    fn write(&self, reference: &str, value: &str) -> Result<()>;
    fn delete(&self, reference: &str) -> Result<()>;
}
pub struct SystemVault;
impl Vault for SystemVault {
    fn read(&self, r: &str) -> Result<Zeroizing<String>> {
        Ok(Zeroizing::new(
            keyring::Entry::new("com.yingqu.desktop.backup", r)
                .context("CREDENTIAL_ERROR")?
                .get_password()
                .map_err(|_| anyhow::anyhow!("CREDENTIAL_ERROR"))?,
        ))
    }
    fn write(&self, r: &str, v: &str) -> Result<()> {
        keyring::Entry::new("com.yingqu.desktop.backup", r)
            .context("CREDENTIAL_ERROR")?
            .set_password(v)
            .map_err(|_| anyhow::anyhow!("CREDENTIAL_ERROR"))
    }
    fn delete(&self, r: &str) -> Result<()> {
        match keyring::Entry::new("com.yingqu.desktop.backup", r)
            .context("CREDENTIAL_ERROR")?
            .delete_credential()
        {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => bail!("CREDENTIAL_ERROR"),
        }
    }
}

pub fn get(core: &Core) -> Result<Option<View>> {
    let current: Option<String> = core.backup_db(|db| {
        use rusqlite::OptionalExtension;
        Ok(db
            .query_row(
                "SELECT space_id FROM backup_current_space WHERE id=1",
                [],
                |r| r.get(0),
            )
            .optional()?)
    })?;
    Ok(core
        .backup_db(repository::spaces)?
        .into_iter()
        .find(|s| Some(&s.space_id) == current.as_ref())
        .map(Into::into))
}

pub fn client(core: &Core, space_id: &str) -> Result<Client<S3Store>> {
    let spaces = core.backup_db(repository::spaces)?;
    let space = spaces
        .into_iter()
        .find(|s| s.space_id == space_id)
        .context("BACKUP_SPACE_NOT_FOUND")?;
    if !space.enabled {
        bail!("BACKUP_SPACE_INACTIVE");
    }
    let raw = SystemVault.read(
        space
            .credential_ref
            .as_deref()
            .context("BACKUP_CREDENTIALS_MISSING")?,
    )?;
    let credentials: Credentials = serde_json::from_str(&raw).context("CREDENTIAL_ERROR")?;
    let store = S3Store::new(
        &space,
        &credentials.access_key_id,
        &credentials.secret_access_key,
    )?;
    Ok(Client::new(
        store,
        space,
        core.backup_db(repository::device_id)?,
    ))
}

/// Only the settings editor requests plaintext; normal settings views stay redacted.
pub fn credentials(core: &Core, space_id: &str, vault: &impl Vault) -> Result<Credentials> {
    let space = core
        .backup_db(repository::spaces)?
        .into_iter()
        .find(|space| space.space_id == space_id)
        .context("BACKUP_SPACE_NOT_FOUND")?;
    let reference = space
        .credential_ref
        .as_deref()
        .context("BACKUP_CREDENTIALS_MISSING")?;
    serde_json::from_str(&vault.read(reference)?).context("CREDENTIAL_ERROR")
}

pub async fn test_draft(core: &Core, update: Update) -> Result<()> {
    test_draft_with(core, update, &SystemVault, |space, credentials| {
        S3Store::new(
            space,
            &credentials.access_key_id,
            &credentials.secret_access_key,
        )
    })
    .await
}

/// Check the draft's read access and library identity without saving or creating a library.
/// Write permissions are exercised by the saved-space probe and actual backup operations.
pub async fn test_draft_with<S: ObjectStore>(
    core: &Core,
    mut update: Update,
    vault: &impl Vault,
    make_store: impl FnOnce(&Space, &Credentials) -> Result<S>,
) -> Result<()> {
    update.endpoint = update.endpoint.trim_end_matches('/').to_owned();
    update.prefix = format!("{}/", update.prefix.trim_end_matches('/'));
    validate_target(&update.endpoint, &update.bucket, &update.prefix)?;
    let old = core
        .backup_db(repository::spaces)?
        .into_iter()
        .find(|space| {
            space.endpoint == update.endpoint
                && space.bucket == update.bucket
                && space.prefix == update.prefix
        });
    let credentials = match update.credentials.take() {
        Some(credentials) => credentials,
        None => credentials(
            core,
            &old.as_ref().context("BACKUP_CREDENTIALS_MISSING")?.space_id,
            vault,
        )?,
    };
    if credentials.access_key_id.is_empty() || credentials.secret_access_key.is_empty() {
        bail!("BACKUP_CREDENTIALS_MISSING");
    }
    let device = core.backup_db(repository::device_id)?;
    let mut space = Space {
        space_id: old
            .as_ref()
            .map(|space| space.space_id.clone())
            .unwrap_or_else(|| Uuid::new_v4().to_string()),
        library_id: old
            .as_ref()
            .map(|space| space.library_id.clone())
            .unwrap_or_else(|| Uuid::new_v4().to_string()),
        endpoint: update.endpoint,
        bucket: update.bucket,
        prefix: update.prefix,
        role: update.role,
        writer_device_id: device.clone(),
        writer_epoch: 1,
        credential_ref: None,
        enabled: true,
    };
    let store = make_store(&space, &credentials)?;
    if let Some(object) = store.get(&format!("{}library.json", space.prefix)).await? {
        let library: Library =
            serde_json::from_slice(&object.body).context("BACKUP_INVALID_LIBRARY")?;
        library.validate()?;
        if old
            .as_ref()
            .is_some_and(|space| space.library_id != library.library_id)
        {
            bail!("BACKUP_LIBRARY_MISMATCH");
        }
        space.library_id = library.library_id.clone();
        space.writer_device_id = library.writer_device_id.clone();
        space.writer_epoch = library.writer_epoch;
        if space.role == Role::Primary {
            space.authorize_write(&device, &library)?;
        }
    } else if space.role == Role::Reader {
        bail!("BACKUP_LIBRARY_NOT_FOUND");
    }
    store.list(&space.prefix, None).await?;
    Ok(())
}

pub async fn save(core: &Core, update: Update) -> Result<View> {
    save_with(core, update, &SystemVault, |s, c| {
        S3Store::new(s, &c.access_key_id, &c.secret_access_key)
    })
    .await
}

pub async fn save_with<S: ObjectStore>(
    core: &Core,
    mut update: Update,
    vault: &impl Vault,
    make_store: impl FnOnce(&Space, &Credentials) -> Result<S>,
) -> Result<View> {
    let _guard = core.backup_config_lock.lock().await;
    update.endpoint = update.endpoint.trim_end_matches('/').to_owned();
    update.prefix = format!("{}/", update.prefix.trim_end_matches('/'));
    validate_target(&update.endpoint, &update.bucket, &update.prefix)?;
    let device = core.backup_db(repository::device_id)?;
    let spaces = core.backup_db(repository::spaces)?;
    let old = spaces.into_iter().find(|s| {
        s.endpoint == update.endpoint && s.bucket == update.bucket && s.prefix == update.prefix
    });
    if !update.enabled && update.credentials.is_none() {
        if let Some(mut space) = old.clone() {
            if space.role == update.role {
                space.enabled = false;
                core.backup_db(|db| repository::save_space(db, &space))?;
                return Ok(space.into());
            }
        }
    }
    let credentials = match update.credentials.take() {
        Some(v) => v,
        None => {
            let reference = old
                .as_ref()
                .and_then(|s| s.credential_ref.as_deref())
                .context("BACKUP_CREDENTIALS_MISSING")?;
            serde_json::from_str(&vault.read(reference)?).context("CREDENTIAL_ERROR")?
        }
    };
    if credentials.access_key_id.is_empty() || credentials.secret_access_key.is_empty() {
        bail!("BACKUP_CREDENTIALS_MISSING");
    }
    let mut space = Space {
        space_id: old
            .as_ref()
            .map(|s| s.space_id.clone())
            .unwrap_or_else(|| Uuid::new_v4().to_string()),
        library_id: old
            .as_ref()
            .map(|s| s.library_id.clone())
            .unwrap_or_else(|| Uuid::new_v4().to_string()),
        endpoint: update.endpoint,
        bucket: update.bucket,
        prefix: update.prefix,
        role: update.role,
        writer_device_id: device.clone(),
        writer_epoch: 1,
        credential_ref: None,
        enabled: true,
    };
    let store = make_store(&space, &credentials)?;
    let key = format!("{}library.json", space.prefix);
    let library = match store.get(&key).await? {
        Some(object) => {
            let library: Library =
                serde_json::from_slice(&object.body).context("BACKUP_INVALID_LIBRARY")?;
            library.validate()?;
            if old
                .as_ref()
                .is_some_and(|s| s.library_id != library.library_id)
            {
                bail!("BACKUP_LIBRARY_MISMATCH");
            }
            space.library_id = library.library_id.clone();
            space.writer_device_id = library.writer_device_id.clone();
            space.writer_epoch = library.writer_epoch;
            if space.role == Role::Primary {
                space.authorize_write(&device, &library)?;
            }
            library
        }
        None => {
            if space.role == Role::Reader {
                bail!("BACKUP_LIBRARY_NOT_FOUND");
            }
            Library {
                schema_version: 1,
                library_id: space.library_id.clone(),
                mode: "single_writer".into(),
                writer_device_id: device.clone(),
                writer_epoch: 1,
                created_at: chrono::Utc::now().to_rfc3339(),
            }
        }
    };
    let client = Client::new(store, space.clone(), device);
    if space.role == Role::Primary {
        client.create(library).await?;
    } else {
        client.connect().await?;
    }
    let reference = Uuid::new_v4().to_string();
    let raw = Zeroizing::new(serde_json::to_string(&credentials)?);
    vault.write(&reference, &raw)?;
    space.credential_ref = Some(reference.clone());
    space.enabled = update.enabled;
    let saved=core.backup_db(|db|{
        let tx=db.unchecked_transaction()?;
        repository::save_space(&tx,&space)?;
        tx.execute("INSERT INTO backup_current_space VALUES(1,?) ON CONFLICT(id) DO UPDATE SET space_id=excluded.space_id",[&space.space_id])?;
        tx.execute("UPDATE backup_spaces SET enabled=0 WHERE space_id<>?",[&space.space_id])?;
        tx.execute("UPDATE backup_operations SET state='blocked',last_error='BACKUP_SPACE_INACTIVE' WHERE space_id<>? AND state NOT IN ('completed','canceled')",[&space.space_id])?;
        if let Some(reference)=old.as_ref().and_then(|s|s.credential_ref.as_ref()) {
            tx.execute("INSERT OR IGNORE INTO backup_credential_cleanup VALUES(?)",[reference])?;
        }
        tx.commit()?;
        Ok(())
    });
    if let Err(error) = saved {
        vault
            .delete(&reference)
            .context("BACKUP_CREDENTIAL_ROLLBACK_FAILED")?;
        return Err(error);
    }
    cleanup_credentials(core, vault)?;
    Ok(space.into())
}

/// Failed old-reference removal remains durable and can be retried without losing new settings.
pub fn cleanup_credentials(core: &Core, vault: &impl Vault) -> Result<()> {
    let references:Vec<String>=core.backup_db(|db|{
        let mut q=db.prepare("SELECT reference FROM backup_credential_cleanup WHERE reference NOT IN (SELECT credential_ref FROM backup_spaces WHERE credential_ref IS NOT NULL)")?;
        let rows=q.query_map([],|r|r.get(0))?;Ok(rows.collect::<rusqlite::Result<_>>()?)
    })?;
    for reference in references {
        if vault.delete(&reference).is_ok() {
            core.backup_db(|db| {
                db.execute(
                    "DELETE FROM backup_credential_cleanup WHERE reference=?",
                    [&reference],
                )?;
                Ok(())
            })?;
        }
    }
    Ok(())
}
