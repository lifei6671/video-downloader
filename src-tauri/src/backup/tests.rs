use super::{
    model::{Library, Role, Space},
    repository, scheduler, settings,
    store::*,
};
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

#[path = "sync_tests.rs"]
mod sync_tests;
#[path = "sync_online_tests.rs"]
mod sync_online_tests;

#[tokio::test]
#[ignore = "requires explicitly authorized root s3-r2-test.local.json"]
async fn real_r2_m2_dedicated_prefix() {
    real_r2_scenario(24, false).await;
}

#[tokio::test]
#[ignore = "requires explicitly authorized root s3-r2-test.local.json"]
async fn real_r2_m6_multipart_restart_dedicated_prefix() {
    real_r2_scenario(100 * 1024 * 1024, true).await;
}

// Test-only transport observation: no keys, URLs, or credentials enter the audit.
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
    fn multipart_sessions(&self,prefix:&str)->StoreFuture<'_,Vec<UploadSession>> {self.count("LIST_UPLOADS");self.inner.multipart_sessions(prefix)}
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
        if key.ends_with(".mp4") {self.audit.lock().unwrap().video_streams+=1;}
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
        if matches!(&operation,Write::PutFile{key,..}|Write::UploadFilePart{key,..} if key.ends_with(".mp4")) {self.audit.lock().unwrap().video_writes+=1;}
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

fn r2_operation(core: &Core) -> Result<scheduler::Operation> {
    scheduler::enqueue(core)?;
    let op = scheduler::operations(core)?
        .into_iter()
        .find(|op| op.kind == "upload" && op.state != "completed")
        .context("R2_UPLOAD_OPERATION_MISSING")?;
    core.backup_db(|db| {
        db.execute(
            "UPDATE backup_operations SET state='running' WHERE operation_id=?",
            [&op.operation_id],
        )?;
        Ok(())
    })?;
    Ok(op)
}

