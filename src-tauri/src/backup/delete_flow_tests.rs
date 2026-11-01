use super::*;

struct History {
    root: tempfile::TempDir,
    core: Arc<Core>,
    provider: Arc<S3Provider<Memory>>,
    store: Memory,
    asset: String,
    task: String,
}
async fn history_fixture() -> History {
    let (f, provider, engine) = sync_fixture().await;
    engine
        .execute_upload(&claim(&engine, &f.core))
        .await
        .unwrap();
    let task = uuid::Uuid::new_v4().to_string();
    f.core.backup_db(|db| {
        let snapshot = serde_json::json!({"id":task,"asset_id":f.id,"revision":1,"status":"completed",
            "video":{"platform":"youtube","video_id":"12345678901","canonical_url":"https://www.youtube.com/watch?v=12345678901","title":"删除流程验证"},
            "bytes":24,"total_bytes":24,"speed_bps":null,"error":null,"path":f.root.path().join("video.mp4"),
            "created_at":1,"completed_at":2,"canceling":false,"download_requested":true});
        db.execute("INSERT INTO download_tasks(id,snapshot) VALUES(?1,?2)",params![task,snapshot.to_string()])?;
        db.execute("UPDATE media_assets SET task_id=?1 WHERE asset_id=?2",params![task,f.id])?;
        Ok(())
    }).unwrap();
    let Fixture {
        root,
        core,
        store,
        space: _,
        id,
    } = f;
    drop(engine);
    drop(core);
    let core = Core::open(
        &root.path().join("app.sqlite"),
        root.path().join("downloads"),
        "unused".into(),
        vec![],
        |_| {},
    )
    .unwrap();
    History {
        root,
        core,
        provider,
        store,
        asset: id,
        task,
    }
}

#[tokio::test]
async fn delete_keeps_one_history_row_and_local_file_until_remote_cleanup_is_confirmed() {
    let f = history_fixture().await;
    crate::backup::lifecycle::delete_task(&f.core, &f.task)
        .await
        .unwrap();
    assert_eq!(f.core.list().len(), 1);
    assert!(f.root.path().join("video.mp4").is_file());
    let operation = deletion_operation(&f.core);
    f.store.0.lock().unwrap().fail_delete = true;
    assert!(delete_pass(&f.core, f.provider.as_ref(), &operation)
        .await
        .is_err());
    assert_eq!(
        f.core.list().len(),
        1,
        "a failed remote delete retains the original task"
    );
    assert!(
        f.root.path().join("video.mp4").is_file(),
        "remote failure retains the local copy"
    );
    assert_eq!(
        crate::backup::reconcile::list_for_display(&f.core, &f.provider.scope().space_id)
            .unwrap()
            .len(),
        1
    );
    f.store.0.lock().unwrap().fail_delete = false;
    delete_pass(&f.core, f.provider.as_ref(), &operation)
        .await
        .unwrap();
    assert!(!f.root.path().join("video.mp4").exists());
    assert!(f.core.list().is_empty());
    assert!(
        crate::backup::reconcile::list_for_display(&f.core, &f.provider.scope().space_id)
            .unwrap()
            .is_empty()
    );
    let op = scheduler::operations(&f.core)
        .unwrap()
        .into_iter()
        .find(|op| op.operation_id == operation)
        .unwrap();
    assert_eq!(op.phase, "cleanup_verify");
    assert_eq!(op.payload["local_cleanup_done"], true);
}

#[tokio::test]
async fn delete_retains_a_changed_local_file_after_remote_cleanup() {
    let f = history_fixture().await;
    crate::backup::lifecycle::delete_task(&f.core, &f.task)
        .await
        .unwrap();
    std::fs::write(f.root.path().join("video.mp4"), b"a newer user-owned video").unwrap();
    let operation = deletion_operation(&f.core);
    assert!(delete_pass(&f.core, f.provider.as_ref(), &operation)
        .await
        .is_err());
    assert!(
        f.provider
            .read_asset(&f.asset)
            .await
            .unwrap()
            .unwrap()
            .snapshot
            .deleted
    );
    assert!(f.root.path().join("video.mp4").is_file());
    assert_eq!(f.core.list().len(), 1);
    assert_eq!(
        scheduler::operations(&f.core)
            .unwrap()
            .into_iter()
            .find(|op| op.operation_id == operation)
            .unwrap()
            .phase,
        "local_cleanup"
    );
}

#[tokio::test]
async fn delete_resumes_finalizing_without_repeating_remote_deletion() {
    let f = history_fixture().await;
    f.core.backup_db(|db| {
        db.execute_batch("CREATE TRIGGER stop_history_delete BEFORE DELETE ON download_tasks BEGIN SELECT RAISE(ABORT,'TEST_FINALIZE_FAILED'); END;")?;
        Ok(())
    }).unwrap();
    crate::backup::lifecycle::delete_task(&f.core, &f.task)
        .await
        .unwrap();
    let operation = deletion_operation(&f.core);
    assert!(delete_pass(&f.core, f.provider.as_ref(), &operation)
        .await
        .is_err());
    assert!(!f.root.path().join("video.mp4").exists());
    assert_eq!(f.core.list().len(), 1);
    assert_eq!(
        scheduler::operations(&f.core)
            .unwrap()
            .into_iter()
            .find(|op| op.operation_id == operation)
            .unwrap()
            .phase,
        "finalizing"
    );
    let writes = f.store.0.lock().unwrap().writes;
    f.core
        .backup_db(|db| {
            db.execute_batch("DROP TRIGGER stop_history_delete")?;
            Ok(())
        })
        .unwrap();
    let History {
        root,
        core,
        provider,
        store,
        asset: _,
        task: _,
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
    delete_pass(&core, provider.as_ref(), &operation)
        .await
        .unwrap();
    assert!(core.list().is_empty());
    assert_eq!(store.0.lock().unwrap().writes, writes);
}

#[tokio::test]
async fn legacy_cleanup_verification_finishes_hiding_the_original_asset() {
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
    // Older builds had no atomic local-cleanup marker for unbound assets.
    f.core.backup_db(|db| {
        db.execute("DELETE FROM asset_hidden_history WHERE asset_id=?",[&f.id])?;
        db.execute("UPDATE backup_operations SET payload_json=json_remove(payload_json,'$.local_cleanup_done') WHERE operation_id=?",[&operation])?;
        db.execute("UPDATE asset_remote_objects SET remote_state='pending_delete' WHERE space_id=?1 AND asset_id=?2",params![f.space.space_id,f.id])?;
        Ok(())
    }).unwrap();
    assert_eq!(
        crate::backup::reconcile::list_for_display(&f.core, &f.space.space_id)
            .unwrap()
            .len(),
        1
    );
    delete_pass(&f.core, provider.as_ref(), &operation)
        .await
        .unwrap();
    assert!(
        crate::backup::reconcile::list_for_display(&f.core, &f.space.space_id)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        scheduler::operations(&f.core)
            .unwrap()
            .into_iter()
            .find(|op| op.operation_id == operation)
            .unwrap()
            .payload["local_cleanup_done"],
        true
    );
}
