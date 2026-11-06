//! Injectable S3 operations used by the current backup provider.
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::{future::Future, pin::Pin};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ObjectInfo {
    pub key: String,
    pub etag: String,
    pub bytes: u64,
    pub sha256: Option<String>,
}

pub struct ObjectStream {
    pub info: ObjectInfo,
    pub content_range: Option<String>,
    pub reader: Box<dyn tokio::io::AsyncRead + Send + Unpin>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Part {
    pub number: u32,
    pub etag: String,
    pub bytes: u64,
}
#[derive(Clone, Debug)]
pub struct UploadSession {
    pub key: String,
    pub upload_id: String,
}

pub struct Page {
    pub objects: Vec<ObjectInfo>,
    pub next: Option<String>,
}

pub type StoreFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

#[derive(Clone, Debug)]
pub struct Object {
    pub body: Vec<u8>,
    pub etag: String,
}

#[derive(Clone, Debug)]
pub enum Condition {
    Absent,
    Match(String),
}

#[derive(Clone, Debug)]
pub enum Write {
    PutFile {
        key: String,
        path: PathBuf,
        bytes: u64,
        sha256: String,
        progress: Option<std::sync::Arc<std::sync::atomic::AtomicU64>>,
    },
    UploadFilePart {
        key: String,
        upload_id: String,
        part: u32,
        path: PathBuf,
        offset: u64,
        bytes: u64,
        progress: Option<std::sync::Arc<std::sync::atomic::AtomicU64>>,
    },
    Put {
        key: String,
        body: Vec<u8>,
        condition: Condition,
    },
    Delete {
        key: String,
    },
    DeleteIfMatch {
        key: String,
        etag: String,
    },
    CreateMultipart {
        key: String,
        sha256: String,
    },
    UploadPart {
        key: String,
        upload_id: String,
        part: u32,
        body: Vec<u8>,
    },
    CompleteMultipart {
        key: String,
        upload_id: String,
        parts: Vec<(u32, String)>,
    },
    AbortMultipart {
        key: String,
        upload_id: String,
    },
}

/// Transport methods are called only by the authorized backup client.
pub trait ObjectStore: Send + Sync {
    fn multipart_sessions(&self, _prefix: &str) -> StoreFuture<'_, Vec<UploadSession>> {
        Box::pin(async { bail!("BACKUP_MULTIPART_UNSUPPORTED") })
    }
    fn uploads(&self, key: &str) -> StoreFuture<'_, Vec<String>>;
    fn get(&self, key: &str) -> StoreFuture<'_, Option<Object>>;
    fn write(&self, operation: Write) -> StoreFuture<'_, String>;
    fn head(&self, key: &str) -> StoreFuture<'_, Option<ObjectInfo>>;
    fn list(&self, prefix: &str, token: Option<String>) -> StoreFuture<'_, Page>;
    fn stream(&self, key: &str, offset: u64, etag: Option<String>)
        -> StoreFuture<'_, ObjectStream>;
    fn parts(&self, key: &str, upload_id: &str) -> StoreFuture<'_, Vec<Part>>;
}
