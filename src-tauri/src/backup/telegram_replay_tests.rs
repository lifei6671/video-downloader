use super::super::{
    model::Space,
    repository,
    sync_engine::{self, SyncEngine},
    sync_read,
};
use super::super::{
    telegram_lifecycle,
    telegram_transfer::{Envelope, Io, RemoteDocument, Transport},
};
use super::*;
use crate::desktop_core::Core;
use rusqlite::params;
use serde_json::json;
use std::{
    path::Path,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
};

#[derive(Default)]
struct Memory {
    next: i32,
    events: Vec<(i32, Vec<u8>)>,
    media: BTreeMap<i32, Vec<u8>>,
    documents: BTreeMap<i32, RemoteDocument>,
    receipts: BTreeMap<(String, String), MediaRef>,
}

#[tokio::test]
async fn historical_missing_document_id_is_resolved_without_changing_cursor_bytes() {
    let root = tempfile::tempdir().unwrap();
    let core = open(root.path());
    let net = Arc::new(Remote::default());
    let provider = setup(&core, net.clone(), None);
    let engine = SyncEngine::new(core.clone(), provider.clone());
    seed(&core, root.path());
    engine.execute_upload(&claim(&core, &engine)).await.unwrap();
    let (revision, body, documents) = {
        let data = net.data.lock().unwrap();
        let (id, body) = data.events[0].clone();
        (
            i64::from(id),
            body,
            data.documents.values().cloned().collect::<Vec<_>>(),
        )
    };
    let mut raw: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let expected = raw["change"]["media"]["location"]["document_id"].clone();
    raw["change"]["media"]["location"]
        .as_object_mut()
        .unwrap()
        .remove("document_id");
    let historical = serde_json::to_vec(&raw).unwrap();
    assert!(Event::decode(&provider.target, &historical).is_err());
    assert!(Event::decode_with_documents(&provider.target, &historical, &[]).is_err());
    let decoded = Event::decode_with_documents(&provider.target, &historical, &documents).unwrap();
    assert_eq!(
        serde_json::to_value(decoded.change.media.as_ref().unwrap()).unwrap()["location"]
            ["document_id"],
        expected
    );
    let projected = replay_decoded(
        &provider.target,
        revision,
        vec![(revision, historical.clone())],
        None,
        |bytes| Event::decode_with_documents(&provider.target, bytes, &documents),
    )
    .unwrap();
    assert_eq!(
        projected.cursor.evidence[&revision],
        format!("{:x}", Sha256::digest(&historical))
    );
    let mut changed = documents.clone();
    for document in &mut changed {
        document.bytes += 1;
    }
    assert!(Event::decode_with_documents(&provider.target, &historical, &changed).is_err());
    let mut deletion = projected.assets.values().next().unwrap().clone();
    deletion.deleted = true;
    deletion.revision = 100;
    let mut recreated = decoded.change;
    recreated.recreate = true;
    recreated.base = Some(deletion.clone());
    recreated.cover = ReferenceChange::Clear;
    recreated.transcript = ReferenceChange::Clear;
    if let MediaLocation::TelegramMessage { message_id, .. } =
        &mut recreated.media.as_mut().unwrap().location
    {
        *message_id = 50;
    }
    assert!(validate_incarnation(&recreated, Some(&deletion), Some(100)).is_err());
    if let MediaLocation::TelegramMessage { message_id, .. } =
        &mut recreated.media.as_mut().unwrap().location
    {
        *message_id = 101;
    }
    validate_incarnation(&recreated, Some(&deletion), Some(100)).unwrap();
    let mut edit = Event::decode_with_documents(&provider.target, &historical, &documents)
        .unwrap()
        .change;
    edit.base = Some(projected.assets.values().next().unwrap().clone());
    edit.media = None;
    edit.fields.clear();
    edit.cover = ReferenceChange::Unchanged;
    edit.transcript = ReferenceChange::Unchanged;
    let body = Event::encode(&provider.target, &edit).unwrap();
    let revision = revision + 1;
    let recovered = replay(
        &provider.target,
        revision,
        vec![(revision, body.clone())],
        None,
    )
    .unwrap();
    assert_eq!(
        recovered.assets.values().next().unwrap().media,
        projected.assets.values().next().unwrap().media
    );
    assert!(
        replay(
            &provider.target,
            revision,
            vec![(revision, body)],
            Some(&projected.cursor)
        )
        .is_err(),
        "a base snapshot never replaces missing observed byte evidence"
    );
    core.shutdown().await;
}

