//! Injectable S3 operations behind an enforced single-writer boundary.
use super::model::{Library, Role, Space};
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
#[derive(Clone,Debug)]
pub struct UploadSession {pub key:String,pub upload_id:String}

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
    fn multipart_sessions(&self,_prefix:&str)->StoreFuture<'_,Vec<UploadSession>> {
        Box::pin(async {bail!("BACKUP_MULTIPART_UNSUPPORTED")})
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

pub struct Client<S> {
    store: S,
    space: Space,
    device_id: String,
}

impl<S: ObjectStore> Client<S> {
    pub fn new(store: S, space: Space, device_id: String) -> Self {
        Self {
            store,
            space,
            device_id,
        }
    }

    pub fn space(&self) -> &Space {
        &self.space
    }
    fn check_key(&self, key: &str) -> Result<()> {
        if self.space.prefix.is_empty()
            || !self.space.prefix.ends_with('/')
            || !key.starts_with(&self.space.prefix)
            || key.contains('\\')
            || key
                .split('/')
                .any(|s| s == ".." || s == "." || s.is_empty())
        {
            bail!("BACKUP_INVALID_KEY");
        }
        Ok(())
    }
    pub async fn get(&self, key: &str) -> Result<Option<Object>> {
        self.check_key(key)?;
        self.store.get(key).await
    }
    pub async fn head(&self, key: &str) -> Result<Option<ObjectInfo>> {
        self.check_key(key)?;
        self.store.head(key).await
    }
    pub async fn uploads(&self, key: &str) -> Result<Vec<String>> {
        self.check_key(key)?;
        self.store.uploads(key).await
    }
    pub async fn parts(&self, key: &str, id: &str) -> Result<Vec<Part>> {
        self.check_key(key)?;
        self.store.parts(key, id).await
    }
    pub async fn list(&self, prefix: &str, token: Option<String>) -> Result<Page> {
        self.check_key(prefix.trim_end_matches('/'))?;
        self.store.list(prefix, token).await
    }
    pub async fn stream(
        &self,
        key: &str,
        offset: u64,
        etag: Option<String>,
    ) -> Result<ObjectStream> {
        self.check_key(key)?;
        self.store.stream(key, offset, etag).await
    }

    pub async fn connect(&self) -> Result<Library> {
        let key = format!("{}library.json", self.space.prefix);
        let object = self
            .store
            .get(&key)
            .await?
            .ok_or_else(|| anyhow::anyhow!("BACKUP_LIBRARY_NOT_FOUND"))?;
        let library: Library = serde_json::from_slice(&object.body)?;
        library.validate()?;
        if library.library_id != self.space.library_id {
            bail!("BACKUP_LIBRARY_MISMATCH");
        }
        Ok(library)
    }

    /// Only initialization may write before an existing library can be read.
    pub async fn create(&self, library: Library) -> Result<Library> {
        self.space.authorize_write(&self.device_id, &library)?;
        let key = format!("{}library.json", self.space.prefix);
        match self
            .store
            .write(Write::Put {
                key,
                body: serde_json::to_vec(&library)?,
                condition: Condition::Absent,
            })
            .await
        {
            Ok(_) => {}
            Err(e) if e.to_string() == "BACKUP_PRECONDITION_FAILED" => {}
            Err(e) => return Err(e),
        }
        let stored = self.connect().await?;
        self.space.authorize_write(&self.device_id, &stored)?;
        Ok(stored)
    }

    pub async fn write(&self, operation: Write) -> Result<String> {
        // Reader denial precedes even a library fetch, including probe/delete/multipart paths.
        if self.space.role != Role::Primary || !self.space.enabled {
            bail!("BACKUP_READ_ONLY");
        }
        let library = self.connect().await?;
        self.space.authorize_write(&self.device_id, &library)?;
        let key = match &operation {
            Write::PutFile { key, .. }
            | Write::UploadFilePart { key, .. }
            | Write::Put { key, .. }
            | Write::Delete { key }
            | Write::DeleteIfMatch { key, .. }
            | Write::CreateMultipart { key, .. }
            | Write::UploadPart { key, .. }
            | Write::CompleteMultipart { key, .. }
            | Write::AbortMultipart { key, .. } => key,
        };
        self.check_key(key)?;
        if !key.starts_with(&self.space.prefix)
            || key == &format!("{}library.json", self.space.prefix)
            || key
                .split('/')
                .any(|s| s == ".." || s == "." || s.is_empty())
            || key.contains('\\')
        {
            bail!("BACKUP_INVALID_KEY");
        }
        self.store.write(operation).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        collections::HashMap,
        sync::{
            atomic::{AtomicUsize, Ordering},
            Arc, Mutex,
        },
    };