async fn real_r2_scenario(bytes: u64, restart: bool) {
    let raw = zeroize::Zeroizing::new(
        std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .join("s3-r2-test.local.json"),
        )
        .unwrap(),
    );
    #[derive(serde::Deserialize)]
    struct Config {
        schema_version: u32,
        environment: String,
        endpoint: String,
        region: String,
        bucket: String,
        prefix: String,
        primary: settings::Credentials,
        reader: settings::Credentials,
    }
    let config: Config = serde_json::from_str(&raw).expect("invalid dedicated test configuration");
    assert_eq!(config.schema_version, 1);
    assert_eq!(config.environment, "dedicated-test");
    assert_eq!(config.region, "auto");
    super::s3_client::validate_target(&config.endpoint, &config.bucket, &config.prefix).unwrap();
    let mut f = Fixture::new(bytes).await;
    let prefix = format!("{}run-{}/", config.prefix, uuid::Uuid::new_v4());
    f.space.endpoint = config.endpoint;
    f.space.bucket = config.bucket;
    f.space.prefix = prefix.clone();
    f.core
        .backup_db(|db| {
            db.execute(
                "UPDATE backup_spaces SET endpoint=?1,bucket=?2,prefix=?3 WHERE space_id=?4",
                params![f.space.endpoint, f.space.bucket, prefix, f.space.space_id],
            )?;
            Ok(())
        })
        .unwrap();
    let device = f.core.backup_db(repository::device_id).unwrap();
    let make = |space: &Space, credentials: &settings::Credentials| {
        super::s3_client::S3Store::new(
            space,
            &credentials.access_key_id,
            &credentials.secret_access_key,
        )
        .unwrap()
    };
    let journal = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join(".local/r2-test-runs.jsonl");
    std::fs::create_dir_all(journal.parent().unwrap()).unwrap();
    {
        use std::io::Write as _;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(journal)
            .unwrap();
        writeln!(
            file,
            "{}",
            serde_json::json!({"prefix":prefix,"asset_id":f.id,"library_id":f.space.library_id})
        )
        .unwrap();
    }
    let primary_audit = Arc::new(Mutex::new(R2Audit {
        lose_first_part_response: restart,
        ..Default::default()
    }));
    let reader_audit = Arc::new(Mutex::new(R2Audit::default()));
    let primary = Client::new(
        ObservedR2 {
            inner: make(&f.space, &config.primary),
            audit: primary_audit.clone(),
        },
        f.space.clone(),
        device.clone(),
    );
    let library = Library {
        schema_version: 1,
        library_id: f.space.library_id.clone(),
        mode: "single_writer".into(),
        writer_device_id: device,
        writer_epoch: 1,
        created_at: chrono::Utc::now().to_rfc3339(),
    };
    let result = async {
        primary
            .create(library.clone())
            .await
            .context("R2_CREATE_LIBRARY")?;
        primary.create(library).await?;
        scheduler::test_connection(&f.core, &primary).await?;
        f.core.verify_backup_asset(&f.id).await?;
        let hash=f.core.backup_assets()?.first().context("R2_ASSET_MISSING")?.content_sha256.clone().context("R2_HASH_MISSING")?;
        f.core.backup_db(|db|{db.execute("INSERT INTO asset_transcripts(asset_id,content_sha256,text,model_revision,pipeline_revision,completed_at) VALUES(?1,?2,'dedicated fixture text','fixture','fixture',1)",params![f.id,hash])?;Ok(())})?;
        let op = r2_operation(&f.core)?;
        let (_tx, rx) = tokio::sync::watch::channel(false);
        if restart {
            anyhow::ensure!(scheduler::upload(&f.core, &primary, &op, rx.clone()).await.err().context("R2_EXPECTED_INTERRUPTION")?.to_string() == "BACKUP_NETWORK_ERROR", "R2_WRONG_INTERRUPTION");
            anyhow::ensure!(primary.get(&format!("{prefix}records/{}.json",f.id)).await?.is_none());
            // Close the actual SQLite connection before reopening the same device directory.
            let retired = std::mem::replace(&mut f.core, Core::open(
                &f.root.path().join("restart-placeholder.sqlite"),
                f.root.path().join("restart-placeholder-downloads"),
                "unused".into(), vec![], |_| {},
            )?);
            anyhow::ensure!(Arc::strong_count(&retired) == 1);
            drop(retired);
            f.core = Core::open(&f.root.path().join("app.sqlite"),f.root.path().join("downloads"),"unused".into(),vec![],|_|{})?;
            let resumed = r2_operation(&f.core)?;
            anyhow::ensure!(resumed.payload["upload_id"].as_str().is_some());
            scheduler::upload(&f.core, &primary, &resumed, rx).await?;
            let audit = primary_audit.lock().unwrap();
            anyhow::ensure!(audit.uploaded_parts == vec![1,2,3,4,5,6,7]);
            anyhow::ensure!(audit.calls.get("LIST_PARTS").copied().unwrap_or(0) > 0);
            eprintln!("R2 multipart restart: 100 MiB, application upload calls for parts 1..7 each succeeded once; ListParts reconciled acknowledged-lost part 1");
        } else {
            scheduler::upload(&f.core, &primary, &op, rx).await?;
        }
        let mut reader_space = f.space.clone();
        reader_space.role = Role::Reader;
        let reader_root=tempfile::tempdir()?;
        let reader_core=Core::open(&reader_root.path().join("app.sqlite"),reader_root.path().join("downloads"),"unused".into(),vec![],|_|{})?;
        reader_core.backup_db(|db|repository::save_space(db,&reader_space))?;
        let reader = Client::new(
            ObservedR2 { inner: make(&reader_space, &config.reader), audit: reader_audit.clone() },
            reader_space,
            reader_core.backup_db(repository::device_id)?,
        );
        scheduler::test_connection(&f.core, &reader).await?;
        let key = format!("{prefix}records/{}.json", f.id);
        let record: super::record::Record =
            serde_json::from_slice(&reader.get(&key).await?.context("R2_RECORD_MISSING")?.body)?;
        record.validate(&reader.connect().await?, &prefix)?;
        let super::record::Record::Active(active) = record else {
            bail!("unexpected tombstone");
        };
        let mut stream = reader.stream(&active.media.key, 0, None).await?;
        let mut body = Vec::new();
        tokio::io::AsyncReadExt::read_to_end(&mut stream.reader, &mut body).await?;
        anyhow::ensure!(tag(&body) == active.media.sha256);
        anyhow::ensure!(reader.write(Write::Delete { key }).await.err().context("R2_READER_WRITE_ALLOWED")?.to_string() == "BACKUP_READ_ONLY");
        super::reconcile::refresh(&reader_core,&reader).await?;
        anyhow::ensure!(reader_core.list().is_empty());anyhow::ensure!(reader_core.backup_assets()?.first().context("R2_IMPORTED_ASSET_MISSING")?.path.is_none());
        anyhow::ensure!(super::reconcile::transcript(&reader_core,&reader,&f.id).await?.context("R2_TRANSCRIPT_MISSING")?.text == "dedicated fixture text");
        anyhow::ensure!(reader_core.backup_assets()?.first().context("R2_IMPORTED_ASSET_MISSING")?.path.is_none());
        let restored=super::restore::ensure_with(&reader_core,&reader,&f.id).await?;
        anyhow::ensure!(repository::fingerprint(&restored)?.0 == active.media.sha256);
        super::lifecycle::release(&reader_core,&f.id).await?;
        anyhow::ensure!(!restored.exists());
        super::restore::ensure_with(&reader_core,&reader,&f.id).await?;
        super::lifecycle::delete(&f.core,&f.space.space_id,&f.id).await?;
        super::lifecycle::remote(&f.core,&primary,&scheduler::operations(&f.core)?.into_iter().find(|op|op.kind == "delete").context("R2_DELETE_OPERATION_MISSING")?).await?;
        anyhow::ensure!(primary.head(&active.media.key).await?.is_none(), "R2_MEDIA_NOT_DELETED");
        if let Some(transcript) = &active.transcript {
            anyhow::ensure!(primary.head(&transcript.key).await?.is_none(), "R2_TRANSCRIPT_NOT_DELETED");
        }
        super::reconcile::refresh(&reader_core,&reader).await?;
        anyhow::ensure!(reader_core.backup_assets()?.first().context("R2_IMPORTED_ASSET_MISSING")?.asset_state == "deleted");
        super::lifecycle::release(&reader_core,&f.id).await?;
        anyhow::ensure!(super::restore::ensure_with(&reader_core,&reader,&f.id).await.is_err());
        let audit = reader_audit.lock().unwrap();
        anyhow::ensure!(audit.calls.keys().all(|method| matches!(*method,"GET"|"HEAD"|"LIST")));
        eprintln!("R2 reader transport audit: {:?}; lifecycle and lazy restore passed",audit.calls);
        Ok::<_, anyhow::Error>(())
    }
    .await;
    if let Err(error) = &result {
        eprintln!("R2 scenario: {error:#}");
    }
    // No bucket-wide enumeration: this fresh run owns only these exact asset/probe/library keys.
    let raw_store = make(&f.space, &config.primary);
    // A failed multipart scenario can leave an upload without a visible object.
    // Only inspect media keys durably owned by this test's asset.
    let owned: Vec<String> = (|| -> Result<Vec<String>> {
        let db = rusqlite::Connection::open(f.root.path().join("app.sqlite"))?;
        let mut statement = db.prepare(
            "SELECT object_key FROM backup_owned_objects WHERE space_id=?1 AND asset_id=?2",
        )?;
        let keys = statement
            .query_map(params![f.space.space_id, f.id], |row| row.get(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(keys)
    })()
    .unwrap();
    for key in owned
        .into_iter()
        .filter(|key| key.starts_with(&format!("{prefix}media/{}/", f.id)))
    {
        for upload_id in raw_store.uploads(&key).await.unwrap() {
            raw_store
                .write(Write::AbortMultipart {
                    key: key.clone(),
                    upload_id,
                })
                .await
                .unwrap();
        }
        assert!(raw_store.uploads(&key).await.unwrap().is_empty());
    }
    let mut token = None;
    let mut cleanup = Vec::new();
    loop {
        let page = raw_store
            .list(&prefix, token)
            .await
            .expect("R2_RUN_PREFIX_LIST_CLEANUP");
        for object in page.objects {
            assert!(
                object.key == format!("{prefix}library.json")
                    || object.key == format!("{prefix}records/{}.json", f.id)
                    || object.key.starts_with(&format!("{prefix}media/{}/", f.id))
                    || object
                        .key
                        .starts_with(&format!("{prefix}transcripts/{}/", f.id))
                    || object.key.starts_with(&format!("{prefix}probes/"))
            );
            cleanup.push(object.key);
        }
        token = page.next;
        if token.is_none() {
            break;
        }
    }
    for key in cleanup {
        raw_store.write(Write::Delete { key }).await.unwrap();
    }
    assert!(raw_store
        .list(&prefix, None)
        .await
        .unwrap()
        .objects
        .is_empty());
    eprintln!("R2 dedicated run cleanup: object prefix empty; owned multipart uploads empty");
    result.unwrap();
}
#[derive(Default)]
struct State {
    ignore_conditions: bool,
    ignore_delete_conditions: bool,
    edit_during_file: Option<(std::sync::Weak<Core>,String,String,serde_json::Value)>,
    hold_upload: Option<(Arc<tokio::sync::Notify>,Arc<tokio::sync::Notify>)>,
    hold_probe: Option<(Arc<tokio::sync::Notify>,Arc<tokio::sync::Notify>)>,
    hold_list: Option<(Arc<tokio::sync::Notify>,Arc<tokio::sync::Notify>)>,
    objects: BTreeMap<String, (Vec<u8>, Option<String>)>,
    uploads: BTreeMap<String, Upload>,
    writes: usize,
    video_gets: usize,
    part_writes: Vec<(String,u32)>,
    part_reads: usize,
    get_count: usize,
    head_count: usize,
    stream_offsets: Vec<u64>,
    break_stream: Option<usize>,
    corrupt_stream: bool,
    fail_after: Option<&'static str>,
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
    key.rsplit('/').next().is_some_and(|name| name == "video.mp4" || name.starts_with("video-") && name.ends_with(".mp4"))
}
fn tag(body: &[u8]) -> String {
    format!("{:x}", Sha256::digest(body))
}
impl ObjectStore for Memory {
    fn multipart_sessions(&self,prefix:&str)->StoreFuture<'_,Vec<UploadSession>> {
        let sessions=self.0.lock().unwrap().uploads.iter().filter(|(_,v)|v.0.starts_with(prefix)).map(|(id,v)|UploadSession {key:v.0.clone(),upload_id:id.clone()}).collect();
        Box::pin(async move {Ok(sessions)})
    }
    fn get(&self, key: &str) -> StoreFuture<'_, Option<Object>> {
        let mut s = self.0.lock().unwrap();
        s.get_count+=1;
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
        let mut state=self.0.lock().unwrap();state.head_count+=1;
        let v = state.objects
            .get(key)
            .map(|(b, h)| ObjectInfo {
                key: key.into(),
                etag: tag(b),
                bytes: b.len() as u64,
                sha256: h.clone(),
            });
        Box::pin(async move { Ok(v) })
    }
    fn list(&self, prefix: &str, token: Option<String>) -> StoreFuture<'_, Page> {
        let gate=self.0.lock().unwrap().hold_list.take();
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
        Box::pin(async move {if let Some((started,release))=gate {started.notify_one();release.notified().await;} Ok(Page { objects, next }) })
    }
    fn stream(
        &self,
        key: &str,
        offset: u64,
        etag: Option<String>,
    ) -> StoreFuture<'_, ObjectStream> {
        let key = key.to_owned();
        let store = self.clone();
        Box::pin(async move {
            let mut s = store.0.lock().unwrap();
            if is_video_key(&key) {s.video_gets += 1;}
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
        let gate=if matches!(&write,Write::PutFile{..}) {self.0.lock().unwrap().hold_upload.take()} else if matches!(&write,Write::Put{key,..} if key.contains("/probes/")) {self.0.lock().unwrap().hold_probe.take()} else {None};
        Box::pin(async move {
            if let Some((started,release))=gate {started.notify_one();release.notified().await;}
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
                        Condition::Absent if old.is_some() && !s.ignore_conditions => bail!("BACKUP_PRECONDITION_FAILED"),
                        Condition::Match(t) if !s.ignore_conditions && old.is_none_or(|(b, _)| tag(b) != t) => {
                            bail!("BACKUP_PRECONDITION_FAILED")
                        }
                        _ => {}
                    }
                    let t = tag(&body);
                    let deleted = body.windows(9).any(|b| b == b"\"deleted\"");
                    s.objects.insert(key, (body, None));
                    if deleted {
                        if let Some((core, space, reader)) = s.stop_after_tombstone.take() {
                            core.upgrade().unwrap().backup_db(|db|{db.execute("UPDATE backup_spaces SET enabled=?2,role=?3 WHERE space_id=?1",params![space,reader,if reader {"reader"} else {"primary"}])?;db.execute("UPDATE backup_operations SET state='blocked' WHERE space_id=? AND kind='delete'",[space])?;Ok(())})?;
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
                    if let Some((core,id,field,value))=s.edit_during_file.take() {
                        core.upgrade().unwrap().backup_db(|db|super::sync_engine::mark_local(db,&id,[(field,value)].into_iter().collect(),false))?;
                    }
                    (t, "file")
                }
                Write::DeleteIfMatch { key, etag } => {
                    if !s.ignore_delete_conditions && s.objects.get(&key).is_some_and(|(body,_)|tag(body)!=etag) {bail!("BACKUP_PRECONDITION_FAILED");}
                    s.objects.remove(&key);
                    (String::new(),"delete")
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
                    s.part_writes.push((upload_id.clone(),part));
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
                bail!("BACKUP_NETWORK_ERROR");
            }
            Ok(etag)
        })
    }
}