#[tokio::test]
async fn telegram_message_updates_replay_latest_transcript_and_preserve_history() {
    let root = tempfile::tempdir().unwrap();
    let core = open(root.path());
    let net = Arc::new(Remote::default());
    let p = setup(&core, net.clone(), None);
    let engine = SyncEngine::new(core.clone(), p.clone());
    let (asset, _) = seed(&core, root.path());
    engine.execute_upload(&claim(&core, &engine)).await.unwrap();
    let initial = net.data.lock().unwrap().events[0].clone();
    let first_change = Event::decode(&p.target, &initial.1).unwrap().change;
    let physical = p
        .target
        .message_id(first_change.media.as_ref().unwrap())
        .unwrap();
    let initial_revision = (i64::from(physical) << 32) | 1;
    let original = replay(
        &p.target,
        initial_revision,
        vec![(initial_revision, initial.1.clone())],
        None,
    )
    .unwrap();
    let base = original.assets[&asset].clone();
    let physical = p.target.message_id(base.media.as_ref().unwrap()).unwrap();
    let mut change = Event::decode(&p.target, &initial.1).unwrap().change;
    change.base = Some(base.clone());
    change.fields.clear();
    change.field_bases.clear();
    change.media = None;
    change.cover = ReferenceChange::Unchanged;
    change.operation_id = id();
    change.conflicts.clear();
    change.resolved_conflicts.clear();
    let mut transcript = base.media.clone().unwrap();
    transcript.bytes = 64;
    transcript.sha256 = "ab".repeat(32);
    transcript.container = "json".into();
    if let MediaLocation::TelegramMessage {
        message_id,
        document_id,
        ..
    } = &mut transcript.location
    {
        *message_id = i64::from(physical);
        *document_id = 9001;
    }
    change.transcript = ReferenceChange::Set(transcript.clone());
    let revision1 = (i64::from(physical) << 32) | 2;
    let body1 = Event::encode(&p.target, &change).unwrap();
    let first = replay(
        &p.target,
        revision1,
        vec![
            (initial_revision, initial.1.clone()),
            (revision1, body1.clone()),
        ],
        Some(&original.cursor),
    )
    .unwrap();
    assert_eq!(first.assets[&asset].transcript.as_ref(), Some(&transcript));
    change.base = Some(first.assets[&asset].clone());
    change.operation_id = id();
    if let MediaLocation::TelegramMessage { document_id, .. } = &mut transcript.location {
        *document_id = 9002;
    }
    transcript.sha256 = "cd".repeat(32);
    change.transcript = ReferenceChange::Set(transcript.clone());
    let revision2 = revision1 + 1;
    let body2 = Event::encode(&p.target, &change).unwrap();
    let second = replay(
        &p.target,
        revision2,
        vec![
            (revision2, body2.clone()),
            (initial_revision, initial.1.clone()),
            (revision1, body1),
        ],
        Some(&first.cursor),
    )
    .unwrap();
    assert_eq!(second.assets[&asset].revision, revision2 as u64);
    assert_eq!(second.assets[&asset].transcript.as_ref(), Some(&transcript));
    assert_eq!(second.cursor.evidence.len(), 3);
    assert_eq!(
        replay(
            &p.target,
            revision2,
            vec![(initial_revision, initial.1), (revision2, body2)],
            Some(&first.cursor)
        )
        .err()
        .unwrap()
        .to_string(),
        "TELEGRAM_INDEX_CHANGED"
    );
    let mut deleted = second.assets[&asset].clone();
    deleted.deleted = true;
    deleted.revision = ((physical as u64 + 10) << 32) | 1;
    change.recreate = true;
    change.base = Some(deleted.clone());
    change.transcript = ReferenceChange::Unchanged;
    change.media = base.media.clone();
    if let Some(MediaRef {
        location: MediaLocation::TelegramMessage { message_id, .. },
        ..
    }) = &mut change.media
    {
        *message_id = i64::from(physical) + 11;
    }
    validate_incarnation(&change, Some(&deleted), Some(deleted.revision)).unwrap();
    if let Some(MediaRef {
        location: MediaLocation::TelegramMessage { message_id, .. },
        ..
    }) = &mut change.media
    {
        *message_id = i64::from(physical) + 9;
    }
    assert!(validate_incarnation(&change, Some(&deleted), Some(deleted.revision)).is_err());
    core.shutdown().await;
}
#[derive(Default)]
struct Remote {
    pulls: AtomicUsize,
    removed_groups: Mutex<Vec<Vec<RemoteDocument>>>,
    group_only: AtomicBool,
    delete_batches: Mutex<Vec<Vec<i32>>>,
    keep_message: std::sync::atomic::AtomicI32,
    data: Mutex<Memory>,
    accepted: AtomicBool,
    lost: AtomicBool,
    incomplete: AtomicBool,
    video_downloads: AtomicUsize,
    body_downloads: AtomicUsize,
    expired: AtomicBool,
    network_changes: AtomicUsize,
    denied_delete: AtomicBool,
    lost_delete: AtomicBool,
    before_publish: AtomicBool,
}
struct Provider {
    target: Target,
    remote: Arc<Remote>,
}
impl Provider {
    fn projection(&self, cursor: Option<Cursor>) -> Result<Projection> {
        let data = self.remote.data.lock().unwrap();
        replay(
            &self.target,
            (i64::from(data.next) << 32) | 1,
            data.events
                .iter()
                .map(|(id, body)| ((i64::from(*id) << 32) | 1, body.clone()))
                .collect(),
            cursor.as_ref(),
        )
    }
}
impl BackupProvider for Provider {
    fn scope(&self) -> &Scope {
        &self.target.scope
    }
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            change_coordination: ChangeCoordination::EventReplay,
            incremental_discovery: false,
            range_read: false,
            resumable_upload: false,
            max_single_media_bytes: 100_000_000,
            can_delete_owned_media: !self.remote.denied_delete.load(Ordering::SeqCst),
        }
    }
    fn open_library(&self, _: bool) -> ProviderFuture<'_, LibraryDescriptor> {
        Box::pin(async {
            Ok(LibraryDescriptor {
                library_id: self.scope().library_id.clone(),
                protocol: 2,
            })
        })
    }
    fn pull_changes(&self, c: Option<Checkpoint>) -> ProviderFuture<'_, ChangeBatch> {
        Box::pin(async move {
            self.remote.pulls.fetch_add(1, Ordering::SeqCst);
            let cursor = c.map(|c| serde_json::from_str(&c.value)).transpose()?;
            let p = self.projection(cursor)?;
            Ok(ChangeBatch {
                assets: p
                    .assets
                    .into_values()
                    .map(|s| remote(self.scope(), s))
                    .collect(),
                checkpoint: OpaqueState {
                    scope: self.scope().clone(),
                    value: serde_json::to_string(&p.cursor)?,
                },
                complete: !self.remote.incomplete.load(Ordering::SeqCst),
            })
        })
    }
    fn read_asset(&self, id: &str) -> ProviderFuture<'_, Option<RemoteAsset>> {
        let id = id.to_owned();
        Box::pin(async move {
            Ok(self
                .projection(None)?
                .assets
                .remove(&id)
                .map(|s| remote(self.scope(), s)))
        })
    }
    fn publish_change(
        &self,
        change: AssetChange,
        _: Option<VersionToken>,
    ) -> ProviderFuture<'_, PublishReceipt> {
        Box::pin(async move {
            if self.remote.before_publish.load(Ordering::SeqCst) {
                bail!("TELEGRAM_TRANSFER_CANCELED");
            }
            if change.delete && self.remote.denied_delete.load(Ordering::SeqCst) {
                bail!("BACKUP_DELETE_UNSUPPORTED");
            }
            let projected = self.projection(None)?;
            if !projected.operations.contains_key(&change.operation_id) {
                validate_incarnation(
                    &change,
                    projected.assets.get(&change.asset_id),
                    projected.last_deletions.get(&change.asset_id).copied(),
                )?;
            }
            let body = Event::encode(&self.target, &change)?;
            {
                let mut data = self.remote.data.lock().unwrap();
                if !data.events.iter().any(|(_, old)| old == &body) {
                    data.next += 1;
                    let id = data.next;
                    data.events.push((id, body));
                }
            }
            if self.remote.lost.swap(false, Ordering::SeqCst) {
                bail!("TELEGRAM_CONNECTION_ERROR");
            }
            if self.remote.accepted.load(Ordering::SeqCst) {
                return Ok(PublishReceipt::Accepted {
                    operation_id: change.operation_id,
                });
            }
            Ok(PublishReceipt::Converged {
                asset: Box::new(remote(
                    self.scope(),
                    self.projection(None)?
                        .assets
                        .remove(&change.asset_id)
                        .unwrap(),
                )),
            })
        })
    }
    fn upload_media(
        &self,
        source: MediaSource,
        _: Option<ResumeState>,
    ) -> ProviderFuture<'_, UploadReceipt> {
        Box::pin(async move {
            if let Some(c) = &source.control {
                (c.check)()?;
            }
            let projection = self.projection(None)?;
            let generation = projection.last_deletions.get(&source.asset_id).copied();
            if projection
                .assets
                .get(&source.asset_id)
                .is_some_and(|s| s.deleted && source.recreate_revision != Some(s.revision))
            {
                bail!("BACKUP_ASSET_DELETED");
            }
            let mut data = self.remote.data.lock().unwrap();
            let key = (source.operation_id.clone(), source.kind.clone());
            let reference = if let Some(value) = data.receipts.get(&key) {
                value.clone()
            } else {
                let body = std::fs::read(source.path)?;
                assert_eq!(format!("{:x}", Sha256::digest(&body)), source.sha256);
                data.next += 1;
                let id = data.next;
                let r = MediaRef {
                    scope: self.scope().clone(),
                    location: MediaLocation::TelegramMessage {
                        account_id: self.target.account_id.clone(),
                        channel_id: self.target.channel_id.clone(),
                        message_id: id as i64,
                        document_id: id as i64,
                    },
                    bytes: body.len() as u64,
                    sha256: source.sha256,
                    container: source.container,
                };
                let envelope = Envelope {
                    generation,
                    yingqu_media: 2,
                    library_id: self.scope().library_id.clone(),
                    asset_id: source.asset_id.clone(),
                    operation_id: source.operation_id.clone(),
                    device_id: source.device_id.clone(),
                    kind: source.kind.clone(),
                    bytes: body.len() as u64,
                    sha256: r.sha256.clone(),
                    container: r.container.clone(),
                };
                data.documents.insert(
                    id,
                    RemoteDocument {
                        message_id: id,
                        document_id: id as i64,
                        bytes: body.len() as u64,
                        caption: envelope.caption()?,
                    },
                );
                data.media.insert(id, body);
                data.receipts.insert(key, r.clone());
                r
            };
            Ok(UploadReceipt {
                reference,
                operation_id: source.operation_id,
                device_id: source.device_id,
            })
        })
    }
    fn cancel_upload(&self, _: ResumeState, _: &str) -> ProviderFuture<'_, ()> {
        Box::pin(async { Ok(()) })
    }
    fn download_media(&self, r: MediaRef, _: u64) -> ProviderFuture<'_, MediaStream> {
        Box::pin(async move {
            let id = self.target.message_id(&r)?;
            if r.container == "mp4" {
                self.remote.video_downloads.fetch_add(1, Ordering::SeqCst);
            }
            let bytes = self.remote.data.lock().unwrap().media[&id].clone();
            let reader: Box<dyn tokio::io::AsyncRead + Unpin + Send> = if r.container == "mp4"
                && self
                    .remote
                    .network_changes
                    .try_update(Ordering::SeqCst, Ordering::SeqCst, |v| v.checked_sub(1))
                    .is_ok()
            {
                Box::new(Expired {
                    prefix: false,
                    error: "TELEGRAM_NETWORK_CHANGED",
                })
            } else if self.remote.expired.swap(false, Ordering::SeqCst) {
                Box::new(Expired {
                    prefix: false,
                    error: "TELEGRAM_FILE_REFERENCE_EXPIRED",
                })
            } else {
                Box::new(std::io::Cursor::new(bytes.clone()))
            };
            Ok(MediaStream {
                bytes: bytes.len() as u64,
                offset: 0,
                reader,
            })
        })
    }
    fn verify_media(&self, r: MediaRef) -> ProviderFuture<'_, ()> {
        Box::pin(async move {
            let (_cancel, cancel) = tokio::sync::watch::channel(false);
            let (_stop, stop) = tokio::sync::watch::channel(false);
            telegram_lifecycle::verify(
                &MemoryTransport(&self.remote),
                &self.target,
                &r,
                &Io::new(cancel, stop, None),
            )
            .await
        })
    }
    fn delete_media(&self, _: MediaRef, _: &str) -> ProviderFuture<'_, ()> {
        Box::pin(async { bail!("BACKUP_CLEANUP_UNSUPPORTED") })
    }
    fn reconcile_deleted_asset(&self, asset: &str, _: &str) -> ProviderFuture<'_, DeleteCleanup> {
        let asset = asset.to_owned();
        Box::pin(async move {
            let (_cancel, cancel) = tokio::sync::watch::channel(false);
            let (_stop, stop) = tokio::sync::watch::channel(false);
            let transport = MemoryTransport(&self.remote);
            let projected = self.projection(None)?;
            let media = transport
                .media(&Io::new(cancel.clone(), stop.clone(), None))
                .await?;
            telegram_lifecycle::reconcile_snapshot(
                &transport,
                &self.target,
                &asset,
                &Io::new(cancel, stop, None),
                &projected,
                media,
                vec![],
            )
            .await?;
            Ok(DeleteCleanup::Complete)
        })
    }
}
async fn delete_pass(core: &Arc<Core>, p: &Provider, operation: &str) -> Result<()> {
    core.backup_db(|db| {
        db.execute(
            "UPDATE backup_operations SET state='running' WHERE operation_id=?",
            [operation],
        )?;
        Ok(())
    })?;
    super::super::sync_delete::execute(core, p, operation).await
}
#[tokio::test]
async fn telegram_t4_release_delete_lost_response_recreate_and_late_cleanup() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let core = open(a.path());
    let net = Arc::new(Remote::default());
    let p = setup(&core, net.clone(), None);
    let engine = SyncEngine::new(core.clone(), p.clone());
    let (asset, body) = seed(&core, a.path());
    engine.execute_upload(&claim(&core, &engine)).await.unwrap();
    let stale = p.projection(None).unwrap().assets[&asset].clone();
    let other = open(b.path());
    let q = setup(&other, net.clone(), Some(p.scope().library_id.clone()));
    let reader = SyncEngine::new(other.clone(), q.clone());
    reader.pull().await.unwrap();
    let path = sync_read::ensure(&other, q.as_ref(), &asset).await.unwrap();
    sync_read::cover(&other, q.as_ref(), &asset).await.unwrap();
    sync_read::transcript(&other, q.as_ref(), &asset)
        .await
        .unwrap();
    super::super::sync_delete::release_with(&other, q.as_ref(), &asset)
        .await
        .unwrap();
    assert!(!path.exists());
    let path = sync_read::ensure(&other, q.as_ref(), &asset).await.unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), body);
    let delete = super::super::sync_delete::intent(&core, &p.scope().space_id, &asset).unwrap();
    assert!(
        a.path().join("original.mp4").exists(),
        "intent does not remove the only copy"
    );
    net.lost_delete.store(true, Ordering::SeqCst);
    assert_eq!(
        delete_pass(&core, &p, &delete)
            .await
            .unwrap_err()
            .to_string(),
        "TELEGRAM_CONNECTION_ERROR"
    );
    let unknown_bytes: i64 = core.backup_db(|db| Ok(db.query_row("SELECT json_extract(payload_json,'$.timings.foreground.remote_cleanup.details.unknown_media_verify_bytes') FROM backup_operations WHERE operation_id=?",[&delete],|r|r.get(0))?)).unwrap();
    assert_eq!(
        unknown_bytes, 0,
        "a registered media reference needs no full-file download for cleanup"
    );
    assert!(
        a.path().join("original.mp4").is_file(),
        "unconfirmed remote cleanup retains the local video"
    );
    for _ in 0..3 {
        delete_pass(&core, &p, &delete).await.unwrap();
    }
    assert!(net.data.lock().unwrap().media.is_empty());
    assert_eq!(
        net.data.lock().unwrap().events.len(),
        2,
        "events are retained"
    );
    reader.pull().await.unwrap();
    assert_eq!(
        std::fs::read(&path).unwrap(),
        body,
        "other device retains its valid copy"
    );
    let cleaned = uuid::Uuid::new_v4().to_string();
    other.backup_db(|db| {
        db.execute("INSERT INTO backup_operations(operation_id,space_id,asset_id,kind,state,phase,payload_json) VALUES(?1,?2,?3,'upload','canceled','cancel_cleaned','{\"receipt\":\"retained\"}')",rusqlite::params![cleaned,q.scope().space_id,asset])?;
        db.execute("INSERT INTO backup_operation_captures VALUES(?1,?2)",rusqlite::params![cleaned,serde_json::json!({"cover":[1,2,3],"transcript":{"text":"old captured text"}}).to_string()])?;
        Ok(())
    }).unwrap();
    sync_engine::request_upload_with(&other, q.clone(), &asset).unwrap();
    other.backup_db(|db| {
        assert_eq!(db.query_row("SELECT state FROM backup_operations WHERE operation_id=?",[&cleaned],|r|r.get::<_,String>(0))?,"completed");
        assert_eq!(db.query_row("SELECT COUNT(*) FROM backup_operation_captures WHERE operation_id=?",[&cleaned],|r|r.get::<_,i64>(0))?,0);
        assert_eq!(db.query_row("SELECT json_extract(payload_json,'$.receipt') FROM backup_operations WHERE operation_id=?",[&cleaned],|r|r.get::<_,String>(0))?,"retained");
        Ok(())
    }).unwrap();

    let upload = super::super::scheduler::operations(&other)
        .unwrap()
        .into_iter()
        .find(|o| o.kind == "upload" && o.state != "completed")
        .unwrap()
        .operation_id;
    other
        .backup_db(|db| {
            db.execute(
                "UPDATE backup_operations SET state='running' WHERE operation_id=?",
                [&upload],
            )?;
            Ok(())
        })
        .unwrap();
    reader.execute_upload(&upload).await.unwrap();
    let current = q.projection(None).unwrap();
    assert!(!current.assets[&asset].deleted);
    assert_eq!(net.data.lock().unwrap().media.len(), 3);
    assert!(net
        .data
        .lock()
        .unwrap()
        .documents
        .values()
        .all(|d| serde_json::from_str::<Envelope>(&d.caption)
            .unwrap()
            .generation
            == Some((5u64 << 32) | 1)));
    // Retry an already observed delete after the new incarnation: neither local nor remote is removed.
    core.backup_db(|db| {
        db.execute(
            "UPDATE backup_operations SET phase='tombstone',state='running' WHERE operation_id=?",
            [&delete],
        )?;
        Ok(())
    })
    .unwrap();
    super::super::sync_delete::execute(&core, p.as_ref(), &delete)
        .await
        .unwrap();
    assert_eq!(net.data.lock().unwrap().media.len(), 3);
    assert!(path.exists());
    let mut old = Event::decode(&q.target, &net.data.lock().unwrap().events[0].1)
        .unwrap()
        .change;
    old.operation_id = id();
    old.base = Some(stale);
    old.media = None;
    old.cover = ReferenceChange::Unchanged;
    old.transcript = ReferenceChange::Unchanged;
    assert_eq!(
        q.publish_change(old, None).await.unwrap_err().to_string(),
        "BACKUP_REUPLOAD_STALE"
    );
    let second = super::super::sync_delete::intent(&other, &q.scope().space_id, &asset).unwrap();
    assert_ne!(second, delete);
    delete_pass(&other, &q, &second).await.unwrap();
    assert!(net.data.lock().unwrap().media.is_empty());
    core.shutdown().await;
    other.shutdown().await;
}
#[tokio::test]
async fn telegram_t4_replaced_document_and_corrupt_index_never_remove_local_or_advance() {
    let root = tempfile::tempdir().unwrap();
    let core = open(root.path());
    let net = Arc::new(Remote::default());
    let p = setup(&core, net.clone(), None);
    let engine = SyncEngine::new(core.clone(), p.clone());
    let (asset, body) = seed(&core, root.path());
    engine.execute_upload(&claim(&core, &engine)).await.unwrap();
    engine.pull().await.unwrap();
    let reference = p.projection(None).unwrap().assets[&asset]
        .media
        .clone()
        .unwrap();
    let message = p.target.message_id(&reference).unwrap();
    net.data
        .lock()
        .unwrap()
        .documents
        .get_mut(&message)
        .unwrap()
        .document_id += 100;
    assert_eq!(
        super::super::sync_delete::release_with(&core, p.as_ref(), &asset)
            .await
            .unwrap_err()
            .to_string(),
        "TELEGRAM_MEDIA_CHANGED"
    );
    assert_eq!(
        std::fs::read(root.path().join("original.mp4")).unwrap(),
        body
    );
    let before = p.projection(None).unwrap().cursor;
    net.data.lock().unwrap().events[0].1 = b"invalid index".to_vec();
    assert!(p.projection(Some(before)).is_err());
    assert!(engine.pull().await.is_err());
    assert_eq!(pull_journal(&core).last().unwrap().1, "error");
    assert_eq!(
        core.backup_assets().unwrap()[0].metadata["title"],
        "原始标题"
    );
    core.shutdown().await;
}
#[tokio::test]
async fn telegram_t4_owned_cleanup_keeps_published_replaced_foreign_and_next_generation() {
    use super::super::telegram_transfer_journal::{Entry, SendState};
    let root = tempfile::tempdir().unwrap();
    let core = open(root.path());
    let net = Arc::new(Remote::default());
    let p = setup(&core, net.clone(), None);
    let engine = SyncEngine::new(core.clone(), p.clone());
    let (asset, _) = seed(&core, root.path());
    engine.execute_upload(&claim(&core, &engine)).await.unwrap();
    let (_c, c) = tokio::sync::watch::channel(false);
    let (_s, s) = tokio::sync::watch::channel(false);
    let io = Io::new(c, s, None);
    let transport = MemoryTransport(&net);
    let original = net.data.lock().unwrap().documents[&1].clone();
    let mut entry = Entry {
        target: p.target.clone(),
        envelope: serde_json::from_str(&original.caption).unwrap(),
        random_id: 1,
        state: SendState::Confirmed,
        document: Some(original.clone()),
        last_error: None,
    };
    telegram_lifecycle::cancel(
        &transport,
        &p.target,
        &entry,
        &p.projection(None).unwrap(),
        &io,
    )
    .await
    .unwrap();
    assert_eq!(
        net.data.lock().unwrap().media.len(),
        3,
        "published operations are never canceled remotely"
    );
    entry.envelope.operation_id = id();
    entry.document.as_mut().unwrap().message_id = 10;
    entry.document.as_mut().unwrap().caption = entry.envelope.caption().unwrap();
    {
        let mut data = net.data.lock().unwrap();
        let bytes = data.media[&1].clone();
        data.media.insert(10, bytes);
        data.documents.insert(10, entry.document.clone().unwrap());
    }
    net.data
        .lock()
        .unwrap()
        .documents
        .get_mut(&10)
        .unwrap()
        .document_id += 1;
    assert_eq!(
        telegram_lifecycle::cancel(
            &transport,
            &p.target,
            &entry,
            &p.projection(None).unwrap(),
            &io
        )
        .await
        .unwrap_err()
        .to_string(),
        "TELEGRAM_MEDIA_CHANGED"
    );
    net.data
        .lock()
        .unwrap()
        .documents
        .insert(10, entry.document.clone().unwrap());
    telegram_lifecycle::cancel(
        &transport,
        &p.target,
        &entry,
        &p.projection(None).unwrap(),
        &io,
    )
    .await
    .unwrap();
    assert!(!net.data.lock().unwrap().documents.contains_key(&10));
    // A lost send with no evidence stays uncertain; no guessed message is removed.
    entry.document = None;
    entry.state = SendState::Sending;
    assert_eq!(
        telegram_lifecycle::cancel(
            &transport,
            &p.target,
            &entry,
            &p.projection(None).unwrap(),
            &io
        )
        .await
        .unwrap_err()
        .to_string(),
        "TELEGRAM_SEND_UNCONFIRMED"
    );
    let delete = super::super::sync_delete::intent(&core, &p.scope().space_id, &asset).unwrap();
    // Old-generation late upload, unrelated asset, and in-flight re-upload have distinct captions.
    {
        let mut data = net.data.lock().unwrap();
        let bytes = data.media[&1].clone();
        for (message, foreign, generation) in [
            (10, false, None),
            (11, true, None),
            (12, false, Some((5u64 << 32) | 1)),
        ] {
            let mut e = entry.envelope.clone();
            e.generation = generation;
            if foreign {
                e.asset_id = id();
            }
            data.documents.insert(
                message,
                RemoteDocument {
                    message_id: message,
                    document_id: message as i64,
                    bytes: bytes.len() as u64,
                    caption: e.caption().unwrap(),
                },
            );
            data.media.insert(
                message,
                if message == 10 {
                    vec![42; bytes.len()]
                } else {
                    bytes.clone()
                },
            );
        }
    }
    delete_pass(&core, &p, &delete).await.unwrap();
    let stats: (i64,i64) = core.backup_db(|db| Ok(db.query_row("SELECT json_extract(payload_json,'$.timings.foreground.remote_cleanup.details.unknown_media_verify_bytes'),COALESCE(json_extract(payload_json,'$.timings.foreground.remote_cleanup.details.steps.unknown_media_verify.calls'),0) FROM backup_operations WHERE operation_id=?",[&delete],|r|Ok((r.get(0)?,r.get(1)?)))?)).unwrap();
    assert_eq!(
        stats,
        (0, 0),
        "unregistered attachments use identity/generation checks without body download"
    );
    assert_eq!(
        net.body_downloads.load(Ordering::SeqCst),
        0,
        "deletion downloaded an attachment body"
    );
    let remaining: Vec<_> = net.data.lock().unwrap().documents.keys().copied().collect();
    assert_eq!(
        remaining,
        vec![11, 12],
        "cleanup is limited to the tombstoned incarnation"
    );
    core.shutdown().await;
}
#[tokio::test]
async fn telegram_t4_cancel_cleanup_then_upload_uses_new_operation_and_messages() {
    use super::super::telegram_transfer_journal::{Entry, SendState};
    let root = tempfile::tempdir().unwrap();
    let core = open(root.path());
    let net = Arc::new(Remote::default());
    let p = setup(&core, net.clone(), None);
    let engine = SyncEngine::new(core.clone(), p.clone());
    let (asset, _) = seed(&core, root.path());
    let old = claim(&core, &engine);
    net.before_publish.store(true, Ordering::SeqCst);
    assert!(engine.execute_upload(&old).await.is_err());
    let capture: String = core
        .backup_db(|db| {
            Ok(db.query_row(
                "SELECT capture_json FROM backup_operation_captures WHERE operation_id=?",
                [&old],
                |r| r.get(0),
            )?)
        })
        .unwrap();
    assert!(serde_json::from_str::<serde_json::Value>(&capture)
        .unwrap()
        .get("cover")
        .is_none());
    let original_cover = super::super::scheduler::operation(&core, &old)
        .unwrap()
        .payload["capture"]["cover"]
        .clone();
    assert!(original_cover.is_array());
    let (_c, c) = tokio::sync::watch::channel(false);
    let (_s, s) = tokio::sync::watch::channel(false);
    let io = Io::new(c, s, None);
    let documents: Vec<_> = net
        .data
        .lock()
        .unwrap()
        .documents
        .values()
        .cloned()
        .collect();
    for document in documents {
        let entry = Entry {
            target: p.target.clone(),
            envelope: serde_json::from_str(&document.caption).unwrap(),
            random_id: 1,
            state: SendState::Confirmed,
            document: Some(document),
            last_error: None,
        };
        telegram_lifecycle::cancel(
            &MemoryTransport(&net),
            &p.target,
            &entry,
            &p.projection(None).unwrap(),
            &io,
        )
        .await
        .unwrap();
    }
    assert!(net.data.lock().unwrap().documents.is_empty());
    super::super::scheduler::cancel(&core, &old).unwrap();
    super::super::scheduler::finish_upload_cleanup(&core, &old).unwrap();
    assert_eq!(
        core.backup_db(|db| Ok(db.query_row(
            "SELECT capture_json FROM backup_operation_captures WHERE operation_id=?",
            [&old],
            |r| r.get::<_, String>(0)
        )?))
        .unwrap(),
        capture
    );
    assert_eq!(
        super::super::scheduler::operation(&core, &old)
            .unwrap()
            .payload["capture"]["cover"],
        original_cover
    );
    net.before_publish.store(false, Ordering::SeqCst);
    sync_engine::request_upload_with(&core, p.clone(), &asset).unwrap();
    core.backup_db(|db| {
        assert_eq!(
            db.query_row(
                "SELECT state FROM backup_operations WHERE operation_id=?",
                [&old],
                |r| r.get::<_, String>(0)
            )?,
            "completed"
        );
        assert_eq!(
            db.query_row(
                "SELECT COUNT(*) FROM backup_operation_captures WHERE operation_id=?",
                [&old],
                |r| r.get::<_, i64>(0)
            )?,
            0
        );
        assert!(crate::cover_store::read(db, crate::cover_store::Owner::Capture, &old)?.is_none());
        Ok(())
    })
    .unwrap();
    let new = claim(&core, &engine);
    assert_ne!(new, old);
    engine.execute_upload(&new).await.unwrap();
    assert_eq!(net.data.lock().unwrap().documents.len(), 3);
    assert!(net
        .data
        .lock()
        .unwrap()
        .documents
        .values()
        .all(|d| d.message_id > 3));
    core.shutdown().await;
}
struct MemoryTransport<'a>(&'a Remote);
impl Transport for MemoryTransport<'_> {
    type Uploaded = ();
    type Prepared = ();
    fn max_bytes(&self) -> u64 {
        100_000_000
    }
    fn can_upload(&self) -> bool {
        true
    }
    fn can_delete(&self) -> bool {
        !self.0.denied_delete.load(Ordering::SeqCst)
    }
    fn upload<'a>(
        &'a self,
        _: &'a mut (dyn tokio::io::AsyncRead + Unpin + Send),
        _: u64,
        _: String,
    ) -> ProviderFuture<'a, ()> {
        Box::pin(async { unreachable!() })
    }
    fn send<'a>(
        &'a self,
        _: ((), ()),
        _: String,
        _: i64,
        _: Option<crate::backup::provider::VideoAttributes>,
        _: &'a Io,
        _: &'a mut (dyn FnMut() -> Result<()> + Send),
    ) -> ProviderFuture<'a, Option<i32>> {
        Box::pin(async { unreachable!() })
    }
    fn inspect(&self, id: i32) -> ProviderFuture<'_, RemoteDocument> {
        Box::pin(async move {
            self.0
                .data
                .lock()
                .unwrap()
                .documents
                .get(&id)
                .cloned()
                .context("TELEGRAM_MEDIA_MISSING")
        })
    }
    fn find<'a>(
        &'a self,
        e: &'a Envelope,
        _: &'a Io,
    ) -> ProviderFuture<'a, Option<RemoteDocument>> {
        Box::pin(async move {
            Ok(self
                .0
                .data
                .lock()
                .unwrap()
                .documents
                .values()
                .find(|d| d.caption == e.caption().unwrap())
                .cloned())
        })
    }
    fn download(
        &self,
        d: RemoteDocument,
    ) -> ProviderFuture<'_, Box<dyn tokio::io::AsyncRead + Unpin + Send>> {
        Box::pin(async move {
            self.0.body_downloads.fetch_add(1, Ordering::SeqCst);
            let bytes = self
                .0
                .data
                .lock()
                .unwrap()
                .media
                .get(&d.message_id)
                .cloned()
                .context("TELEGRAM_MEDIA_MISSING")?;
            Ok(Box::new(std::io::Cursor::new(bytes)) as _)
        })
    }
    fn media<'a>(&'a self, _: &'a Io) -> ProviderFuture<'a, Vec<RemoteDocument>> {
        Box::pin(async {
            Ok(self
                .0
                .data
                .lock()
                .unwrap()
                .documents
                .values()
                .cloned()
                .collect())
        })
    }
    fn remove(&self, d: RemoteDocument) -> ProviderFuture<'_, ()> {
        Box::pin(async move {
            if !self.can_delete() {
                bail!("BACKUP_DELETE_UNSUPPORTED");
            }
            {
                let mut data = self.0.data.lock().unwrap();
                if let Some(current) = data.documents.get(&d.message_id) {
                    if current != &d {
                        bail!("TELEGRAM_MEDIA_CHANGED");
                    }
                }
                data.documents.remove(&d.message_id);
                data.media.remove(&d.message_id);
            }
            if self.0.lost_delete.swap(false, Ordering::SeqCst) {
                bail!("TELEGRAM_CONNECTION_ERROR");
            }
            Ok(())
        })
    }
}
impl telegram_lifecycle::OwnedMessageRemoval for MemoryTransport<'_> {
    async fn read_owned_ids(
        &self,
        groups: &[(i32, Vec<RemoteDocument>)],
        _: &Io,
    ) -> Result<Vec<i32>> {
        let data = self.0.data.lock().unwrap();
        let mut ids = Vec::new();
        for (id, expected) in groups {
            if self.0.group_only.load(Ordering::SeqCst) {
                self.0.removed_groups.lock().unwrap().push(expected.clone());
            } else if let Some(actual) = data.documents.get(id) {
                if expected.as_slice() != std::slice::from_ref(actual) {
                    bail!("TELEGRAM_MEDIA_CHANGED");
                }
            } else {
                continue;
            }
            ids.push(*id);
        }
        Ok(ids)
    }

    async fn delete_verified_ids(&self, ids: &[i32]) -> Result<()> {
        self.0.delete_batches.lock().unwrap().push(ids.to_vec());
        if !self.0.group_only.load(Ordering::SeqCst) {
            let mut data = self.0.data.lock().unwrap();
            for id in ids {
                if self.0.keep_message.load(Ordering::SeqCst) == *id {
                    continue;
                }
                data.documents.remove(id);
                data.media.remove(id);
            }
            if self.0.lost_delete.swap(false, Ordering::SeqCst) {
                bail!("TELEGRAM_CONNECTION_ERROR");
            }
        }
        Ok(())
    }

    async fn confirm_ids_gone(&self, ids: &[i32]) -> Result<()> {
        let data = self.0.data.lock().unwrap();
        if ids.iter().any(|id| data.documents.contains_key(id)) {
            bail!("BACKUP_CLEANUP_PENDING");
        }
        Ok(())
    }
}
struct Expired {
    prefix: bool,
    error: &'static str,
}
impl tokio::io::AsyncRead for Expired {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        if !self.prefix {
            buf.put_slice(b"stale");
            self.prefix = true;
            std::task::Poll::Ready(Ok(()))
        } else {
            std::task::Poll::Ready(Err(std::io::Error::other(self.error)))
        }
    }
}
fn id() -> String {
    uuid::Uuid::new_v4().to_string()
}
fn open(root: &Path) -> Arc<Core> {
    Core::open(
        &root.join("app.sqlite"),
        root.to_owned(),
        "unused".into(),
        vec![],
        |_| {},
    )
    .unwrap()
}
fn setup(core: &Arc<Core>, remote: Arc<Remote>, library: Option<String>) -> Arc<Provider> {
    let space = Space {
        space_id: id(),
        library_id: library.unwrap_or_else(id),
        endpoint: String::new(),
        bucket: String::new(),
        prefix: String::new(),

        credential_ref: None,
        enabled: true,
    };
    let provider = Arc::new(Provider {
        target: Target {
            scope: Scope {
                space_id: space.space_id.clone(),
                library_id: space.library_id.clone(),
                provider_type: ProviderType::Telegram,
                protocol: 2,
            },
            account_id: "123".into(),
            channel_id: "456".into(),
        },
        remote,
    });
    core.backup_db(|db|{
        repository::save_space(db,&space)?;
        db.execute("UPDATE backup_provider_config SET provider_type='telegram',profile='telegram',protocol=2,capabilities_json=?2 WHERE space_id=?1",params![space.space_id,serde_json::to_string(&provider.capabilities())?])?;
        db.execute("INSERT INTO telegram_spaces VALUES(?1,'123','456',0)",[&space.space_id])?;
        super::super::spaces::select_local(db,&space.space_id,true)
    }).unwrap();
    provider
}
fn seed(core: &Core, root: &Path) -> (String, Vec<u8>) {
    let asset = id();
    let path = root.join("original.mp4");
    let body = b"\0\0\0\x10ftypisom1234".repeat(8192);
    std::fs::write(&path, &body).unwrap();
    let (hash, source) = repository::fingerprint(&path).unwrap();
    core.backup_db(|db|{
        db.execute("INSERT INTO media_assets VALUES(?1,NULL,?2,'original.mp4',?3,?4,'active')",params![asset,json!({"title":"原始标题","platform":"youtube","video_id":"12345678901","canonical_url":"https://www.youtube.com/watch?v=12345678901"}).to_string(),hash,body.len() as i64])?;
        db.execute("INSERT INTO asset_local_files VALUES(?1,?2,?3,?4,?5,unixepoch(),'present')",params![asset,path.to_string_lossy(),body.len() as i64,hash,source.to_string()])?;
        db.execute("INSERT INTO asset_transcripts VALUES(?1,?2,'中文 English 123','synthetic',NULL,'test',1,NULL)",params![asset,hash])?;
        crate::cover_store::put(db, crate::cover_store::Owner::Asset, &asset, b"\x89PNG\r\n\x1a\nfixture")?;
        sync_engine::register_download(db,&asset)
    }).unwrap();
    (asset, body)
}
fn claim(core: &Core, engine: &SyncEngine) -> String {
    engine.enqueue().unwrap();
    let operation = super::super::scheduler::operations(core)
        .unwrap()
        .into_iter()
        .find(|o| o.kind == "upload" && o.state != "completed")
        .unwrap()
        .operation_id;
    core.backup_db(|db| {
        db.execute(
            "UPDATE backup_operations SET state='running',attempt=attempt+1 WHERE operation_id=?",
            [&operation],
        )?;
        Ok(())
    })
    .unwrap();
    operation
}

