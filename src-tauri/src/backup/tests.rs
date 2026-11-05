use super::{model::Space, repository, scheduler, settings, store::*};
use crate::desktop_core::Core;
use anyhow::{bail, Context, Result};
use rusqlite::params;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    io::{Read, Seek},
    sync::{Arc, Mutex},
};

type Upload = (String, String, BTreeMap<u32, Vec<u8>>);

#[path = "sync_online_tests.rs"]
mod sync_online_tests;
#[path = "sync_tests.rs"]
mod sync_tests;

#[derive(Default)]
struct R2Audit {
    video_streams: u64,
    video_writes: u64,
    calls: BTreeMap<&'static str, usize>,
    uploaded_parts: Vec<u32>,
    lose_first_part_response: bool,
}
struct ObservedR2 {
    inner: super::s3_client::S3Store,
    audit: Arc<Mutex<R2Audit>>,
}
impl ObservedR2 {
    fn count(&self, method: &'static str) {
        *self.audit.lock().unwrap().calls.entry(method).or_default() += 1;
    }
}
impl ObjectStore for ObservedR2 {
    fn multipart_sessions(&self, prefix: &str) -> StoreFuture<'_, Vec<UploadSession>> {
        self.count("LIST_UPLOADS");
        self.inner.multipart_sessions(prefix)
    }
    fn get(&self, key: &str) -> StoreFuture<'_, Option<Object>> {
        self.count("GET");
        self.inner.get(key)
    }
    fn head(&self, key: &str) -> StoreFuture<'_, Option<ObjectInfo>> {
        self.count("HEAD");
        self.inner.head(key)
    }
    fn list(&self, prefix: &str, token: Option<String>) -> StoreFuture<'_, Page> {
        self.count("LIST");
        self.inner.list(prefix, token)
    }
    fn stream(
        &self,
        key: &str,
        offset: u64,
        etag: Option<String>,
    ) -> StoreFuture<'_, ObjectStream> {
        self.count("GET");
        if key.ends_with(".mp4") {
            self.audit.lock().unwrap().video_streams += 1;
        }
        self.inner.stream(key, offset, etag)
    }
    fn uploads(&self, key: &str) -> StoreFuture<'_, Vec<String>> {
        self.count("LIST_UPLOADS");
        self.inner.uploads(key)
    }
    fn parts(&self, key: &str, id: &str) -> StoreFuture<'_, Vec<Part>> {
        self.count("LIST_PARTS");
        self.inner.parts(key, id)
    }
    fn write(&self, operation: Write) -> StoreFuture<'_, String> {
        self.count("WRITE");
        if matches!(&operation,Write::PutFile{key,..}|Write::UploadFilePart{key,..} if key.ends_with(".mp4"))
        {
            self.audit.lock().unwrap().video_writes += 1;
        }
        Box::pin(async move {
            let part = match &operation {
                Write::UploadFilePart { part, .. } => Some(*part),
                _ => None,
            };
            let result = self.inner.write(operation).await?;
            if let Some(part) = part {
                let mut audit = self.audit.lock().unwrap();
                audit.uploaded_parts.push(part);
                if std::mem::take(&mut audit.lose_first_part_response) {
                    bail!("BACKUP_NETWORK_ERROR");
                }
            }
            Ok(result)
        })
    }
}

