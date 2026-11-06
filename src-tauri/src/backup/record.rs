//! Portable source identity and original-object references used by the current S3 format.
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ObjectRef {
    pub key: String,
    pub bytes: u64,
    pub sha256: String,
    pub container: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Source {
    pub platform: String,
    pub video_id: String,
    pub canonical_url: Option<String>,
}