#[tokio::test]
async fn telegram_t3_queue_fresh_database_rebuilds_metadata_without_video_then_restores_on_demand()
{
    let a = tempfile::tempdir().unwrap();
    let core = open(a.path());
    let net = Arc::new(Remote::default());
    let provider = setup(&core, net.clone(), None);
    let engine = SyncEngine::new(core.clone(), provider.clone());
    let (asset, body) = seed(&core, a.path());
    let operation = claim(&core, &engine);
    engine.execute_upload(&operation).await.unwrap();
    assert_eq!(
        net.data.lock().unwrap().media.len(),
        3,
        "video, cover and transcript use the shared operation"
    );
    let b = tempfile::tempdir().unwrap();
    let fresh = open(b.path());
    let other = setup(
        &fresh,
        net.clone(),
        Some(provider.scope().library_id.clone()),
    );
    let reader = SyncEngine::new(fresh.clone(), other.clone());
    assert_eq!(reader.pull().await.unwrap(), 1);
    let assets = fresh.backup_assets().unwrap();
    assert_eq!(assets.len(), 1);
    assert_eq!(assets[0].metadata["title"], "原始标题");
    assert!(assets[0].path.is_none());
    assert_eq!(net.video_downloads.load(Ordering::SeqCst), 0);
    let text = sync_read::transcript(&fresh, other.as_ref(), &assets[0].asset_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(text.text, "中文 English 123");
    assert!(
        sync_read::cover(&fresh, other.as_ref(), &assets[0].asset_id)
            .await
            .unwrap()
            .starts_with(b"\x89PNG")
    );
    assert_eq!(
        net.video_downloads.load(Ordering::SeqCst),
        0,
        "small object reads must not download video"
    );
    let restored = sync_read::ensure(&fresh, other.as_ref(), &assets[0].asset_id)
        .await
        .unwrap();
    assert_eq!(std::fs::read(restored).unwrap(), body);
    assert_eq!(net.video_downloads.load(Ordering::SeqCst), 1);
    sync_read::ensure(&fresh, other.as_ref(), &assets[0].asset_id)
        .await
        .unwrap();
    assert_eq!(
        net.video_downloads.load(Ordering::SeqCst),
        1,
        "valid local copy is reused"
    );
    assert!(a.path().join("original.mp4").is_file());
    assert_eq!(assets[0].asset_id, asset);
    fresh.shutdown().await;
    core.shutdown().await;
}
#[tokio::test]
async fn telegram_t3_lost_publish_survives_restart_without_duplicate_media_or_event() {
    let root = tempfile::tempdir().unwrap();
    let core = open(root.path());
    let net = Arc::new(Remote::default());
    let provider = setup(&core, net.clone(), None);
    let engine = SyncEngine::new(core.clone(), provider.clone());
    seed(&core, root.path());
    let operation = claim(&core, &engine);
    net.lost.store(true, Ordering::SeqCst);
    assert!(engine.execute_upload(&operation).await.is_err());
    let target = provider.target.clone();
    drop(engine);
    drop(provider);
    core.shutdown().await;
    drop(core);
    let core = open(root.path());
    let provider = Arc::new(Provider {
        target,
        remote: net.clone(),
    });
    let engine = SyncEngine::new(core.clone(), provider);
    core.backup_db(|db| {
        db.execute(
            "UPDATE backup_operations SET state='running' WHERE operation_id=?",
            [&operation],
        )?;
        Ok(())
    })
    .unwrap();
    engine.execute_upload(&operation).await.unwrap();
    {
        let data = net.data.lock().unwrap();
        assert_eq!(data.media.len(), 3);
        assert_eq!(data.events.len(), 1);
    }
    assert_eq!(
        super::super::scheduler::operations(&core).unwrap()[0].state,
        "completed"
    );
    core.shutdown().await;
}
#[tokio::test]
async fn telegram_t3_accepted_and_incomplete_prefix_never_acknowledge_pending_edits() {
    let root = tempfile::tempdir().unwrap();
    let core = open(root.path());
    let net = Arc::new(Remote::default());
    let p = setup(&core, net.clone(), None);
    let engine = SyncEngine::new(core.clone(), p.clone());
    let (asset, _) = seed(&core, root.path());
    let operation = claim(&core, &engine);
    net.accepted.store(true, Ordering::SeqCst);
    assert_eq!(
        engine
            .execute_upload(&operation)
            .await
            .unwrap_err()
            .to_string(),
        "BACKUP_PUBLICATION_UNCONFIRMED"
    );
    let ack: i64 = core
        .backup_db(|db| {
            Ok(db.query_row(
                "SELECT ack_revision FROM backup_space_assets WHERE local_asset_id=?",
                [&asset],
                |r| r.get(0),
            )?)
        })
        .unwrap();
    assert_eq!(ack, 0);
    net.incomplete.store(true, Ordering::SeqCst);
    assert_eq!(
        engine.pull().await.unwrap_err().to_string(),
        "BACKUP_INCOMPLETE_DISCOVERY"
    );
    let checkpoints: i64 = core
        .backup_db(|db| {
            Ok(
                db.query_row("SELECT COUNT(*) FROM backup_sync_checkpoints", [], |r| {
                    r.get(0)
                })?,
            )
        })
        .unwrap();
    assert_eq!(checkpoints, 0);
    net.accepted.store(false, Ordering::SeqCst);
    net.incomplete.store(false, Ordering::SeqCst);
    engine.execute_upload(&operation).await.unwrap();
    assert_eq!(net.data.lock().unwrap().events.len(), 1);
    core.shutdown().await;
}
#[tokio::test]
async fn telegram_t3_switch_to_s3_blocks_old_queue_and_keeps_original_pending_payload() {
    let root = tempfile::tempdir().unwrap();
    let core = open(root.path());
    let net = Arc::new(Remote::default());
    let p = setup(&core, net.clone(), None);
    let engine = SyncEngine::new(core.clone(), p.clone());
    seed(&core, root.path());
    let operation = claim(&core, &engine);
    let s3 = Space {
        space_id: id(),
        library_id: id(),
        endpoint: "https://example.com".into(),
        bucket: "b".into(),
        prefix: "p/".into(),

        credential_ref: None,
        enabled: true,
    };
    let before = super::super::scheduler::operations(&core).unwrap()[0]
        .payload
        .clone();
    core.backup_db(|db| {
        repository::save_space(db, &s3)?;
        super::super::spaces::select_local(db, &s3.space_id, true)
    })
    .unwrap();
    assert!(engine.execute_upload(&operation).await.is_err());
    assert!(net.data.lock().unwrap().media.is_empty());
    let op = super::super::scheduler::operations(&core)
        .unwrap()
        .remove(0);
    assert_eq!(op.state, "blocked");
    assert_eq!(op.payload, before);
    core.shutdown().await;
}
#[test]
fn telegram_initial_schema_keeps_s3_unique_target_and_allows_multiple_telegram_spaces() {
    let db = rusqlite::Connection::open_in_memory().unwrap();
    repository::init(&db).unwrap();
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM telegram_media_sends", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    db.execute(
        "INSERT INTO telegram_media_sends VALUES('operation','event','{}')",
        [],
    )
    .unwrap();
    let mk = |space: &str, endpoint: &str| Space {
        space_id: space.into(),
        library_id: id(),
        endpoint: endpoint.into(),
        bucket: "b".into(),
        prefix: "p/".into(),

        credential_ref: None,
        enabled: false,
    };
    repository::save_space(&db, &mk("tg-a", "")).unwrap();
    repository::save_space(&db, &mk("tg-b", "")).unwrap();
    repository::save_space(&db, &mk("s3-a", "https://example.com")).unwrap();
    assert!(repository::save_space(&db, &mk("s3-b", "https://example.com")).is_err());
    assert!(db
        .prepare("PRAGMA foreign_key_check")
        .unwrap()
        .query([])
        .unwrap()
        .next()
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn telegram_t3_original_edits_replay_gaps_duplicates_and_preserve_same_field_conflict() {
    let root = tempfile::tempdir().unwrap();
    let core = open(root.path());
    let net = Arc::new(Remote::default());
    let p = setup(&core, net.clone(), None);
    let engine = SyncEngine::new(core.clone(), p.clone());
    seed(&core, root.path());
    let operation = claim(&core, &engine);
    engine.execute_upload(&operation).await.unwrap();
    let first = net.data.lock().unwrap().events[0].clone();
    let first = (i64::from(first.0), first.1);
    let baseline = p
        .projection(None)
        .unwrap()
        .assets
        .into_values()
        .next()
        .unwrap();
    let edit = |text: &str| {
        let mut change = Event::decode(&p.target, &first.1).unwrap().change;
        change.base = Some(baseline.clone());
        change.fields = BTreeMap::from([("title".into(), json!(text))]);
        change.field_bases.clear();
        change.operation_id = id();
        change.device_id = id();
        change.media = None;
        change.cover = ReferenceChange::Unchanged;
        change.transcript = ReferenceChange::Unchanged;
        Event::encode(&p.target, &change).unwrap()
    };
    let a = edit("先到的标题");
    let b = edit("另一台设备的标题");
    assert!(
        !String::from_utf8(a.clone())
            .unwrap()
            .contains(&p.scope().space_id),
        "portable events must not retain a local space ID"
    );
    let events = vec![
        (31, b.clone()),
        first.clone(),
        (20, a.clone()),
        (40, a.clone()),
    ];
    let replayed = replay(&p.target, 40, events.clone(), None).unwrap();
    let asset = replayed.assets.values().next().unwrap();
    assert_eq!(asset.fields["title"], "先到的标题");
    assert_eq!(asset.conflicts.len(), 1);
    assert_eq!(asset.conflicts[0].value, "另一台设备的标题");
    assert_eq!(replayed.operations.len(), 3);
    assert!(
        replay(
            &p.target,
            41,
            vec![first.clone(), (20, a.clone()), (31, b)],
            Some(&replayed.cursor)
        )
        .is_err(),
        "removing an observed duplicate message is an index change too"
    );
    let mut changed = Event::decode(&p.target, &a).unwrap().change;
    changed
        .fields
        .insert("title".into(), json!("changed payload"));
    assert_eq!(
        replay(
            &p.target,
            40,
            vec![
                first.clone(),
                (20, a),
                (40, Event::encode(&p.target, &changed).unwrap())
            ],
            None
        )
        .err()
        .unwrap()
        .to_string(),
        "TELEGRAM_OPERATION_MISMATCH"
    );
    let first_projection = replay(&p.target, 100, vec![first.clone()], None).unwrap();
    assert!(
        replay(
            &p.target,
            first.0,
            vec![first.clone()],
            Some(&first_projection.cursor)
        )
        .is_ok(),
        "removing unrelated high message IDs does not remove an index event"
    );
    assert_eq!(
        replay(&p.target, 100, vec![], Some(&first_projection.cursor))
            .err()
            .unwrap()
            .to_string(),
        "TELEGRAM_INDEX_CHANGED"
    );
    let mut wrong = p.target.clone();
    wrong.account_id = "999".into();
    assert!(Event::decode(&wrong, &first.1).is_err());
    core.shutdown().await;
}
#[tokio::test]
async fn telegram_t3_confirmed_prefix_is_durable_before_any_business_pull_checkpoint() {
    use super::super::telegram_provider::TelegramProvider;
    let root = tempfile::tempdir().unwrap();
    let core = open(root.path());
    let net = Arc::new(Remote::default());
    let p = setup(&core, net.clone(), None);
    let engine = SyncEngine::new(core.clone(), p.clone());
    seed(&core, root.path());
    let operation = claim(&core, &engine);
    engine.execute_upload(&operation).await.unwrap();
    let cursor = p.projection(None).unwrap().cursor;
    let space = p.scope().space_id.clone();
    TelegramProvider::observe(&core, &space, &cursor).unwrap();
    assert_eq!(
        core.backup_db(|db| Ok(db.query_row(
            "SELECT COUNT(*) FROM backup_sync_checkpoints",
            [],
            |r| r.get::<_, i64>(0)
        )?))
        .unwrap(),
        0,
        "observations cannot advance the business import checkpoint"
    );
    drop(engine);
    drop(p);
    core.shutdown().await;
    drop(core);
    let core = open(root.path());
    let empty = Cursor {
        high_water: cursor.high_water,
        evidence: BTreeMap::new(),
    };
    assert_eq!(
        TelegramProvider::observe(&core, &space, &empty)
            .unwrap_err()
            .to_string(),
        "TELEGRAM_INDEX_CHANGED"
    );
    let mut edited = cursor.clone();
    *edited.evidence.values_mut().next().unwrap() = "edited".into();
    assert_eq!(
        TelegramProvider::observe(&core, &space, &edited)
            .unwrap_err()
            .to_string(),
        "TELEGRAM_INDEX_CHANGED"
    );
    core.shutdown().await;
}
#[tokio::test]
async fn telegram_t3_pending_connection_is_dropped_when_backup_stops_or_space_changes() {
    use super::super::spaces;
    for switch in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let core = open(root.path());
        let p = setup(&core, Arc::new(Remote::default()), None);
        let dropped = Arc::new(AtomicBool::new(false));
        struct Request(Arc<AtomicBool>);
        impl Drop for Request {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }
        let request = Request(dropped.clone());
        let work = async move {
            let _request = request;
            std::future::pending::<Result<()>>().await
        };
        let stop = async {
            tokio::task::yield_now().await;
            if switch {
                let other = Space {
                    space_id: id(),
                    library_id: id(),
                    endpoint: "https://example.com".into(),
                    bucket: "b".into(),
                    prefix: "p/".into(),

                    credential_ref: None,
                    enabled: true,
                };
                core.backup_db(|db| {
                    repository::save_space(db, &other)?;
                    spaces::select_local(db, &other.space_id, true)
                })
                .unwrap();
            } else {
                spaces::set_enabled(&core, false).await.unwrap();
            }
        };
        let (result, ()) = tokio::join!(
            spaces::await_preparation(&core, &p.scope().space_id, work),
            stop
        );
        assert_eq!(result.unwrap_err().to_string(), "CANCELED");
        assert!(dropped.load(Ordering::SeqCst));
        core.shutdown().await;
    }
}
#[tokio::test]
async fn telegram_t4_corrupt_remote_release_keeps_the_only_valid_local_copy() {
    let root = tempfile::tempdir().unwrap();
    let core = open(root.path());
    let net = Arc::new(Remote::default());
    let p = setup(&core, net.clone(), None);
    let engine = SyncEngine::new(core.clone(), p.clone());
    let (asset, original) = seed(&core, root.path());
    let operation = claim(&core, &engine);
    engine.execute_upload(&operation).await.unwrap();
    for bytes in net.data.lock().unwrap().media.values_mut() {
        bytes[0] ^= 1;
    }
    let result = super::super::sync_delete::release_with(&core, p.as_ref(), &asset).await;
    assert_eq!(result.unwrap_err().to_string(), "BACKUP_CHECKSUM_MISMATCH");
    assert_eq!(
        std::fs::read(root.path().join("original.mp4")).unwrap(),
        original
    );
    net.denied_delete.store(true, Ordering::SeqCst);
    let delete = super::super::sync_delete::intent(&core, &p.scope().space_id, &asset).unwrap();
    assert_eq!(
        super::super::sync_delete::execute(&core, p.as_ref(), &delete)
            .await
            .unwrap_err()
            .to_string(),
        "BACKUP_DELETE_UNSUPPORTED"
    );
    assert_eq!(
        std::fs::read(root.path().join("original.mp4")).unwrap(),
        original
    );
    core.shutdown().await;
}

#[tokio::test]
async fn telegram_t3_provider_reference_expiry_restarts_small_and_video_reads_once_from_zero() {
    let a = tempfile::tempdir().unwrap();
    let core = open(a.path());
    let net = Arc::new(Remote::default());
    let p = setup(&core, net.clone(), None);
    let engine = SyncEngine::new(core.clone(), p.clone());
    seed(&core, a.path());
    let operation = claim(&core, &engine);
    engine.execute_upload(&operation).await.unwrap();
    let b = tempfile::tempdir().unwrap();
    let fresh = open(b.path());
    let other = setup(&fresh, net.clone(), Some(p.scope().library_id.clone()));
    let reader = SyncEngine::new(fresh.clone(), other.clone());
    reader.pull().await.unwrap();
    let asset = fresh.backup_assets().unwrap().remove(0);
    net.expired.store(true, Ordering::SeqCst);
    assert_eq!(
        sync_read::transcript(&fresh, other.as_ref(), &asset.asset_id)
            .await
            .unwrap()
            .unwrap()
            .text,
        "中文 English 123"
    );
    net.expired.store(true, Ordering::SeqCst);
    let path = sync_read::ensure(&fresh, other.as_ref(), &asset.asset_id)
        .await
        .unwrap();
    assert_eq!(
        std::fs::read(path).unwrap(),
        std::fs::read(a.path().join("original.mp4")).unwrap()
    );
    assert_eq!(net.video_downloads.load(Ordering::SeqCst), 2);
    fresh.shutdown().await;
    core.shutdown().await;
}

#[tokio::test]
async fn proxy_changes_restore_same_operation_with_full_checksum_and_no_cloud_writes() {
    let a = tempfile::tempdir().unwrap();
    let core = open(a.path());
    let net = Arc::new(Remote::default());
    let p = setup(&core, net.clone(), None);
    let engine = SyncEngine::new(core.clone(), p.clone());
    let (_, original) = seed(&core, a.path());
    engine.execute_upload(&claim(&core, &engine)).await.unwrap();
    let b = tempfile::tempdir().unwrap();
    let fresh = open(b.path());
    let other = setup(&fresh, net.clone(), Some(p.scope().library_id.clone()));
    SyncEngine::new(fresh.clone(), other.clone())
        .pull()
        .await
        .unwrap();
    let asset = fresh.backup_assets().unwrap().remove(0);
    let before = {
        let data = net.data.lock().unwrap();
        (data.events.clone(), data.media.clone())
    };
    net.network_changes.store(2, Ordering::SeqCst);
    let path = sync_read::ensure(&fresh, other.as_ref(), &asset.asset_id)
        .await
        .unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), original);
    assert_eq!(
        super::super::repository::fingerprint(&path).unwrap().0,
        format!("{:x}", Sha256::digest(&original))
    );
    assert_eq!(net.video_downloads.load(Ordering::SeqCst), 3);
    let operations = super::super::scheduler::operations(&fresh).unwrap();
    let restores: Vec<_> = operations
        .iter()
        .filter(|op| op.kind == "restore")
        .collect();
    assert_eq!(restores.len(), 1);
    assert_eq!(restores[0].state, "completed");
    assert!(restores[0].last_error.is_none());
    {
        let data = net.data.lock().unwrap();
        assert_eq!((data.events.clone(), data.media.clone()), before);
    }
    fresh.shutdown().await;
    core.shutdown().await;
}
#[tokio::test]
async fn deletion_only_wakes_the_queue_and_keeps_the_permit_until_claimed() {
    let root = tempfile::tempdir().unwrap();
    let core = open(root.path());
    let provider = setup(&core, Arc::new(Remote::default()), None);
    let (asset, _) = seed(&core, root.path());
    let discovery = core.backup_wake.notified();
    tokio::pin!(discovery);
    discovery.as_mut().enable();
    super::super::sync_delete::delete(&core, &provider.scope().space_id, &asset)
        .await
        .unwrap();
    tokio::select! { biased;
        _ = &mut discovery => panic!("deletion woke discovery"),
        _ = core.delete_queue_wake.notified() => (),
        _ = std::future::ready(()) => panic!("missing pending queue wakeup"),
    }
    core.shutdown().await;
}

