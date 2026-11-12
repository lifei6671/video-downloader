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

pub(super) fn publication_field(metadata: &Value) -> Option<(&'static str, Value)> {
    if let Some(at) = metadata["published_at"]
        .as_i64()
        .filter(|&at| at > 0 && chrono::DateTime::from_timestamp(at, 0).is_some())
    {
        return Some(("published_at", at.into()));
    }
    let date = metadata["published_date"].as_str()?;
    let parsed = chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").ok()?;
    (date.len() == 10 && parsed.format("%Y-%m-%d").to_string() == date)
        .then(|| ("published_date", date.into()))
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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