#[tokio::test]
async fn reader_discovery_is_paginated_idempotent_and_never_downloads_video() {
    let f = Fixture::new(24).await;
    f.core.verify_backup_asset(&f.id).await.unwrap();
    let hash = f.core.backup_assets().unwrap()[0]
        .content_sha256
        .clone()
        .unwrap();
    f.core.backup_db(|db|{db.execute("INSERT INTO asset_transcripts(asset_id,content_sha256,text,model_revision,pipeline_revision,completed_at) VALUES(?1,?2,'portable text','m','p',1)",params![f.id,hash])?;Ok(())}).unwrap();
    f.upload().await.unwrap();
    let other = tempfile::tempdir().unwrap();
    let reader = Core::open(
        &other.path().join("reader.sqlite"),
        other.path().join("downloads"),
        "unused".into(),
        vec![],
        |_| {},
    )
    .unwrap();
    let mut space = f.space.clone();
    space.role = Role::Reader;
    reader
        .backup_db(|db| repository::save_space(db, &space))
        .unwrap();
    let client = Client::new(
        f.store.clone(),
        space,
        reader.backup_db(repository::device_id).unwrap(),
    );
    let key = format!("{}records/{}.json", f.space.prefix, f.id);
    let bad_key = format!("{}records/{}.json", f.space.prefix, uuid::Uuid::new_v4());
    f.store
        .0
        .lock()
        .unwrap()
        .objects
        .insert(bad_key, (b"{bad json".to_vec(), None));
    let result = super::reconcile::refresh(&reader, &client).await.unwrap();
    assert_eq!(result.imported, 1);
    assert_eq!(result.rejected.len(), 1);
    let again = super::reconcile::refresh(&reader, &client).await.unwrap();
    assert_eq!(again.unchanged, 1);
    assert!(reader.list().is_empty());
    let assets = reader.backup_assets().unwrap();
    assert_eq!(assets.len(), 1);
    assert_eq!(assets[0].asset_id, f.id);
    assert!(assets[0].path.is_none());
    assert_eq!(
        super::reconcile::transcript(&reader, &client, &f.id)
            .await
            .unwrap()
            .unwrap()
            .text,
        "portable text"
    );
    assert_eq!(f.store.0.lock().unwrap().video_gets, 0);
    f.store.0.lock().unwrap().fail_list = true;
    assert!(super::reconcile::refresh(&reader, &client).await.is_err());
    assert_eq!(reader.backup_assets().unwrap().len(), 1);
    f.store.0.lock().unwrap().fail_list = false;
    let local = other.path().join("retained.mp4");
    std::fs::write(&local, b"kept").unwrap();
    reader
        .backup_db(|db| {
            db.execute(
                "UPDATE asset_local_files SET path=?1,local_state='present' WHERE asset_id=?2",
                params![local.to_string_lossy(), f.id],
            )?;
            Ok(())
        })
        .unwrap();
    let tombstone = super::record::Record::Deleted(super::record::Deleted {
        schema_version: 1,
        library_id: f.space.library_id.clone(),
        asset_id: f.id.clone(),
        record_revision: 2,
        writer_epoch: 1,
        deleted_at: chrono::Utc::now().to_rfc3339(),
        delete_operation_id: uuid::Uuid::new_v4().to_string(),
    });
    f.store
        .0
        .lock()
        .unwrap()
        .objects
        .insert(key, (serde_json::to_vec(&tombstone).unwrap(), None));
    super::reconcile::refresh(&reader, &client).await.unwrap();
    assert_eq!(reader.backup_assets().unwrap()[0].asset_state, "deleted");
    assert!(local.exists());
    assert_eq!(f.store.0.lock().unwrap().video_gets, 0);
}

#[tokio::test]
async fn remote_record_validation_rejects_cross_prefix_unknown_version_and_bad_sizes() {
    let f = Fixture::new(24).await;
    f.upload().await.unwrap();
    let client = f.client();
    let library = client.connect().await.unwrap();
    let value = client
        .get(&format!("{}records/{}.json", f.space.prefix, f.id))
        .await
        .unwrap()
        .unwrap();
    let original: super::record::Record = serde_json::from_slice(&value.body).unwrap();
    for mutation in 0..4 {
        let mut record = original.clone();
        let super::record::Record::Active(v) = &mut record else {
            panic!()
        };
        match mutation {
            0 => v.media.key = "outside/video.mp4".into(),
            1 => v.schema_version = 99,
            2 => v.media.bytes = 0,
            _ => v.original_filename = "../escape.mp4".into(),
        };
        assert!(record.validate(&library, &f.space.prefix).is_err());
    }
    let mut json: serde_json::Value = serde_json::from_slice(&value.body).unwrap();
    json["future_field"] = true.into();
    let compatible: super::record::Record = serde_json::from_value(json).unwrap();
    compatible.validate(&library, &f.space.prefix).unwrap();
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
            role: Role::Primary,
            writer_device_id: device.clone(),
            writer_epoch: 1,
            credential_ref: None,
            enabled: true,
        };
        core.backup_db(|db|{repository::save_space(db,&space)?;db.execute("INSERT INTO media_assets VALUES(?1,NULL,?2,'video.mp4',NULL,?3,'active')",params![id,serde_json::json!({"platform":"youtube","video_id":"12345678901","title":"测试","completed_at":2}).to_string(),i64::try_from(bytes.max(24))?])?;db.execute("INSERT INTO asset_local_files(asset_id,path,bytes,local_state) VALUES(?1,?2,?3,'present')",params![id,path.to_string_lossy(),i64::try_from(bytes.max(24))?])?;Ok(())}).unwrap();
        let store = Memory::default();
        Client::new(store.clone(), space.clone(), device.clone())
            .create(Library {
                schema_version: 1,
                library_id: space.library_id.clone(),
                mode: "single_writer".into(),
                writer_device_id: device,
                writer_epoch: 1,
                created_at: "2026-10-30T00:00:00Z".into(),
            })
            .await
            .unwrap();
        Self {
            root,
            core,
            store,
            space,
            id,
        }
    }
    fn client(&self) -> Client<Memory> {
        Client::new(
            self.store.clone(),
            self.space.clone(),
            self.core.backup_db(repository::device_id).unwrap(),
        )
    }
    fn operation(&self) -> scheduler::Operation {
        scheduler::enqueue(&self.core).unwrap();
        let op = scheduler::operations(&self.core)
            .unwrap()
            .into_iter()
            .find(|o| o.kind == "upload" && o.state != "completed")
            .unwrap();
        self.core
            .backup_db(|db| {
                db.execute(
                    "UPDATE backup_operations SET state='running' WHERE operation_id=?",
                    [&op.operation_id],
                )?;
                Ok(())
            })
            .unwrap();
        op
    }
    async fn upload(&self) -> Result<()> {
        let op = self.operation();
        let (_tx, rx) = tokio::sync::watch::channel(false);
        scheduler::upload(&self.core, &self.client(), &op, rx).await
    }
}

