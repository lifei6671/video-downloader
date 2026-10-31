use super::*;
use crate::backup::{s3_provider, sync_read};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

const DELETED_AT: u64 = 1_700_000_000;

async fn deleted_fixture() -> (
    Fixture,
    Arc<S3Provider<Memory>>,
    SyncEngine,
    String,
    RemoteAsset,
) {
    let (f, provider, engine) = sync_fixture().await;
    engine
        .execute_upload(&claim(&engine, &f.core))
        .await
        .unwrap();
    sync_delete::delete(&f.core, &f.space.space_id, &f.id)
        .await
        .unwrap();
    let operation = deletion_operation(&f.core);
    delete_pass(&f.core, provider.as_ref(), &operation)
        .await
        .unwrap();
    let key = format!("test/records/{}.json", f.id);
    let object = f.store.get(&key).await.unwrap().unwrap();
    let mut wire: s3_provider::WireRecord = serde_json::from_slice(&object.body).unwrap();
    wire.deleted_at = Some(DELETED_AT);
    f.store
        .write(Write::Put {
            key,
            body: serde_json::to_vec(&wire).unwrap(),
            condition: Condition::Match(object.etag),
        })
        .await
        .unwrap();
    let remote = provider.read_asset(&f.id).await.unwrap().unwrap();
    (f, provider, engine, operation, remote)
}

#[tokio::test]
async fn retention_age_boundary_compacts_only_deleted_records_and_restarts_as_index() {
    let (f, provider, engine, _, remote) = deleted_fixture().await;
    let key = format!("test/records/{}.json", f.id);
    assert!(provider
        .compact_deleted_records(vec![remote.clone()], 2, DELETED_AT + 2 * 86400 - 1)
        .await
        .unwrap()
        .is_empty());
    assert!(f.store.get(&key).await.unwrap().is_some());
    let result = provider
        .compact_deleted_records(vec![remote], 2, DELETED_AT + 2 * 86400)
        .await
        .unwrap();
    assert_eq!(result.len(), 1);
    assert!(f.store.get(&key).await.unwrap().is_none());
    let indexed = provider.read_asset(&f.id).await.unwrap().unwrap();
    assert!(indexed.snapshot.deleted);
    assert!(indexed.snapshot.fields.is_empty());
    assert!(indexed.snapshot.media.is_none());
    assert_eq!(
        serde_json::from_str::<Value>(&indexed.version.value).unwrap()["archived"],
        true
    );
    engine.pull().await.unwrap();
    let gets = f.store.0.lock().unwrap().get_count;
    engine.pull().await.unwrap();
    assert_eq!(
        f.store.0.lock().unwrap().get_count - gets,
        1,
        "unchanged index only needs the existing library GET"
    );
    let (_root, other_core, other, _) = second_device(&f).await;
    assert_ne!(
        f.core.backup_db(repository::device_id).unwrap(),
        other_core.backup_db(repository::device_id).unwrap()
    );
    assert!(
        other
            .read_asset(&f.id)
            .await
            .unwrap()
            .unwrap()
            .snapshot
            .deleted
    );
    assert!(
        sync_read::list(&other_core, &other.scope().space_id)
            .unwrap()
            .is_empty(),
        "new device must not show compacted deleted assets"
    );
}

#[tokio::test]
async fn retention_historical_timestamp_is_initialized_once_without_immediate_deletion() {
    let (f, provider, _, _, remote) = deleted_fixture().await;
    let key = format!("test/records/{}.json", f.id);
    let object = f.store.get(&key).await.unwrap().unwrap();
    let mut wire: Value = serde_json::from_slice(&object.body).unwrap();
    wire.as_object_mut().unwrap().remove("deleted_at");
    f.store
        .write(Write::Put {
            key: key.clone(),
            body: serde_json::to_vec(&wire).unwrap(),
            condition: Condition::Match(object.etag),
        })
        .await
        .unwrap();
    assert!(provider
        .compact_deleted_records(vec![remote.clone()], 1, DELETED_AT)
        .await
        .unwrap()
        .is_empty());
    let body = f.store.get(&key).await.unwrap().unwrap().body;
    assert_eq!(
        serde_json::from_slice::<Value>(&body).unwrap()["deleted_at"],
        DELETED_AT
    );
    assert!(provider
        .compact_deleted_records(vec![remote.clone()], 1, DELETED_AT + 600)
        .await
        .unwrap()
        .is_empty());
    assert_eq!(body, f.store.get(&key).await.unwrap().unwrap().body);
    assert_eq!(
        provider
            .compact_deleted_records(vec![remote], 1, DELETED_AT + 86400)
            .await
            .unwrap()
            .len(),
        1
    );
}

#[derive(Clone)]
struct RetentionFault {
    inner: Memory,
    fail_index: bool,
    fail_active_record: bool,
    lose_delete: Arc<AtomicBool>,
    replace_on_delete: Arc<Mutex<Option<Vec<u8>>>>,
    barrier: Option<Arc<tokio::sync::Barrier>>,
    reads: Arc<AtomicUsize>,
    hold_record_read: Option<(Arc<tokio::sync::Notify>, Arc<tokio::sync::Notify>)>,
    hold_sessions: Option<(Arc<tokio::sync::Notify>, Arc<tokio::sync::Notify>)>,
    hold_delete: Option<(Arc<tokio::sync::Notify>, Arc<tokio::sync::Notify>)>,
}
impl RetentionFault {
    fn new(inner: Memory) -> Self {
        Self {
            inner,
            fail_index: false,
            fail_active_record: false,
            lose_delete: Default::default(),
            replace_on_delete: Default::default(),
            barrier: None,
            reads: Default::default(),
            hold_delete: None,
            hold_record_read: None,
            hold_sessions: None,
        }
    }
}
impl ObjectStore for RetentionFault {
    fn multipart_sessions(&self, prefix: &str) -> StoreFuture<'_, Vec<UploadSession>> {
        let prefix = prefix.to_owned();
        Box::pin(async move {
            let value = self.inner.multipart_sessions(&prefix).await?;
            if prefix.contains("/media/") {
                if let Some((started, release)) = &self.hold_sessions {
                    started.notify_one();
                    release.notified().await;
                }
            }
            Ok(value)
        })
    }
    fn get(&self, key: &str) -> StoreFuture<'_, Option<Object>> {
        let key = key.to_owned();
        Box::pin(async move {
            let value = self.inner.get(&key).await?;
            if self.hold_record_read.is_some()
                && key.contains("/records/")
                && self.reads.fetch_add(1, Ordering::SeqCst) == 0
            {
                if let Some((started, release)) = &self.hold_record_read {
                    started.notify_one();
                    release.notified().await;
                }
            }
            if key.contains("/deletions/") && self.reads.fetch_add(1, Ordering::SeqCst) < 2 {
                if let Some(barrier) = &self.barrier {
                    barrier.wait().await;
                }
            }
            Ok(value)
        })
    }
    fn head(&self, key: &str) -> StoreFuture<'_, Option<ObjectInfo>> {
        self.inner.head(key)
    }
    fn list(&self, prefix: &str, token: Option<String>) -> StoreFuture<'_, Page> {
        self.inner.list(prefix, token)
    }
    fn stream(
        &self,
        key: &str,
        offset: u64,
        etag: Option<String>,
    ) -> StoreFuture<'_, ObjectStream> {
        self.inner.stream(key, offset, etag)
    }
    fn uploads(&self, key: &str) -> StoreFuture<'_, Vec<String>> {
        self.inner.uploads(key)
    }
    fn parts(&self, key: &str, id: &str) -> StoreFuture<'_, Vec<Part>> {
        self.inner.parts(key, id)
    }
    fn write(&self, write: Write) -> StoreFuture<'_, String> {
        Box::pin(async move {
            if matches!(&write, Write::Put {key,..} if key.contains("/deletions/"))
                && self.fail_index
            {
                bail!("BACKUP_ACCESS_DENIED");
            }
            if self.fail_active_record
                && matches!(&write, Write::Put {key,body,..} if key.contains("/records/") && serde_json::from_slice::<Value>(body).is_ok_and(|value|value["state"]=="active"))
            {
                bail!("BACKUP_NETWORK_ERROR");
            }
            let deleting = matches!(&write, Write::DeleteIfMatch { .. });
            if deleting {
                if let Some((started, release)) = &self.hold_delete {
                    started.notify_one();
                    release.notified().await;
                }
            }
            if let Write::DeleteIfMatch { key, .. } = &write {
                if let Some(body) = self.replace_on_delete.lock().unwrap().take() {
                    self.inner
                        .0
                        .lock()
                        .unwrap()
                        .objects
                        .insert(key.clone(), (body, None));
                }
            }
            let value = self.inner.write(write).await?;
            if deleting && self.lose_delete.swap(false, Ordering::SeqCst) {
                bail!("BACKUP_NETWORK_ERROR");
            }
            Ok(value)
        })
    }
}
fn fault_provider(f: &Fixture, store: RetentionFault) -> S3Provider<RetentionFault> {
    S3Provider::new(
        store,
        f.space.clone(),
        capabilities(),
        f.core.backup_db(repository::device_id).unwrap(),
    )
}