fn pull_journal(core: &Core) -> Vec<(String, String)> {
    core.backup_db(|db| {
        let mut stmt = db.prepare("SELECT trigger,outcome,started_at_ms,ended_at_ms FROM backup_pull_runs ORDER BY run_id")?;
        let mut rows = stmt.query([])?;
        let mut runs = Vec::new();
        while let Some(row) = rows.next()? {
            let start: i64 = row.get(2)?;
            let end: i64 = row.get(3)?;
            assert!(start > 0 && start <= end);
            runs.push((row.get(0)?, row.get(1)?));
        }
        Ok(runs)
    }).unwrap()
}

#[tokio::test]
async fn pull_journal_distinguishes_worker_wakes_and_idle_manual_refresh() {
    let root = tempfile::tempdir().unwrap();
    let core = open(root.path());
    let provider = setup(&core, Arc::new(Remote::default()), None);
    let engine = SyncEngine::new(core.clone(), provider.clone());
    let (stop, cancel) = tokio::sync::watch::channel(false);
    let mut worker = Box::pin(sync_engine::run_with(
        core.clone(),
        cancel,
        Arc::new(move |_, _| Ok(provider.clone())),
    ));
    for source in ["startup", "window_wake", "backup_wake"] {
        match source {
            "window_wake" => {
                core.backup_db(|db| {
                    db.execute(
                        "UPDATE backup_window_sync SET last_success_ms=last_success_ms-30000",
                        [],
                    )?;
                    Ok(())
                })
                .unwrap();
                core.set_backup_foreground(true);
            }
            "backup_wake" => core.wake_backup(),
            _ => {}
        }
        tokio::select! { biased; _ = &mut worker => panic!("worker stopped"), _ = std::future::ready(()) => {} }
    }
    engine.refresh().await.unwrap();
    let expected = ["startup", "window_wake", "backup_wake", "manual_refresh"]
        .map(|s| (s.to_owned(), "ok".to_owned()));
    assert_eq!(pull_journal(&core), expected);
    stop.send_replace(true);
    worker.await;
    core.shutdown().await;
}

