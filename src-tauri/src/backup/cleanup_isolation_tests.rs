use super::*;
use serde_json::{json, Value};

fn another_asset(f: &Fixture) -> String {
    let other = uuid::Uuid::new_v4().to_string();
    f.core.backup_db(|db| {
        db.execute("INSERT INTO media_assets SELECT ?1,NULL,metadata_json,original_filename,content_sha256,content_bytes,asset_state FROM media_assets WHERE asset_id=?2",params![other,f.id])?;
        db.execute("INSERT INTO asset_local_files SELECT ?1,path,bytes,sha256,source_json,verified_at,local_state FROM asset_local_files WHERE asset_id=?2",params![other,f.id])?;
        sync_engine::register_download(db, &other)
    }).unwrap();
    other
}

#[tokio::test]
async fn a_stalled_cleanup_yields_to_new_uploads_and_is_dropped_on_shutdown() {
    let (f, provider, engine) = sync_fixture().await;
    let (old, original) = canceled_upload(&f, provider.scope());
    engine.enqueue().unwrap();
    let activity = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let wrapped = Arc::new(ReceiptProvider {
        inner: provider,
        accepted: false.into(),
        cleanup_error: Some("HANG"),
        cleanup_activity: Some(activity.clone()),
    });
    let (stop, cancel) = tokio::sync::watch::channel(false);
    let worker = tokio::spawn(sync_engine::run_with(
        f.core.clone(),
        cancel,
        Arc::new(move |_, _| Ok(wrapped.clone())),
    ));
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        while activity.load(std::sync::atomic::Ordering::SeqCst) == 0 {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let other = another_asset(&f);
    engine.enqueue().unwrap();
    f.core.wake_backup();
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            if scheduler::operations(&f.core)
                .unwrap()
                .iter()
                .any(|o| o.asset_id.as_deref() == Some(&other) && o.state == "completed")
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert!(
        activity.load(std::sync::atomic::Ordering::SeqCst) >= 1000,
        "cleanup future was not dropped"
    );
    let protected = scheduler::operations(&f.core)
        .unwrap()
        .into_iter()
        .find(|o| {
            o.asset_id.as_deref() == Some(&f.id) && o.operation_id != old && o.kind == "upload"
        })
        .unwrap();
    assert_eq!(protected.state, "retry_wait");
    assert_eq!(
        protected.last_error.as_deref(),
        Some("BACKUP_CLEANUP_PENDING")
    );
    stop.send_replace(true);
    tokio::time::timeout(std::time::Duration::from_secs(3), worker)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        scheduler::operation(&f.core, &old).unwrap().payload,
        original,
        "yield or shutdown was recorded as failure"
    );
    f.core.shutdown().await;
}

