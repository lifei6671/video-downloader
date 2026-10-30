//! Persisted backup identities, independent of download history.
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Primary,
    Reader,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Library {
    pub schema_version: u32,
    pub library_id: String,
    pub mode: String,
    pub writer_device_id: String,
    pub writer_epoch: u64,
    pub created_at: String,
}

impl Library {
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != 1 || self.mode != "single_writer" {
            bail!("BACKUP_UNSUPPORTED_SCHEMA");
        }
        uuid::Uuid::parse_str(&self.library_id)?;
        uuid::Uuid::parse_str(&self.writer_device_id)?;
        if self.writer_epoch == 0 || self.writer_epoch > i64::MAX as u64 {
            bail!("BACKUP_INVALID_LIBRARY");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Space {
    pub space_id: String,
    pub library_id: String,
    pub endpoint: String,
    pub bucket: String,
    pub prefix: String,
    pub role: Role,
    pub writer_device_id: String,
    pub writer_epoch: u64,
    pub credential_ref: Option<String>,
    pub enabled: bool,
}

impl Space {
    pub fn authorize_write(&self, device_id: &str, library: &Library) -> Result<()> {
        library.validate()?;
        if !self.enabled || self.role != Role::Primary {
            bail!("BACKUP_READ_ONLY");
        }
        if self.library_id != library.library_id
            || device_id != library.writer_device_id
            || self.writer_device_id != library.writer_device_id
            || self.writer_epoch != library.writer_epoch
        {
            bail!("BACKUP_WRITER_CHANGED");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Asset {
    pub asset_id: String,
    pub task_id: Option<String>,
    pub metadata: Value,
    pub original_filename: String,
    pub content_sha256: Option<String>,
    pub content_bytes: u64,
    pub asset_state: String,
    pub path: Option<PathBuf>,
    pub local_state: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AssetTranscript {
    pub asset_id: String,
    // Legacy results remain readable before their source has been fingerprinted.
    // An absent digest is never eligible for remote publication or hash reuse.
    pub content_sha256: Option<String>,
    pub text: String,
    pub model_revision: String,
    pub model_name: Option<String>,
    pub pipeline_revision: String,
    pub completed_at: u64,
}