#[derive(Default)]
struct State {
    ignore_conditions: bool,
    ignore_delete_conditions: bool,
    edit_during_file: Option<(std::sync::Weak<Core>, String, String, serde_json::Value)>,
    hold_upload: Option<(Arc<tokio::sync::Notify>, Arc<tokio::sync::Notify>)>,
    hold_probe: Option<(Arc<tokio::sync::Notify>, Arc<tokio::sync::Notify>)>,
    hold_list: Option<(Arc<tokio::sync::Notify>, Arc<tokio::sync::Notify>)>,
    hold_stream: Option<(Arc<tokio::sync::Notify>, Arc<tokio::sync::Notify>)>,
    objects: BTreeMap<String, (Vec<u8>, Option<String>)>,
    uploads: BTreeMap<String, Upload>,
    writes: usize,
    video_gets: usize,
    part_writes: Vec<(String, u32)>,
    part_reads: usize,
    get_count: usize,
    head_count: usize,
    stream_offsets: Vec<u64>,
    break_stream: Option<usize>,
    corrupt_stream: bool,
    fail_after: Option<&'static str>,
    fail_after_code: Option<&'static str>,
    fail_part_once: Option<u32>,
    fail_part_remaining: Option<(u32, u8)>,
    fail_delete: bool,
    fail_record: bool,
    stop_after_tombstone: Option<(std::sync::Weak<Core>, String, bool)>,
    fail_list: bool,
    ignore_range: bool,
    conflict_record_once: bool,
}
#[derive(Clone, Default)]
struct Memory(Arc<Mutex<State>>);
fn is_video_key(key: &str) -> bool {
    key.rsplit('/')
        .next()
        .is_some_and(|name| name.starts_with("video-") && name.ends_with(".mp4"))
}
fn tag(body: &[u8]) -> String {
    format!("{:x}", Sha256::digest(body))
}
impl ObjectStore for Memory {
    fn multipart_sessions(&self, prefix: &str) -> StoreFuture<'_, Vec<UploadSession>> {
        let sessions = self
            .0
            .lock()
            .unwrap()
            .uploads
            .iter()
            .filter(|(_, v)| v.0.starts_with(prefix))
            .map(|(id, v)| UploadSession {
                key: v.0.clone(),
                upload_id: id.clone(),
            })
            .collect();
        Box::pin(async move { Ok(sessions) })
    }
    fn get(&self, key: &str) -> StoreFuture<'_, Option<Object>> {
        let mut s = self.0.lock().unwrap();
        s.get_count += 1;
        if is_video_key(key) {
            s.video_gets += 1;
        }
        let v = s.objects.get(key).map(|(body, _)| Object {
            body: body.clone(),
            etag: tag(body),
        });
        Box::pin(async move { Ok(v) })
    }
    fn head(&self, key: &str) -> StoreFuture<'_, Option<ObjectInfo>> {
        let mut state = self.0.lock().unwrap();
        state.head_count += 1;
        let v = state.objects.get(key).map(|(b, h)| ObjectInfo {
            key: key.into(),
            etag: tag(b),
            bytes: b.len() as u64,
            sha256: h.clone(),
        });
        Box::pin(async move { Ok(v) })
    }
    fn list(&self, prefix: &str, token: Option<String>) -> StoreFuture<'_, Page> {
        let gate = self.0.lock().unwrap().hold_list.take();
        if self.0.lock().unwrap().fail_list {
            return Box::pin(async { bail!("BACKUP_NETWORK_ERROR") });
        }
        let all: Vec<_> = self
            .0
            .lock()
            .unwrap()
            .objects
            .iter()
            .filter(|(k, _)| k.starts_with(prefix))
            .map(|(k, (b, h))| ObjectInfo {
                key: k.clone(),
                etag: tag(b),
                bytes: b.len() as u64,
                sha256: h.clone(),
            })
            .collect();
        let offset = token.and_then(|s| s.parse::<usize>().ok()).unwrap_or(0);
        let next = (offset + 1 < all.len()).then(|| (offset + 1).to_string());
        let objects = all.into_iter().skip(offset).take(1).collect();
        Box::pin(async move {
            if let Some((started, release)) = gate {
                started.notify_one();
                release.notified().await;
            }
            Ok(Page { objects, next })
        })
    }
    fn stream(
        &self,
        key: &str,
        offset: u64,
        etag: Option<String>,
    ) -> StoreFuture<'_, ObjectStream> {
        let key = key.to_owned();
        let store = self.clone();
        let gate = store.0.lock().unwrap().hold_stream.take();
        Box::pin(async move {
            if let Some((started, release)) = gate {
                started.notify_one();
                release.notified().await;
            }
            let mut s = store.0.lock().unwrap();
            if is_video_key(&key) {
                s.video_gets += 1;
            }
            s.stream_offsets.push(offset);
            let broken = s.break_stream.take();
            let corrupt = s.corrupt_stream;
            let (body, hash) = s
                .objects
                .get(&key)
                .ok_or_else(|| anyhow::anyhow!("BACKUP_OBJECT_NOT_FOUND"))?;
            let t = tag(body);
            if etag.is_some_and(|e| e != t) {
                bail!("BACKUP_PRECONDITION_FAILED");
            }
            let offset = if s.ignore_range { 0 } else { offset };
            let mut content = body[offset as usize..].to_vec();
            if corrupt {
                let last = content.len() - 1;
                content[last] ^= 1;
            }
            let reader: Box<dyn tokio::io::AsyncRead + Send + Unpin> = if let Some(at) = broken {
                Box::new(BrokenRead {
                    body: content,
                    position: 0,
                    at,
                })
            } else {
                Box::new(std::io::Cursor::new(content))
            };
            Ok(ObjectStream {
                info: ObjectInfo {
                    key,
                    etag: t,
                    bytes: body.len() as u64 - offset,
                    sha256: hash.clone(),
                },
                content_range: (offset > 0)
                    .then(|| format!("bytes {offset}-{}/{}", body.len() - 1, body.len())),
                reader,
            })
        })
    }
    fn uploads(&self, key: &str) -> StoreFuture<'_, Vec<String>> {
        let ids = self
            .0
            .lock()
            .unwrap()
            .uploads
            .iter()
            .filter(|(_, v)| v.0 == key)
            .map(|(k, _)| k.clone())
            .collect();
        Box::pin(async move { Ok(ids) })
    }
    fn parts(&self, _key: &str, id: &str) -> StoreFuture<'_, Vec<Part>> {
        self.0.lock().unwrap().part_reads += 1;
        let value = self.0.lock().unwrap().uploads.get(id).map(|v| {
            v.2.iter()
                .map(|(n, b)| Part {
                    number: *n,
                    etag: tag(b),
                    bytes: b.len() as u64,
                })
                .collect()
        });
        Box::pin(async move { value.ok_or_else(|| anyhow::anyhow!("BACKUP_UPLOAD_EXPIRED")) })
    }
    fn write(&self, write: Write) -> StoreFuture<'_, String> {
        let gate = if matches!(&write, Write::PutFile { .. }) {
            self.0.lock().unwrap().hold_upload.take()
        } else if matches!(&write,Write::Put{key,..} if key.contains("/probes/")) {
            self.0.lock().unwrap().hold_probe.take()
        } else {
            None
        };
        Box::pin(async move {
            if let Some((started, release)) = gate {
                started.notify_one();
                release.notified().await;
            }
            let mut s = self.0.lock().unwrap();
            s.writes += 1;
            let (etag, kind) = match write {
                Write::Put {
                    key,
                    body,
                    condition,
                } => {
                    if s.conflict_record_once && key.contains("records/") {
                        s.conflict_record_once = false;
                        bail!("BACKUP_PRECONDITION_FAILED");
                    }
                    if s.fail_record && key.contains("records/") {
                        bail!("BACKUP_NETWORK_ERROR");
                    }
                    let old = s.objects.get(&key);
                    match condition {
                        Condition::Absent if old.is_some() && !s.ignore_conditions => {
                            bail!("BACKUP_PRECONDITION_FAILED")
                        }
                        Condition::Match(t)
                            if !s.ignore_conditions && old.is_none_or(|(b, _)| tag(b) != t) =>
                        {
                            bail!("BACKUP_PRECONDITION_FAILED")
                        }
                        _ => {}
                    }
                    let t = tag(&body);
                    let deleted = body.windows(9).any(|b| b == b"\"deleted\"");
                    s.objects.insert(key, (body, None));
                    if deleted {
                        if let Some((core, space, reader)) = s.stop_after_tombstone.take() {
                            core.upgrade().unwrap().backup_db(|db|{db.execute("UPDATE backup_spaces SET enabled=?2 WHERE space_id=?1",params![space,reader])?;db.execute("UPDATE backup_operations SET state='blocked' WHERE space_id=? AND kind='delete'",[space])?;Ok(())})?;
                        }
                    }
                    (t, "put")
                }
                Write::PutFile {
                    key,
                    path,
                    bytes,
                    sha256,
                    ..
                } => {
                    if s.objects.contains_key(&key) {
                        bail!("BACKUP_PRECONDITION_FAILED");
                    }
                    let b = std::fs::read(path)?;
                    assert_eq!(b.len() as u64, bytes);
                    let t = tag(&b);
                    s.objects.insert(key, (b, Some(sha256)));
                    if let Some((core, id, field, value)) = s.edit_during_file.take() {
                        core.upgrade().unwrap().backup_db(|db| {
                            super::sync_engine::mark_local(
                                db,
                                &id,
                                [(field, value)].into_iter().collect(),
                                false,
                            )
                        })?;
                    }
                    (t, "file")
                }
                Write::DeleteIfMatch { key, etag } => {
                    if !s.ignore_delete_conditions
                        && s.objects
                            .get(&key)
                            .is_some_and(|(body, _)| tag(body) != etag)
                    {
                        bail!("BACKUP_PRECONDITION_FAILED");
                    }
                    s.objects.remove(&key);
                    (String::new(), "delete")
                }
                Write::Delete { key } => {
                    if s.fail_delete {
                        bail!("BACKUP_NETWORK_ERROR");
                    }
                    s.objects.remove(&key);
                    (String::new(), "delete")
                }
                Write::CreateMultipart { key, sha256 } => {
                    let id = uuid::Uuid::new_v4().to_string();
                    s.uploads.insert(id.clone(), (key, sha256, BTreeMap::new()));
                    (id, "create")
                }
                Write::UploadFilePart {
                    upload_id,
                    part,
                    path,
                    offset,
                    bytes,
                    ..
                } => {
                    s.part_writes.push((upload_id.clone(), part));
                    if let Some((number, remaining)) = &mut s.fail_part_remaining {
                        if *number == part && *remaining > 0 {
                            *remaining -= 1;
                            bail!("BACKUP_SERVICE_UNAVAILABLE");
                        }
                    }
                    if s.fail_part_once == Some(part) {
                        s.fail_part_once = None;
                        bail!("BACKUP_SERVICE_UNAVAILABLE");
                    }
                    let mut f = std::fs::File::open(path)?;
                    f.seek(std::io::SeekFrom::Start(offset))?;
                    let mut b = Vec::new();
                    f.take(bytes).read_to_end(&mut b)?;
                    let t = tag(&b);
                    s.uploads
                        .get_mut(&upload_id)
                        .ok_or_else(|| anyhow::anyhow!("BACKUP_UPLOAD_EXPIRED"))?
                        .2
                        .insert(part, b);
                    (t, "part")
                }
                Write::UploadPart {
                    upload_id,
                    part,
                    body,
                    ..
                } => {
                    let t = tag(&body);
                    s.uploads
                        .get_mut(&upload_id)
                        .ok_or_else(|| anyhow::anyhow!("BACKUP_UPLOAD_EXPIRED"))?
                        .2
                        .insert(part, body);
                    (t, "part")
                }
                Write::CompleteMultipart {
                    key,
                    upload_id,
                    parts,
                } => {
                    let (_, hash, stored) = s
                        .uploads
                        .remove(&upload_id)
                        .ok_or_else(|| anyhow::anyhow!("BACKUP_UPLOAD_EXPIRED"))?;
                    let mut b = Vec::new();
                    for (n, t) in parts {
                        assert_eq!(tag(&stored[&n]), t);
                        b.extend_from_slice(&stored[&n]);
                    }
                    let t = tag(&b);
                    s.objects.insert(key, (b, Some(hash)));
                    (t, "complete")
                }
                Write::AbortMultipart { upload_id, .. } => {
                    s.uploads.remove(&upload_id);
                    (String::new(), "abort")
                }
            };
            if s.fail_after == Some(kind) {
                s.fail_after = None;
                bail!(s.fail_after_code.take().unwrap_or("BACKUP_NETWORK_ERROR"));
            }
            Ok(etag)
        })
    }
}