#[tokio::test]
async fn retention_failed_index_never_deletes_record_unknown_delete_is_confirmed() {
    let (f, _, _, _, remote) = deleted_fixture().await;
    let mut faulty = RetentionFault::new(f.store.clone());
    faulty.fail_index = true;
    let provider = fault_provider(&f, faulty);
    assert!(provider
        .compact_deleted_records(vec![remote.clone()], 1, DELETED_AT + 86400)
        .await
        .is_err());
    assert!(f
        .store
        .get(&format!("test/records/{}.json", f.id))
        .await
        .unwrap()
        .is_some());
    let faulty = RetentionFault::new(f.store.clone());
    faulty.lose_delete.store(true, Ordering::SeqCst);
    let provider = fault_provider(&f, faulty);
    assert_eq!(
        provider
            .compact_deleted_records(vec![remote], 1, DELETED_AT + 86400)
            .await
            .unwrap()
            .len(),
        1
    );
    assert!(
        provider
            .read_asset(&f.id)
            .await
            .unwrap()
            .unwrap()
            .snapshot
            .deleted
    );
}

#[tokio::test]
async fn retention_conditional_delete_does_not_remove_concurrent_reupload() {
    let (f, provider, engine) = sync_fixture().await;
    engine
        .execute_upload(&claim(&engine, &f.core))
        .await
        .unwrap();
    let key = format!("test/records/{}.json", f.id);
    let mut active: Value =
        serde_json::from_slice(&f.store.get(&key).await.unwrap().unwrap().body).unwrap();
    sync_delete::delete(&f.core, &f.space.space_id, &f.id)
        .await
        .unwrap();
    delete_pass(&f.core, provider.as_ref(), &deletion_operation(&f.core))
        .await
        .unwrap();
    let object = f.store.get(&key).await.unwrap().unwrap();
    let mut deleted: s3_provider::WireRecord = serde_json::from_slice(&object.body).unwrap();
    deleted.deleted_at = Some(DELETED_AT);
    f.store
        .write(Write::Put {
            key: key.clone(),
            body: serde_json::to_vec(&deleted).unwrap(),
            condition: Condition::Match(object.etag),
        })
        .await
        .unwrap();
    active["record_revision"] = (deleted.record_revision + 1).into();
    active["operation_id"] = uuid::Uuid::new_v4().to_string().into();
    let active_body = serde_json::to_vec(&active).unwrap();
    let faulty = RetentionFault::new(f.store.clone());
    *faulty.replace_on_delete.lock().unwrap() = Some(active_body.clone());
    let faulty = fault_provider(&f, faulty);
    let remote = provider.read_asset(&f.id).await.unwrap().unwrap();
    assert!(faulty
        .compact_deleted_records(vec![remote], 1, DELETED_AT + 86400)
        .await
        .unwrap()
        .is_empty());
    assert_eq!(f.store.get(&key).await.unwrap().unwrap().body, active_body);
    assert!(
        !provider
            .read_asset(&f.id)
            .await
            .unwrap()
            .unwrap()
            .snapshot
            .deleted,
        "active record takes priority over historical deletion index"
    );
}

#[tokio::test]
async fn retention_two_databases_compact_same_shard_without_losing_either_entry() {
    let (f, _, _, _, _) = deleted_fixture().await;
    let (_root, bcore, b, _) = second_device(&f).await;
    let id = format!("{}{}", &f.id[..2], &uuid::Uuid::new_v4().to_string()[2..]);
    let mut wire: Value = serde_json::from_slice(
        &f.store
            .get(&format!("test/records/{}.json", f.id))
            .await
            .unwrap()
            .unwrap()
            .body,
    )
    .unwrap();
    wire["asset_id"] = id.clone().into();
    wire["operation_id"] = uuid::Uuid::new_v4().to_string().into();
    f.store
        .write(Write::Put {
            key: format!("test/records/{id}.json"),
            body: serde_json::to_vec(&wire).unwrap(),
            condition: Condition::Absent,
        })
        .await
        .unwrap();
    let mut store = RetentionFault::new(f.store.clone());
    store.barrier = Some(Arc::new(tokio::sync::Barrier::new(2)));
    let a = fault_provider(&f, store.clone());
    let bspace = bcore
        .backup_db(repository::spaces)
        .unwrap()
        .into_iter()
        .find(|s| s.space_id == b.scope().space_id)
        .unwrap();
    let b = S3Provider::new(
        store,
        bspace,
        capabilities(),
        bcore.backup_db(repository::device_id).unwrap(),
    );
    let ar = a.read_asset(&f.id).await.unwrap().unwrap();
    let br = b.read_asset(&id).await.unwrap().unwrap();
    let (ar, br) = tokio::time::timeout(std::time::Duration::from_secs(3), async {
        tokio::join!(
            a.compact_deleted_records(vec![ar], 1, DELETED_AT + 86400),
            b.compact_deleted_records(vec![br], 1, DELETED_AT + 86400)
        )
    })
    .await
    .unwrap();
    assert_eq!(ar.unwrap().len(), 1);
    assert_eq!(br.unwrap().len(), 1);
    let index = f
        .store
        .get(&format!("test/deletions/{}.json", &f.id[..2]))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&index.body).unwrap()["assets"]
            .as_object()
            .unwrap()
            .len(),
        2
    );
    assert_ne!(a.scope().space_id, b.scope().space_id);
}