#[tokio::test]
async fn recent_window_wakes_keep_the_scheduled_deadline_even_after_manual_refresh() {
    let root = tempfile::tempdir().unwrap();
    let core = open(root.path());
    core.set_backup_foreground(false);
    let provider = setup(&core, Arc::new(Remote::default()), None);
    let engine = SyncEngine::new(core.clone(), provider.clone());
    let (stop, cancel) = tokio::sync::watch::channel(false);
    let mut worker = Box::pin(sync_engine::run_with(
        core.clone(),
        cancel,
        Arc::new(move |_, _| Ok(provider.clone())),
    ));
    tokio::select! { biased; _ = &mut worker => panic!("worker stopped"), _ = std::future::ready(()) => {} }

    // Return from background with one second left in the foreground interval.
    core.backup_db(|db| {
        db.execute("UPDATE backup_window_sync SET last_success_ms=CAST(unixepoch('subsec')*1000 AS INTEGER)-29000", [])?;
        Ok(())
    }).unwrap();
    core.set_backup_foreground(true);
    tokio::select! { biased; _ = &mut worker => panic!("worker stopped"), _ = std::future::ready(()) => {} }
    assert!(core
        .backup_db(|db| Ok(db.query_row(
            "SELECT last_skipped_ms IS NOT NULL FROM backup_window_sync",
            [],
            |r| r.get::<_, bool>(0)
        )?))
        .unwrap());
    engine.refresh().await.unwrap();
    core.set_backup_foreground(true);
    tokio::select! { biased; _ = &mut worker => panic!("worker stopped"), _ = std::future::ready(()) => {} }
    assert_eq!(pull_journal(&core).len(), 2);
    // The second skipped wake must preserve that first deadline, even though the manual refresh is newer.
    tokio::select! {
        _ = &mut worker => panic!("worker stopped"),
        result = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while pull_journal(&core).len() < 3 {
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        }) => result.expect("window wake postponed scheduled discovery"),
    }
    let expected =
        ["startup", "manual_refresh", "scheduled"].map(|s| (s.to_owned(), "ok".to_owned()));
    assert_eq!(pull_journal(&core), expected);
    stop.send_replace(true);
    worker.await;
    core.shutdown().await;
}

