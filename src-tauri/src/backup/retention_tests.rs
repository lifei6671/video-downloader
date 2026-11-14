use super::*;
use crate::backup::{s3_provider, sync_read};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicUsize, Ordering};

const DELETED_AT: u64 = 1_700_000_000;

#[derive(Clone)]
struct RetentionFault {
    inner: Memory,
    fail_active_record: bool,
    reads: Arc<AtomicUsize>,
    hold_record_read: Option<(Arc<tokio::sync::Notify>, Arc<tokio::sync::Notify>)>,
    hold_sessions: Option<(Arc<tokio::sync::Notify>, Arc<tokio::sync::Notify>)>,
}
impl RetentionFault {
    fn new(inner: Memory) -> Self {
        Self {
            inner,
            fail_active_record: false,
            reads: Default::default(),
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
            if self.fail_active_record
                && matches!(&write, Write::Put {key,body,..} if key.contains("/records/") && serde_json::from_slice::<Value>(body).is_ok_and(|value|value["state"]=="active"))
            {
                bail!("BACKUP_NETWORK_ERROR");
            }
            self.inner.write(write).await
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
async fn retention_per_video_tombstone_preserves_local_text_and_survives_old_delete_pass() {
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
    assert_eq!(wire.state, "deleted");
    assert!(wire.fields.is_empty() && wire.media.is_none());
    assert!(f.store.0.lock().unwrap().objects.keys().all(|key| !key.contains("/deletions/")));
    wire.deleted_at = Some(DELETED_AT);
    f.store
        .write(Write::Put {
            key,
            body: serde_json::to_vec(&wire).unwrap(),
            condition: Condition::Match(object.etag),
        })
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
                crate::cover_store::put(db, crate::cover_store::Owner::Asset, &id, &cover)?;
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
        assert!(locator
            .rsplit('/')
            .next()
            .unwrap()
            .contains(&format!("-r{deleted_revision}-")));
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
            assert!(locator
                .rsplit('/')
                .next()
                .unwrap()
                .contains(&format!("-r{deleted_revision}-")));
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
            "test/media/{}/video-r{deleted_revision}-{}.mp4",
            f.id,
            "a".repeat(64)
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
        old_wire["deleted_at"] = DELETED_AT.into();
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
    let body = b"{\"text\":\"reference conflict\"}".to_vec();
    let reference_key = format!("test/media/{}/transcript-{}.json", f.id, tag(&body));
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
    assert!(locator
        .rsplit('/')
        .next()
        .unwrap()
        .contains(&format!("-r{newer}-")));
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
    for (field, extension, body) in [
        ("transcript", "json", transcript_body),
        (
            "cover",
            "png",
            b"\x89PNG\r\n\x1a\nhistorical cover".to_vec(),
        ),
    ] {
        let operation = uuid::Uuid::new_v4().to_string();
        let device = bcore.backup_db(repository::device_id).unwrap();
        let old_key = format!("test/media/{}/{field}-{}.{}", f.id, tag(&body), extension);
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
        assert!(locator
            .rsplit('/')
            .next()
            .unwrap()
            .contains(&format!("-r{revision}-")));
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
    let retried = scheduler::operations(&f.core)
        .unwrap()
        .into_iter()
        .find(|op| op.operation_id == operation)
        .unwrap();
    assert_eq!(retried.state, "completed");
    let previous: serde_json::Value =
        serde_json::from_str(owned["resume_transcript"]["value"].as_str().unwrap()).unwrap();
    let current = a.read_asset(&f.id).await.unwrap().unwrap();
    let MediaLocation::S3Object { locator } =
        &current.snapshot.transcript.as_ref().unwrap().location
    else {
        panic!("S3 expected");
    };
    assert!(locator
        .rsplit('/')
        .next()
        .unwrap()
        .contains(&format!("-r{current_revision}-")));
    assert_ne!(
        locator,
        previous["key"].as_str().unwrap(),
        "restarted transfer did not select the current destination"
    );
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