#[tokio::test]
async fn retention_reupload_after_compaction_preserves_local_text_and_survives_old_delete_pass() {
    let (f, a, engine) = sync_fixture().await;
    engine
        .execute_upload(&claim(&engine, &f.core))
        .await
        .unwrap();
    let (_root, bcore, b, bengine) = second_device(&f).await;
    let id = bcore.backup_assets().unwrap()[0].asset_id.clone();
    let file = sync_read::ensure(&bcore, b.as_ref(), &id).await.unwrap();
    local_text(&bcore, &id, "local transcript stays available");
    edit(&bcore, &id, "title", json!("title after deletion"));
    sync_delete::delete(&f.core, &f.space.space_id, &f.id)
        .await
        .unwrap();
    let deletion = deletion_operation(&f.core);
    delete_pass(&f.core, a.as_ref(), &deletion).await.unwrap();
    let key = format!("test/records/{}.json", f.id);
    let object = f.store.get(&key).await.unwrap().unwrap();
    let mut wire: s3_provider::WireRecord = serde_json::from_slice(&object.body).unwrap();
    wire.deleted_at = Some(DELETED_AT);
    f.store
        .write(Write::Put {
            key,
            body: serde_json::to_vec(&wire).unwrap(),
            condition: Condition::Match(object.etag),
        })
        .await
        .unwrap();
    a.compact_deleted_records(
        vec![a.read_asset(&f.id).await.unwrap().unwrap()],
        1,
        DELETED_AT + 86400,
    )
    .await
    .unwrap();
    bengine.pull().await.unwrap();
    assert!(file.is_file());
    assert_eq!(
        bcore.asset_transcript(&id).unwrap().unwrap().text,
        "local transcript stays available"
    );
    assert!(scheduler::operations(&bcore)
        .unwrap()
        .iter()
        .all(|op| op.kind != "upload"));
    sync_engine::request_upload_with(&bcore, b.clone(), &id).unwrap();
    let still_deleted: bool = bcore
        .backup_db(|db| {
            Ok(db.query_row(
                "SELECT remote_deleted FROM backup_space_assets WHERE local_asset_id=?",
                [&id],
                |r| r.get(0),
            )?)
        })
        .unwrap();
    assert!(
        still_deleted,
        "request acceptance must not claim the remote asset has already converged"
    );
    // Another metadata discovery during an explicit re-upload cannot cancel that request.
    bengine.pull().await.unwrap();
    bengine
        .execute_upload(&claim(&bengine, &bcore))
        .await
        .unwrap();
    let remote = b.read_asset(&f.id).await.unwrap().unwrap();
    assert!(!remote.snapshot.deleted);
    assert!(remote.snapshot.revision > wire.record_revision);
    assert_eq!(remote.snapshot.fields["title"], "title after deletion");
    assert!(remote.snapshot.transcript.is_some());
    delete_pass(&f.core, a.as_ref(), &deletion).await.unwrap();
    assert!(!a.read_asset(&f.id).await.unwrap().unwrap().snapshot.deleted);
    assert!(file.is_file());
}

#[tokio::test]
async fn retention_settings_are_local_persistent_validated_and_space_scoped() {
    let f = Fixture::new(24).await;
    let vault = Vault::default();
    stored_fixture_credentials(&f, &vault);
    assert_eq!(
        spaces::retention(&f.core, &f.space.space_id).unwrap(),
        (false, 30, None)
    );
    spaces::set_enabled(&f.core, false).await.unwrap();
    let mut update = update_for(&f.space.endpoint, &f.space.bucket, &f.space.prefix, "r2");
    update.enabled = false;
    update.credentials = None;
    update.activate = false;
    update.cleanup_deleted_records = true;
    update.deleted_record_retention_days = 14;
    spaces::save_with::<Memory>(&f.core, update, &vault, |_, _, _, _| {
        panic!("disabled policy save must not access cloud")
    })
    .await
    .unwrap();
    let root = f.root.path().to_owned();
    drop(f.core);
    let restarted = Core::open(
        &root.join("app.sqlite"),
        root.join("downloads"),
        "unused".into(),
        vec![],
        |_| {},
    )
    .unwrap();
    assert_eq!(
        spaces::retention(&restarted, &f.space.space_id).unwrap(),
        (true, 14, None)
    );
    assert_eq!(
        spaces::retention(&restarted, &uuid::Uuid::new_v4().to_string()).unwrap(),
        (false, 30, None)
    );
    let mut other = f.space.clone();
    other.space_id = uuid::Uuid::new_v4().to_string();
    other.bucket = "other-test-bucket".into();
    other.enabled = false;
    restarted
        .backup_db(|db| repository::save_space(db, &other))
        .unwrap();
    let mut other_update = update_for(&other.endpoint, &other.bucket, &other.prefix, "r2");
    other_update.enabled = false;
    other_update.activate = false;
    other_update.credentials = None;
    other_update.deleted_record_retention_days = 90;
    spaces::save_with::<Memory>(&restarted, other_update, &vault, |_, _, _, _| {
        panic!("inactive space policy stays local")
    })
    .await
    .unwrap();
    assert_eq!(
        spaces::retention(&restarted, &other.space_id).unwrap(),
        (false, 90, None)
    );
    assert_eq!(
        spaces::retention(&restarted, &f.space.space_id).unwrap(),
        (true, 14, None)
    );
    for days in [0, 3651] {
        let mut update = update_for(&f.space.endpoint, &f.space.bucket, &f.space.prefix, "r2");
        update.deleted_record_retention_days = days;
        assert_eq!(
            spaces::save_with::<Memory>(&restarted, update, &vault, |_, _, _, _| panic!(
                "invalid days precede vault/network"
            ))
            .await
            .unwrap_err()
            .to_string(),
            "BACKUP_INVALID_RETENTION_DAYS"
        );
    }
}

#[tokio::test]
async fn retention_maintenance_uses_current_space_and_persists_schedule_without_a_page() {
    let (f, provider, engine, _, _) = deleted_fixture().await;
    engine.pull().await.unwrap();
    f.core.backup_db(|db| { db.execute("INSERT INTO backup_record_retention(space_id,enabled,days,delete_verified) VALUES(?,1,1,1)",[&f.space.space_id])?; Ok(()) }).unwrap();
    let resolver: sync_engine::ProviderResolver = Arc::new(move |_, _| Ok(provider.clone()));
    sync_delete::compact_current(&f.core, resolver)
        .await
        .unwrap();
    assert!(f
        .store
        .get(&format!("test/records/{}.json", f.id))
        .await
        .unwrap()
        .is_none());
    let resolver: sync_engine::ProviderResolver =
        Arc::new(|_, _| panic!("future schedule must not access provider"));
    sync_delete::compact_current(&f.core, resolver)
        .await
        .unwrap();
    spaces::set_enabled(&f.core, false).await.unwrap();
    f.core
        .backup_db(|db| {
            db.execute("UPDATE backup_record_retention SET next_check_at=0", [])?;
            Ok(())
        })
        .unwrap();
    let resolver: sync_engine::ProviderResolver =
        Arc::new(|_, _| panic!("disabled space must not clean remote records"));
    sync_delete::compact_current(&f.core, resolver)
        .await
        .unwrap();
}