#[tokio::test]
async fn deferred_refresh_receives_queue_setup_failure_without_consuming_the_delete() {
    let root = tempfile::tempdir().unwrap();
    let core = open(root.path());
    let net = Arc::new(Remote::default());
    let provider = setup(&core, net.clone(), None);
    let (asset, _) = seed(&core, root.path());
    let operation =
        super::super::sync_delete::intent(&core, &provider.scope().space_id, &asset).unwrap();
    let engine = SyncEngine::new(core.clone(), provider);
    let mut refresh = Box::pin(engine.refresh());
    tokio::select! { biased; _ = &mut refresh => panic!("refresh skipped the delete"), _ = std::future::ready(()) => {} }
    let (stop, cancel) = tokio::sync::watch::channel(false);
    let worker = tokio::spawn(sync_engine::run_with(
        core.clone(),
        cancel,
        Arc::new(|_, _| anyhow::bail!("BACKUP_ACCESS_DENIED")),
    ));
    let error = tokio::time::timeout(std::time::Duration::from_secs(2), refresh)
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(error.to_string(), "BACKUP_ACCESS_DENIED");
    let pending = super::super::scheduler::operations(&core)
        .unwrap()
        .into_iter()
        .find(|op| op.operation_id == operation)
        .unwrap();
    assert_eq!(pending.state, "pending");
    assert_eq!(pending.attempt, 0);
    assert_eq!(net.pulls.load(Ordering::SeqCst), 0);
    stop.send_replace(true);
    worker.await.unwrap();
    core.shutdown().await;
}