fn canceled_upload(f: &Fixture, scope: &Scope) -> (String, Value) {
    let id = uuid::Uuid::new_v4().to_string();
    let payload = json!({"scope":scope,"capture":{"evidence":"unchanged"},
        "resume_cover":{"scope":scope,"value":"private-owned-state"}});
    f.core.backup_db(|db| {
        db.execute("INSERT INTO backup_operations(operation_id,space_id,asset_id,kind,state,phase,payload_json,attempt)
            VALUES(?1,?2,?3,'upload','canceled','superseded',?4,1)", params![id,f.space.space_id,f.id,payload.to_string()])?;
        db.execute("INSERT INTO telegram_media_sends VALUES(?1,'cover',?2)", params![id,r#"{"state":"sending","random_id":123,"evidence":"preserve"}"#])?;
        Ok(())
    }).unwrap();
    (id, payload)
}

#[tokio::test]
async fn failed_cleanup_releases_other_assets_without_resending_uncertain_asset() {
    let (f, provider, engine) = sync_fixture().await;
    let (old, original) = canceled_upload(&f, provider.scope());
    let other = another_asset(&f);
    engine.enqueue().unwrap();
    let replacement = scheduler::operations(&f.core)
        .unwrap()
        .into_iter()
        .find(|o| o.asset_id.as_deref() == Some(&f.id) && o.state == "pending")
        .unwrap()
        .operation_id;
    let wrapped = Arc::new(ReceiptProvider {
        inner: provider,
        accepted: false.into(),
        cleanup_error: Some("TELEGRAM_SEND_UNCONFIRMED"),
        cleanup_activity: None,
    });
    let (stop, cancel) = tokio::sync::watch::channel(false);
    let worker = tokio::spawn(sync_engine::run_with(
        f.core.clone(),
        cancel,
        Arc::new(move |_, _| Ok(wrapped.clone())),
    ));
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if scheduler::operation(&f.core, &old).unwrap().payload["cleanup_attempt"] == 1
                && scheduler::operations(&f.core)
                    .unwrap()
                    .iter()
                    .any(|o| o.asset_id.as_deref() == Some(&other) && o.state == "completed")
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    stop.send_replace(true);
    tokio::time::timeout(std::time::Duration::from_secs(3), worker)
        .await
        .unwrap()
        .unwrap();
    let old_op = scheduler::operation(&f.core, &old).unwrap();
    assert_eq!(
        (old_op.state.as_str(), old_op.phase.as_str(), old_op.attempt),
        ("canceled", "superseded", 1)
    );
    assert_eq!(old_op.payload["resume_cover"], original["resume_cover"]);
    assert_eq!(old_op.payload["capture"], original["capture"]);
    assert_eq!(old_op.payload["cleanup_attempt"], 1);
    assert!(scheduler::uploads_to_clean(&f.core, &f.space.space_id)
        .unwrap()
        .is_empty());
    let waiting = scheduler::operation(&f.core, &replacement).unwrap();
    assert_eq!((waiting.state.as_str(), waiting.attempt), ("retry_wait", 0));
    assert_eq!(
        waiting.last_error.as_deref(),
        Some("TELEGRAM_SEND_UNCONFIRMED")
    );
    assert_eq!(waiting.payload["cleanup_dependency"], old);
    f.core
        .backup_db(|db| {
            let raw: String = db.query_row(
                "SELECT entry_json FROM telegram_media_sends WHERE operation_id=?",
                [&old],
                |r| r.get(0),
            )?;
            assert_eq!(
                raw,
                r#"{"state":"sending","random_id":123,"evidence":"preserve"}"#
            );
            let proofs: i64 = db.query_row(
                "SELECT COUNT(*) FROM backup_upload_proofs WHERE remote_asset_id=?",
                [&f.id],
                |r| r.get(0),
            )?;
            assert_eq!(proofs, 0);
            assert!(scheduler::next_sync_operation(db, &f.space.space_id)?.is_none());
            Ok(())
        })
        .unwrap();
    f.core.shutdown().await;
}

#[tokio::test]
async fn cleanup_backoff_is_bounded_and_explicit_retry_preserves_owned_evidence() {
    let (f, provider, engine) = sync_fixture().await;
    let (old, original) = canceled_upload(&f, provider.scope());
    engine.enqueue().unwrap();
    for attempt in 1..=5 {
        let op = scheduler::operation(&f.core, &old).unwrap();
        scheduler::defer_upload_cleanup(&f.core, &op, "TELEGRAM_SEND_UNCONFIRMED").unwrap();
        assert_eq!(
            scheduler::operation(&f.core, &old).unwrap().payload["cleanup_attempt"],
            attempt
        );
    }
    f.core
        .backup_db(|db| {
            db.execute(
                "UPDATE backup_operations SET next_retry_at=unixepoch()-1 WHERE operation_id=?",
                [&old],
            )?;
            Ok(())
        })
        .unwrap();
    assert!(scheduler::uploads_to_clean(&f.core, &f.space.space_id)
        .unwrap()
        .is_empty());
    sync_engine::request_upload_with(&f.core, provider, &f.id).unwrap();
    let retried = scheduler::operation(&f.core, &old).unwrap();
    assert_eq!(retried.payload["resume_cover"], original["resume_cover"]);
    assert!(retried.payload.get("cleanup_attempt").is_none());
    assert_eq!(
        scheduler::uploads_to_clean(&f.core, &f.space.space_id)
            .unwrap()
            .len(),
        1
    );
    f.core.shutdown().await;
}

#[tokio::test]
async fn cleanup_success_releases_only_its_dependency_and_does_not_revive_user_cancellation() {
    for user_canceled in [false, true] {
        let (f, provider, engine) = sync_fixture().await;
        let (old, _) = canceled_upload(&f, provider.scope());
        engine.enqueue().unwrap();
        scheduler::defer_upload_cleanup(
            &f.core,
            &scheduler::operation(&f.core, &old).unwrap(),
            "TELEGRAM_SEND_UNCONFIRMED",
        )
        .unwrap();
        let replacement = scheduler::operations(&f.core)
            .unwrap()
            .into_iter()
            .find(|o| o.operation_id != old && o.kind == "upload")
            .unwrap();
        if user_canceled {
            scheduler::cancel(&f.core, &replacement.operation_id).unwrap();
        }
        scheduler::finish_upload_cleanup(&f.core, &old).unwrap();
        let after = scheduler::operation(&f.core, &replacement.operation_id).unwrap();
        assert_eq!(
            after.state,
            if user_canceled { "canceled" } else { "pending" }
        );
        assert_eq!(after.attempt, 0);
        if !user_canceled {
            assert!(after.last_error.is_none());
            assert!(after.payload.get("cleanup_dependency").is_none());
            assert_eq!(
                f.core
                    .backup_db(|db| scheduler::next_sync_operation(db, &f.space.space_id))
                    .unwrap()
                    .unwrap()
                    .0,
                replacement.operation_id
            );
        }
        assert_eq!(
            scheduler::operation(&f.core, &old).unwrap().phase,
            "cancel_cleaned"
        );
        f.core.shutdown().await;
    }
}

#[tokio::test]
async fn cleanup_honors_flood_wait_and_network_change_remains_silent() {
    let (f, provider, engine) = sync_fixture().await;
    let (old, original) = canceled_upload(&f, provider.scope());
    engine.enqueue().unwrap();
    scheduler::defer_upload_cleanup(
        &f.core,
        &scheduler::operation(&f.core, &old).unwrap(),
        "TELEGRAM_NETWORK_CHANGED",
    )
    .unwrap();
    assert_eq!(
        scheduler::operation(&f.core, &old).unwrap().payload,
        original
    );
    scheduler::defer_upload_cleanup(
        &f.core,
        &scheduler::operation(&f.core, &old).unwrap(),
        "TELEGRAM_FLOOD_WAIT:1234",
    )
    .unwrap();
    let remaining: i64 = f
        .core
        .backup_db(|db| {
            Ok(db.query_row(
                "SELECT next_retry_at-unixepoch() FROM backup_operations WHERE operation_id=?",
                [&old],
                |r| r.get(0),
            )?)
        })
        .unwrap();
    assert!((1232..=1234).contains(&remaining));
    f.core.backup_db(|db| {
        db.execute("UPDATE backup_operations SET state='canceled' WHERE operation_id<>?1 AND kind='upload'",[&old])?;
        db.execute("UPDATE backup_operations SET state='blocked',last_error='BACKUP_ASSET_DELETED',next_retry_at=NULL WHERE operation_id=?",[&old])?;
        Ok(())
    }).unwrap();
    scheduler::defer_upload_cleanup(
        &f.core,
        &scheduler::operation(&f.core, &old).unwrap(),
        "TELEGRAM_NETWORK_CHANGED",
    )
    .unwrap();
    let remaining: i64 = f
        .core
        .backup_db(|db| {
            Ok(db.query_row(
                "SELECT next_retry_at-unixepoch() FROM backup_operations WHERE operation_id=?",
                [&old],
                |r| r.get(0),
            )?)
        })
        .unwrap();
    assert!((1..=2).contains(&remaining));
    f.core.shutdown().await;
}

#[tokio::test]
async fn cleanup_never_overwrites_a_later_delete_or_remote_tombstone() {
    for remote_deleted in [false, true] {
        let (f, provider, engine) = sync_fixture().await;
        let (old, _) = canceled_upload(&f, provider.scope());
        engine.enqueue().unwrap();
        scheduler::defer_upload_cleanup(
            &f.core,
            &scheduler::operation(&f.core, &old).unwrap(),
            "TELEGRAM_SEND_UNCONFIRMED",
        )
        .unwrap();
        let replacement = scheduler::operations(&f.core)
            .unwrap()
            .into_iter()
            .find(|o| o.operation_id != old && o.kind == "upload")
            .unwrap();
        let reason = if remote_deleted {
            "BACKUP_ASSET_DELETED"
        } else {
            "BACKUP_DELETE_PENDING"
        };
        if remote_deleted {
            f.core.backup_db(|db| {
                db.execute("UPDATE media_assets SET asset_state='deleted' WHERE asset_id=?",[&f.id])?;
                db.execute("UPDATE backup_operations SET state='blocked',last_error=?2 WHERE operation_id=?1",params![replacement.operation_id,reason])?;
                Ok(())
            }).unwrap();
        } else {
            crate::backup::sync_delete::intent(&f.core, &f.space.space_id, &f.id).unwrap();
        }
        scheduler::defer_upload_cleanup(
            &f.core,
            &scheduler::operation(&f.core, &old).unwrap(),
            "TELEGRAM_SEND_UNCONFIRMED",
        )
        .unwrap();
        let after_failure = scheduler::operation(&f.core, &replacement.operation_id).unwrap();
        assert_eq!(after_failure.state, "blocked");
        assert_eq!(after_failure.last_error.as_deref(), Some(reason));
        scheduler::finish_upload_cleanup(&f.core, &old).unwrap();
        let after_success = scheduler::operation(&f.core, &replacement.operation_id).unwrap();
        assert_eq!(after_success.state, "blocked");
        assert_eq!(after_success.last_error.as_deref(), Some(reason));
        f.core.shutdown().await;
    }
}

#[tokio::test]
async fn superseded_cleanup_reclaims_capture_atomically_and_retains_unfinished_evidence() {
    let (f, provider, _) = sync_fixture().await;
    let (old, _) = canceled_upload(&f, provider.scope());
    let body =
        json!({"cover":vec![128u8;256*1024],"transcript":{"text":"captured text"}}).to_string();
    f.core.backup_db(|db| {
        db.execute("INSERT INTO backup_operation_captures VALUES(?1,?2)",params![old,body])?;
        db.execute("INSERT INTO backup_operations(operation_id,space_id,asset_id,kind,state,phase,payload_json) VALUES('keep',?1,?2,'upload','canceled','captured','{}')",params![f.space.space_id,f.id])?;
        db.execute("INSERT INTO backup_operation_captures VALUES('keep',?1)",[&body])?;
        db.execute_batch("CREATE TRIGGER fail_capture_cleanup BEFORE DELETE ON backup_operation_captures BEGIN SELECT RAISE(ABORT,'injected capture cleanup failure'); END;")?;
        Ok(())
    }).unwrap();
    assert!(scheduler::finish_upload_cleanup(&f.core, &old).is_err());
    let old_op = scheduler::operation(&f.core, &old).unwrap();
    assert_eq!(
        (old_op.state.as_str(), old_op.phase.as_str()),
        ("canceled", "superseded")
    );
    f.core
        .backup_db(|db| {
            db.execute_batch("DROP TRIGGER fail_capture_cleanup;")?;
            Ok(())
        })
        .unwrap();
    scheduler::finish_upload_cleanup(&f.core, &old).unwrap();
    let old_op = scheduler::operation(&f.core, &old).unwrap();
    assert_eq!(
        (old_op.state.as_str(), old_op.phase.as_str()),
        ("completed", "cancel_cleaned")
    );
    f.core
        .backup_db(|db| {
            assert_eq!(
                db.query_row(
                    "SELECT COUNT(*) FROM backup_operation_captures WHERE operation_id=?",
                    [&old],
                    |r| r.get::<_, i64>(0)
                )?,
                0
            );
            assert_eq!(
                db.query_row(
                    "SELECT capture_json FROM backup_operation_captures WHERE operation_id='keep'",
                    [],
                    |r| r.get::<_, String>(0)
                )?,
                body
            );
            assert_eq!(
                db.query_row(
                    "SELECT entry_json FROM telegram_media_sends WHERE operation_id=?",
                    [&old],
                    |r| r.get::<_, String>(0)
                )?,
                r#"{"state":"sending","random_id":123,"evidence":"preserve"}"#
            );
            // A pre-fix completed row is reclaimed on restart without touching its receipt.
            db.execute(
                "INSERT INTO backup_operation_captures VALUES(?1,?2)",
                params![old, body],
            )?;
            repository::init(db)?;
            assert_eq!(
                db.query_row(
                    "SELECT COUNT(*) FROM backup_operation_captures WHERE operation_id=?",
                    [&old],
                    |r| r.get::<_, i64>(0)
                )?,
                0
            );
            assert_eq!(
                db.query_row(
                    "SELECT capture_json FROM backup_operation_captures WHERE operation_id='keep'",
                    [],
                    |r| r.get::<_, String>(0)
                )?,
                body
            );
            Ok(())
        })
        .unwrap();
    f.core.shutdown().await;
}
