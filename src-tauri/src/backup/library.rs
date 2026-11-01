//! Current portable S3 library metadata. No protocol conversion or migration.
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LibraryMetadata {
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