    #[derive(Clone, Default)]
    struct Memory {
        values: Arc<Mutex<HashMap<String, Object>>>,
        writes: Arc<AtomicUsize>,
    }
    impl ObjectStore for Memory {
        fn uploads(&self, _key: &str) -> StoreFuture<'_, Vec<String>> {
            Box::pin(async { Ok(vec![]) })
        }
        fn head(&self, _key: &str) -> StoreFuture<'_, Option<ObjectInfo>> {
            Box::pin(async { bail!("UNIMPLEMENTED_TEST_READ") })
        }
        fn list(&self, _prefix: &str, _token: Option<String>) -> StoreFuture<'_, Page> {
            Box::pin(async { bail!("UNIMPLEMENTED_TEST_READ") })
        }
        fn stream(
            &self,
            _key: &str,
            _offset: u64,
            _etag: Option<String>,
        ) -> StoreFuture<'_, ObjectStream> {
            Box::pin(async { bail!("UNIMPLEMENTED_TEST_READ") })
        }
        fn parts(&self, _key: &str, _upload_id: &str) -> StoreFuture<'_, Vec<Part>> {
            Box::pin(async { bail!("UNIMPLEMENTED_TEST_READ") })
        }
        fn get(&self, key: &str) -> StoreFuture<'_, Option<Object>> {
            let value = self.values.lock().unwrap().get(key).cloned();
            Box::pin(async move { Ok(value) })
        }
        fn write(&self, op: Write) -> StoreFuture<'_, String> {
            Box::pin(async move {
                self.writes.fetch_add(1, Ordering::SeqCst);
                let mut values = self.values.lock().unwrap();
                match op {
                    Write::Put {
                        key,
                        body,
                        condition,
                    } => {
                        let current = values.get(&key);
                        let valid = match condition {
                            Condition::Absent => current.is_none(),
                            Condition::Match(etag) => current.is_some_and(|v| v.etag == etag),
                        };
                        if !valid {
                            bail!("BACKUP_PRECONDITION_FAILED");
                        }
                        let etag = uuid::Uuid::new_v4().to_string();
                        values.insert(
                            key,
                            Object {
                                body,
                                etag: etag.clone(),
                            },
                        );
                        Ok(etag)
                    }
                    Write::Delete { key } => {
                        values.remove(&key);
                        Ok(String::new())
                    }
                    _ => Ok(String::new()),
                }
            })
        }
    }
    fn setup(role: Role) -> (Memory, Space, Library) {
        let library = Library {
            schema_version: 1,
            library_id: uuid::Uuid::new_v4().to_string(),
            mode: "single_writer".into(),
            writer_device_id: uuid::Uuid::new_v4().to_string(),
            writer_epoch: 1,
            created_at: "2026-10-30T00:00:00Z".into(),
        };
        let space = Space {
            space_id: uuid::Uuid::new_v4().to_string(),
            library_id: library.library_id.clone(),
            endpoint: "https://example.r2.cloudflarestorage.com".into(),
            bucket: "test".into(),
            prefix: "yingqu/v1/".into(),
            role,
            writer_device_id: library.writer_device_id.clone(),
            writer_epoch: 1,
            credential_ref: None,
            enabled: true,
        };
        (Memory::default(), space, library)
    }
    #[tokio::test]
    async fn reader_denies_every_mutation_without_transport_writes() {
        let (store, space, library) = setup(Role::Reader);
        let client = Client::new(store.clone(), space, library.writer_device_id.clone());
        assert!(client.create(library).await.is_err());
        let key = "yingqu/v1/media/example".to_owned();
        let ops = [
            Write::PutFile {
                key: key.clone(),
                path: "never-read.mp4".into(),
                bytes: 1,
                sha256: "a".repeat(64),
                progress: None,
            },
            Write::UploadFilePart {
                key: key.clone(),
                upload_id: "u".into(),
                part: 1,
                path: "never-read.mp4".into(),
                offset: 0,
                bytes: 1,
                progress: None,
            },
            Write::Put {
                key: key.clone(),
                body: vec![],
                condition: Condition::Absent,
            },
            Write::Delete { key: key.clone() },
            Write::DeleteIfMatch { key: key.clone(), etag: "unchanged".into() },
            Write::CreateMultipart {
                key: key.clone(),
                sha256: "a".repeat(64),
            },
            Write::UploadPart {
                key: key.clone(),
                upload_id: "u".into(),
                part: 1,
                body: vec![],
            },
            Write::CompleteMultipart {
                key: key.clone(),
                upload_id: "u".into(),
                parts: vec![],
            },
            Write::AbortMultipart {
                key,
                upload_id: "u".into(),
            },
        ];
        for op in ops {
            assert_eq!(
                client.write(op).await.unwrap_err().to_string(),
                "BACKUP_READ_ONLY"
            );
        }
        assert_eq!(store.writes.load(Ordering::SeqCst), 0);
    }
    #[tokio::test]
    async fn library_creation_is_conditional_and_changed_writer_blocks_writes() {
        let (store, space, library) = setup(Role::Primary);
        let client = Client::new(store.clone(), space, library.writer_device_id.clone());
        assert_eq!(client.create(library.clone()).await.unwrap(), library);
        assert_eq!(client.create(library.clone()).await.unwrap(), library);
        let mut changed = library;
        changed.writer_epoch += 1;
        store
            .values
            .lock()
            .unwrap()
            .get_mut("yingqu/v1/library.json")
            .unwrap()
            .body = serde_json::to_vec(&changed).unwrap();
        let before = store.writes.load(Ordering::SeqCst);
        assert_eq!(
            client
                .write(Write::Delete {
                    key: "yingqu/v1/media/example".into()
                })
                .await
                .unwrap_err()
                .to_string(),
            "BACKUP_WRITER_CHANGED"
        );
        assert_eq!(store.writes.load(Ordering::SeqCst), before);
    }
}