struct Fixture {
    root: tempfile::TempDir,
    core: Arc<Core>,
    store: Memory,
    space: Space,
    id: String,
}
impl Fixture {
    async fn new(bytes: u64) -> Self {
        let root = tempfile::tempdir().unwrap();
        let core = Core::open(
            &root.path().join("app.sqlite"),
            root.path().join("downloads"),
            "unused".into(),
            vec![],
            |_| {},
        )
        .unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let path = root.path().join("video.mp4");
        std::fs::write(&path, b"\0\0\0\x18ftypisom\0\0\0\0isommp42").unwrap();
        std::fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len(bytes.max(24))
            .unwrap();
        let device = core.backup_db(repository::device_id).unwrap();
        let space = Space {
            space_id: uuid::Uuid::new_v4().to_string(),
            library_id: uuid::Uuid::new_v4().to_string(),
            endpoint: "https://0123456789abcdef0123456789abcdef.r2.cloudflarestorage.com".into(),
            bucket: "test-bucket".into(),
            prefix: "test/".into(),

            credential_ref: None,
            enabled: true,
        };
        core.backup_db(|db|{repository::save_space(db,&space)?;db.execute("INSERT INTO media_assets VALUES(?1,NULL,?2,'video.mp4',NULL,?3,'active')",params![id,serde_json::json!({"platform":"youtube","video_id":"12345678901","title":"测试","completed_at":2}).to_string(),i64::try_from(bytes.max(24))?])?;db.execute("INSERT INTO asset_local_files(asset_id,path,bytes,local_state) VALUES(?1,?2,?3,'present')",params![id,path.to_string_lossy(),i64::try_from(bytes.max(24))?])?;Ok(())}).unwrap();
        let store = Memory::default();
        use crate::backup::provider::BackupProvider;
        let provider = crate::backup::s3_provider::S3Provider::new(
            store.clone(),
            space.clone(),
            sync_tests::capabilities(),
            device,
        );
        provider.open_library(true).await.unwrap();
        Self {
            root,
            core,
            store,
            space,
            id,
        }
    }
    async fn upload(&self) -> Result<()> {
        use crate::backup::provider::BackupProvider;
        self.core.verify_backup_asset(&self.id).await?;
        let caps = sync_tests::capabilities();
        self.core.backup_db(|db| {
            db.execute(
                "UPDATE backup_provider_config SET capabilities_json=? WHERE space_id=?",
                params![serde_json::to_string(&caps)?, self.space.space_id],
            )?;
            db.execute(
                "INSERT OR REPLACE INTO backup_current_space VALUES(1,?)",
                [&self.space.space_id],
            )?;
            crate::backup::sync_engine::register_download(db, &self.id)
        })?;
        let provider = Arc::new(crate::backup::s3_provider::S3Provider::new(
            self.store.clone(),
            self.space.clone(),
            caps,
            self.core.backup_db(repository::device_id)?,
        ));
        provider.open_library(false).await?;
        let engine = crate::backup::sync_engine::SyncEngine::new(self.core.clone(), provider);
        engine.enqueue()?;
        let operation = scheduler::operations(&self.core)?
            .into_iter()
            .find(|o| o.kind == "upload" && o.state != "completed")
            .context("fixture upload absent")?;
        self.core.backup_db(|db| {
            db.execute(
                "UPDATE backup_operations SET state='running' WHERE operation_id=?",
                [&operation.operation_id],
            )?;
            Ok(())
        })?;
        engine.execute_upload(&operation.operation_id).await
    }
}
#[derive(Default)]
struct Vault(Mutex<BTreeMap<String, String>>);
impl settings::Vault for Vault {
    fn read(&self, k: &str) -> Result<zeroize::Zeroizing<String>> {
        Ok(zeroize::Zeroizing::new(self.0.lock().unwrap()[k].clone()))
    }
    fn write(&self, k: &str, v: &str) -> Result<()> {
        self.0.lock().unwrap().insert(k.into(), v.into());
        Ok(())
    }
    fn delete(&self, k: &str) -> Result<()> {
        self.0.lock().unwrap().remove(k);
        Ok(())
    }
}
struct BrokenRead {
    body: Vec<u8>,
    position: usize,
    at: usize,
}
impl tokio::io::AsyncRead for BrokenRead {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
        out: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        if self.position >= self.at {
            return std::task::Poll::Ready(Err(std::io::Error::other("injected disconnect")));
        }
        let end = (self.position + out.remaining())
            .min(self.at)
            .min(self.body.len());
        out.put_slice(&self.body[self.position..end]);
        self.position = end;
        std::task::Poll::Ready(Ok(()))
    }
}