#[tokio::test]
async fn retention_bad_index_stops_cleanup_and_worker_keeps_error_and_retry_schedule() {
    let (f, provider, engine, _, remote) = deleted_fixture().await;
    engine.pull().await.unwrap();
    let key = format!("test/deletions/{}.json", &f.id[..2]);
    f.store.write(Write::Put{key,body:serde_json::to_vec(&json!({"schema_version":1,"library_id":uuid::Uuid::new_v4().to_string(),"assets":{}})).unwrap(),condition:Condition::Absent}).await.unwrap();
    assert!(provider
        .compact_deleted_records(vec![remote], 1, DELETED_AT + 86400)
        .await
        .unwrap_err()
        .to_string()
        .contains("BACKUP_INVALID_DELETION_INDEX"));
    f.core.backup_db(|db|{db.execute("INSERT INTO backup_record_retention(space_id,enabled,days,delete_verified) VALUES(?,1,1,1)",[&f.space.space_id])?;Ok(())}).unwrap();
    let resolver: sync_engine::ProviderResolver = Arc::new(move |_, _| Ok(provider.clone()));
    assert!(sync_delete::compact_current(&f.core, resolver)
        .await
        .is_err());
    let (next, error): (i64, String) = f
        .core
        .backup_db(|db| {
            Ok(db.query_row(
                "SELECT next_check_at,last_error FROM backup_record_retention WHERE space_id=?",
                [&f.space.space_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?)
        })
        .unwrap();
    assert!(next > chrono::Utc::now().timestamp());
    assert_eq!(error, "BACKUP_INVALID_DELETION_INDEX");
    assert!(spaces::get(&f.core)
        .unwrap()
        .unwrap()
        .last_error
        .unwrap()
        .contains("BACKUP_INVALID_DELETION_INDEX"));
    assert!(f
        .store
        .get(&format!("test/records/{}.json", f.id))
        .await
        .unwrap()
        .is_some());
}
#[tokio::test]
async fn retention_never_cleans_active_records_or_unsupported_delete_provider() {
    let (f, provider, engine) = sync_fixture().await;
    engine
        .execute_upload(&claim(&engine, &f.core))
        .await
        .unwrap();
    let active = provider.read_asset(&f.id).await.unwrap().unwrap();
    let writes = f.store.0.lock().unwrap().writes;
    assert!(provider
        .compact_deleted_records(vec![active.clone()], 1, u64::MAX / 2)
        .await
        .unwrap()
        .is_empty());
    assert_eq!(writes, f.store.0.lock().unwrap().writes);
    let mut caps = capabilities();
    caps.can_delete_owned_media = false;
    let unsupported = S3Provider::new(
        f.store.clone(),
        f.space.clone(),
        caps,
        f.core.backup_db(repository::device_id).unwrap(),
    );
    assert_eq!(
        unsupported
            .compact_deleted_records(vec![active], 1, DELETED_AT)
            .await
            .unwrap_err()
            .to_string(),
        "BACKUP_DELETE_UNSUPPORTED"
    );
    assert_eq!(writes, f.store.0.lock().unwrap().writes);
}

#[tokio::test]
async fn retention_space_switch_cancels_old_cleanup_then_switch_back_resumes() {
    let (f, _, engine, delete_operation, _) = deleted_fixture().await;
    engine.pull().await.unwrap();
    f.core.backup_db(|db|{db.execute("INSERT INTO backup_record_retention(space_id,enabled,days,delete_verified) VALUES(?,1,1,1)",[&f.space.space_id])?;Ok(())}).unwrap();
    let payload = scheduler::operations(&f.core)
        .unwrap()
        .into_iter()
        .find(|op| op.operation_id == delete_operation)
        .unwrap()
        .payload;
    let started = Arc::new(tokio::sync::Notify::new());
    let mut store = RetentionFault::new(f.store.clone());
    store.hold_delete = Some((started.clone(), Default::default()));
    let provider = Arc::new(fault_provider(&f, store));
    let resolver: sync_engine::ProviderResolver = Arc::new(move |_, _| Ok(provider.clone()));
    let mut other = f.space.clone();
    other.space_id = uuid::Uuid::new_v4().to_string();
    other.library_id = uuid::Uuid::new_v4().to_string();
    other.prefix = "other/".into();
    let switch = async {
        started.notified().await;
        f.core
            .backup_db(|db| {
                repository::save_space(db, &other)?;
                spaces::select_local(db, &other.space_id, true)
            })
            .unwrap();
    };
    let (result, ()) = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        tokio::join!(sync_delete::compact_current(&f.core, resolver), switch)
    })
    .await
    .unwrap();
    assert!(result.is_err());
    let key = format!("test/records/{}.json", f.id);
    assert!(f.store.get(&key).await.unwrap().is_some());
    assert!(!f
        .store
        .0
        .lock()
        .unwrap()
        .objects
        .keys()
        .any(|key| key.starts_with("other/")));
    f.core
        .backup_db(|db| {
            spaces::select_local(db, &f.space.space_id, true)?;
            db.execute(
                "UPDATE backup_record_retention SET next_check_at=0 WHERE space_id=?",
                [&f.space.space_id],
            )?;
            Ok(())
        })
        .unwrap();
    let provider = Arc::new(S3Provider::new(
        f.store.clone(),
        f.space.clone(),
        capabilities(),
        f.core.backup_db(repository::device_id).unwrap(),
    ));
    let resolver: sync_engine::ProviderResolver = Arc::new(move |_, _| Ok(provider.clone()));
    sync_delete::compact_current(&f.core, resolver)
        .await
        .unwrap();
    assert!(f.store.get(&key).await.unwrap().is_none());
    assert_eq!(
        payload,
        scheduler::operations(&f.core)
            .unwrap()
            .into_iter()
            .find(|op| op.operation_id == delete_operation)
            .unwrap()
            .payload
    );
}

#[tokio::test]
async fn retention_probe_rejects_ignored_delete_conditions_without_disabling_normal_backup() {
    let store = Memory::default();
    store.0.lock().unwrap().ignore_delete_conditions = true;
    let operation = uuid::Uuid::new_v4().to_string();
    assert_eq!(
        s3_provider::probe_with_cleanup(&store, "test/", &operation, true)
            .await
            .unwrap_err()
            .to_string(),
        "BACKUP_CONDITIONAL_DELETE_UNSUPPORTED"
    );
    assert!(store.0.lock().unwrap().objects.is_empty());
    s3_provider::probe(&store, "test/", &uuid::Uuid::new_v4().to_string())
        .await
        .unwrap();
    store.0.lock().unwrap().ignore_delete_conditions = false;
    s3_provider::probe_with_cleanup(&store, "test/", &uuid::Uuid::new_v4().to_string(), true)
        .await
        .unwrap();
    assert!(store.0.lock().unwrap().objects.is_empty());
}