#[tokio::test]
async fn small_upload_unknown_success_reconciles_and_does_not_republish() {
    let f = Fixture::new(24).await;
    f.store.0.lock().unwrap().fail_after = Some("file");
    assert!(f.upload().await.is_err());
    assert!(!f
        .store
        .0
        .lock()
        .unwrap()
        .objects
        .keys()
        .any(|k| k.contains("records/")));
    f.upload().await.unwrap();
    assert_eq!(f.core.backup_assets().unwrap()[0].asset_id, f.id);
    let writes = f.store.0.lock().unwrap().writes;
    scheduler::enqueue(&f.core).unwrap();
    assert_eq!(scheduler::operations(&f.core).unwrap().len(), 1);
    assert_eq!(f.store.0.lock().unwrap().writes, writes);
    assert!(f.root.path().join("video.mp4").exists());
}

#[tokio::test]
async fn manual_upload_reuses_queue_and_enforces_primary_active_asset() {
    let mut f = Fixture::new(24).await;
    scheduler::request_upload(&f.core, &f.space.space_id, &f.id).unwrap();
    scheduler::request_upload(&f.core, &f.space.space_id, &f.id).unwrap();
    let ops = scheduler::operations(&f.core).unwrap();
    assert_eq!(ops.len(), 1);
    assert_eq!(ops[0].state, "pending");
    let op = f.operation();
    scheduler::request_upload(&f.core, &f.space.space_id, &f.id).unwrap();
    assert_eq!(scheduler::operations(&f.core).unwrap()[0].state, "running");
    scheduler::cancel(&f.core, &op.operation_id).unwrap();
    scheduler::request_upload(&f.core, &f.space.space_id, &f.id).unwrap();
    let resumed = scheduler::operations(&f.core).unwrap();
    assert_eq!(resumed.len(), 1);
    assert_eq!(resumed[0].operation_id, op.operation_id);
    assert_eq!(resumed[0].state, "pending");
    f.space.role = Role::Reader;
    f.core
        .backup_db(|db| repository::save_space(db, &f.space))
        .unwrap();
    assert_eq!(
        scheduler::request_upload(&f.core, &f.space.space_id, &f.id)
            .unwrap_err()
            .to_string(),
        "BACKUP_READ_ONLY"
    );
    f.space.role = Role::Primary;
    f.space.enabled = false;
    f.core
        .backup_db(|db| repository::save_space(db, &f.space))
        .unwrap();
    assert_eq!(
        scheduler::request_upload(&f.core, &f.space.space_id, &f.id)
            .unwrap_err()
            .to_string(),
        "BACKUP_SPACE_INACTIVE"
    );
    f.space.enabled = true;
    f.core
        .backup_db(|db| {
            repository::save_space(db, &f.space)?;
            db.execute(
                "UPDATE media_assets SET asset_state='deleted' WHERE asset_id=?",
                [&f.id],
            )?;
            Ok(())
        })
        .unwrap();
    assert_eq!(
        scheduler::request_upload(&f.core, &f.space.space_id, &f.id)
            .unwrap_err()
            .to_string(),
        "BACKUP_ASSET_DELETED"
    );
}

#[tokio::test]
async fn multipart_recovers_unknown_create_part_and_completion() {
    let f = Fixture::new(100 * 1024 * 1024).await;
    for phase in ["create", "part", "complete"] {
        f.store.0.lock().unwrap().fail_after = Some(phase);
        assert!(f.upload().await.is_err(), "injected {phase}");
    }
    f.upload().await.unwrap();
    assert!(f.store.0.lock().unwrap().uploads.is_empty());
    let s = f.store.0.lock().unwrap();
    let (_, (bytes, hash)) = s
        .objects
        .iter()
        .find(|(k, _)| k.ends_with("video.mp4"))
        .unwrap();
    assert_eq!(Some(tag(bytes)), *hash);
}

