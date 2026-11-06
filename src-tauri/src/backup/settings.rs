//! Backup credentials and rotation cleanup.
use super::repository;
use crate::desktop_core::Core;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
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