#[tokio::test]
async fn retention_old_delete_interleaved_with_reupload_cannot_republish_or_remove_new_objects() {
    for gate_sessions in [false, true] {
        let (f, a, engine) = sync_fixture().await;
        engine
            .execute_upload(&claim(&engine, &f.core))
            .await
            .unwrap();
        let (_root, bcore, b, bengine) = second_device(&f).await;
        let id = bcore.backup_assets().unwrap()[0].asset_id.clone();
        sync_read::ensure(&bcore, b.as_ref(), &id).await.unwrap();
        sync_delete::delete(&f.core, &f.space.space_id, &f.id)
            .await
            .unwrap();
        let deletion = deletion_operation(&f.core);
        delete_pass(&f.core, a.as_ref(), &deletion).await.unwrap();
        bengine.pull().await.unwrap();
        let deleted_revision = a
            .read_asset(&f.id)
            .await
            .unwrap()
            .unwrap()
            .snapshot
            .revision;
        let started = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let mut fault = RetentionFault::new(f.store.clone());
        if gate_sessions {
            fault.hold_sessions = Some((started.clone(), release.clone()));
        } else {
            fault.hold_record_read = Some((started.clone(), release.clone()));
        }
        let guarded = fault_provider(&f, fault);
        let reupload = async {
            started.notified().await;
            sync_engine::request_upload_with(&bcore, b.clone(), &id).unwrap();
            bengine
                .execute_upload(&claim(&bengine, &bcore))
                .await
                .unwrap();
            // A later ordinary metadata update must keep the recreated incarnation's path.
            local_text(&bcore, &id, "transcript after recreation");
            bcore.backup_db(|db| {
                let cover = b"\x89PNG\r\n\x1a\nnew cover".to_vec();
                db.execute("INSERT INTO asset_covers VALUES(?1,?2) ON CONFLICT(asset_id) DO UPDATE SET body=excluded.body",params![id,cover])?;
                let raw:String=db.query_row("SELECT pending_json FROM backup_space_assets WHERE space_id=?1 AND local_asset_id=?2",params![b.scope().space_id,id],|r|r.get(0))?;
                let mut pending:sync_engine::PendingChanges=serde_json::from_str(&raw)?;
                pending.cover=pending.transcript;
                db.execute("UPDATE backup_space_assets SET pending_json=?3 WHERE space_id=?1 AND local_asset_id=?2",params![b.scope().space_id,id,serde_json::to_string(&pending)?])?;
                Ok(())
            }).unwrap();
            bengine
                .execute_upload(&claim(&bengine, &bcore))
                .await
                .unwrap();
            release.notify_one();
        };
        let (old, ()) = tokio::time::timeout(std::time::Duration::from_secs(3), async {
            tokio::join!(delete_pass(&f.core, &guarded, &deletion), reupload)
        })
        .await
        .unwrap();
        old.unwrap();
        let remote = a.read_asset(&f.id).await.unwrap().unwrap();
        assert!(!remote.snapshot.deleted);
        let media = remote.snapshot.media.clone().unwrap();
        let MediaLocation::S3Object { locator } = media.location else {
            panic!("expected S3 media");
        };
        assert!(locator.rsplit('/').next().unwrap().contains(&format!("-r{deleted_revision}-")));
        assert!(
            f.store.head(&locator).await.unwrap().is_some(),
            "old cleanup removed reuploaded media at gate {gate_sessions}"
        );
        for reference in [
            remote.snapshot.cover.as_ref(),
            remote.snapshot.transcript.as_ref(),
        ] {
            let MediaLocation::S3Object { locator } = &reference.unwrap().location else {
                panic!("S3 reference expected");
            };
            assert!(locator.rsplit('/').next().unwrap().contains(&format!("-r{deleted_revision}-")));
            assert!(
                f.store.head(locator).await.unwrap().is_some(),
                "old cleanup removed subsequent metadata at gate {gate_sessions}"
            );
        }
        delete_pass(&f.core, a.as_ref(), &deletion).await.unwrap();
        assert!(!a.read_asset(&f.id).await.unwrap().unwrap().snapshot.deleted);
        assert!(f.store.head(&locator).await.unwrap().is_some());
        // An unfinished new upload is protected too. A later explicit deletion still cleans it.
        let protected = format!(
            "test/media/{}/reuploads/{deleted_revision}/{}/{}/{}/video.mp4",
            f.id,
            "a".repeat(64),
            bcore.backup_db(repository::device_id).unwrap(),
            uuid::Uuid::new_v4()
        );
        let upload_id = f
            .store
            .write(Write::CreateMultipart {
                key: protected.clone(),
                sha256: "a".repeat(64),
            })
            .await
            .unwrap();
        // Temporarily expose an older tombstone to exercise the object/session generation filter directly.
        let active = f
            .store
            .get(&format!("test/records/{}.json", f.id))
            .await
            .unwrap()
            .unwrap();
        let mut old_wire: Value = serde_json::from_slice(&active.body).unwrap();
        old_wire["state"] = "deleted".into();
        old_wire["record_revision"] = deleted_revision.into();
        old_wire["fields"] = json!({});
        for field in ["media", "cover", "transcript"] {
            old_wire[field] = Value::Null;
        }
        f.store
            .write(Write::Put {
                key: format!("test/records/{}.json", f.id),
                body: serde_json::to_vec(&old_wire).unwrap(),
                condition: Condition::Match(active.etag),
            })
            .await
            .unwrap();
        a.reconcile_deleted_asset(&f.id, &deletion).await.unwrap();
        assert!(f.store.0.lock().unwrap().uploads.contains_key(&upload_id));
        assert!(f.store.head(&locator).await.unwrap().is_some());
        // Restore the active record, then delete explicitly: its newer revision collects generation N.
        let current = f
            .store
            .get(&format!("test/records/{}.json", f.id))
            .await
            .unwrap()
            .unwrap();
        f.store
            .write(Write::Put {
                key: format!("test/records/{}.json", f.id),
                body: active.body,
                condition: Condition::Match(current.etag),
            })
            .await
            .unwrap();
        sync_delete::delete(&f.core, &f.space.space_id, &f.id)
            .await
            .unwrap();
        assert_eq!(deletion_operation(&f.core), deletion);
        delete_pass(&f.core, a.as_ref(), &deletion).await.unwrap();
        let repeated = scheduler::operations(&f.core)
            .unwrap()
            .into_iter()
            .find(|op| op.operation_id == deletion)
            .unwrap();
        assert_eq!(
            repeated.payload["cleanup_pass"], 1,
            "new explicit deletion must restart its verification passes"
        );
        assert!(f.store.head(&locator).await.unwrap().is_none());
        assert!(!f.store.0.lock().unwrap().uploads.contains_key(&upload_id));
    }
}

