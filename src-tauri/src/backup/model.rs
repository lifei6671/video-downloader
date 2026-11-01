//! Persisted backup identities, independent of download history.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Space {
    pub space_id: String,
    pub library_id: String,
    pub endpoint: String,
    pub bucket: String,
    pub prefix: String,
    pub credential_ref: Option<String>,
    pub enabled: bool,
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
    // Unverified local transcripts are never eligible for remote publication or hash reuse.
    pub content_sha256: Option<String>,
    pub text: String,
    pub model_revision: String,
    pub model_name: Option<String>,
    pub pipeline_revision: String,
    pub completed_at: u64,
}