#[tokio::test]
async fn ready_deletions_drain_before_one_catchup_shared_with_manual_refreshes() {
    let root = tempfile::tempdir().unwrap();
    let core = open(root.path());
    let net = Arc::new(Remote::default());
    let provider = setup(&core, net.clone(), None);
    for _ in 0..2 {
        let (asset, _) = seed(&core, root.path());
        let scope = provider.scope();
        let remote = super::super::sync_read::remote_id(&core, &scope.space_id, &asset).unwrap();
        core.backup_db(|db| {
            db.execute("INSERT INTO backup_operations(operation_id,space_id,asset_id,kind,state,phase,payload_json)
                VALUES(?1,?2,?3,'delete','pending','superseded',?4)",
                params![id(),scope.space_id,asset,json!({"scope":scope,"remote_asset_id":remote}).to_string()])?;
            Ok(())
        }).unwrap();
    }
    let engine = SyncEngine::new(core.clone(), provider.clone());
    let mut refreshes = Box::pin(async { tokio::join!(engine.refresh(), engine.refresh()) });
    tokio::select! { biased; _ = &mut refreshes => panic!("refresh skipped the batch"), _ = std::future::ready(()) => {} }
    assert_eq!(net.pulls.load(Ordering::SeqCst), 0);
    assert!(pull_journal(&core).is_empty());
    let (stop, cancel) = tokio::sync::watch::channel(false);
    let mut worker = Box::pin(sync_engine::run_with(
        core.clone(),
        cancel,
        Arc::new(move |_, _| Ok(provider.clone())),
    ));
    // Superseded deletes finish locally. Poll until the worker first needs a wakeup.
    tokio::select! { biased; _ = &mut worker => panic!("worker stopped"), _ = std::future::ready(()) => {} }
    let operations = super::super::scheduler::operations(&core).unwrap();
    assert_eq!(
        operations
            .iter()
            .filter(|op| op.kind == "delete" && op.state == "completed")
            .count(),
        2
    );
    tokio::select! {
        result = &mut refreshes => { result.0.unwrap(); result.1.unwrap(); }
        _ = &mut worker => panic!("worker stopped"),
        _ = tokio::time::sleep(std::time::Duration::from_secs(2)) => panic!("catch-up missed batch completion"),
    }
    assert_eq!(net.pulls.load(Ordering::SeqCst), 1);
    core.set_backup_foreground(true);
    tokio::select! { biased; _ = &mut worker => panic!("worker stopped"), _ = std::future::ready(()) => {} }
    assert_eq!(pull_journal(&core), [("post_delete".into(), "ok".into())]);
    assert!(core
        .backup_db(|db| Ok(db.query_row(
            "SELECT last_skipped_ms >= last_success_ms FROM backup_window_sync",
            [],
            |r| r.get::<_, bool>(0)
        )?))
        .unwrap());
    assert!(!core
        .backup_db(|db| Ok(db.query_row(
            "SELECT foreground_delete_busy FROM backup_pull_runs",
            [],
            |r| r.get::<_, bool>(0)
        )?))
        .unwrap());
    stop.send_replace(true);
    worker.await;
    core.shutdown().await;
}

#[tokio::test]
async fn telegram_t3_existing_background_worker_claims_telegram_queue_and_converges() {
    let root = tempfile::tempdir().unwrap();
    let core = open(root.path());
    let net = Arc::new(Remote::default());
    let p = setup(&core, net.clone(), None);
    let (asset, _) = seed(&core, root.path());
    let resolver: sync_engine::ProviderResolver = Arc::new(move |_, _| Ok(p.clone()));
    let (stop, cancel) = tokio::sync::watch::channel(false);
    let worker = tokio::spawn(sync_engine::run_with(core.clone(), cancel, resolver));
    let completed = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            if super::super::scheduler::operations(&core)
                .unwrap()
                .iter()
                .any(|o| o.asset_id.as_deref() == Some(&asset) && o.state == "completed")
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await;
    stop.send_replace(true);
    worker.await.unwrap();
    completed.unwrap();
    {
        let data = net.data.lock().unwrap();
        assert_eq!(data.media.len(), 3);
        assert_eq!(data.events.len(), 1);
    }
    core.shutdown().await;
}