#[tokio::test]
async fn retention_reupload_preserves_unresolved_text_and_missing_reference_conflicts() {
    let (f, a, engine) = sync_fixture().await;
    engine
        .execute_upload(&claim(&engine, &f.core))
        .await
        .unwrap();
    let (root, bcore, b, bengine) = second_device(&f).await;
    let id = bcore.backup_assets().unwrap()[0].asset_id.clone();
    sync_read::ensure(&bcore, b.as_ref(), &id).await.unwrap();
    // C downloaded the video before the old conflicts existed and never observed them.
    let (_croot, ccore, c, cengine) = second_device(&f).await;
    let cid = ccore.backup_assets().unwrap()[0].asset_id.clone();
    sync_read::ensure(&ccore, c.as_ref(), &cid).await.unwrap();
    edit(&f.core, &f.id, "title", json!("A title"));
    edit(&bcore, &id, "title", json!("B conflict title"));
    engine
        .execute_upload(&claim(&engine, &f.core))
        .await
        .unwrap();
    bengine
        .execute_upload(&claim(&bengine, &bcore))
        .await
        .unwrap();
    let key = format!("test/records/{}.json", f.id);
    let object = f.store.get(&key).await.unwrap().unwrap();
    let mut wire: Value = serde_json::from_slice(&object.body).unwrap();
    let reference_key = format!(
        "test/transcripts/{}/{}/{}/{}/transcript.json",
        f.id,
        "b".repeat(64),
        f.core.backup_db(repository::device_id).unwrap(),
        uuid::Uuid::new_v4()
    );
    let body = b"{\"text\":\"reference conflict\"}".to_vec();
    f.store
        .write(Write::Put {
            key: reference_key.clone(),
            body: body.clone(),
            condition: Condition::Absent,
        })
        .await
        .unwrap();
    wire["conflicts"].as_array_mut().unwrap().push(json!({"field":"transcript","value":{"key":reference_key,"bytes":body.len(),"sha256":tag(&body),"container":"json"},"device_id":bcore.backup_db(repository::device_id).unwrap(),"operation_id":uuid::Uuid::new_v4().to_string()}));
    wire["record_revision"] = (wire["record_revision"].as_u64().unwrap() + 1).into();
    wire["operation_id"] = uuid::Uuid::new_v4().to_string().into();
    f.store
        .write(Write::Put {
            key,
            body: serde_json::to_vec(&wire).unwrap(),
            condition: Condition::Match(object.etag),
        })
        .await
        .unwrap();
    engine.pull().await.unwrap();
    bengine.pull().await.unwrap();
    let before = crate::backup::conflicts::export_data(&bcore, &b.scope().space_id, &id).unwrap()
        ["conflicts"]
        .clone();
    assert_eq!(before.as_array().unwrap().len(), 2);
    sync_delete::delete(&f.core, &f.space.space_id, &f.id)
        .await
        .unwrap();
    delete_pass(&f.core, a.as_ref(), &deletion_operation(&f.core))
        .await
        .unwrap();
    assert!(f.store.get(&reference_key).await.unwrap().is_none());
    bengine.pull().await.unwrap();
    sync_engine::request_upload_with(&bcore, b.clone(), &id).unwrap();
    cengine.pull().await.unwrap();
    sync_engine::request_upload_with(&ccore, c.clone(), &cid).unwrap();
    cengine
        .execute_upload(&claim(&cengine, &ccore))
        .await
        .unwrap();
    assert!(c
        .read_asset(&f.id)
        .await
        .unwrap()
        .unwrap()
        .snapshot
        .conflicts
        .is_empty());
    // Production discovery runs independently and can arrive before the queued upload captures.
    bengine.pull().await.unwrap();
    let discovered = conflicts::export_data(&bcore, &b.scope().space_id, &id).unwrap();
    for previous in before.as_array().unwrap() {
        assert!(
            discovered["conflicts"]
                .as_array()
                .unwrap()
                .contains(previous),
            "active discovery discarded a retained local conflict before capture"
        );
    }
    drop(bengine);
    drop(bcore);
    let bcore = Core::open(
        &root.path().join("b.sqlite"),
        root.path().join("b-downloads"),
        "unused".into(),
        vec![],
        |_| {},
    )
    .unwrap();
    let bengine = SyncEngine::new(bcore.clone(), b.clone());
    // An explicit retry after restart clears the obsolete recreate flag, not retained versions.
    sync_engine::request_upload_with(&bcore, b.clone(), &id).unwrap();
    bengine
        .execute_upload(&claim(&bengine, &bcore))
        .await
        .unwrap();
    let after = crate::backup::conflicts::export_data(&bcore, &b.scope().space_id, &id).unwrap();
    let versions = after["conflicts"].as_array().unwrap();
    let raw:String=bcore.backup_db(|db| Ok(db.query_row("SELECT pending_json FROM backup_space_assets WHERE space_id=?1 AND local_asset_id=?2",params![b.scope().space_id,id],|r|r.get(0))?)).unwrap();
    assert!(
        serde_json::from_str::<sync_engine::PendingChanges>(&raw)
            .unwrap()
            .retained_conflicts
            .is_empty(),
        "confirmed publication did not acknowledge the retained versions"
    );
    assert_eq!(
        versions.len(),
        3,
        "two old versions plus the new title conflict must survive"
    );
    for previous in before.as_array().unwrap() {
        assert!(
            versions.contains(previous),
            "re-upload discarded unresolved conflict evidence"
        );
    }
    let local = crate::backup::conflicts::local_data(&bcore, &b.scope().space_id, &id).unwrap();
    let text = local
        .conflicts
        .iter()
        .find(|c| c.field == "title" && c.value == "B conflict title")
        .unwrap();
    assert_eq!(
        crate::backup::conflicts::detail(&bcore, b.as_ref(), &id, &text.id)
            .await
            .unwrap()
            .value,
        "B conflict title"
    );
    let reference = local
        .conflicts
        .iter()
        .find(|c| c.field == "transcript")
        .unwrap();
    let missing = crate::backup::conflicts::detail(&bcore, b.as_ref(), &id, &reference.id)
        .await
        .unwrap();
    assert!(!missing.can_adopt);
    assert!(missing.value.contains("引用记录已保留"));
    assert_eq!(
        after["conflicts"],
        crate::backup::conflicts::export_data(&bcore, &b.scope().space_id, &id).unwrap()
            ["conflicts"]
    );
}

#[tokio::test]
async fn retention_stale_listing_during_compaction_does_not_permanently_hide_new_tombstone() {
    let (f, a, engine) = sync_fixture().await;
    engine
        .execute_upload(&claim(&engine, &f.core))
        .await
        .unwrap();
    let key = "test/library.json";
    let object = f.store.get(key).await.unwrap().unwrap();
    let mut library: Value = serde_json::from_slice(&object.body).unwrap();
    library["deletion_indexes"] = true.into();
    f.store
        .write(Write::Put {
            key: key.into(),
            body: serde_json::to_vec(&library).unwrap(),
            condition: Condition::Match(object.etag),
        })
        .await
        .unwrap();
    let (_root, bcore, b, bengine) = second_device(&f).await;
    let id = bcore.backup_assets().unwrap()[0].asset_id.clone();
    let file = sync_read::ensure(&bcore, b.as_ref(), &id).await.unwrap();
    let started = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    f.store.0.lock().unwrap().hold_list = Some((started.clone(), release.clone()));
    let compact = async {
        started.notified().await;
        sync_delete::delete(&f.core, &f.space.space_id, &f.id)
            .await
            .unwrap();
        delete_pass(&f.core, a.as_ref(), &deletion_operation(&f.core))
            .await
            .unwrap();
        let remote = a.read_asset(&f.id).await.unwrap().unwrap();
        let now = chrono::Utc::now().timestamp().max(0) as u64 + 86401;
        a.compact_deleted_records(vec![remote], 1, now)
            .await
            .unwrap();
        release.notify_one();
    };
    let (pull, ()) = tokio::time::timeout(std::time::Duration::from_secs(3), async {
        tokio::join!(bengine.pull(), compact)
    })
    .await
    .unwrap();
    pull.unwrap();
    bengine.pull().await.unwrap();
    let deleted: bool = bcore
        .backup_db(|db| {
            Ok(db.query_row(
                "SELECT remote_deleted FROM backup_space_assets WHERE local_asset_id=?",
                [&id],
                |r| r.get(0),
            )?)
        })
        .unwrap();
    assert!(
        deleted,
        "new index ETag was cached while its tombstone was skipped forever"
    );
    assert!(
        file.is_file(),
        "metadata discovery deleted another device's local video"
    );
}

