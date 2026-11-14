//! Current portable S3 library metadata. No protocol conversion or migration.
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub(super) fn target_id(parts: &[&str]) -> String {
    let mut hash = Sha256::new();
    for part in parts {
        hash.update((part.len() as u64).to_be_bytes());
        hash.update(part.as_bytes());
    }
    let mut bytes = [0; 16];
    bytes.copy_from_slice(&hash.finalize()[..16]);
    uuid::Builder::from_custom_bytes(bytes)
        .into_uuid()
        .to_string()
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LibraryMetadata {
    #[serde(default)]
    pub deletion_indexes: bool,
    pub schema_version: u32,
    pub library_id: String,
    pub mode: String,
    pub created_at: String,
}
impl LibraryMetadata {
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != 2 {
            bail!("BACKUP_UNSUPPORTED_SCHEMA");
        }
        uuid::Uuid::parse_str(&self.library_id).context("BACKUP_INVALID_LIBRARY")?;
        if self.mode != "multi_writer" {
            bail!("BACKUP_INVALID_LIBRARY");
        }
        Ok(())
    }
}