#[tokio::test]
async fn reader_probe_has_zero_writes_and_failed_primary_probe_retains_cleanup() {
    let mut f = Fixture::new(24).await;
    let before = f.store.0.lock().unwrap().writes;
    f.space.role = Role::Reader;
    scheduler::test_connection(&f.core, &f.client())
        .await
        .unwrap();
    assert_eq!(f.store.0.lock().unwrap().writes, before);
    f.space.role = Role::Primary;
    f.store.0.lock().unwrap().fail_delete = true;
    assert!(scheduler::test_connection(&f.core, &f.client())
        .await
        .is_err());
    assert!(scheduler::operations(&f.core)
        .unwrap()
        .iter()
        .any(|o| o.kind == "probe" && o.state == "retry_wait"));
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
#[tokio::test]
async fn credential_rotation_returns_no_secret_and_keeps_only_current_reference() {
    let f = Fixture::new(24).await;
    let vault = Vault::default();
    for secret in ["first-secret", "second-secret"] {
        let view = settings::save_with(
            &f.core,
            settings::Update {
                endpoint: f.space.endpoint.clone(),
                bucket: f.space.bucket.clone(),
                prefix: f.space.prefix.clone(),
                role: Role::Primary,
                enabled: true,
                credentials: Some(settings::Credentials {
                    access_key_id: "test-id".into(),
                    secret_access_key: secret.into(),
                }),
            },
            &vault,
            |_, _| Ok(f.store.clone()),
        )
        .await
        .unwrap();
        assert!(!serde_json::to_string(&view).unwrap().contains(secret));
        let editable = settings::credentials(&f.core, &view.space_id, &vault).unwrap();
        assert_eq!(editable.access_key_id, "test-id");
        assert_eq!(editable.secret_access_key, secret);
        assert_eq!(vault.0.lock().unwrap().len(), 1);
    }
}

#[tokio::test]
async fn draft_connection_uses_unsaved_values_without_mutating_settings_or_remote() {
    let f = Fixture::new(24).await;
    let vault = Vault::default();
    let writes = f.store.0.lock().unwrap().writes;
    let draft = |role| settings::Update {
        endpoint: f.space.endpoint.clone(),
        bucket: "draft-bucket".into(),
        prefix: "draft".into(),
        role,
        enabled: false,
        credentials: Some(settings::Credentials {
            access_key_id: "draft-key".into(),
            secret_access_key: "draft-secret".into(),
        }),
    };
    settings::test_draft_with(
        &f.core,
        draft(Role::Primary),
        &vault,
        |space, credentials| {
            assert_eq!(space.bucket, "draft-bucket");
            assert_eq!(space.prefix, "draft/");
            assert_eq!(credentials.secret_access_key, "draft-secret");
            Ok(f.store.clone())
        },
    )
    .await
    .unwrap();
    assert_eq!(
        settings::test_draft_with(&f.core, draft(Role::Reader), &vault, |_, _| Ok(f
            .store
            .clone()))
        .await
        .unwrap_err()
        .to_string(),
        "BACKUP_LIBRARY_NOT_FOUND"
    );
    f.store.0.lock().unwrap().fail_list = true;
    assert!(
        settings::test_draft_with(&f.core, draft(Role::Primary), &vault, |_, _| Ok(f
            .store
            .clone()))
        .await
        .is_err()
    );
    let spaces = f.core.backup_db(repository::spaces).unwrap();
    assert_eq!(spaces.len(), 1);
    assert_eq!(spaces[0].bucket, f.space.bucket);
    assert_eq!(f.store.0.lock().unwrap().writes, writes);
    assert!(scheduler::operations(&f.core).unwrap().is_empty());
    assert!(vault.0.lock().unwrap().is_empty());
}

#[tokio::test]
async fn draft_connection_checks_existing_writer_and_reuses_only_matching_credentials() {
    let f = Fixture::new(24).await;
    let vault = Vault::default();
    let view = settings::save_with(
        &f.core,
        settings::Update {
            endpoint: f.space.endpoint.clone(),
            bucket: f.space.bucket.clone(),
            prefix: f.space.prefix.clone(),
            role: Role::Primary,
            enabled: true,
            credentials: Some(settings::Credentials {
                access_key_id: "stored-key".into(),
                secret_access_key: "stored-secret".into(),
            }),
        },
        &vault,
        |_, _| Ok(f.store.clone()),
    )
    .await
    .unwrap();
    let update = |role| settings::Update {
        endpoint: f.space.endpoint.clone(),
        bucket: f.space.bucket.clone(),
        prefix: f.space.prefix.clone(),
        role,
        enabled: true,
        credentials: None,
    };
    settings::test_draft_with(&f.core, update(Role::Reader), &vault, |_, credentials| {
        assert_eq!(credentials.secret_access_key, "stored-secret");
        Ok(f.store.clone())
    })
    .await
    .unwrap();
    let mut other = update(Role::Primary);
    other.prefix = "other/".into();
    assert_eq!(
        settings::test_draft_with::<Memory>(&f.core, other, &vault, |_, _| panic!(
            "must not reuse credentials across targets"
        ))
        .await
        .unwrap_err()
        .to_string(),
        "BACKUP_CREDENTIALS_MISSING"
    );
    let key = format!("{}library.json", f.space.prefix);
    {
        let mut state = f.store.0.lock().unwrap();
        let object = state.objects.get_mut(&key).unwrap();
        let mut library: Library = serde_json::from_slice(&object.0).unwrap();
        library.writer_device_id = uuid::Uuid::new_v4().to_string();
        object.0 = serde_json::to_vec(&library).unwrap();
    }
    assert!(
        settings::test_draft_with(&f.core, update(Role::Primary), &vault, |_, _| Ok(f
            .store
            .clone()))
        .await
        .is_err()
    );
    assert_eq!(
        settings::get(&f.core).unwrap().unwrap().space_id,
        view.space_id
    );
}

#[tokio::test]
async fn expired_upload_restarts_and_cancel_aborts_only_its_own_parts() {
    let f = Fixture::new(100 * 1024 * 1024).await;
    f.store.0.lock().unwrap().fail_after = Some("part");
    assert!(f.upload().await.is_err());
    f.store.0.lock().unwrap().uploads.clear();
    f.store.0.lock().unwrap().fail_after = Some("part");
    assert!(f.upload().await.is_err());
    let op = f.operation();
    scheduler::cancel(&f.core, &op.operation_id).unwrap();
    scheduler::cleanup_canceled(&f.core, &f.client(), &op)
        .await
        .unwrap();
    assert!(f.store.0.lock().unwrap().uploads.is_empty());
    assert!(!f
        .store
        .0
        .lock()
        .unwrap()
        .objects
        .keys()
        .any(|k| k.contains("records/")));
}
#[tokio::test]
async fn changed_source_aborts_previous_parts_before_publishing_new_content() {
    use std::io::Write as _;
    let f = Fixture::new(100 * 1024 * 1024).await;
    f.store.0.lock().unwrap().fail_after = Some("part");
    assert!(f.upload().await.is_err());
    let old = f
        .store
        .0
        .lock()
        .unwrap()
        .uploads
        .keys()
        .next()
        .unwrap()
        .clone();
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .open(f.root.path().join("video.mp4"))
        .unwrap();
    file.seek(std::io::SeekFrom::Start(30)).unwrap();
    file.write_all(b"changed").unwrap();
    drop(file);
    f.upload().await.unwrap();
    assert!(!f.store.0.lock().unwrap().uploads.contains_key(&old));
}

#[tokio::test]
async fn disabled_space_cancellation_does_not_block_current_queue() {
    let f = Fixture::new(24).await;
    let op = f.operation();
    scheduler::cancel(&f.core, &op.operation_id).unwrap();
    assert_eq!(scheduler::cleanup_candidates(&f.core).unwrap().len(), 1);
    f.core
        .backup_db(|db| {
            db.execute("UPDATE backup_spaces SET enabled=0", [])?;
            Ok(())
        })
        .unwrap();
    assert!(scheduler::cleanup_candidates(&f.core).unwrap().is_empty());
}

#[tokio::test]
async fn transcript_republish_failure_preserves_previous_record_object() {
    let f = Fixture::new(24).await;
    f.core.verify_backup_asset(&f.id).await.unwrap();
    let hash = f.core.backup_assets().unwrap()[0]
        .content_sha256
        .clone()
        .unwrap();
    f.core.backup_db(|db|{db.execute("INSERT INTO asset_transcripts(asset_id,content_sha256,text,model_revision,pipeline_revision,completed_at) VALUES(?1,?2,'old text','model','pipeline',1)",params![f.id,hash])?;Ok(())}).unwrap();
    f.upload().await.unwrap();
    let key = format!("{}records/{}.json", f.space.prefix, f.id);
    let old = f.client().get(&key).await.unwrap().unwrap();
    f.core
        .backup_db(|db| {
            db.execute(
                "UPDATE asset_transcripts SET text='new text' WHERE asset_id=?",
                [&f.id],
            )?;
            Ok(())
        })
        .unwrap();
    f.store.0.lock().unwrap().fail_record = true;
    assert!(f.upload().await.is_err());
    assert_eq!(f.client().get(&key).await.unwrap().unwrap().body, old.body);
    let super::record::Record::Active(record) = serde_json::from_slice(&old.body).unwrap() else {
        panic!("active expected")
    };
    let reference = record.transcript.unwrap();
    let body = f.client().get(&reference.key).await.unwrap().unwrap().body;
    assert_eq!(tag(&body), reference.sha256);
    f.store.0.lock().unwrap().fail_record = false;
    f.upload().await.unwrap();
    super::lifecycle::delete(&f.core, &f.space.space_id, &f.id)
        .await
        .unwrap();
    super::lifecycle::remote(&f.core, &f.client(), &deletion(&f.core))
        .await
        .unwrap();
    assert_eq!(f.store.0.lock().unwrap().objects.len(), 2); // all transcript versions removed
}
#[tokio::test]
async fn disable_existing_space_requires_no_credentials_or_network() {
    let f = Fixture::new(24).await;
    let view = settings::save_with(
        &f.core,
        settings::Update {
            endpoint: f.space.endpoint.clone(),
            bucket: f.space.bucket.clone(),
            prefix: f.space.prefix.clone(),
            role: Role::Primary,
            enabled: false,
            credentials: None,
        },
        &Vault::default(),
        |_, _| -> Result<Memory> { panic!("must not initialize transport") },
    )
    .await
    .unwrap();
    assert!(!view.enabled);
}

#[tokio::test]
async fn reading_old_remote_record_preserves_new_local_transcript() {
    let f = Fixture::new(24).await;
    f.core.verify_backup_asset(&f.id).await.unwrap();
    let hash = f.core.backup_assets().unwrap()[0]
        .content_sha256
        .clone()
        .unwrap();
    f.core.backup_db(|db|{db.execute("INSERT INTO asset_transcripts(asset_id,content_sha256,text,model_revision,pipeline_revision,completed_at) VALUES(?1,?2,'old','m','p',1)",params![f.id,hash])?;Ok(())}).unwrap();
    f.upload().await.unwrap();
    f.core
        .backup_db(|db| {
            db.execute(
                "UPDATE asset_transcripts SET text='new local text',source_json='{}',content_sha256='new-local-content-hash'",
                [],
            )?;
            Ok(())
        })
        .unwrap();
    assert_eq!(
        super::reconcile::transcript(&f.core, &f.client(), &f.id)
            .await
            .unwrap()
            .unwrap()
            .text,
        "new local text"
    );
    let mut space = f.space.clone();
    space.role = Role::Reader;
    f.core
        .backup_db(|db| repository::save_space(db, &space))
        .unwrap();
    f.store.0.lock().unwrap().objects.clear();
    let client = Client::new(f.store.clone(), space, "reader".into());
    assert_eq!(
        super::reconcile::transcript(&f.core, &client, &f.id)
            .await
            .unwrap()
            .unwrap()
            .text,
        "new local text"
    );
    assert_eq!(
        f.core.asset_transcript(&f.id).unwrap().unwrap().text,
        "new local text"
    );
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
impl Fixture {
    async fn reader(&self) -> (tempfile::TempDir, Arc<Core>, Client<Memory>) {
        let root = tempfile::tempdir().unwrap();
        let core = Core::open(
            &root.path().join("app.sqlite"),
            root.path().join("downloads"),
            "unused".into(),
            vec![],
            |_| {},
        )
        .unwrap();
        let mut space = self.space.clone();
        space.role = Role::Reader;
        core.backup_db(|db| repository::save_space(db, &space))
            .unwrap();
        let client = Client::new(
            self.store.clone(),
            space,
            core.backup_db(repository::device_id).unwrap(),
        );
        super::reconcile::refresh(&core, &client).await.unwrap();
        (root, core, client)
    }
}
#[tokio::test]
async fn restore_concurrent_consumers_download_once_and_local_hit_needs_no_network() {
    let f = Fixture::new(24).await;
    f.upload().await.unwrap();
    let (_root, core, client) = f.reader().await;
    let (a, b, c) = tokio::join!(
        super::restore::ensure_with(&core, &client, &f.id),
        super::restore::ensure_with(&core, &client, &f.id),
        super::restore::ensure_with(&core, &client, &f.id)
    );
    let path = a.unwrap();
    assert_eq!(path, b.unwrap());
    assert_eq!(path, c.unwrap());
    assert_eq!(f.store.0.lock().unwrap().video_gets, 1);
    f.store.0.lock().unwrap().objects.clear();
    assert_eq!(
        super::restore::ensure_with(&core, &client, &f.id)
            .await
            .unwrap(),
        path
    );
    assert!(core.list().is_empty());
}
#[tokio::test]
async fn restore_resumes_partial_bytes_without_overwriting_same_name() {
    let f = Fixture::new(2 * 1024 * 1024).await;
    f.upload().await.unwrap();
    let (root, core, client) = f.reader().await;
    let directory = root.path().join("downloads");
    std::fs::create_dir_all(&directory).unwrap();
    let existing = directory.join("video.mp4");
    std::fs::write(&existing, b"user-owned").unwrap();
    f.store.0.lock().unwrap().break_stream = Some(4096);
    assert!(super::restore::ensure_with(&core, &client, &f.id)
        .await
        .is_err());
    drop(core);
    let core = Core::open(
        &root.path().join("app.sqlite"),
        root.path().join("downloads"),
        "unused".into(),
        vec![],
        |_| {},
    )
    .unwrap();
    let path = super::restore::ensure_with(&core, &client, &f.id)
        .await
        .unwrap();
    assert_ne!(path, existing);
    assert_eq!(std::fs::read(&existing).unwrap(), b"user-owned");
    assert_eq!(f.store.0.lock().unwrap().stream_offsets, vec![0, 4096]);
}
#[tokio::test]
async fn restore_rejects_corrupt_content_and_recovers_verified_committed_file() {
    let f = Fixture::new(24).await;
    f.upload().await.unwrap();
    let (root, core, client) = f.reader().await;
    f.store.0.lock().unwrap().corrupt_stream = true;
    assert_eq!(
        super::restore::ensure_with(&core, &client, &f.id)
            .await
            .unwrap_err()
            .to_string(),
        "BACKUP_CHECKSUM_MISMATCH"
    );
    assert!(core.backup_assets().unwrap()[0].path.is_none());
    f.store.0.lock().unwrap().corrupt_stream = false;
    let op = scheduler::operations(&core)
        .unwrap()
        .into_iter()
        .find(|o| o.kind == "restore")
        .unwrap();
    let target = root
        .path()
        .join("downloads/committed.mp4")
        .canonicalize()
        .unwrap_or_else(|_| {
            root.path()
                .join("downloads")
                .canonicalize()
                .unwrap()
                .join("committed.mp4")
        });
    std::fs::copy(f.root.path().join("video.mp4"), &target).unwrap();
    let mut payload = op.payload;
    payload["target"] = target.to_string_lossy().into_owned().into();
    core.backup_db(|db| {
        db.execute(
            "UPDATE backup_operations SET payload_json=?1,phase='committing' WHERE operation_id=?2",
            params![payload.to_string(), op.operation_id],
        )?;
        Ok(())
    })
    .unwrap();
    let before = f.store.0.lock().unwrap().video_gets;
    assert_eq!(
        super::restore::ensure_with(&core, &client, &f.id)
            .await
            .unwrap(),
        target
    );
    assert_eq!(f.store.0.lock().unwrap().video_gets, before);
}

#[tokio::test]
async fn restored_reader_video_reuses_existing_transcription_engine_without_remote_writes() {
    let f = Fixture::new(24).await;
    f.upload().await.unwrap();
    let (root, core, client) = f.reader().await;
    super::restore::ensure_with(&core, &client, &f.id)
        .await
        .unwrap();
    drop(core);
    let project = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap();
    let python = project.join(if cfg!(windows) {
        ".venv/Scripts/python.exe"
    } else {
        ".venv/bin/python"
    });
    let core = Core::open(
        &root.path().join("app.sqlite"),
        root.path().join("downloads"),
        python.to_string_lossy().into_owned(),
        vec![project
            .join("tests/fixtures/desktop_engine.py")
            .to_string_lossy()
            .into_owned()],
        |_| {},
    )
    .unwrap();
    core.start_transcription(&f.id, false).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let value = core.transcript(&f.id).unwrap().unwrap();
            if value.state != "running" {
                assert_eq!(value.state, "completed", "{:?}", value.error_code);
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        core.asset_transcript(&f.id).unwrap().unwrap().text,
        "欢迎体验。\nEnglish text."
    );
    assert!(core.list().is_empty());
    let before = f.store.0.lock().unwrap().writes;
    scheduler::enqueue(&core).unwrap();
    assert_eq!(f.store.0.lock().unwrap().writes, before);
    let old = core.transcript(&f.id).unwrap().unwrap();
    let path = core.backup_assets().unwrap()[0].path.clone().unwrap();
    std::fs::remove_file(&path).unwrap();
    std::fs::write(&path, b"user file must be preserved").unwrap();
    // The explicit missing-path fixture models a released copy while reserving its old name.
    core.backup_db(|db| {
        db.execute(
            "UPDATE asset_local_files SET path=NULL,local_state='missing' WHERE asset_id=?",
            [&f.id],
        )?;
        Ok(())
    })
    .unwrap();
    super::restore::ensure_with(&core, &client, &f.id)
        .await
        .unwrap();
    assert!(!core.transcript(&f.id).unwrap().unwrap().source_changed);
    let reused = core.start_transcription(&f.id, false).unwrap();
    assert_eq!(reused.operation_id, old.operation_id);
    assert_eq!(reused.state, "completed");
    core.shutdown().await;
}
#[tokio::test]
async fn cached_active_record_cannot_restore_a_remote_tombstone() {
    let f = Fixture::new(24).await;
    f.upload().await.unwrap();
    let (_root, core, client) = f.reader().await;
    let tombstone = super::record::Record::Deleted(super::record::Deleted {
        schema_version: 1,
        library_id: f.space.library_id.clone(),
        asset_id: f.id.clone(),
        record_revision: 2,
        writer_epoch: 1,
        deleted_at: chrono::Utc::now().to_rfc3339(),
        delete_operation_id: uuid::Uuid::new_v4().to_string(),
    });
    f.store.0.lock().unwrap().objects.insert(
        format!("{}records/{}.json", f.space.prefix, f.id),
        (serde_json::to_vec(&tombstone).unwrap(), None),
    );
    assert_eq!(
        super::restore::ensure_with(&core, &client, &f.id)
            .await
            .unwrap_err()
            .to_string(),
        "BACKUP_ASSET_DELETED"
    );
    assert_eq!(f.store.0.lock().unwrap().video_gets, 0);
}
#[tokio::test]
async fn restore_checks_media_header_even_when_digest_and_size_match() {
    let f = Fixture::new(24).await;
    f.upload().await.unwrap();
    let key = format!("{}records/{}.json", f.space.prefix, f.id);
    let old = f.client().get(&key).await.unwrap().unwrap();
    let mut record: super::record::Record = serde_json::from_slice(&old.body).unwrap();
    let super::record::Record::Active(v) = &mut record else {
        panic!()
    };
    let bytes = vec![0u8; 24];
    v.media.sha256 = tag(&bytes);
    v.media.key = format!(
        "{}media/{}/{}/video.mp4",
        f.space.prefix, f.id, v.media.sha256
    );
    f.store
        .0
        .lock()
        .unwrap()
        .objects
        .insert(v.media.key.clone(), (bytes, Some(v.media.sha256.clone())));
    f.store
        .0
        .lock()
        .unwrap()
        .objects
        .insert(key, (serde_json::to_vec(&record).unwrap(), None));
    let (_root, core, client) = f.reader().await;
    assert_eq!(
        super::restore::ensure_with(&core, &client, &f.id)
            .await
            .unwrap_err()
            .to_string(),
        "INVALID_MEDIA_CONTENT"
    );
    assert!(core.backup_assets().unwrap()[0].path.is_none());
}

#[tokio::test]
async fn local_video_does_not_wait_for_the_remote_restore_slot() {
    let f = Fixture::new(24).await;
    let _slot = f.core.backup_restore_lock.lock().await;
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        super::restore::ensure_with(&f.core, &f.client(), &f.id),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(result.exists());
    assert_eq!(f.store.0.lock().unwrap().video_gets, 0);
}

#[tokio::test]
async fn cancel_restore_persists_missing_state_and_keeps_valid_local_files() {
    let f = Fixture::new(24).await;
    f.core.backup_db(|db|{db.execute("UPDATE asset_local_files SET local_state='restoring' WHERE asset_id=?",[&f.id])?;db.execute("INSERT INTO backup_operations(operation_id,space_id,asset_id,kind,state,phase,payload_json) VALUES('restore',?1,?2,'restore','running','downloading','{}')",params![f.space.space_id,f.id])?;Ok(())}).unwrap();
    f.core.cancel_asset_restore(&f.id).unwrap();
    assert_eq!(f.core.backup_assets().unwrap()[0].local_state, "missing");
    assert_eq!(scheduler::operations(&f.core).unwrap()[0].state, "canceled");
    assert!(f.root.path().join("video.mp4").is_file());
}

fn deletion(core: &Core) -> scheduler::Operation {
    scheduler::operations(core)
        .unwrap()
        .into_iter()
        .find(|o| o.kind == "delete" && o.state != "completed")
        .unwrap()
}
#[tokio::test]
async fn lifecycle_offline_delete_then_tombstone_and_exact_cleanup() {
    let f = Fixture::new(24).await;
    f.upload().await.unwrap();
    let (_root, reader, client) = f.reader().await;
    let local = super::restore::ensure_with(&reader, &client, &f.id)
        .await
        .unwrap();
    let unrelated = "test/media/another-asset/keep.mp4";
    f.store
        .0
        .lock()
        .unwrap()
        .objects
        .insert(unrelated.into(), (vec![1], None));
    let before = f.store.0.lock().unwrap().writes;
    super::lifecycle::delete(&f.core, &f.space.space_id, &f.id)
        .await
        .unwrap();
    assert_eq!(f.store.0.lock().unwrap().writes, before);
    assert!(!f.root.path().join("video.mp4").exists());
    assert_eq!(
        super::reconcile::list(&f.core, &f.space.space_id).unwrap()[0].remote_state,
        "pending_delete"
    );
    f.store.0.lock().unwrap().fail_delete = true;
    let op = deletion(&f.core);
    assert!(super::lifecycle::remote(&f.core, &f.client(), &op)
        .await
        .is_err());
    super::reconcile::refresh(&f.core, &f.client())
        .await
        .unwrap();
    assert_eq!(
        super::reconcile::list(&f.core, &f.space.space_id).unwrap()[0].remote_state,
        "pending_delete"
    );
    super::reconcile::refresh(&reader, &client).await.unwrap();
    assert_eq!(reader.backup_assets().unwrap()[0].asset_state, "deleted");
    assert!(local.exists());
    f.store.0.lock().unwrap().fail_delete = false;
    super::lifecycle::remote(&f.core, &f.client(), &op)
        .await
        .unwrap();
    super::lifecycle::remote(&f.core, &f.client(), &op)
        .await
        .unwrap();
    let store = f.store.0.lock().unwrap();
    assert_eq!(store.objects.len(), 3); // library, minimal tombstone, unrelated
    let raw = &store.objects[&format!("test/records/{}.json", f.id)].0;
    let record: serde_json::Value = serde_json::from_slice(raw).unwrap();
    assert_eq!(record["state"], "deleted");
    assert!(record.get("title").is_none());
    assert!(record.get("media").is_none());
    assert!(store.objects.contains_key(unrelated));
}
#[tokio::test]
async fn lifecycle_local_failure_stops_remote_and_missing_file_can_retry() {
    let f = Fixture::new(24).await;
    f.upload().await.unwrap();
    let path = f.root.path().join("video.mp4");
    std::fs::write(&path, b"replacement unrelated content").unwrap();
    let before = f.store.0.lock().unwrap().writes;
    assert!(super::lifecycle::delete(&f.core, &f.space.space_id, &f.id)
        .await
        .is_err());
    let op = deletion(&f.core);
    assert_eq!(op.state, "blocked");
    assert!(super::lifecycle::remote(&f.core, &f.client(), &op)
        .await
        .is_err());
    assert_eq!(f.store.0.lock().unwrap().writes, before);
    assert_eq!(
        std::fs::read(&path).unwrap(),
        b"replacement unrelated content"
    );
    std::fs::remove_file(path).unwrap();
    super::lifecycle::remote(&f.core, &f.client(), &op)
        .await
        .unwrap();
}
#[tokio::test]
async fn lifecycle_release_and_reader_permissions_never_write_remote() {
    let f = Fixture::new(24).await;
    f.upload().await.unwrap();
    let (_root, reader, client) = f.reader().await;
    let path = super::restore::ensure_with(&reader, &client, &f.id)
        .await
        .unwrap();
    let writes = f.store.0.lock().unwrap().writes;
    assert_eq!(
        super::lifecycle::delete(&reader, &f.space.space_id, &f.id)
            .await
            .unwrap_err()
            .to_string(),
        "BACKUP_READ_ONLY"
    );
    assert!(path.exists());
    super::lifecycle::release(&reader, &f.id).await.unwrap();
    assert!(!path.exists());
    assert!(super::restore::ensure_with(&reader, &client, &f.id)
        .await
        .unwrap()
        .exists());
    super::lifecycle::release(&f.core, &f.id).await.unwrap();
    assert_eq!(f.store.0.lock().unwrap().writes, writes);
    assert_eq!(f.core.backup_assets().unwrap()[0].asset_state, "active");
}
#[tokio::test]
async fn lifecycle_delete_before_publication_prevents_old_upload_revival() {
    let f = Fixture::new(24).await;
    f.store.0.lock().unwrap().fail_after = Some("file");
    assert!(f.upload().await.is_err());
    let old = scheduler::operations(&f.core).unwrap()[0].clone();
    super::lifecycle::delete(&f.core, &f.space.space_id, &f.id)
        .await
        .unwrap();
    let op = deletion(&f.core);
    super::lifecycle::remote(&f.core, &f.client(), &op)
        .await
        .unwrap();
    let (_tx, rx) = tokio::sync::watch::channel(false);
    assert!(scheduler::upload(&f.core, &f.client(), &old, rx)
        .await
        .is_err());
    assert!(scheduler::retry(&f.core, &old.operation_id).is_err());
    assert_eq!(f.store.0.lock().unwrap().objects.len(), 2);
}

#[tokio::test]
async fn lifecycle_restart_finishes_durable_delete_intent() {
    let f = Fixture::new(24).await;
    f.upload().await.unwrap();
    super::lifecycle::intent(&f.core, &f.space.space_id, &f.id).unwrap();
    let Fixture {
        root,
        core,
        store,
        space,
        id,
    } = f;
    drop(core);
    let core = Core::open(
        &root.path().join("app.sqlite"),
        root.path().join("downloads"),
        "unused".into(),
        vec![],
        |_| {},
    )
    .unwrap();
    let client = Client::new(
        store.clone(),
        space,
        core.backup_db(repository::device_id).unwrap(),
    );
    super::lifecycle::remote(&core, &client, &deletion(&core))
        .await
        .unwrap();
    assert!(!root.path().join("video.mp4").exists());
    assert_eq!(
        core.backup_assets()
            .unwrap()
            .into_iter()
            .find(|a| a.asset_id == id)
            .unwrap()
            .local_state,
        "missing"
    );
    assert_eq!(store.0.lock().unwrap().objects.len(), 2);
}
#[tokio::test]
async fn lifecycle_release_removes_only_owned_partial_restore() {
    let f = Fixture::new(8192).await;
    f.upload().await.unwrap();
    let (_root, core, client) = f.reader().await;
    f.store.0.lock().unwrap().break_stream = Some(4096);
    assert!(super::restore::ensure_with(&core, &client, &f.id)
        .await
        .is_err());
    let op = scheduler::operations(&core)
        .unwrap()
        .into_iter()
        .find(|o| o.kind == "restore")
        .unwrap();
    let part = std::path::PathBuf::from(op.payload["part"].as_str().unwrap());
    assert!(part.exists());
    let other = part.parent().unwrap().join("another-operation.part");
    std::fs::write(&other, b"keep").unwrap();
    let writes = f.store.0.lock().unwrap().writes;
    super::lifecycle::release(&core, &f.id).await.unwrap();
    assert!(!part.exists());
    assert!(other.exists());
    assert_eq!(f.store.0.lock().unwrap().writes, writes);
}

#[tokio::test]
async fn lifecycle_setting_change_stops_in_progress_remote_cleanup() {
    for reader in [false, true] {
        let f = Fixture::new(24).await;
        f.upload().await.unwrap();
        super::lifecycle::delete(&f.core, &f.space.space_id, &f.id)
            .await
            .unwrap();
        let before = f.store.0.lock().unwrap().writes;
        f.store.0.lock().unwrap().stop_after_tombstone =
            Some((Arc::downgrade(&f.core), f.space.space_id.clone(), reader));
        assert!(
            super::lifecycle::remote(&f.core, &f.client(), &deletion(&f.core))
                .await
                .is_err()
        );
        assert_eq!(f.store.0.lock().unwrap().writes, before + 1); // tombstone only, zero Delete/Abort
        assert_eq!(deletion(&f.core).state, "blocked");
        assert_eq!(
            super::reconcile::list(&f.core, &f.space.space_id).unwrap()[0].remote_state,
            "pending_delete"
        );
    }
}

#[tokio::test]
async fn restore_range_ignored_restarts_from_zero_without_appending_full_object() {
    let f = Fixture::new(8192).await;
    f.upload().await.unwrap();
    let (_root, core, client) = f.reader().await;
    f.store.0.lock().unwrap().break_stream = Some(4096);
    assert!(super::restore::ensure_with(&core, &client, &f.id)
        .await
        .is_err());
    f.store.0.lock().unwrap().ignore_range = true;
    let path = super::restore::ensure_with(&core, &client, &f.id)
        .await
        .unwrap();
    assert_eq!(
        std::fs::read(path).unwrap(),
        std::fs::read(f.root.path().join("video.mp4")).unwrap()
    );
    assert_eq!(f.store.0.lock().unwrap().stream_offsets, vec![0, 4096, 0]);
}
#[tokio::test]
async fn restore_changed_remote_version_discards_old_partial_bytes() {
    let f = Fixture::new(8192).await;
    f.upload().await.unwrap();
    let (_root, core, client) = f.reader().await;
    f.store.0.lock().unwrap().break_stream = Some(4096);
    assert!(super::restore::ensure_with(&core, &client, &f.id)
        .await
        .is_err());
    let path = f.root.path().join("video.mp4");
    let mut body = std::fs::read(&path).unwrap();
    body[4090] = 42;
    std::fs::write(&path, &body).unwrap();
    f.core.verify_backup_asset(&f.id).await.unwrap();
    f.upload().await.unwrap();
    let restored = super::restore::ensure_with(&core, &client, &f.id)
        .await
        .unwrap();
    assert_eq!(std::fs::read(restored).unwrap(), body);
    assert_eq!(f.store.0.lock().unwrap().stream_offsets, vec![0, 0]);
}
#[tokio::test]
async fn lifecycle_tombstone_conflict_and_unknown_success_reconcile() {
    for unknown in [false, true] {
        let f = Fixture::new(24).await;
        f.upload().await.unwrap();
        super::lifecycle::delete(&f.core, &f.space.space_id, &f.id)
            .await
            .unwrap();
        let op = deletion(&f.core);
        if unknown {
            f.store.0.lock().unwrap().fail_after = Some("put");
            assert!(super::lifecycle::remote(&f.core, &f.client(), &op)
                .await
                .is_err());
        } else {
            f.store.0.lock().unwrap().conflict_record_once = true;
        }
        super::lifecycle::remote(&f.core, &f.client(), &op)
            .await
            .unwrap();
        assert_eq!(f.store.0.lock().unwrap().objects.len(), 2);
    }
}
#[tokio::test]
async fn hidden_history_stays_local_and_survives_remote_refresh() {
    let f = Fixture::new(24).await;
    f.upload().await.unwrap();
    let (_root, core, client) = f.reader().await;
    let writes = f.store.0.lock().unwrap().writes;
    super::lifecycle::hide_history(&core, &f.id).unwrap();
    super::reconcile::refresh(&core, &client).await.unwrap();
    assert!(super::reconcile::list(&core, &f.space.space_id)
        .unwrap()
        .is_empty());
    assert_eq!(core.backup_assets().unwrap().len(), 1);
    assert_eq!(f.store.0.lock().unwrap().writes, writes);
}

#[tokio::test]
async fn retry_policy_persists_403_block_and_429_5xx_bounded_backoff() {
    let f = Fixture::new(24).await;
    let op = f.operation();
    for (code, expected) in [
        ("BACKUP_ACCESS_DENIED", "blocked"),
        ("BACKUP_RATE_LIMITED", "retry_wait"),
        ("BACKUP_SERVICE_UNAVAILABLE", "retry_wait"),
        ("BACKUP_PRECONDITION_FAILED", "retry_wait"),
    ] {
        f.core
            .backup_db(|db| {
                db.execute(
                    "UPDATE backup_operations SET state='running',attempt=1 WHERE operation_id=?",
                    [&op.operation_id],
                )?;
                Ok(())
            })
            .unwrap();
        scheduler::record_failure(&f.core, &op, code).unwrap();
        let current = scheduler::operations(&f.core).unwrap().remove(0);
        assert_eq!(current.state, expected);
        assert_eq!(current.last_error.as_deref(), Some(code));
        let delay: i64 = f
            .core
            .backup_db(|db| {
                Ok(db.query_row(
                    "SELECT next_retry_at-unixepoch() FROM backup_operations WHERE operation_id=?",
                    [&op.operation_id],
                    |r| r.get(0),
                )?)
            })
            .unwrap();
        assert!((0..=3).contains(&delay));
    }
    f.core
        .backup_db(|db| {
            db.execute(
                "UPDATE backup_operations SET state='running' WHERE operation_id=?",
                [&op.operation_id],
            )?;
            Ok(())
        })
        .unwrap();
    let mut exhausted = op.clone();
    exhausted.attempt = 4;
    scheduler::record_failure(&f.core, &exhausted, "BACKUP_RATE_LIMITED").unwrap();
    assert_eq!(scheduler::operations(&f.core).unwrap()[0].state, "blocked");
}