#[tokio::test]
async fn retention_reupload_after_a_newer_delete_retries_with_latest_revision() {
    let (f, a, engine) = sync_fixture().await;
    engine
        .execute_upload(&claim(&engine, &f.core))
        .await
        .unwrap();
    let (_root, bcore, b, bengine) = second_device(&f).await;
    let id = bcore.backup_assets().unwrap()[0].asset_id.clone();
    sync_read::ensure(&bcore, b.as_ref(), &id).await.unwrap();
    let (_croot, ccore, c, cengine) = second_device(&f).await;
    let cid = ccore.backup_assets().unwrap()[0].asset_id.clone();
    sync_read::ensure(&ccore, c.as_ref(), &cid).await.unwrap();
    sync_delete::delete(&f.core, &f.space.space_id, &f.id)
        .await
        .unwrap();
    delete_pass(&f.core, a.as_ref(), &deletion_operation(&f.core))
        .await
        .unwrap();
    bengine.pull().await.unwrap();
    sync_engine::request_upload_with(&bcore, b.clone(), &id).unwrap();
    let key = format!("test/records/{}.json", f.id);
    let object = f.store.get(&key).await.unwrap().unwrap();
    let mut wire: Value = serde_json::from_slice(&object.body).unwrap();
    let newer = wire["record_revision"].as_u64().unwrap() + 2;
    wire["record_revision"] = newer.into();
    wire["operation_id"] = uuid::Uuid::new_v4().to_string().into();
    f.store
        .write(Write::Put {
            key,
            body: serde_json::to_vec(&wire).unwrap(),
            condition: Condition::Match(object.etag),
        })
        .await
        .unwrap();
    assert_eq!(
        bengine
            .execute_upload(&claim(&bengine, &bcore))
            .await
            .unwrap_err()
            .to_string(),
        "BACKUP_REUPLOAD_STALE"
    );
    assert!(b.read_asset(&f.id).await.unwrap().unwrap().snapshot.deleted);
    // execute_upload is normally wrapped by queue_loop, which persists this terminal error.
    bcore.backup_db(|db| {db.execute("UPDATE backup_operations SET state='blocked',last_error='BACKUP_REUPLOAD_STALE' WHERE kind='upload' AND state='running'",[])?;Ok(())}).unwrap();
    cengine.pull().await.unwrap();
    sync_engine::request_upload_with(&ccore, c.clone(), &cid).unwrap();
    cengine
        .execute_upload(&claim(&cengine, &ccore))
        .await
        .unwrap();
    bengine.pull().await.unwrap();
    sync_engine::request_upload_with(&bcore, b.clone(), &id).unwrap();
    bengine
        .execute_upload(&claim(&bengine, &bcore))
        .await
        .unwrap();
    let remote = b.read_asset(&f.id).await.unwrap().unwrap();
    assert!(!remote.snapshot.deleted);
    let MediaLocation::S3Object { locator } = remote.snapshot.media.unwrap().location else {
        panic!("expected S3");
    };
    assert!(locator.rsplit('/').next().unwrap().contains(&format!("-r{newer}-")));
    assert!(f.store.head(&locator).await.unwrap().is_some());
}

#[tokio::test]
async fn retention_adopting_old_reference_conflicts_uploads_into_current_incarnation() {
    let (f, a, engine) = sync_fixture().await;
    engine
        .execute_upload(&claim(&engine, &f.core))
        .await
        .unwrap();
    let (_root, bcore, b, bengine) = second_device(&f).await;
    let id = bcore.backup_assets().unwrap()[0].asset_id.clone();
    sync_read::ensure(&bcore, b.as_ref(), &id).await.unwrap();
    sync_delete::delete(&f.core, &f.space.space_id, &f.id)
        .await
        .unwrap();
    delete_pass(&f.core, a.as_ref(), &deletion_operation(&f.core))
        .await
        .unwrap();
    let revision = a
        .read_asset(&f.id)
        .await
        .unwrap()
        .unwrap()
        .snapshot
        .revision;
    bengine.pull().await.unwrap();
    sync_engine::request_upload_with(&bcore, b.clone(), &id).unwrap();
    bengine
        .execute_upload(&claim(&bengine, &bcore))
        .await
        .unwrap();
    local_text(&bcore, &id, "adopted historical transcript");
    let mut transcript = bcore
        .backup_db(|db| repository::transcript(db, &id))
        .unwrap()
        .unwrap();
    transcript.asset_id = f.id.clone();
    let transcript_body = serde_json::to_vec(&transcript).unwrap();
    // Clear this fixture edit: the next operation is solely an explicit conflict choice.
    bcore
        .backup_db(|db| {
            db.execute("UPDATE backup_space_assets SET pending_json='{}'", [])?;
            Ok(())
        })
        .unwrap();
    let key = format!("test/records/{}.json", f.id);
    let object = f.store.get(&key).await.unwrap().unwrap();
    let mut wire: Value = serde_json::from_slice(&object.body).unwrap();
    for (field, folder, extension, body) in [
        ("transcript", "transcripts", "json", transcript_body),
        (
            "cover",
            "media",
            "png",
            b"\x89PNG\r\n\x1a\nhistorical cover".to_vec(),
        ),
    ] {
        let operation = uuid::Uuid::new_v4().to_string();
        let device = bcore.backup_db(repository::device_id).unwrap();
        let old_key = format!(
            "test/{folder}/{}/{}/{device}/{operation}/{field}.{extension}",
            f.id,
            tag(&body)
        );
        f.store
            .write(Write::Put {
                key: old_key.clone(),
                body: body.clone(),
                condition: Condition::Absent,
            })
            .await
            .unwrap();
        wire["conflicts"].as_array_mut().unwrap().push(json!({"field":field,"value":{"key":old_key,"bytes":body.len(),"sha256":tag(&body),"container":extension},"device_id":device,"operation_id":operation}));
    }
    wire["record_revision"] = (wire["record_revision"].as_u64().unwrap() + 1).into();
    wire["operation_id"] = uuid::Uuid::new_v4().to_string().into();
    f.store
        .write(Write::Put {
            key,
            body: serde_json::to_vec(&wire).unwrap(),
            condition: Condition::Match(object.etag),
        })
        .await
        .unwrap();
    bengine.pull().await.unwrap();
    for field in ["transcript", "cover"] {
        let local = conflicts::local_data(&bcore, &b.scope().space_id, &id).unwrap();
        let conflict = local.conflicts.iter().find(|c| c.field == field).unwrap();
        conflicts::resolve(&bcore, b.as_ref(), &id, &conflict.id, true)
            .await
            .unwrap();
        bengine
            .execute_upload(&claim(&bengine, &bcore))
            .await
            .unwrap();
        let current = b.read_asset(&f.id).await.unwrap().unwrap();
        let reference = if field == "cover" {
            current.snapshot.cover.unwrap()
        } else {
            current.snapshot.transcript.unwrap()
        };
        let MediaLocation::S3Object { locator } = reference.location else {
            panic!("S3 reference expected");
        };
        assert!(locator.rsplit('/').next().unwrap().contains(&format!("-r{revision}-")));
        assert!(f.store.head(&locator).await.unwrap().is_some());
    }
    assert!(conflicts::local_data(&bcore, &b.scope().space_id, &id)
        .unwrap()
        .conflicts
        .is_empty());
    assert_eq!(
        sync_read::transcript(&bcore, b.as_ref(), &id)
            .await
            .unwrap()
            .unwrap()
            .text,
        "adopted historical transcript"
    );
}