#[tokio::test]
async fn changed_attachment_identity_on_parked_delete_preserves_its_file_and_allows_successor_to_finish(
) -> Result<()> {
    use crate::backup::sync_delete;
    let root = tempfile::tempdir()?;
    let core = open(root.path());
    let net = Arc::new(Remote::default());
    let provider = setup(&core, net.clone(), None);
    let engine = SyncEngine::new(core.clone(), provider.clone());
    let mut assets = Vec::new();
    for name in ["first", "second"] {
        let folder = root.path().join(name);
        std::fs::create_dir(&folder)?;
        let (asset, _) = seed(&core, &folder);
        engine.execute_upload(&claim(&core, &engine)).await?;
        assets.push(asset);
    }
    let space = &provider.scope().space_id;
    let a = sync_delete::intent(&core, space, &assets[0])?;
    let b = sync_delete::intent(&core, space, &assets[1])?;
    let mut projected = provider.projection(None)?;
    for id in &assets {
        projected.assets.get_mut(id).unwrap().deleted = true;
    }
    let revisions: Vec<_> = assets
        .iter()
        .map(|id| (id.as_str(), projected.assets[id].revision))
        .collect();
    core.backup_db(|db| {
        db.execute("UPDATE backup_operations SET phase='objects',state='running',attempt=1 WHERE operation_id=?", [&a])?;
        assert!(sync_delete::park_confirmed_archive(db,space,&a,revisions[0].1)?);
        Ok(())
    })?;
    let transport = MemoryTransport(&net);
    let (_stop, rx) = tokio::sync::watch::channel(false);
    let io = Io::new(rx.clone(), rx, None);
    let mut stray = transport
        .media(&io)
        .await?
        .into_iter()
        .find(|doc| {
            serde_json::from_str::<Envelope>(&doc.caption)
                .is_ok_and(|e| e.asset_id == assets[0] && e.kind == "video")
        })
        .unwrap();
    stray.message_id = 2000;
    stray.bytes += 1;
    {
        let mut data = net.data.lock().unwrap();
        data.media
            .insert(stray.message_id, vec![42; stray.bytes as usize]);
        data.documents.insert(stray.message_id, stray);
    }
    let err = telegram_lifecycle::reconcile_deleted_messages(
        &transport,
        &provider.target,
        &revisions,
        &io,
        &projected,
        transport.media(&io).await?,
        vec![],
    )
    .await
    .unwrap_err();
    assert_eq!(err.to_string(), "TELEGRAM_MEDIA_CHANGED");
    assert!(net.delete_batches.lock().unwrap().is_empty());
    assert!(sync_delete::recover_cleanup(
        &core,
        provider.scope(),
        &b,
        &err
    )?);
    assert!(core
        .backup_db(|db| sync_delete::parked_archive(db, space, &b))?
        .is_none());
    delete_pass(&core, &provider, &b).await?;
    assert!(root.path().join("first/original.mp4").exists());
    assert!(!root.path().join("second/original.mp4").exists());
    assert!(net.data.lock().unwrap().documents.contains_key(&2000));
    core.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn two_confirmed_deletions_share_one_message_batch_and_retry_only_the_survivor() -> Result<()>
{
    let root = tempfile::tempdir()?;
    let core = open(root.path());
    let net = Arc::new(Remote::default());
    let provider = setup(&core, net.clone(), None);
    let engine = SyncEngine::new(core.clone(), provider.clone());
    let mut assets = Vec::new();
    for name in ["first", "second"] {
        let folder = root.path().join(name);
        std::fs::create_dir(&folder)?;
        let (asset, _) = seed(&core, &folder);
        engine.execute_upload(&claim(&core, &engine)).await?;
        assets.push(asset);
    }
    let mut projected = provider.projection(None)?;
    for asset in &assets {
        projected.assets.get_mut(asset).unwrap().deleted = true;
    }
    let deletions: Vec<_> = assets
        .iter()
        .map(|id| (id.as_str(), projected.assets[id].revision))
        .collect();
    let transport = MemoryTransport(&net);
    let (_stop, rx) = tokio::sync::watch::channel(false);
    let io = Io::new(rx.clone(), rx, None);
    let videos: Vec<_> = net
        .data
        .lock()
        .unwrap()
        .documents
        .values()
        .filter(|d| serde_json::from_str::<Envelope>(&d.caption).unwrap().kind == "video")
        .cloned()
        .collect();
    assert_eq!(videos.len(), 2);
    let survivor = videos[0].message_id;
    net.keep_message.store(survivor, Ordering::SeqCst);
    let mut old_index = videos[0].clone();
    old_index.message_id = 1999;
    let mut envelope: Envelope = serde_json::from_str(&old_index.caption)?;
    envelope.kind = "library".into();
    envelope
        .asset_id
        .clone_from(&provider.target.scope.library_id);
    old_index.caption = envelope.caption()?;
    let mut edited = old_index.clone();
    edited.document_id += 1;
    net.data
        .lock()
        .unwrap()
        .documents
        .insert(old_index.message_id, edited);
    let cleanup = || {
        telegram_lifecycle::reconcile_deleted_messages(
            &transport,
            &provider.target,
            &deletions,
            &io,
            &projected,
            videos.clone(),
            vec![(old_index.message_id, vec![old_index.clone()])],
        )
    };
    assert_eq!(
        cleanup().await.unwrap_err().to_string(),
        "TELEGRAM_MEDIA_CHANGED"
    );
    assert!(net.delete_batches.lock().unwrap().is_empty());
    net.data
        .lock()
        .unwrap()
        .documents
        .insert(old_index.message_id, old_index.clone());
    assert_eq!(
        cleanup().await.unwrap_err().to_string(),
        "BACKUP_CLEANUP_PENDING"
    );
    let mut ids: Vec<_> = videos.iter().map(|v| v.message_id).collect();
    ids.push(old_index.message_id);
    ids.sort();
    assert_eq!(net.delete_batches.lock().unwrap().as_slice(), &[ids]);
    net.keep_message.store(0, Ordering::SeqCst);
    cleanup().await?;
    assert_eq!(net.delete_batches.lock().unwrap()[1], [survivor]);
    core.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn batched_cleanup_checks_every_group_before_deleting_and_retries_only_survivors() {
    let root = tempfile::tempdir().unwrap();
    let core = open(root.path());
    let net = Arc::new(Remote::default());
    let provider = setup(&core, net.clone(), None);
    let engine = SyncEngine::new(core.clone(), provider.clone());
    let (asset, _) = seed(&core, root.path());
    engine.execute_upload(&claim(&core, &engine)).await.unwrap();
    let mut projection = provider.projection(None).unwrap();
    projection.assets.get_mut(&asset).unwrap().deleted = true;
    let (_stop, stop) = tokio::sync::watch::channel(false);
    let io = Io::new(stop.clone(), stop, None);
    let transport = MemoryTransport(&net);
    let plan: Vec<_> = {
        let mut data = net.data.lock().unwrap();
        let template = data.documents.values().next().unwrap().clone();
        (1000..1102)
            .map(|id| {
                let doc = RemoteDocument {
                    message_id: id,
                    ..template.clone()
                };
                data.documents.insert(id, doc.clone());
                (id, vec![doc])
            })
            .collect()
    };
    let cleanup = |groups| {
        telegram_lifecycle::reconcile_snapshot(
            &transport,
            &provider.target,
            &asset,
            &io,
            &projection,
            vec![],
            groups,
        )
    };
    let mut changed = plan.clone();
    changed.last_mut().unwrap().1[0].bytes += 1;
    assert_eq!(
        cleanup(changed).await.unwrap_err().to_string(),
        "TELEGRAM_MEDIA_CHANGED"
    );
    assert!(net.delete_batches.lock().unwrap().is_empty());

    net.data.lock().unwrap().documents.remove(&1000);
    net.keep_message.store(1101, Ordering::SeqCst);
    assert_eq!(
        cleanup(plan.clone()).await.unwrap_err().to_string(),
        "BACKUP_CLEANUP_PENDING"
    );
    assert_eq!(
        *net.delete_batches.lock().unwrap(),
        [(1001..1101).collect::<Vec<_>>(), vec![1101]]
    );
    net.keep_message.store(0, Ordering::SeqCst);
    cleanup(plan.clone()).await.unwrap();
    assert_eq!(net.delete_batches.lock().unwrap()[2], [1101]);
    cleanup(plan).await.unwrap();
    assert_eq!(net.delete_batches.lock().unwrap().len(), 3);
    core.shutdown().await;
}

#[tokio::test]
async fn deletion_groups_media_and_archived_events_by_physical_message_and_checks_cancel_and_ownership(
) {
    let root = tempfile::tempdir().unwrap();
    let core = open(root.path());
    let net = Arc::new(Remote::default());
    let p = setup(&core, net.clone(), None);
    let engine = SyncEngine::new(core.clone(), p.clone());
    let (asset, _) = seed(&core, root.path());
    engine.execute_upload(&claim(&core, &engine)).await.unwrap();
    let mut projected = p.projection(None).unwrap();
    projected.assets.get_mut(&asset).unwrap().deleted = true;
    projected.assets.get_mut(&asset).unwrap().revision = 1000;
    let mut media: Vec<_> = net
        .data
        .lock()
        .unwrap()
        .documents
        .values()
        .cloned()
        .collect();
    assert_eq!(media.len(), 3);
    for document in &mut media {
        document.message_id = 42;
    }
    for reference in projected.references.get_mut(&asset).unwrap() {
        if let MediaLocation::TelegramMessage { message_id, .. } = &mut reference.location {
            *message_id = 42;
        }
    }
    let mut event = media[0].clone();
    event.document_id += 999;
    event.caption = "archived exact event evidence".into();
    net.group_only.store(true, Ordering::SeqCst);
    let (cancel_tx, cancel) = tokio::sync::watch::channel(false);
    let (_stop_tx, stop) = tokio::sync::watch::channel(false);
    let io = Io::new(cancel, stop, None);
    let transport = MemoryTransport(&net);
    let groups = telegram_lifecycle::reconcile_snapshot(
        &transport,
        &p.target,
        &asset,
        &io,
        &projected,
        media.clone(),
        vec![(42, vec![event.clone()])],
    )
    .await
    .unwrap();
    assert_eq!(groups, 1);
    let sent = net.removed_groups.lock().unwrap().pop().unwrap();
    assert_eq!(sent.len(), 4);
    assert!(sent.iter().any(|d| d == &event));
    let mut live = projected.assets[&asset].clone();
    live.deleted = false;
    projected.assets.insert("other".into(), live);
    projected
        .references
        .insert("other".into(), projected.references[&asset].clone());
    assert_eq!(
        telegram_lifecycle::reconcile_snapshot(
            &transport,
            &p.target,
            &asset,
            &io,
            &projected,
            media.clone(),
            vec![]
        )
        .await
        .unwrap_err()
        .to_string(),
        "BACKUP_MEDIA_IN_USE"
    );
    assert!(net.removed_groups.lock().unwrap().is_empty());
    projected.assets.remove("other");
    cancel_tx.send_replace(true);
    assert_eq!(
        telegram_lifecycle::reconcile_snapshot(
            &transport,
            &p.target,
            &asset,
            &io,
            &projected,
            media,
            vec![]
        )
        .await
        .unwrap_err()
        .to_string(),
        "TELEGRAM_TRANSFER_CANCELED"
    );
    assert!(net.removed_groups.lock().unwrap().is_empty());
    core.shutdown().await;
}

#[tokio::test]
async fn canceling_an_unpublished_send_checks_identity_without_reading_its_body() {
    use crate::backup::telegram_transfer_journal::{Entry, SendState};
    let root = tempfile::tempdir().unwrap();
    let core = open(root.path());
    let net = Arc::new(Remote::default());
    let provider = setup(&core, net.clone(), None);
    let envelope = Envelope {
        yingqu_media: 2,
        library_id: provider.scope().library_id.clone(),
        asset_id: id(),
        device_id: id(),
        operation_id: id(),
        kind: "video".into(),
        bytes: 8,
        sha256: format!("{:x}", Sha256::digest(b"expected")),
        container: "mp4".into(),
        generation: None,
    };
    let document = RemoteDocument {
        message_id: 10,
        document_id: 10,
        bytes: 8,
        caption: envelope.caption().unwrap(),
    };
    {
        let mut data = net.data.lock().unwrap();
        data.documents.insert(10, document);
        data.media.insert(10, b"corrupt!".to_vec());
    }
    let entry = Entry {
        target: provider.target.clone(),
        envelope,
        random_id: 1,
        state: SendState::Sending,
        document: None,
        last_error: None,
    };
    let (_stop, rx) = tokio::sync::watch::channel(false);
    let io = Io::new(rx.clone(), rx, None);
    telegram_lifecycle::cancel(
        &MemoryTransport(&net),
        &provider.target,
        &entry,
        &provider.projection(None).unwrap(),
        &io,
    )
    .await
    .unwrap();
    assert!(!net.data.lock().unwrap().documents.contains_key(&10));
    assert_eq!(net.body_downloads.load(Ordering::SeqCst), 0);
    core.shutdown().await;
}