#[tokio::test]
async fn retention_metadata_restart_after_another_recreation_keeps_text() {
    let (f, a, engine) = sync_fixture().await;
    engine
        .execute_upload(&claim(&engine, &f.core))
        .await
        .unwrap();
    let (_root, bcore, b, bengine) = second_device(&f).await;
    let bid = bcore.backup_assets().unwrap()[0].asset_id.clone();
    sync_read::ensure(&bcore, b.as_ref(), &bid).await.unwrap();
    async fn recreate(
        core: &Arc<Core>,
        provider: Arc<S3Provider<Memory>>,
        engine: &SyncEngine,
        id: &str,
    ) -> u64 {
        let current = provider.read_asset(id).await.unwrap().unwrap();
        let deletion = AssetChange {
            recreate: false,
            resolved_conflicts: vec![],
            asset_id: id.into(),
            base: Some(current.snapshot),
            fields: Default::default(),
            field_bases: Default::default(),
            media: None,
            cover: ReferenceChange::Unchanged,
            transcript: ReferenceChange::Unchanged,
            delete: true,
            device_id: core.backup_db(repository::device_id).unwrap(),
            operation_id: uuid::Uuid::new_v4().to_string(),
            conflicts: vec![],
        };
        sync_engine::publish(provider.as_ref(), deletion)
            .await
            .unwrap();
        engine.pull().await.unwrap();
        let revision = provider
            .read_asset(id)
            .await
            .unwrap()
            .unwrap()
            .snapshot
            .revision;
        sync_engine::request_upload_with(core, provider.clone(), id).unwrap();
        engine.execute_upload(&claim(engine, core)).await.unwrap();
        revision
    }
    recreate(&bcore, b.clone(), &bengine, &bid).await;
    engine.pull().await.unwrap();
    local_text(&f.core, &f.id, "pending text survives incarnation change");
    let operation = claim(&engine, &f.core);
    let mut fault = RetentionFault::new(f.store.clone());
    fault.fail_active_record = true;
    let failing = SyncEngine::new(f.core.clone(), Arc::new(fault_provider(&f, fault)));
    assert_eq!(
        failing
            .execute_upload(&operation)
            .await
            .unwrap_err()
            .to_string(),
        "BACKUP_CONCURRENT_UPDATE"
    );
    let owned = scheduler::operations(&f.core)
        .unwrap()
        .into_iter()
        .find(|op| op.operation_id == operation)
        .unwrap()
        .payload;
    assert!(owned.get("resume_transcript").is_some());
    let current_revision = recreate(&bcore, b.clone(), &bengine, &bid).await;
    engine.pull().await.unwrap();
    // Whole-transfer retry selects the current destination and discards the previous state.
    engine.execute_upload(&operation).await.unwrap();
    let retried=scheduler::operations(&f.core).unwrap().into_iter().find(|op|op.operation_id==operation).unwrap();
    assert_eq!(retried.state,"completed");
    let previous:serde_json::Value=serde_json::from_str(owned["resume_transcript"]["value"].as_str().unwrap()).unwrap();
    let current = a.read_asset(&f.id).await.unwrap().unwrap();
    let MediaLocation::S3Object { locator } =
        &current.snapshot.transcript.as_ref().unwrap().location
    else {
        panic!("S3 expected");
    };
    assert!(locator.rsplit('/').next().unwrap().contains(&format!("-r{current_revision}-")));
    assert_ne!(locator,previous["key"].as_str().unwrap(),"restarted transfer did not select the current destination");
    assert_eq!(
        sync_read::transcript(&f.core, a.as_ref(), &f.id)
            .await
            .unwrap()
            .unwrap()
            .text,
        "pending text survives incarnation change"
    );
}

#[tokio::test]
async fn retention_inflight_ack_only_clears_captured_conflict_versions() {
    let (f, a, engine) = sync_fixture().await;
    engine
        .execute_upload(&claim(&engine, &f.core))
        .await
        .unwrap();
    let first = ConflictVersion {
        field: "title".into(),
        value: json!("first retained version"),
        device_id: f.core.backup_db(repository::device_id).unwrap(),
        operation_id: uuid::Uuid::new_v4().to_string(),
    };
    let second = ConflictVersion {
        value: json!("later retained version"),
        operation_id: uuid::Uuid::new_v4().to_string(),
        ..first.clone()
    };
    fn retain(core: &Core, space: &str, id: &str, version: ConflictVersion) {
        core.backup_db(|db| {
            let raw:String=db.query_row("SELECT pending_json FROM backup_space_assets WHERE space_id=?1 AND local_asset_id=?2",params![space,id],|r|r.get(0))?;
            let mut pending:sync_engine::PendingChanges=serde_json::from_str(&raw)?;
            pending.retained_conflicts.push(version);
            db.execute("UPDATE backup_space_assets SET pending_json=?3 WHERE space_id=?1 AND local_asset_id=?2",params![space,id,serde_json::to_string(&pending)?])?;Ok(())
        }).unwrap();
    }
    retain(&f.core, &f.space.space_id, &f.id, first.clone());
    let started = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let mut fault = RetentionFault::new(f.store.clone());
    fault.hold_record_read = Some((started.clone(), release.clone()));
    let gated = SyncEngine::new(f.core.clone(), Arc::new(fault_provider(&f, fault)));
    let operation = claim(&gated, &f.core);
    let add_later = async {
        started.notified().await;
        retain(&f.core, &f.space.space_id, &f.id, second.clone());
        release.notify_one();
    };
    let (sent, ()) = tokio::time::timeout(std::time::Duration::from_secs(3), async {
        tokio::join!(gated.execute_upload(&operation), add_later)
    })
    .await
    .unwrap();
    sent.unwrap();
    let remote = a.read_asset(&f.id).await.unwrap().unwrap();
    assert_eq!(remote.snapshot.conflicts, vec![first.clone()]);
    let raw:String=f.core.backup_db(|db|Ok(db.query_row("SELECT pending_json FROM backup_space_assets WHERE space_id=?1 AND local_asset_id=?2",params![f.space.space_id,f.id],|r|r.get(0))?)).unwrap();
    assert_eq!(
        serde_json::from_str::<sync_engine::PendingChanges>(&raw)
            .unwrap()
            .retained_conflicts,
        vec![second.clone()]
    );
    engine
        .execute_upload(&claim(&engine, &f.core))
        .await
        .unwrap();
    let versions = a
        .read_asset(&f.id)
        .await
        .unwrap()
        .unwrap()
        .snapshot
        .conflicts;
    assert_eq!(versions.len(), 2);
    assert!(versions.contains(&first) && versions.contains(&second));
}
