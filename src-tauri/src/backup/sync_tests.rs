#[path = "retention_tests.rs"]
mod retention_tests;
#[path = "flat_upload_tests.rs"]
mod flat_upload_tests;
use super::*;
use crate::backup::conflicts;
use crate::backup::spaces;

fn stored_fixture_credentials(f: &Fixture, vault: &Vault) {
    vault.0.lock().unwrap().insert(
        "stored-key".into(),
        serde_json::json!({"access_key_id":"existing-key","secret_access_key":"existing-secret"})
            .to_string(),
    );
    f.core
        .backup_db(|db| {
            db.execute(
                "UPDATE backup_spaces SET credential_ref='stored-key' WHERE space_id=?",
                [&f.space.space_id],
            )?;
            spaces::select_local(db, &f.space.space_id, true)
        })
        .unwrap();
}
// An accepted probe write whose response never arrives exercises cancellation and unknown commits.
struct HangingProbe {
    inner: Memory,
    started: Arc<tokio::sync::Notify>,
    dropped: Arc<std::sync::atomic::AtomicBool>,
}
struct ProbeRequestGuard(Arc<std::sync::atomic::AtomicBool>);
impl Drop for ProbeRequestGuard {
    fn drop(&mut self) {
        self.0.store(true, std::sync::atomic::Ordering::SeqCst);
    }
}
impl ObjectStore for HangingProbe {
    fn get(&self, key: &str) -> StoreFuture<'_, Option<Object>> {
        self.inner.get(key)
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
    fn write(&self, operation: Write) -> StoreFuture<'_, String> {
        Box::pin(async move {
            let _guard = ProbeRequestGuard(self.dropped.clone());
            self.inner.write(operation).await?;
            self.started.notify_one();
            std::future::pending().await
        })
    }
}
#[tokio::test]
async fn settings_enable_stop_cancels_stalled_connection_without_losing_probe_or_credentials() {
    let f = Fixture::new(24).await;
    let vault = Vault::default();
    stored_fixture_credentials(&f, &vault);
    spaces::set_enabled(&f.core, false).await.unwrap();
    spaces::set_enabled(&f.core, true).await.unwrap();
    assert!(spaces::enabled(&f.core).unwrap());
    assert!(spaces::get(&f.core).unwrap().unwrap().connection_pending);
    assert!(!spaces::get(&f.core).unwrap().unwrap().enabled);
    let started = Arc::new(tokio::sync::Notify::new());
    let dropped = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let prepare = spaces::prepare_current_with(&f.core, &vault, |_, _, _, _| {
        Ok(HangingProbe {
            inner: f.store.clone(),
            started: started.clone(),
            dropped: dropped.clone(),
        })
    });
    let stop = async {
        started.notified().await;
        // The connection holds backup_config_lock; stop must not wait for it or the store.
        tokio::time::timeout(
            std::time::Duration::from_millis(500),
            spaces::set_enabled(&f.core, false),
        )
        .await
        .unwrap()
        .unwrap();
    };
    let (result, ()) = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        tokio::join!(prepare, stop)
    })
    .await
    .unwrap();
    assert_eq!(result.unwrap_err().to_string(), "CANCELED");
    assert!(dropped.load(std::sync::atomic::Ordering::SeqCst));
    assert!(f.core.backup_config_lock.try_lock().is_ok());
    assert!(!spaces::enabled(&f.core).unwrap());
    assert!(!spaces::get(&f.core).unwrap().unwrap().enabled);
    let count: i64 = f
        .core
        .backup_db(|db| {
            Ok(
                db.query_row("SELECT COUNT(*) FROM backup_probe_cleanup", [], |r| {
                    r.get(0)
                })?,
            )
        })
        .unwrap();
    assert_eq!(count, 1);
    assert!(vault.0.lock().unwrap().contains_key("stored-key"));
    spaces::cleanup_probes_with(&f.core, &vault, |_, _, _, _| Ok(f.store.clone()))
        .await
        .unwrap();
    assert_eq!(vault.0.lock().unwrap().len(), 1);
    spaces::set_enabled(&f.core, true).await.unwrap();
    spaces::prepare_current_with(&f.core, &vault, |_, _, _, _| Ok(f.store.clone()))
        .await
        .unwrap();
    let connected = spaces::get(&f.core).unwrap().unwrap();
    assert!(connected.enabled && !connected.connection_pending);
    assert_eq!(connected.space_id, f.space.space_id);
    assert_eq!(connected.library_id, f.space.library_id);
}
#[tokio::test]
async fn settings_enable_pending_connection_worker_never_dispatches_unverified_provider() {
    let (f, provider, _) = sync_fixture().await;
    f.core
        .backup_db(|db| {
            db.execute(
                "UPDATE backup_provider_config SET capabilities_json=NULL",
                [],
            )?;
            Ok(())
        })
        .unwrap();
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let observed = calls.clone();
    let (stop, cancel) = tokio::sync::watch::channel(false);
    let worker = tokio::spawn(sync_engine::run_with(
        f.core.clone(),
        cancel,
        Arc::new(move |_, _| {
            observed.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(provider.clone())
        }),
    ));
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while spaces::get(&f.core).unwrap().unwrap().last_error.is_none() {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    stop.send_replace(true);
    f.core.wake_backup();
    tokio::time::timeout(std::time::Duration::from_secs(1), worker)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0);
    assert!(spaces::get(&f.core).unwrap().unwrap().connection_pending);
}
#[tokio::test]
async fn settings_enable_probe_deadline_drops_network_future() {
    let store = HangingProbe {
        inner: Memory::default(),
        started: Default::default(),
        dropped: Default::default(),
    };
    let operation = uuid::Uuid::new_v4().to_string();
    let error = spaces::probe_with_timeout(
        &store,
        "test/",
        &operation,
        std::time::Duration::from_millis(20),
    )
    .await
    .unwrap_err();
    assert_eq!(error.to_string(), "BACKUP_TIMEOUT");
    assert!(store.dropped.load(std::sync::atomic::Ordering::SeqCst));
}
#[tokio::test]
async fn settings_enable_pending_connection_keeps_new_download_and_stop_prevents_late_activation() {
    let f = Fixture::new(24).await;
    let vault = Vault::default();
    stored_fixture_credentials(&f, &vault);
    spaces::set_enabled(&f.core, true).await.unwrap();
    f.core
        .backup_db(|db| sync_engine::register_download(db, &f.id))
        .unwrap();
    let pending:String=f.core.backup_db(|db|Ok(db.query_row("SELECT pending_json FROM backup_space_assets WHERE space_id=? AND local_asset_id=?",params![f.space.space_id,f.id],|r|r.get(0))?)).unwrap();
    assert!(pending.contains("title"));
    // Simulate a stop while metadata preparation completes; the final commit re-reads desired state.
    let result = spaces::prepare_current_with(&f.core, &vault, |_, _, _, _| {
        f.core.backup_db(|db| {
            db.execute("UPDATE backup_sync_preference SET enabled=0", [])?;
            Ok(())
        })?;
        Ok(f.store.clone())
    })
    .await;
    assert!(result.is_ok() || result.unwrap_err().to_string() == "CANCELED");
    assert!(!spaces::enabled(&f.core).unwrap());
    assert!(!spaces::get(&f.core).unwrap().unwrap().enabled);
    assert_eq!(
        spaces::current_id(&f.core).unwrap().as_deref(),
        Some(f.space.space_id.as_str())
    );
    let after:String=f.core.backup_db(|db|Ok(db.query_row("SELECT pending_json FROM backup_space_assets WHERE space_id=? AND local_asset_id=?",params![f.space.space_id,f.id],|r|r.get(0))?)).unwrap();
    assert_eq!(pending, after);
}
#[tokio::test]
async fn single_protocol_invalid_initialization_identity_is_read_only() {
    let f = Fixture::new(24).await;
    let key = format!("{}library.json", f.space.prefix);
    let initial = f.store.get(&key).await.unwrap().unwrap();
    let marker = serde_json::json!({
        "schema_version": 2, "library_id": "invalid-identity", "mode": "migrating",
        "created_at": "2026-01-01T00:00:00Z",
        "migration": {
            "operation_id": uuid::Uuid::new_v4().to_string(),
            "original": serde_json::from_slice::<serde_json::Value>(&initial.body).unwrap(),
            "next_page": null, "converted": 0
        }
    });
    f.store.write(Write::Put {
        key: key.clone(), body: serde_json::to_vec(&marker).unwrap(),
        condition: Condition::Match(initial.etag),
    }).await.unwrap();
    let before = f.store.get(&key).await.unwrap().unwrap();
    let writes = f.store.0.lock().unwrap().writes;
    let root = tempfile::tempdir().unwrap();
    let core = Core::open(&root.path().join("new.sqlite"), root.path().join("downloads"), "unused".into(), vec![], |_| {}).unwrap();
    let vault = Vault::default();
    let mut update = update_for(&f.space.endpoint, &f.space.bucket, &f.space.prefix, "r2");
    update.activate = false;
    assert_eq!(spaces::save_with(&core, update, &vault, |_,_,_,_| Ok(f.store.clone())).await.unwrap_err().to_string(), "BACKUP_INVALID_LIBRARY");
    let after = f.store.get(&key).await.unwrap().unwrap();
    assert_eq!(before.body, after.body);
    assert_eq!(before.etag, after.etag);
    assert_eq!(writes, f.store.0.lock().unwrap().writes);
    assert!(vault.0.lock().unwrap().is_empty());
    assert!(spaces::list(&core).unwrap().is_empty());
}
#[tokio::test]
async fn single_protocol_enable_initializes_existing_data_with_stored_credentials() {
    let f = Fixture::new(24).await;
    f.upload().await.unwrap();
    let vault = Vault::default();
    stored_fixture_credentials(&f, &vault);
    let record_key = format!("{}records/{}.json", f.space.prefix, f.id);
    let before: serde_json::Value =
        serde_json::from_slice(&f.store.get(&record_key).await.unwrap().unwrap().body).unwrap();
    let video_reads = f.store.0.lock().unwrap().video_gets;
    spaces::set_enabled(&f.core, false)
    .await
    .unwrap();
    spaces::set_enabled(&f.core, true).await.unwrap();
    spaces::prepare_current_with(&f.core, &vault, |_, c, _, _| {
        assert_eq!(c.access_key_id, "existing-key");
        assert_eq!(c.secret_access_key, "existing-secret");
        Ok(f.store.clone())
    })
    .await
    .unwrap();
    let active = spaces::get(&f.core).unwrap().unwrap();
    assert!(active.enabled);
    assert_eq!(active.protocol, 2);
    assert_eq!(active.space_id, f.space.space_id);
    assert_eq!(active.library_id, f.space.library_id);
    let after: serde_json::Value =
        serde_json::from_slice(&f.store.get(&record_key).await.unwrap().unwrap().body).unwrap();
    assert_eq!(after["schema_version"], 2);
    assert_eq!(before["media"], after["media"]);
    assert_eq!(f.store.0.lock().unwrap().video_gets, video_reads);
    assert!(f.root.path().join("video.mp4").is_file());
    assert_eq!(vault.0.lock().unwrap().len(), 1);
    let writes = f.store.0.lock().unwrap().writes;
    spaces::set_enabled(&f.core, false)
    .await
    .unwrap();
    spaces::set_enabled(&f.core, true)
    .await
    .unwrap();
    assert_eq!(f.store.0.lock().unwrap().writes, writes);
}
#[tokio::test]
async fn single_protocol_save_probes_without_confirmation_or_credential_replacement() {
    let f = Fixture::new(24).await;
    f.upload().await.unwrap();
    let vault = Vault::default();
    stored_fixture_credentials(&f, &vault);
    let edit = || {
        let mut value = update_for(&f.space.endpoint, &f.space.bucket, &f.space.prefix, "r2");
        value.activate = false;
        value.credentials = None;
        value
    };
    f.store.0.lock().unwrap().ignore_conditions = true;
    assert!(
        spaces::save_with(&f.core, edit(), &vault, |_, _, _, _| Ok(f.store.clone()))
            .await
            .unwrap_err()
            .to_string()
            .contains("CONDITIONAL")
    );
    let library = f.store.get("test/library.json").await.unwrap().unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&library.body).unwrap()["schema_version"],
        1
    );
    f.store.0.lock().unwrap().ignore_conditions = false;
    let saved = spaces::save_with(&f.core, edit(), &vault, |_, c, _, _| {
        assert_eq!(c.access_key_id, "existing-key");
        Ok(f.store.clone())
    })
    .await
    .unwrap();
    assert!(saved.enabled);
    assert_eq!(saved.protocol, 2);
    assert_eq!(saved.library_id, f.space.library_id);
    assert!(!serde_json::to_value(saved)
        .unwrap()
        .as_object()
        .unwrap()
        .contains_key("protocol"));
}
#[tokio::test]
async fn single_protocol_readding_configuration_keeps_identity_and_explicit_default_selection() {
    let f = Fixture::new(24).await;
    f.upload().await.unwrap();
    let vault = Vault::default();
    stored_fixture_credentials(&f, &vault);
    spaces::remove_with(&f.core, &f.space.space_id, true, &vault)
        .await
        .unwrap();
    let mut edit = update_for(&f.space.endpoint, &f.space.bucket, &f.space.prefix, "r2");
    edit.activate = false;
    let saved = spaces::save_with(&f.core, edit, &vault, |_, _, _, _| Ok(f.store.clone()))
        .await
        .unwrap();
    assert_eq!(saved.protocol, 2);
    assert!(!saved.enabled);
    assert!(spaces::get(&f.core).unwrap().is_none());
    assert_eq!(saved.space_id, f.space.space_id);
    let chosen = spaces::select_with(&f.core, &saved.space_id, &vault, |_, _, _, _| {
        Ok(f.store.clone())
    })
    .await
    .unwrap();
    assert_eq!(chosen.library_id, f.space.library_id);
    assert!(chosen.enabled);
}
#[tokio::test]
async fn single_protocol_initialization_resumes_after_record_write_failure_without_new_keys() {
    let f = Fixture::new(24).await;
    f.upload().await.unwrap();
    let vault = Vault::default();
    stored_fixture_credentials(&f, &vault);
    f.store.0.lock().unwrap().fail_record = true;
    spaces::set_enabled(&f.core, true).await.unwrap();
    assert!(
        spaces::prepare_current_with(&f.core, &vault, |_, _, _, _| Ok(f.store.clone()))
            .await
            .is_err()
    );
    assert!(!spaces::get(&f.core).unwrap().unwrap().enabled);
    assert!(spaces::enabled(&f.core).unwrap());
    assert!(vault.0.lock().unwrap().contains_key("stored-key"));
    f.store.0.lock().unwrap().fail_record = false;
    drop(f.core);
    let restarted = Core::open(
        &f.root.path().join("app.sqlite"),
        f.root.path().join("downloads"),
        "unused".into(),
        vec![],
        |_| {},
    )
    .unwrap();
    spaces::prepare_current_with(&restarted, &vault, |_, c, _, _| {
        assert_eq!(c.access_key_id, "existing-key");
        Ok(f.store.clone())
    })
    .await
    .unwrap();
    assert_eq!(
        spaces::get(&restarted).unwrap().unwrap().library_id,
        f.space.library_id
    );
    assert!(spaces::get(&restarted).unwrap().unwrap().enabled);
}

#[tokio::test]
async fn single_protocol_test_connection_never_changes_library_or_existing_credentials() {
    let f = Fixture::new(24).await;
    let vault = Vault::default();
    stored_fixture_credentials(&f, &vault);
    let before = f.store.get("test/library.json").await.unwrap().unwrap();
    let mut edit = update_for(&f.space.endpoint, &f.space.bucket, &f.space.prefix, "r2");
    edit.credentials = None;
    spaces::test_with(&f.core, edit, &vault, |_, c, _, _| {
        assert_eq!(c.access_key_id, "existing-key");
        Ok(f.store.clone())
    })
    .await
    .unwrap();
    let after = f.store.get("test/library.json").await.unwrap().unwrap();
    assert_eq!(before.etag, after.etag);
    assert_eq!(before.body, after.body);
    assert_eq!(vault.0.lock().unwrap().len(), 1);
    assert!(vault.0.lock().unwrap().contains_key("stored-key"));
}
#[tokio::test]
async fn single_protocol_reconnect_preserves_current_baseline_pending_text_and_scoped_upload() {
    let (f, _, engine) = sync_fixture().await;
    engine
        .execute_upload(&claim(&engine, &f.core))
        .await
        .unwrap();
    edit(&f.core, &f.id, "title", serde_json::json!("unsynced title"));
    let op = claim(&engine, &f.core);
    let before = scheduler::operations(&f.core)
        .unwrap()
        .into_iter()
        .find(|o| o.operation_id == op)
        .unwrap();
    let snapshot = || {
        f.core
            .backup_db(|db| {
                Ok(db.query_row(
                    "SELECT baseline_json,pending_json FROM backup_space_assets WHERE space_id=?",
                    [&f.space.space_id],
                    |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
                )?)
            })
            .unwrap()
    };
    let state = snapshot();
    let vault = Vault::default();
    stored_fixture_credentials(&f, &vault);
    f.core
        .backup_db(|db| {
            db.execute(
                "UPDATE backup_provider_config SET protocol=1 WHERE space_id=?",
                [&f.space.space_id],
            )?;
            Ok(())
        })
        .unwrap();
    spaces::prepare_current_with(&f.core, &vault, |_, _, _, _| Ok(f.store.clone()))
        .await
        .unwrap();
    assert_eq!(state, snapshot());
    let after = scheduler::operations(&f.core)
        .unwrap()
        .into_iter()
        .find(|o| o.operation_id == op)
        .unwrap();
    assert_eq!(before.payload, after.payload);
    assert_eq!(after.state, "pending");
}

#[tokio::test]
async fn settings_actions_delete_old_default_preserves_inherited_global_preference() {
    let (f, _, _) = sync_fixture().await;
    f.core.backup_db(|db| { db.execute("DELETE FROM backup_sync_preference", [])?; Ok(()) }).unwrap();
    assert!(spaces::enabled(&f.core).unwrap());
    spaces::remove_with(&f.core, &f.space.space_id, true, &Vault::default()).await.unwrap();
    assert!(spaces::enabled(&f.core).unwrap());
    assert!(spaces::get(&f.core).unwrap().is_none());
    f.core.backup_db(repository::init).unwrap();
    assert!(spaces::enabled(&f.core).unwrap());
}

#[tokio::test]
async fn settings_actions_saving_stale_enable_flag_does_not_override_global_toggle() {
    let (f, _, _) = sync_fixture().await;
    let vault = Vault::default();
    spaces::save_with(&f.core, update_for(&f.space.endpoint, &f.space.bucket, &f.space.prefix, "r2"), &vault, |_, _, _, _| Ok(f.store.clone())).await.unwrap();
    for enabled in [false, true] {
        spaces::set_enabled(&f.core, enabled).await.unwrap();
        let mut edit = update_for(&f.space.endpoint, &f.space.bucket, &f.space.prefix, "r2");
        edit.activate = false; edit.enabled = !enabled; edit.credentials = None;
        let saved = spaces::save_with(&f.core, edit, &vault, |_, _, _, _| Ok(f.store.clone())).await.unwrap();
        assert_eq!(saved.enabled, enabled);
        assert_eq!(spaces::enabled(&f.core).unwrap(), enabled);
    }
}

#[tokio::test]
async fn settings_actions_toggle_preserves_queue_and_survives_restart_without_a_default() {
    let (f, _, engine) = sync_fixture().await;
    let op = claim(&engine, &f.core);
    let before = scheduler::operations(&f.core)
        .unwrap()
        .into_iter()
        .find(|o| o.operation_id == op)
        .unwrap();
    let writes = f.store.0.lock().unwrap().writes;
    assert!(spaces::enabled(&f.core).unwrap());
    assert!(!spaces::set_enabled(&f.core, false).await.unwrap());
    assert!(!spaces::get(&f.core).unwrap().unwrap().enabled);
    assert_eq!(
        spaces::select(&f.core, &f.space.space_id)
            .await
            .unwrap_err()
            .to_string(),
        "BACKUP_SYNC_DISABLED"
    );
    let paused = scheduler::operations(&f.core)
        .unwrap()
        .into_iter()
        .find(|o| o.operation_id == op)
        .unwrap();
    assert_eq!(paused.state, "blocked");
    assert_eq!(paused.payload, before.payload);
    spaces::set_enabled(&f.core, true).await.unwrap();
    assert_eq!(
        scheduler::operations(&f.core)
            .unwrap()
            .into_iter()
            .find(|o| o.operation_id == op)
            .unwrap()
            .state,
        "pending"
    );
    assert_eq!(f.store.0.lock().unwrap().writes, writes);
    f.core
        .backup_db(|db| {
            db.execute("DELETE FROM backup_current_space", [])?;
            repository::init(db)?;
            repository::init(db)
        })
        .unwrap();
    assert!(spaces::enabled(&f.core).unwrap());
}

#[tokio::test]
async fn settings_actions_delete_requires_confirmation_and_retains_media_edits_and_other_space() {
    let (f, _, engine) = sync_fixture().await;
    let vault = Vault::default();
    spaces::save_with(
        &f.core,
        update_for(&f.space.endpoint, &f.space.bucket, &f.space.prefix, "r2"),
        &vault,
        |_, _, _, _| Ok(f.store.clone()),
    )
    .await
    .unwrap();
    let reference = f
        .core
        .backup_db(repository::spaces)
        .unwrap()
        .into_iter()
        .find(|s| s.space_id == f.space.space_id)
        .unwrap()
        .credential_ref
        .unwrap();
    let mut other = update_for("https://s3.example.test", "other-space", "other/", "s3");
    other.activate = false;
    let other = spaces::save_with(&f.core, other, &vault, |_, _, _, _| Ok(Memory::default()))
        .await
        .unwrap();
    let op = claim(&engine, &f.core);
    let pending = r#"{"title":"未同步标题","transcript":"未同步文案"}"#;
    f.core
        .backup_db(|db| {
            db.execute(
                "UPDATE backup_space_assets SET pending_json=?2 WHERE space_id=?1",
                params![f.space.space_id, pending],
            )?;
            Ok(())
        })
        .unwrap();
    let before = scheduler::operations(&f.core)
        .unwrap()
        .into_iter()
        .find(|o| o.operation_id == op)
        .unwrap();
    let writes = f.store.0.lock().unwrap().writes;
    assert!(spaces::usage(&f.core, &f.space.space_id).unwrap().assets > 0);
    assert_eq!(
        spaces::remove_with(&f.core, &f.space.space_id, false, &vault)
            .await
            .unwrap_err()
            .to_string(),
        "BACKUP_SPACE_REFERENCED"
    );
    assert!(spaces::get(&f.core).unwrap().is_some());
    spaces::remove_with(&f.core, &f.space.space_id, true, &vault)
        .await
        .unwrap();
    assert!(spaces::get(&f.core).unwrap().is_none());
    assert_eq!(
        spaces::list(&f.core)
            .unwrap()
            .iter()
            .map(|s| s.space_id.clone())
            .collect::<Vec<_>>(),
        vec![other.space_id]
    );
    assert!(!vault.0.lock().unwrap().contains_key(&reference));
    assert_eq!(
        spaces::rename(&f.core, &f.space.space_id, "deleted")
            .unwrap_err()
            .to_string(),
        "BACKUP_SPACE_NOT_FOUND"
    );
    assert!(f.root.path().join("video.mp4").is_file());
    assert_eq!(
        f.core
            .backup_db(|db| Ok(db.query_row(
                "SELECT pending_json FROM backup_space_assets WHERE space_id=?",
                [&f.space.space_id],
                |r| r.get::<_, String>(0)
            )?))
            .unwrap(),
        pending
    );
    let after = scheduler::operations(&f.core)
        .unwrap()
        .into_iter()
        .find(|o| o.operation_id == op)
        .unwrap();
    assert_eq!(after.state, "blocked");
    assert_eq!(after.payload, before.payload);
    assert_eq!(f.store.0.lock().unwrap().writes, writes);
}

#[tokio::test]
async fn settings_actions_empty_configuration_can_be_removed_and_explicitly_readded() {
    let root = tempfile::tempdir().unwrap();
    let core = Core::open(
        &root.path().join("settings.sqlite"),
        root.path().join("downloads"),
        "unused".into(),
        vec![],
        |_| {},
    )
    .unwrap();
    let vault = Vault::default();
    let store = Memory::default();
    assert!(!spaces::enabled(&core).unwrap());
    spaces::set_enabled(&core, true).await.unwrap();
    let mut edit = update_for("https://s3.example.test", "saved-config", "saved/", "s3");
    edit.activate = false;
    let saved = spaces::save_with(&core, edit, &vault, |_, _, _, _| Ok(store.clone()))
        .await
        .unwrap();
    spaces::remove_with(&core, &saved.space_id, false, &vault)
        .await
        .unwrap();
    assert!(spaces::list(&core).unwrap().is_empty());
    assert!(spaces::enabled(&core).unwrap());
    let mut edit = update_for("https://s3.example.test", "saved-config", "saved/", "s3");
    edit.activate = false;
    let restored = spaces::save_with(&core, edit, &vault, |_, _, _, _| Ok(store.clone()))
        .await
        .unwrap();
    assert_eq!(restored.space_id, saved.space_id);
    assert_eq!(restored.library_id, saved.library_id);
    assert!(!restored.enabled);
    assert!(spaces::get(&core).unwrap().is_none());
}

#[tokio::test]
async fn p6_manual_request_does_not_report_queued_for_legacy_unknown_or_delete_intent() {
    let (f, p, _) = sync_fixture().await;
    let old = uuid::Uuid::new_v4().to_string();
    f.core.backup_db(|db|{db.execute("INSERT INTO backup_operations(operation_id,space_id,asset_id,kind,state,phase,payload_json,last_error) VALUES(?1,?2,?3,'upload','blocked','unknown','{\"upload_id\":\"old-session\"}','BACKUP_LEGACY_OUTCOME_UNKNOWN')",params![old,f.space.space_id,f.id])?;Ok(())}).unwrap();
    assert_eq!(
        sync_engine::request_upload_with(&f.core, p.clone(), &f.id)
            .unwrap_err()
            .to_string(),
        "BACKUP_LEGACY_REVIEW_REQUIRED"
    );
    let op = scheduler::operations(&f.core).unwrap().remove(0);
    assert_eq!(op.state, "blocked");
    assert_eq!(op.payload["upload_id"], "old-session");
    f.core
        .backup_db(|db| {
            db.execute(
                "UPDATE backup_operations SET state='canceled' WHERE operation_id=?",
                [&old],
            )?;
            Ok(())
        })
        .unwrap();
    assert_eq!(
        sync_engine::request_upload_with(&f.core, p.clone(), &f.id)
            .unwrap_err()
            .to_string(),
        "BACKUP_LEGACY_REVIEW_REQUIRED"
    );
    f.core
        .backup_db(|db| {
            db.execute(
                "UPDATE backup_operations SET state='completed' WHERE operation_id=?",
                [old],
            )?;
            Ok(())
        })
        .unwrap();
    crate::backup::sync_delete::intent(&f.core, &f.space.space_id, &f.id).unwrap();
    assert_eq!(
        sync_engine::request_upload_with(&f.core, p, &f.id)
            .unwrap_err()
            .to_string(),
        "BACKUP_ASSET_DELETED"
    );
}

#[tokio::test]
async fn p6_both_devices_publish_new_downloads_without_import_echo() {
    let (f, a, ae) = sync_fixture().await;
    let (root, bc, b, be) = second_device(&f).await;
    let id = uuid::Uuid::new_v4().to_string();
    let path = root.path().join("b-new.mp4");
    std::fs::write(&path, b"\0\0\0\x18ftypisom\0\0\0\0isommp42").unwrap();
    let metadata = f.core.backup_assets().unwrap()[0].metadata.clone();
    bc.backup_db(|db| {
        db.execute("INSERT INTO media_assets VALUES(?1,NULL,?2,'b-new.mp4',NULL,24,'active')",params![id,metadata.to_string()])?;
        db.execute("INSERT INTO asset_local_files(asset_id,path,bytes,local_state) VALUES(?1,?2,24,'present')",params![id,path.to_string_lossy()])?;
        sync_engine::register_download(db,&id)
    }).unwrap();
    let ao = claim(&ae, &f.core);
    let bo = claim(&be, &bc);
    let (ar, br) = tokio::join!(ae.execute_upload(&ao), be.execute_upload(&bo));
    ar.unwrap();
    br.unwrap();
    ae.pull().await.unwrap();
    be.pull().await.unwrap();
    assert_eq!(
        crate::backup::sync_read::list(&f.core, &a.scope().space_id)
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        crate::backup::sync_read::list(&bc, &b.scope().space_id)
            .unwrap()
            .len(),
        2
    );
    for core in [&f.core, &bc] {
        assert_eq!(
            core.backup_db(|db| Ok(db.query_row(
                "SELECT COUNT(*) FROM backup_upload_proofs",
                [],
                |r| r.get::<_, i64>(0)
            )?))
            .unwrap(),
            1
        );
    }
    let writes = f.store.0.lock().unwrap().writes;
    ae.enqueue().unwrap();
    be.enqueue().unwrap();
    assert_eq!(f.store.0.lock().unwrap().writes, writes);
    assert_eq!(f.store.0.lock().unwrap().video_gets, 0);
}

#[tokio::test]
async fn p6_transcript_conflicts_can_be_read_adopted_and_resumed_after_database_restart() {
    let (f, a, ae) = sync_fixture().await;
    ae.execute_upload(&claim(&ae, &f.core)).await.unwrap();
    let (root, bc, b, be) = second_device(&f).await;
    let bid = bc.backup_assets().unwrap()[0].asset_id.clone();
    local_text(&f.core, &f.id, "A words");
    local_text(&bc, &bid, "B words");
    ae.execute_upload(&claim(&ae, &f.core)).await.unwrap();
    be.execute_upload(&claim(&be, &bc)).await.unwrap();
    let data = conflicts::local_data(&bc, &b.scope().space_id, &bid).unwrap();
    let conflict = data
        .conflicts
        .iter()
        .find(|c| c.field == "transcript")
        .unwrap();
    assert_eq!(
        conflicts::detail(&bc, b.as_ref(), &bid, &conflict.id)
            .await
            .unwrap()
            .value,
        "B words"
    );
    conflicts::resolve(&bc, b.as_ref(), &bid, &conflict.id, true)
        .await
        .unwrap();
    drop(be);
    drop(bc);
    let bc = Core::open(
        &root.path().join("b.sqlite"),
        root.path().join("b-downloads"),
        "unused".into(),
        vec![],
        |_| {},
    )
    .unwrap();
    assert_eq!(
        crate::backup::sync_read::transcript(&bc, b.as_ref(), &bid)
            .await
            .unwrap()
            .unwrap()
            .text,
        "B words"
    );
    let be = SyncEngine::new(bc.clone(), b.clone());
    be.execute_upload(&claim(&be, &bc)).await.unwrap();
    ae.pull().await.unwrap();
    assert_eq!(
        crate::backup::sync_read::transcript(&f.core, a.as_ref(), &f.id)
            .await
            .unwrap()
            .unwrap()
            .text,
        "B words"
    );
    assert!(conflicts::local_data(&f.core, &a.scope().space_id, &f.id)
        .unwrap()
        .conflicts
        .is_empty());
    assert_eq!(f.store.0.lock().unwrap().video_gets, 0);
}

struct ReceiptProvider {
    inner: Arc<S3Provider<Memory>>,
    accepted: std::sync::atomic::AtomicBool,
}
impl BackupProvider for ReceiptProvider {
    fn scope(&self) -> &Scope {
        self.inner.scope()
    }
    fn capabilities(&self) -> ProviderCapabilities {
        self.inner.capabilities()
    }
    fn open_library(&self, c: bool) -> ProviderFuture<'_, LibraryDescriptor> {
        self.inner.open_library(c)
    }
    fn pull_changes(&self, c: Option<Checkpoint>) -> ProviderFuture<'_, ChangeBatch> {
        self.inner.pull_changes(c)
    }
    fn read_asset(&self, id: &str) -> ProviderFuture<'_, Option<RemoteAsset>> {
        self.inner.read_asset(id)
    }
    fn publish_change(
        &self,
        c: AssetChange,
        b: Option<VersionToken>,
    ) -> ProviderFuture<'_, PublishReceipt> {
        if self.accepted.load(std::sync::atomic::Ordering::Relaxed) {
            Box::pin(async move {
                Ok(PublishReceipt::Accepted {
                    operation_id: c.operation_id,
                })
            })
        } else {
            self.inner.publish_change(c, b)
        }
    }
    fn upload_media(
        &self,
        s: MediaSource,
        r: Option<ResumeState>,
    ) -> ProviderFuture<'_, UploadReceipt> {
        self.inner.upload_media(s, r)
    }
    fn cancel_upload(&self, r: ResumeState, o: &str) -> ProviderFuture<'_, ()> {
        self.inner.cancel_upload(r, o)
    }
    fn download_media(&self, r: MediaRef, o: u64) -> ProviderFuture<'_, MediaStream> {
        self.inner.download_media(r, o)
    }
    fn verify_media(&self, r: MediaRef) -> ProviderFuture<'_, ()> {
        self.inner.verify_media(r)
    }
    fn delete_media(&self, r: MediaRef, o: &str) -> ProviderFuture<'_, ()> {
        self.inner.delete_media(r, o)
    }
    fn reconcile_deleted_asset(&self, a: &str, o: &str) -> ProviderFuture<'_, ()> {
        self.inner.reconcile_deleted_asset(a, o)
    }
}

#[tokio::test]
async fn p6_accepted_receipt_cannot_acknowledge_changes_or_prove_uploaded_video() {
    let (f, p, _) = sync_fixture().await;
    let proxy = Arc::new(ReceiptProvider {
        inner: p,
        accepted: std::sync::atomic::AtomicBool::new(true),
    });
    let engine = SyncEngine::new(f.core.clone(), proxy.clone());
    let op = claim(&engine, &f.core);
    assert_eq!(
        engine.execute_upload(&op).await.unwrap_err().to_string(),
        "BACKUP_PUBLICATION_UNCONFIRMED"
    );
    let (ack, proof, pending): (i64, i64, String) = f
        .core
        .backup_db(|db| {
            Ok((
                db.query_row("SELECT ack_revision FROM backup_space_assets", [], |r| {
                    r.get(0)
                })?,
                db.query_row("SELECT COUNT(*) FROM backup_upload_proofs", [], |r| {
                    r.get(0)
                })?,
                db.query_row("SELECT pending_json FROM backup_space_assets", [], |r| {
                    r.get(0)
                })?,
            ))
        })
        .unwrap();
    assert_eq!((ack, proof), (0, 0));
    assert!(
        !serde_json::from_str::<sync_engine::PendingChanges>(&pending)
            .unwrap()
            .is_empty()
    );
    proxy
        .accepted
        .store(false, std::sync::atomic::Ordering::Relaxed);
    engine.execute_upload(&op).await.unwrap();
    assert_eq!(
        f.core
            .backup_db(|db| Ok(db.query_row(
                "SELECT COUNT(*) FROM backup_upload_proofs",
                [],
                |r| r.get::<_, i64>(0)
            )?))
            .unwrap(),
        1
    );
}
use crate::backup::library;

#[test]
fn p1_v3_migration_preserves_targets_operations_and_unknown_provenance() {
    let db = rusqlite::Connection::open_in_memory().unwrap();
    db.execute_batch(include_str!("schema.sql")).unwrap();
    db.execute("UPDATE backup_schema SET version=3", [])
        .unwrap();
    for (space, role) in [("a", "primary"), ("b", "reader")] {
        db.execute("INSERT INTO backup_spaces VALUES(?1,'library','https://old.example','bucket',?1,?2,'device',1,'secret-ref',1)", params![space,role]).unwrap();
    }
    db.execute(
        "INSERT INTO media_assets VALUES('asset',NULL,'{}','video.mp4',NULL,12,'active')",
        [],
    )
    .unwrap();
    db.execute("INSERT INTO asset_remote_objects(space_id,asset_id,record_key,remote_state) VALUES('a','asset','old/record','pending_delete')", []).unwrap();
    db.execute("INSERT INTO backup_operations(operation_id,space_id,asset_id,kind,state,phase,payload_json) VALUES('operation','a','asset','upload','running','completing','{\"upload_id\":\"owned-session\"}')", []).unwrap();
    repository::init(&db).unwrap();
    repository::init(&db).unwrap();
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM backup_provider_config", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        2
    );
    assert_eq!(
        db.query_row(
            "SELECT credential_ref FROM backup_spaces WHERE space_id='a'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "secret-ref"
    );
    let op: (String, String, String) = db
        .query_row(
            "SELECT state,phase,payload_json FROM backup_operations",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(
        op,
        (
            "pending".into(),
            "completing".into(),
            r#"{"upload_id":"owned-session"}"#.into()
        )
    );
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM backup_upload_proofs", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        repository::local_asset_id(&db, "a", "asset")
            .unwrap()
            .as_deref(),
        Some("asset")
    );
    assert_eq!(
        db.query_row("SELECT remote_deleted FROM backup_space_assets", [], |r| {
            r.get::<_, i64>(0)
        })
        .unwrap(),
        1
    );
    assert!(db
        .prepare("PRAGMA foreign_key_check")
        .unwrap()
        .query([])
        .unwrap()
        .next()
        .unwrap()
        .is_none());
}

#[test]
fn p1_same_remote_id_in_two_libraries_gets_separate_local_rows() {
    let db = rusqlite::Connection::open_in_memory().unwrap();
    repository::init(&db).unwrap();
    for space in ["a", "b"] {
        db.execute("INSERT INTO backup_spaces VALUES(?1,?1,'https://host','bucket',?1,'reader','device',1,NULL,1)",[space]).unwrap();
    }
    let remote = uuid::Uuid::new_v4().to_string();
    let a = repository::bind_remote_asset(&db, "a", &remote).unwrap();
    let b = repository::bind_remote_asset(&db, "b", &remote).unwrap();
    assert_ne!(a, b);
    assert_eq!(repository::bind_remote_asset(&db, "a", &remote).unwrap(), a);
    db.execute(
        "UPDATE backup_space_assets SET remote_deleted=1 WHERE space_id='a'",
        [],
    )
    .unwrap();
    assert_eq!(
        db.query_row(
            "SELECT remote_deleted FROM backup_space_assets WHERE space_id='b'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}

#[tokio::test]
async fn p1_connection_initialization_is_resumable_and_metadata_only() {
    let f = Fixture::new(24).await;
    f.upload().await.unwrap();
    let op = uuid::Uuid::new_v4().to_string();
    f.store.0.lock().unwrap().fail_record = true;
    assert!(
        library::initialize(&f.store, &f.space.prefix, &f.space.library_id, &op)
            .await
            .is_err()
    );
    let other = uuid::Uuid::new_v4().to_string();
    assert_eq!(
        library::initialize(&f.store, &f.space.prefix, &f.space.library_id, &other)
            .await
            .unwrap_err()
            .to_string(),
        "BACKUP_MIGRATION_OWNED_BY_OTHER"
    );
    f.store.0.lock().unwrap().fail_record = false;
    let before = f.store.0.lock().unwrap().video_gets;
    let upgraded = library::initialize(&f.store, &f.space.prefix, &f.space.library_id, &op)
        .await
        .unwrap();
    upgraded.validate().unwrap();
    assert!(
        f.client().connect().await.is_err(),
        "development decoder must reject canonical metadata"
    );
    library::initialize(&f.store, &f.space.prefix, &f.space.library_id, &op)
        .await
        .unwrap();
    assert_eq!(f.store.0.lock().unwrap().video_gets, before);
    let key = format!("{}records/{}.json", f.space.prefix, f.id);
    let body = f.store.get(&key).await.unwrap().unwrap().body;
    let record = crate::backup::s3_provider::validate_wire(
        &body,
        &f.space.library_id,
        &f.id,
        &f.space.prefix,
    )
    .unwrap();
    assert_eq!(record.asset_id, f.id);
    assert_eq!(
        record.media.unwrap().key,
        format!(
            "{}media/{}/{}/video.mp4",
            f.space.prefix,
            f.id,
            f.core.backup_assets().unwrap()[0]
                .content_sha256
                .as_ref()
                .unwrap()
        )
    );
}

#[test]
fn p1_migration_splits_existing_cross_library_memberships() {
    let db = rusqlite::Connection::open_in_memory().unwrap();
    db.execute_batch(include_str!("schema.sql")).unwrap();
    db.execute("UPDATE backup_schema SET version=3", [])
        .unwrap();
    let id = uuid::Uuid::new_v4().to_string();
    db.execute(
        "INSERT INTO media_assets VALUES(?1,NULL,'{}','video.mp4','hash',12,'active')",
        [&id],
    )
    .unwrap();
    db.execute("INSERT INTO asset_local_files(asset_id,path,bytes,local_state) VALUES(?1,'old-path',12,'present')",[&id]).unwrap();
    db.execute("INSERT INTO asset_transcripts(asset_id,text,model_revision,pipeline_revision,completed_at) VALUES(?1,'unsynced','m','p',1)",[&id]).unwrap();
    for space in ["a", "b"] {
        db.execute("INSERT INTO backup_spaces VALUES(?1,?1,'https://host','bucket',?1,'primary','device',1,NULL,1)",[space]).unwrap();
        db.execute("INSERT INTO asset_remote_objects(space_id,asset_id,record_key,remote_state) VALUES(?1,?2,'original-key','pending')",params![space,id]).unwrap();
        db.execute("INSERT INTO backup_operations(operation_id,space_id,asset_id,kind,state,phase,payload_json) VALUES(?1,?1,?2,'upload','pending','parts','{\"upload_id\":\"original-id\"}')",params![space,id]).unwrap();
    }
    repository::init(&db).unwrap();
    let a = repository::local_asset_id(&db, "a", &id).unwrap().unwrap();
    let b = repository::local_asset_id(&db, "b", &id).unwrap().unwrap();
    assert_ne!(a, b);
    db.execute(
        "UPDATE asset_transcripts SET text='changed-a' WHERE asset_id=?",
        [&a],
    )
    .unwrap();
    assert_eq!(
        repository::transcript(&db, &b).unwrap().unwrap().text,
        "unsynced"
    );
    assert_eq!(
        db.query_row(
            "SELECT asset_id FROM backup_operations WHERE space_id='b'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        b
    );
    assert_eq!(
        db.query_row(
            "SELECT payload_json FROM backup_operations WHERE space_id='b'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        r#"{"upload_id":"original-id"}"#
    );
    repository::init(&db).unwrap();
    assert_eq!(
        repository::local_asset_id(&db, "b", &id).unwrap().unwrap(),
        b
    );
}

#[derive(Clone)]
struct UpgradeFault {
    inner: Memory,
    lose_key: Arc<Mutex<Option<String>>>,
    barrier: Option<Arc<tokio::sync::Barrier>>,
    reads: Arc<std::sync::atomic::AtomicUsize>,
}
impl ObjectStore for UpgradeFault {
    fn get(&self, key: &str) -> StoreFuture<'_, Option<Object>> {
        let key = key.to_owned();
        Box::pin(async move {
            let result = self.inner.get(&key).await?;
            if key.ends_with("library.json")
                && self.reads.fetch_add(1, std::sync::atomic::Ordering::SeqCst) < 2
            {
                if let Some(barrier) = &self.barrier {
                    barrier.wait().await;
                }
            }
            Ok(result)
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
    fn write(&self, operation: Write) -> StoreFuture<'_, String> {
        Box::pin(async move {
            let key = match &operation {
                Write::Put { key, .. } => Some(key.clone()),
                _ => None,
            };
            let result = self.inner.write(operation).await?;
            let lose = {
                let mut target = self.lose_key.lock().unwrap();
                if *target == key && key.is_some() {
                    target.take();
                    true
                } else {
                    false
                }
            };
            if lose {
                bail!("BACKUP_NETWORK_ERROR");
            }
            Ok(result)
        })
    }
}

#[tokio::test]
async fn p1_two_upgrade_initiators_cannot_both_acquire_migration() {
    let f = Fixture::new(24).await;
    let store = UpgradeFault {
        inner: f.store.clone(),
        lose_key: Default::default(),
        barrier: Some(Arc::new(tokio::sync::Barrier::new(2))),
        reads: Default::default(),
    };
    let a = uuid::Uuid::new_v4().to_string();
    let b = uuid::Uuid::new_v4().to_string();
    let (a, b) = tokio::join!(
        library::initialize(&store, &f.space.prefix, &f.space.library_id, &a),
        library::initialize(&store, &f.space.prefix, &f.space.library_id, &b)
    );
    assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
}

#[tokio::test]
async fn p1_upgrade_unknown_metadata_commit_and_tombstone_remain_recoverable() {
    for fault_at in ["record", "library"] {
        let f = Fixture::new(24).await;
        f.upload().await.unwrap();
        let deleted = uuid::Uuid::new_v4().to_string();
        let key = format!("{}records/{deleted}.json", f.space.prefix);
        f.store.0.lock().unwrap().objects.insert(
            key,
            (
                serde_json::to_vec(&crate::backup::record::Record::Deleted(
                    crate::backup::record::Deleted {
                        schema_version: 1,
                        library_id: f.space.library_id.clone(),
                        asset_id: deleted.clone(),
                        record_revision: 2,
                        writer_epoch: 1,
                        deleted_at: chrono::Utc::now().to_rfc3339(),
                        delete_operation_id: uuid::Uuid::new_v4().to_string(),
                    },
                ))
                .unwrap(),
                None,
            ),
        );
        let lose = if fault_at == "record" {
            format!("{}records/{}.json", f.space.prefix, f.id)
        } else {
            format!("{}library.json", f.space.prefix)
        };
        let store = UpgradeFault {
            inner: f.store.clone(),
            lose_key: Arc::new(Mutex::new(Some(lose))),
            barrier: None,
            reads: Default::default(),
        };
        let op = uuid::Uuid::new_v4().to_string();
        assert!(
            library::initialize(&store, &f.space.prefix, &f.space.library_id, &op)
                .await
                .is_err()
        );
        library::initialize(&store, &f.space.prefix, &f.space.library_id, &op)
            .await
            .unwrap();
        let key = format!("{}records/{deleted}.json", f.space.prefix);
        let object = f.store.get(&key).await.unwrap().unwrap();
        let record = crate::backup::s3_provider::validate_wire(
            &object.body,
            &f.space.library_id,
            &deleted,
            &f.space.prefix,
        )
        .unwrap();
        assert_eq!(record.state, "deleted");
        assert!(record.media.is_none());
        assert_eq!(f.store.0.lock().unwrap().video_gets, 0);
    }
}

#[tokio::test]
async fn p1_corrupt_converted_record_blocks_migration_completion() {
    let f = Fixture::new(24).await;
    f.upload().await.unwrap();
    let op = uuid::Uuid::new_v4().to_string();
    let lib = f.store.get("test/library.json").await.unwrap().unwrap();
    let original: Library = serde_json::from_slice(&lib.body).unwrap();
    let marker = library::LibraryMetadata {
        deletion_indexes: false,
        schema_version: 2,
        library_id: f.space.library_id.clone(),
        mode: "migrating".into(),
        created_at: original.created_at.clone(),
        migration: Some(library::Migration {
            operation_id: op.clone(),
            original,
            next_page: None,
            converted: 0,
        }),
    };
    f.store.0.lock().unwrap().objects.insert(
        "test/library.json".into(),
        (serde_json::to_vec(&marker).unwrap(), None),
    );
    let key = format!("test/records/{}.json", f.id);
    let object = f.store.get(&key).await.unwrap().unwrap();
    let old = serde_json::from_slice(&object.body).unwrap();
    let converted = crate::backup::s3_provider::normalize_record(old, &op).unwrap();
    let mut raw: serde_json::Value = serde_json::from_slice(&converted).unwrap();
    raw["fields"]["title"] = serde_json::json!({"bad":"type"});
    f.store
        .0
        .lock()
        .unwrap()
        .objects
        .insert(key, (serde_json::to_vec(&raw).unwrap(), None));
    assert!(
        library::initialize(&f.store, &f.space.prefix, &f.space.library_id, &op)
            .await
            .is_err()
    );
    let lib = f.store.get("test/library.json").await.unwrap().unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&lib.body).unwrap()["mode"],
        "migrating"
    );
}

#[tokio::test]
async fn p1_legacy_publication_cannot_invent_video_upload_provenance() {
    let f = Fixture::new(24).await;
    f.upload().await.unwrap();
    let op = uuid::Uuid::new_v4().to_string();
    library::initialize(&f.store, &f.space.prefix, &f.space.library_id, &op)
        .await
        .unwrap();
    library::reconcile_local(&f.core, &f.store, &f.space)
        .await
        .unwrap();
    let proofs = f
        .core
        .backup_db(|db| {
            Ok(
                db.query_row("SELECT COUNT(*) FROM backup_upload_proofs", [], |r| {
                    r.get::<_, i64>(0)
                })?,
            )
        })
        .unwrap();
    assert_eq!(proofs, 0);
    library::reconcile_local(&f.core, &f.store, &f.space)
        .await
        .unwrap();
    assert_eq!(
        f.core
            .backup_db(|db| Ok(db.query_row(
                "SELECT COUNT(*) FROM backup_upload_proofs",
                [],
                |r| r.get::<_, i64>(0)
            )?))
            .unwrap(),
        0
    );
    let mut reader = f.space.clone();
    reader.writer_device_id = uuid::Uuid::new_v4().to_string();
    reader.role = Role::Reader;
    f.core
        .backup_db(|db| {
            db.execute("DELETE FROM backup_upload_proofs", [])?;
            Ok(())
        })
        .unwrap();
    library::reconcile_local(&f.core, &f.store, &reader)
        .await
        .unwrap();
    assert_eq!(
        f.core
            .backup_db(|db| Ok(db.query_row(
                "SELECT COUNT(*) FROM backup_upload_proofs",
                [],
                |r| r.get::<_, i64>(0)
            )?))
            .unwrap(),
        0
    );
}

use crate::backup::{
    provider::*,
    s3_provider::{probe, S3Provider},
    sync_engine::{self, SyncEngine},
};
fn capabilities() -> ProviderCapabilities {
    ProviderCapabilities {
        change_coordination: ChangeCoordination::ConditionalWrite,
        incremental_discovery: true,
        range_read: true,
        resumable_upload: false,
        max_single_media_bytes: 5 * 1024_u64.pow(4),
        can_delete_owned_media: true,
    }
}
async fn sync_fixture() -> (Fixture, Arc<S3Provider<Memory>>, SyncEngine) {
    let f = Fixture::new(24).await;
    f.store.0.lock().unwrap().objects.clear();
    f.core
        .backup_db(|db| {
            db.execute("UPDATE backup_provider_config SET protocol=2,capabilities_json=?1", [serde_json::to_string(&capabilities())?])?;
            db.execute(
                "INSERT INTO backup_current_space VALUES(1,?)",
                [&f.space.space_id],
            )?;
            sync_engine::register_download(db, &f.id)
        })
        .unwrap();
    let provider = Arc::new(S3Provider::new(
        f.store.clone(),
        f.space.clone(),
        capabilities(),
        f.core.backup_db(repository::device_id).unwrap(),
    ));
    provider.open_library(true).await.unwrap();
    let engine = SyncEngine::new(f.core.clone(), provider.clone());
    (f, provider, engine)
}

#[tokio::test]
async fn p5_conflict_selection_is_published_and_disappears_on_both_devices() {
    let (f, a, engine) = sync_fixture().await;
    engine
        .execute_upload(&claim(&engine, &f.core))
        .await
        .unwrap();
    let (_root, bcore, b, bengine) = second_device(&f).await;
    let bid = bcore.backup_assets().unwrap()[0].asset_id.clone();
    edit(&f.core, &f.id, "title", serde_json::json!("A title"));
    edit(&bcore, &bid, "title", serde_json::json!("B title"));
    engine
        .execute_upload(&claim(&engine, &f.core))
        .await
        .unwrap();
    bengine
        .execute_upload(&claim(&bengine, &bcore))
        .await
        .unwrap();
    let data = conflicts::local_data(&bcore, &b.scope().space_id, &bid).unwrap();
    assert_eq!(data.conflicts.len(), 1);
    let id = &data.conflicts[0].id;
    assert_eq!(
        conflicts::detail(&bcore, b.as_ref(), &bid, id)
            .await
            .unwrap()
            .value,
        "B title"
    );
    conflicts::resolve(&bcore, b.as_ref(), &bid, id, true)
        .await
        .unwrap();
    bengine
        .execute_upload(&claim(&bengine, &bcore))
        .await
        .unwrap();
    engine.pull().await.unwrap();
    let snapshot = a.read_asset(&f.id).await.unwrap().unwrap().snapshot;
    assert_eq!(snapshot.fields["title"], "B title");
    assert!(snapshot.conflicts.is_empty());
    assert!(conflicts::local_data(&bcore, &b.scope().space_id, &bid)
        .unwrap()
        .conflicts
        .is_empty());
    assert!(conflicts::local_data(&f.core, &a.scope().space_id, &f.id)
        .unwrap()
        .conflicts
        .is_empty());
    assert_eq!(f.store.0.lock().unwrap().video_gets, 0);
}

#[tokio::test]
async fn p5_conflict_choice_racing_new_remote_edit_keeps_both_versions() {
    let (f, a, engine) = sync_fixture().await;
    engine
        .execute_upload(&claim(&engine, &f.core))
        .await
        .unwrap();
    let (_root, bcore, b, bengine) = second_device(&f).await;
    let bid = bcore.backup_assets().unwrap()[0].asset_id.clone();
    edit(&f.core, &f.id, "title", serde_json::json!("first"));
    edit(&bcore, &bid, "title", serde_json::json!("choice"));
    engine
        .execute_upload(&claim(&engine, &f.core))
        .await
        .unwrap();
    bengine
        .execute_upload(&claim(&bengine, &bcore))
        .await
        .unwrap();
    let id = conflicts::local_data(&bcore, &b.scope().space_id, &bid)
        .unwrap()
        .conflicts[0]
        .id
        .clone();
    local_text(&bcore, &bid, "unsynced local text");
    assert_eq!(
        conflicts::resolve(&bcore, b.as_ref(), &bid, &id, true)
            .await
            .unwrap_err()
            .to_string(),
        "BACKUP_LOCAL_CHANGES_PENDING"
    );
    assert_eq!(
        conflicts::local_data(&bcore, &b.scope().space_id, &bid)
            .unwrap()
            .transcript
            .as_deref(),
        Some("unsynced local text")
    );
    bengine
        .execute_upload(&claim(&bengine, &bcore))
        .await
        .unwrap();
    conflicts::resolve(&bcore, b.as_ref(), &bid, &id, true)
        .await
        .unwrap();
    let exported = conflicts::export_data(&bcore, &b.scope().space_id, &bid).unwrap();
    assert_eq!(
        exported["pending"]["choices"][0]["conflict"]["value"],
        "choice"
    );
    assert_eq!(exported["transcript"]["text"], "unsynced local text");
    assert_eq!(exported["conflicts"][0]["value"], "choice");
    edit(&f.core, &f.id, "title", serde_json::json!("newer"));
    engine
        .execute_upload(&claim(&engine, &f.core))
        .await
        .unwrap();
    bengine
        .execute_upload(&claim(&bengine, &bcore))
        .await
        .unwrap();
    let snapshot = a.read_asset(&f.id).await.unwrap().unwrap().snapshot;
    assert_eq!(snapshot.fields["title"], "newer");
    assert!(snapshot.conflicts.iter().any(|c| c.value == "choice"));
}

fn claim(engine: &SyncEngine, core: &Core) -> String {
    engine.enqueue().unwrap();
    let op = scheduler::operations(core)
        .unwrap()
        .into_iter()
        .find(|o| o.kind == "upload" && o.state == "pending")
        .unwrap();
    core.backup_db(|db| {
        db.execute(
            "UPDATE backup_operations SET state='running' WHERE operation_id=?",
            [&op.operation_id],
        )?;
        Ok(())
    })
    .unwrap();
    op.operation_id
}
async fn second_device(
    f: &Fixture,
) -> (
    tempfile::TempDir,
    Arc<Core>,
    Arc<S3Provider<Memory>>,
    SyncEngine,
) {
    let root = tempfile::tempdir().unwrap();
    let core = Core::open(
        &root.path().join("b.sqlite"),
        root.path().join("b-downloads"),
        "unused".into(),
        vec![],
        |_| {},
    )
    .unwrap();
    let mut space = f.space.clone();
    space.space_id = uuid::Uuid::new_v4().to_string();
    space.role = Role::Member;
    core.backup_db(|db| {
        repository::save_space(db, &space)?;
        db.execute("UPDATE backup_provider_config SET protocol=2,capabilities_json=?", [serde_json::to_string(&capabilities())?])?;
        db.execute(
            "INSERT INTO backup_current_space VALUES(1,?)",
            [&space.space_id],
        )?;
        Ok(())
    })
    .unwrap();
    assert_ne!(
        core.backup_db(repository::device_id).unwrap(),
        f.core.backup_db(repository::device_id).unwrap()
    );
    let provider = Arc::new(S3Provider::new(
        f.store.clone(),
        space,
        capabilities(),
        core.backup_db(repository::device_id).unwrap(),
    ));
    let engine = SyncEngine::new(core.clone(), provider.clone());
    engine.pull().await.unwrap();
    (root, core, provider, engine)
}
fn edit(core: &Core, id: &str, field: &str, value: serde_json::Value) {
    core.backup_db(|db| {
        sync_engine::mark_local(db, id, [(field.into(), value)].into_iter().collect(), false)
    })
    .unwrap();
}

#[tokio::test]
async fn p2_two_databases_merge_distinct_fields_and_preserve_same_field_conflict() {
    let (f, a, engine) = sync_fixture().await;
    let op = claim(&engine, &f.core);
    engine.execute_upload(&op).await.unwrap();
    let (_root, bcore, b, bengine) = second_device(&f).await;
    let bid = bcore.backup_assets().unwrap()[0].asset_id.clone();
    assert!(bcore.backup_assets().unwrap()[0].path.is_none());
    let writes = f.store.0.lock().unwrap().writes;
    bengine.enqueue().unwrap();
    assert!(scheduler::operations(&bcore).unwrap().is_empty());
    assert_eq!(
        f.store.0.lock().unwrap().writes,
        writes,
        "import must not echo uploads"
    );
    edit(&f.core, &f.id, "title", serde_json::json!("A title"));
    edit(&bcore, &bid, "author", serde_json::json!("B author"));
    let ao = claim(&engine, &f.core);
    let bo = claim(&bengine, &bcore);
    let (ar, br) = tokio::join!(engine.execute_upload(&ao), bengine.execute_upload(&bo));
    ar.unwrap();
    br.unwrap();
    engine.pull().await.unwrap();
    bengine.pull().await.unwrap();
    let remote = a.read_asset(&f.id).await.unwrap().unwrap().snapshot;
    assert_eq!(remote.fields["title"], "A title");
    assert_eq!(remote.fields["author"], "B author");
    edit(&f.core, &f.id, "title", serde_json::json!("A conflict"));
    edit(&bcore, &bid, "title", serde_json::json!("B conflict"));
    let ao = claim(&engine, &f.core);
    let bo = claim(&bengine, &bcore);
    engine.execute_upload(&ao).await.unwrap();
    bengine.execute_upload(&bo).await.unwrap();
    let remote = b.read_asset(&f.id).await.unwrap().unwrap().snapshot;
    assert_eq!(remote.fields["title"], "A conflict");
    assert!(remote
        .conflicts
        .iter()
        .any(|c| c.field == "title" && c.value == "B conflict"));
    assert_eq!(f.store.0.lock().unwrap().video_gets, 0);
    let proof_count = bcore
        .backup_db(|db| {
            Ok(
                db.query_row("SELECT COUNT(*) FROM backup_upload_proofs", [], |r| {
                    r.get::<_, i64>(0)
                })?,
            )
        })
        .unwrap();
    assert_eq!(proof_count, 0, "metadata writers have never uploaded video");
    assert_eq!(
        f.core
            .backup_db(|db| Ok(db.query_row(
                "SELECT COUNT(*) FROM backup_upload_proofs",
                [],
                |r| r.get::<_, i64>(0)
            )?))
            .unwrap(),
        1
    );
}

#[tokio::test]
async fn p2_unknown_record_commit_and_conditional_conflict_are_reconciled() {
    let (f, provider, engine) = sync_fixture().await;
    let op = claim(&engine, &f.core);
    f.store.0.lock().unwrap().fail_after = Some("put");
    engine.execute_upload(&op).await.unwrap();
    edit(&f.core, &f.id, "title", serde_json::json!("retry title"));
    let op = claim(&engine, &f.core);
    f.store.0.lock().unwrap().conflict_record_once = true;
    engine.execute_upload(&op).await.unwrap();
    assert_eq!(
        provider
            .read_asset(&f.id)
            .await
            .unwrap()
            .unwrap()
            .snapshot
            .fields["title"],
        "retry title"
    );
    let writes = f.store.0.lock().unwrap().writes;
    engine.enqueue().unwrap();
    assert_eq!(f.store.0.lock().unwrap().writes, writes);
}

#[tokio::test]
async fn p2_provider_probe_checks_atomic_semantics_and_cleans_its_namespace() {
    let store = Memory::default();
    let op = uuid::Uuid::new_v4().to_string();
    let caps = probe(&store, "isolated/", &op).await.unwrap();
    caps.require_multi_writer().unwrap();
    let state = store.0.lock().unwrap();
    assert!(state.objects.is_empty());
    assert!(state.uploads.is_empty());
}

#[tokio::test]
async fn p2_missing_atomic_conditions_are_rejected_without_false_readiness() {
    let store = Memory::default();
    store.0.lock().unwrap().ignore_conditions = true;
    assert_eq!(
        probe(&store, "isolated/", &uuid::Uuid::new_v4().to_string())
            .await
            .unwrap_err()
            .to_string(),
        "BACKUP_CONDITIONAL_WRITE_UNSUPPORTED"
    );
    assert!(store.0.lock().unwrap().objects.is_empty());
}

#[tokio::test]
async fn p2_acknowledgement_does_not_clear_newer_local_edit() {
    let (f, provider, engine) = sync_fixture().await;
    f.store.0.lock().unwrap().edit_during_file = Some((
        Arc::downgrade(&f.core),
        f.id.clone(),
        "title".into(),
        serde_json::json!("edited during upload"),
    ));
    let op = claim(&engine, &f.core);
    engine.execute_upload(&op).await.unwrap();
    let pending: String = f
        .core
        .backup_db(|db| {
            Ok(
                db.query_row("SELECT pending_json FROM backup_space_assets", [], |r| {
                    r.get(0)
                })?,
            )
        })
        .unwrap();
    assert!(
        !serde_json::from_str::<sync_engine::PendingChanges>(&pending)
            .unwrap()
            .is_empty()
    );
    let op = claim(&engine, &f.core);
    engine.execute_upload(&op).await.unwrap();
    assert_eq!(
        provider
            .read_asset(&f.id)
            .await
            .unwrap()
            .unwrap()
            .snapshot
            .fields["title"],
        "edited during upload"
    );
    assert_eq!(
        f.store
            .0
            .lock()
            .unwrap()
            .objects
            .keys()
            .filter(|k| is_video_key(k))
            .count(),
        1
    );
}

#[tokio::test]
async fn flat_multipart_retry_restarts_and_cancel_cleans_its_session() {
    let (f, a, _engine) = sync_fixture().await;
    let (_root, _bcore, b, _) = second_device(&f).await;
    let path = f.root.path().join("large.mp4");
    std::fs::write(&path, b"\0\0\0\x18ftypisom\0\0\0\0isommp42").unwrap();
    std::fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_len(100 * 1024 * 1024)
        .unwrap();
    let (hash, _) = repository::fingerprint(&path).unwrap();
    let astate: Arc<Mutex<Option<ResumeState>>> = Default::default();
    let aop = uuid::Uuid::new_v4().to_string();
    let bop = uuid::Uuid::new_v4().to_string();
    let source = |op: &str, device: String, state: Arc<Mutex<Option<ResumeState>>>| MediaSource {
        recreate_revision: None,
        asset_id: f.id.clone(),
        kind: "video".into(),
        path: path.clone(),
        bytes: 100 * 1024 * 1024,
        sha256: hash.clone(),
        container: "mp4".into(),
        operation_id: op.into(),
        device_id: device,
        control: Some(Arc::new(TransferControl {
            check: Box::new(|| Ok(())),
            checkpoint: Box::new(move |value| {
                *state.lock().unwrap() = Some(value);
                Ok(())
            }),
            progress: Default::default(),
        })),
    };
    f.store.0.lock().unwrap().fail_after = Some("create");
    assert!(a
        .upload_media(
            source(
                &aop,
                f.core.backup_db(repository::device_id).unwrap(),
                astate.clone()
            ),
            None
        )
        .await
        .is_err());
    assert_eq!(f.store.0.lock().unwrap().uploads.len(), 1);
    let lost_create = astate.lock().unwrap().clone();
    a.cancel_upload(lost_create.clone().unwrap(), &aop).await.unwrap();
    assert!(f.store.0.lock().unwrap().uploads.is_empty(), "cancel reported success but left an unknown CreateMultipart session");
    f.store.0.lock().unwrap().fail_after = Some("part");
    assert!(a.upload_media(source(&aop,f.core.backup_db(repository::device_id).unwrap(),astate.clone()),lost_create).await.is_err());
    assert_eq!(f.store.0.lock().unwrap().uploads.len(),1);
    let interrupted=astate.lock().unwrap().clone().unwrap();
    assert!(b.cancel_upload(interrupted.clone(),&aop).await.is_err());
    assert!(a.cancel_upload(interrupted.clone(),&bop).await.is_err());
    a.cancel_upload(interrupted.clone(),&aop).await.unwrap();
    assert!(f.store.0.lock().unwrap().uploads.is_empty());
    let receipt=a.upload_media(source(&aop,f.core.backup_db(repository::device_id).unwrap(),astate),Some(interrupted)).await.unwrap();
    assert_eq!(receipt.reference.sha256,hash);
    assert!(!a.capabilities().resumable_upload);
    let store=f.store.0.lock().unwrap();
    assert!(store.uploads.is_empty());
    assert_eq!(store.part_reads,0,"a retry must not read or adopt old parts");
    assert_eq!(store.part_writes.iter().map(|(_,part)|*part).collect::<Vec<_>>(),vec![1,1,2,3,4,5,6,7],"an interrupted upload must resend part 1");
    assert_ne!(store.part_writes[0].0,store.part_writes[1].0,"retry reused the previous multipart session");
}

fn local_text(core: &Core, id: &str, text: &str) {
    core.backup_db(|db|{
        db.execute("INSERT INTO asset_transcripts(asset_id,content_sha256,text,model_revision,pipeline_revision,completed_at) SELECT asset_id,content_sha256,?2,'m','p',1 FROM media_assets WHERE asset_id=?1 ON CONFLICT(asset_id) DO UPDATE SET text=excluded.text",params![id,text])?;
        sync_engine::mark_local(db,id,Default::default(),true)
    }).unwrap();
}
#[tokio::test]
async fn p2_inflight_new_field_keeps_its_actual_base_after_unrelated_ack() {
    let (f, a, engine) = sync_fixture().await;
    let op = claim(&engine, &f.core);
    engine.execute_upload(&op).await.unwrap();
    let (_root, bcore, _b, bengine) = second_device(&f).await;
    let bid = bcore.backup_assets().unwrap()[0].asset_id.clone();
    edit(
        &bcore,
        &bid,
        "author",
        serde_json::json!("remote author A2"),
    );
    let bo = claim(&bengine, &bcore);
    bengine.execute_upload(&bo).await.unwrap();
    edit(&f.core, &f.id, "title", serde_json::json!("A title"));
    local_text(&f.core, &f.id, "text to force a small upload");
    let ao = claim(&engine, &f.core);
    f.store.0.lock().unwrap().edit_during_file = Some((
        Arc::downgrade(&f.core),
        f.id.clone(),
        "author".into(),
        serde_json::json!("local author A1"),
    ));
    engine.execute_upload(&ao).await.unwrap();
    let ao = claim(&engine, &f.core);
    engine.execute_upload(&ao).await.unwrap();
    let remote = a.read_asset(&f.id).await.unwrap().unwrap().snapshot;
    assert_eq!(remote.fields["author"], "remote author A2");
    assert!(remote
        .conflicts
        .iter()
        .any(|c| c.field == "author" && c.value == "local author A1"));
}
#[tokio::test]
async fn flat_equal_transcript_content_reuses_one_object_without_conflict() {
    let (f, a, engine) = sync_fixture().await;
    let op = claim(&engine, &f.core);
    engine.execute_upload(&op).await.unwrap();
    let (_root, bcore, _b, bengine) = second_device(&f).await;
    let bid = bcore.backup_assets().unwrap()[0].asset_id.clone();
    local_text(&f.core, &f.id, "identical transcript");
    local_text(&bcore, &bid, "identical transcript");
    let ao = claim(&engine, &f.core);
    let bo = claim(&bengine, &bcore);
    engine.execute_upload(&ao).await.unwrap();
    bengine.execute_upload(&bo).await.unwrap();
    let remote = a.read_asset(&f.id).await.unwrap().unwrap().snapshot;
    assert!(remote.conflicts.is_empty());
    assert_eq!(
        f.store
            .0
            .lock()
            .unwrap()
            .objects
            .keys()
            .filter(|k| k.rsplit('/').next().is_some_and(|name| name.starts_with("transcript-") && name.ends_with(".json")))
            .count(),
        1,
        "equal immutable content should reuse one shallow object"
    );
}

#[tokio::test]
async fn p2_wrong_space_reference_is_rejected_before_equal_content_deduplication() {
    let (f, provider, engine) = sync_fixture().await;
    let op = claim(&engine, &f.core);
    engine.execute_upload(&op).await.unwrap();
    let baseline = provider.read_asset(&f.id).await.unwrap().unwrap().snapshot;
    let mut reference = baseline.media.clone().unwrap();
    reference.scope.space_id = "other-space".into();
    let change = AssetChange {
        recreate: false,
        resolved_conflicts: vec![],
        asset_id: f.id.clone(),
        base: Some(baseline),
        fields: Default::default(),
        field_bases: Default::default(),
        media: Some(reference),
        cover: ReferenceChange::Unchanged,
        transcript: ReferenceChange::Unchanged,
        delete: false,
        device_id: f.core.backup_db(repository::device_id).unwrap(),
        operation_id: uuid::Uuid::new_v4().to_string(),
        conflicts: vec![],
    };
    let writes = f.store.0.lock().unwrap().writes;
    assert_eq!(
        sync_engine::publish(provider.as_ref(), change)
            .await
            .unwrap_err()
            .to_string(),
        "BACKUP_SCOPE_MISMATCH"
    );
    assert_eq!(f.store.0.lock().unwrap().writes, writes);
}

#[tokio::test]
async fn p3_small_reads_and_lazy_restore_have_no_upload_echo() {
    let (f, a, engine) = sync_fixture().await;
    f.core.verify_backup_asset(&f.id).await.unwrap();
    local_text(&f.core, &f.id, "portable text");
    let op = claim(&engine, &f.core);
    engine.execute_upload(&op).await.unwrap();
    let (_root, bcore, b, bengine) = second_device(&f).await;
    let id = bcore.backup_assets().unwrap()[0].asset_id.clone();
    let entries = crate::backup::sync_read::list(&bcore, &b.scope().space_id).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(
        crate::backup::sync_read::transcript(&bcore, b.as_ref(), &id)
            .await
            .unwrap()
            .unwrap()
            .text,
        "portable text"
    );
    assert_eq!(f.store.0.lock().unwrap().video_gets, 0);
    let path = crate::backup::sync_read::ensure(&bcore, b.as_ref(), &id)
        .await
        .unwrap();
    assert!(path.is_file());
    assert_ne!(path, f.root.path().join("video.mp4"));
    assert_eq!(
        repository::fingerprint(&path).unwrap().0,
        a.read_asset(&f.id)
            .await
            .unwrap()
            .unwrap()
            .snapshot
            .media
            .unwrap()
            .sha256
    );
    let reads = f.store.0.lock().unwrap().video_gets;
    crate::backup::sync_read::ensure(&bcore, b.as_ref(), &id)
        .await
        .unwrap();
    assert_eq!(f.store.0.lock().unwrap().video_gets, reads);
    bengine.enqueue().unwrap();
    assert!(scheduler::operations(&bcore)
        .unwrap()
        .iter()
        .all(|o| o.kind != "upload"));
}

#[tokio::test]
async fn p3_partial_restore_resumes_range_and_preserves_user_filename() {
    let (f, _a, engine) = sync_fixture().await;
    let op = claim(&engine, &f.core);
    engine.execute_upload(&op).await.unwrap();
    let (_root, bcore, b, _) = second_device(&f).await;
    let id = bcore.backup_assets().unwrap()[0].asset_id.clone();
    std::fs::create_dir_all(&bcore.settings().download_directory).unwrap();
    let occupied = bcore.settings().download_directory.join("video.mp4");
    std::fs::write(&occupied, b"user file").unwrap();
    f.store.0.lock().unwrap().break_stream = Some(8);
    assert!(crate::backup::sync_read::ensure(&bcore, b.as_ref(), &id)
        .await
        .is_err());
    let path = crate::backup::sync_read::ensure(&bcore, b.as_ref(), &id)
        .await
        .unwrap();
    assert_ne!(path, occupied);
    assert_eq!(std::fs::read(&occupied).unwrap(), b"user file");
    assert!(f.store.0.lock().unwrap().stream_offsets.contains(&8));
}

fn update_for(
    endpoint: &str,
    bucket: &str,
    prefix: &str,
    profile: &str,
) -> crate::backup::spaces::Update {
    crate::backup::spaces::Update {
        cleanup_deleted_records: false,
        deleted_record_retention_days: 30,
        display_name: None,
        provider_type: ProviderType::S3,
        profile: profile.into(),
        endpoint: endpoint.into(),
        bucket: bucket.into(),
        prefix: prefix.into(),
        region: if profile == "r2" { "auto" } else { "us-east-1" }.into(),
        path_style: true,
        enabled: true,
        activate: true,
        credentials: Some(settings::Credentials {
            access_key_id: "isolated-key".into(),
            secret_access_key: "isolated-secret".into(),
        }),
    }
}

#[tokio::test]
async fn p3_save_configuration_without_activation_preserves_default_and_queue() {
    let (f, _, engine) = sync_fixture().await;
    let vault = Vault::default();
    let a = crate::backup::spaces::save_with(&f.core, update_for(&f.space.endpoint, &f.space.bucket, &f.space.prefix, "r2"), &vault, |_, _, _, _| Ok(f.store.clone())).await.unwrap();
    let operation = claim(&engine, &f.core);
    let before = scheduler::operations(&f.core).unwrap().into_iter().find(|o| o.operation_id == operation).unwrap();
    let store = Memory::default();
    let mut edit = update_for("https://s3.example.test", "saved-only", "saved/", "s3");
    edit.activate = false;
    let b = crate::backup::spaces::save_with(&f.core, edit, &vault, |_, _, _, _| Ok(store.clone())).await.unwrap();
    assert!(!b.enabled);
    assert_eq!(crate::backup::spaces::get(&f.core).unwrap().unwrap().space_id, a.space_id);
    let mut edit = update_for(&b.endpoint, &b.bucket, &b.prefix, "s3");
    edit.activate = false;
    edit.enabled = false;
    edit.credentials = None;
    let same = crate::backup::spaces::save_with(&f.core, edit, &vault, |_, _, _, _| Ok(store.clone())).await.unwrap();
    assert_eq!(same.space_id, b.space_id);
    assert_eq!(crate::backup::spaces::get(&f.core).unwrap().unwrap().space_id, a.space_id);
    let mut edit = update_for(&b.endpoint, &b.bucket, &b.prefix, "s3");
    edit.activate = false;
    edit.enabled = false;
    edit.region = "us-west-2".into();
    edit.credentials = None;
    let changed = crate::backup::spaces::save_with(&f.core, edit, &vault, |_, _, _, _| Ok(store.clone())).await.unwrap();
    assert_eq!(changed.region, "us-west-2");
    assert!(!changed.enabled);
    assert_eq!(crate::backup::spaces::get(&f.core).unwrap().unwrap().space_id, a.space_id);
    let after = scheduler::operations(&f.core).unwrap().into_iter().find(|o| o.operation_id == operation).unwrap();
    assert_eq!(after.state, before.state);
    assert_eq!(after.payload, before.payload);
    assert_eq!(vault.0.lock().unwrap().len(), 2);
    f.core.backup_db(|db| crate::backup::spaces::select_local(db, &b.space_id, true)).unwrap();
    assert_eq!(crate::backup::spaces::get(&f.core).unwrap().unwrap().space_id, b.space_id);
    assert_eq!(scheduler::operations(&f.core).unwrap().into_iter().find(|o| o.operation_id == operation).unwrap().state, "blocked");
}

#[tokio::test]
async fn p3_first_saved_configuration_waits_for_explicit_activation() {
    let root = tempfile::tempdir().unwrap();
    let core = Core::open(&root.path().join("tabs.sqlite"), root.path().join("downloads"), "unused".into(), vec![], |_| {}).unwrap();
    let vault = Vault::default();
    let store = Memory::default();
    let mut edit = update_for("https://s3.example.test", "first-saved", "first/", "s3");
    edit.activate = false;
    edit.display_name = Some("个人存储".into());
    let saved = crate::backup::spaces::save_with(&core, edit, &vault, |_, _, _, _| Ok(store)).await.unwrap();
    assert_eq!(saved.display_name, "个人存储");
    assert!(!saved.enabled);
    assert!(crate::backup::spaces::get(&core).unwrap().is_none());
    assert_eq!(crate::backup::spaces::list(&core).unwrap().len(), 1);
    assert!(scheduler::operations(&core).unwrap().is_empty());
    let mut edit = update_for(&saved.endpoint, &saved.bucket, &saved.prefix, "s3");
    edit.activate = false; edit.enabled = false; edit.credentials = None;
    let unchanged = crate::backup::spaces::save_with(&core, edit, &vault, |_, _, _, _| Ok(Memory::default())).await.unwrap();
    assert_eq!(unchanged.display_name, "个人存储");
    assert!(crate::backup::spaces::get(&core).unwrap().is_none());
}

#[tokio::test]
async fn p3_space_names_are_local_persistent_and_do_not_change_library_or_queue() {
    let (f, _, engine) = sync_fixture().await;
    // Initialize the old schema before claiming work: init intentionally recovers running jobs.
    f.core.backup_db(|db| { db.execute_batch("DROP TABLE backup_space_names")?; repository::init(db)?; repository::init(db) }).unwrap();
    let operation = claim(&engine, &f.core);
    let before = scheduler::operations(&f.core).unwrap().into_iter().find(|o| o.operation_id == operation).unwrap();
    let writes = f.store.0.lock().unwrap().writes;
    f.core.backup_db(|db| { db.execute("UPDATE backup_provider_config SET protocol=1 WHERE space_id=?", [&f.space.space_id])?; Ok(()) }).unwrap();
    let value = crate::backup::spaces::rename(&f.core, &f.space.space_id, "  工作素材  ").unwrap();
    assert_eq!(value.display_name, "工作素材");
    assert_eq!(value.library_id, f.space.library_id);
    assert_eq!(value.endpoint, f.space.endpoint);
    assert_eq!(value.bucket, f.space.bucket);
    assert_eq!(value.prefix, f.space.prefix);
    assert_eq!(value.protocol, 1);
    assert!(value.enabled);
    let db = rusqlite::Connection::open(f.root.path().join("app.sqlite")).unwrap();
    assert_eq!(db.query_row("SELECT display_name FROM backup_space_names WHERE space_id=?", [&f.space.space_id], |r| r.get::<_, String>(0)).unwrap(), "工作素材");
    assert_eq!(db.query_row("SELECT version FROM backup_schema WHERE id=1", [], |r| r.get::<_, i64>(0)).unwrap(), 4);
    let after = scheduler::operations(&f.core).unwrap().into_iter().find(|o| o.operation_id == operation).unwrap();
    assert_eq!(after.state, before.state);
    assert_eq!(after.payload, before.payload);
    assert_eq!(f.store.0.lock().unwrap().writes, writes);
    assert_eq!(crate::backup::spaces::current_id(&f.core).unwrap().as_deref(), Some(f.space.space_id.as_str()));
    assert_eq!(crate::backup::spaces::rename(&f.core, &f.space.space_id, &"名".repeat(65)).unwrap_err().to_string(), "BACKUP_INVALID_SPACE_NAME");
    assert_eq!(crate::backup::spaces::rename(&f.core, &f.space.space_id, "名字\0控制符").unwrap_err().to_string(), "BACKUP_INVALID_SPACE_NAME");
    assert_eq!(crate::backup::spaces::rename(&f.core, "missing-space", "名称").unwrap_err().to_string(), "BACKUP_SPACE_NOT_FOUND");
    assert!(crate::backup::spaces::rename(&f.core, &f.space.space_id, "").unwrap().display_name.is_empty());
}

#[test]
fn p3_save_configuration_activation_flag_is_backward_compatible() {
    let mut value = serde_json::json!({"provider_type":"s3","profile":"s3","endpoint":"https://s3.example.test","bucket":"fixture","prefix":"fixture/","region":"us-east-1","path_style":true,"enabled":true});
    assert!(serde_json::from_value::<crate::backup::spaces::Update>(value.clone()).unwrap().activate);
    value["activate"] = false.into();
    assert!(!serde_json::from_value::<crate::backup::spaces::Update>(value).unwrap().activate);
}
#[tokio::test]
async fn p3_switch_preserves_credentials_queue_ids_and_does_not_migrate_history() {
    let (f, provider, engine) = sync_fixture().await;
    let vault = Vault::default();
    let a = crate::backup::spaces::save_with(
        &f.core,
        update_for(&f.space.endpoint, &f.space.bucket, &f.space.prefix, "r2"),
        &vault,
        |_, _, _, _| Ok(f.store.clone()),
    )
    .await
    .unwrap();
    let queued = claim(&engine, &f.core);
    let payload = scheduler::operations(&f.core)
        .unwrap()
        .into_iter()
        .find(|o| o.operation_id == queued)
        .unwrap()
        .payload;
    let store_b = Memory::default();
    let b = crate::backup::spaces::save_with(
        &f.core,
        update_for("https://s3.example.test", "other-bucket", "other/", "s3"),
        &vault,
        |_, _, _, _| Ok(store_b.clone()),
    )
    .await
    .unwrap();
    assert_ne!(a.space_id, b.space_id);
    assert_eq!(vault.0.lock().unwrap().len(), 2);
    let old = scheduler::operations(&f.core)
        .unwrap()
        .into_iter()
        .find(|o| o.operation_id == queued)
        .unwrap();
    assert_eq!(old.state, "blocked");
    assert_eq!(old.payload, payload);
    assert!(crate::backup::sync_read::list(&f.core, &b.space_id)
        .unwrap()
        .is_empty());
    let new_id = uuid::Uuid::new_v4().to_string();
    f.core.backup_db(|db|{db.execute("INSERT INTO media_assets SELECT ?1,NULL,metadata_json,original_filename,content_sha256,content_bytes,'active' FROM media_assets WHERE asset_id=?2",params![new_id,f.id])?;db.execute("INSERT INTO asset_local_files SELECT ?1,path,bytes,sha256,source_json,verified_at,local_state FROM asset_local_files WHERE asset_id=?2",params![new_id,f.id])?;sync_engine::register_download(db,&new_id)}) .unwrap();
    assert!(f
        .core
        .backup_db(|db| repository::local_asset_id(db, &b.space_id, &new_id))
        .unwrap()
        .is_some());
    assert!(f
        .core
        .backup_db(|db| repository::local_asset_id(db, &a.space_id, &new_id))
        .unwrap()
        .is_none());
    assert!(engine.execute_upload(&queued).await.is_err());
    f.core
        .backup_db(|db| crate::backup::spaces::select_local(db, &a.space_id, true))
        .unwrap();
    provider.open_library(false).await.unwrap();
    let old = scheduler::operations(&f.core)
        .unwrap()
        .into_iter()
        .find(|o| o.operation_id == queued)
        .unwrap();
    assert_eq!(old.state, "pending");
    assert_eq!(old.payload, payload);
    f.core
        .backup_db(|db| {
            db.execute(
                "UPDATE backup_operations SET state='running' WHERE operation_id=?",
                [&queued],
            )?;
            Ok(())
        })
        .unwrap();
    engine.execute_upload(&queued).await.unwrap();
    assert!(!f.store.0.lock().unwrap().objects.is_empty());
    assert!(store_b
        .0
        .lock()
        .unwrap()
        .objects
        .keys()
        .all(|k| !k.contains(&f.id)));
}

#[tokio::test]
async fn p3_background_discovers_assets_while_own_media_upload_is_blocked() {
    let (f, _a, aengine) = sync_fixture().await;
    let (_root, bcore, b, _bengine) = second_device(&f).await;
    // Register B's independent local video without importing A or opening a task page.
    let id = uuid::Uuid::new_v4().to_string();
    let path = bcore.settings().download_directory.join("b.mp4");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, b"\0\0\0\x18ftypisom\0\0\0\0isommp42").unwrap();
    bcore.backup_db(|db|{db.execute("INSERT INTO media_assets VALUES(?1,NULL,?2,'b.mp4',NULL,24,'active')",params![id,serde_json::json!({"platform":"youtube","video_id":"12345678901","title":"B"}).to_string()])?;db.execute("INSERT INTO asset_local_files(asset_id,path,bytes,local_state) VALUES(?1,?2,24,'present')",params![id,path.to_string_lossy()])?;sync_engine::register_download(db,&id)}) .unwrap();
    let started = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    f.store.0.lock().unwrap().hold_upload = Some((started.clone(), release.clone()));
    let (stop, cancel) = tokio::sync::watch::channel(false);
    let resolve = b.clone();
    let worker = tokio::spawn(sync_engine::run_with(
        bcore.clone(),
        cancel,
        Arc::new(move |_, _| Ok(resolve.clone())),
    ));
    tokio::time::timeout(std::time::Duration::from_secs(3), started.notified())
        .await
        .unwrap();
    let op = claim(&aengine, &f.core);
    aengine.execute_upload(&op).await.unwrap();
    bcore.wake_backup();
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            if bcore
                .backup_assets()
                .unwrap()
                .iter()
                .any(|a| a.asset_id == f.id)
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    assert!(bcore.list().is_empty());
    assert_eq!(f.store.0.lock().unwrap().video_gets, 0);
    release.notify_one();
    stop.send_replace(true);
    bcore.wake_backup();
    worker.await.unwrap();
}

#[tokio::test]
async fn p3_missing_record_and_incomplete_listing_do_not_become_deletion() {
    let (f, _a, engine) = sync_fixture().await;
    let op = claim(&engine, &f.core);
    engine.execute_upload(&op).await.unwrap();
    let (_root, bcore, b, bengine) = second_device(&f).await;
    let id = bcore.backup_assets().unwrap()[0].asset_id.clone();
    local_text(&bcore, &id, "unsynced text");
    edit(&bcore, &id, "title", serde_json::json!("unsynced title"));
    f.store.0.lock().unwrap().fail_list = true;
    assert!(bengine.pull().await.is_err());
    f.store.0.lock().unwrap().fail_list = false;
    f.store
        .0
        .lock()
        .unwrap()
        .objects
        .remove(&format!("{}records/{}.json", f.space.prefix, f.id));
    assert_eq!(
        crate::backup::sync_read::ensure(&bcore, b.as_ref(), &id)
            .await
            .unwrap_err()
            .to_string(),
        "BACKUP_OBJECT_NOT_FOUND"
    );
    let entries = crate::backup::sync_read::list(&bcore, &b.scope().space_id).unwrap();
    assert_eq!(entries.len(), 1);
    assert!(!entries[0].remote_deleted);
    assert_eq!(
        bcore.asset_transcript(&id).unwrap().unwrap().text,
        "unsynced text"
    );
    assert_eq!(bcore.backup_assets().unwrap().len(), 1);
}
#[tokio::test]
async fn p3_probe_failure_retains_owned_cleanup_and_temporary_credential_until_retry() {
    let (f, _a, _engine) = sync_fixture().await;
    let vault = Vault::default();
    f.store.0.lock().unwrap().fail_delete = true;
    let error = crate::backup::spaces::test_with(
        &f.core,
        update_for(&f.space.endpoint, &f.space.bucket, &f.space.prefix, "r2"),
        &vault,
        |_, _, _, _| Ok(f.store.clone()),
    )
    .await
    .unwrap_err();
    assert_eq!(error.to_string(), "BACKUP_PROBE_CLEANUP_FAILED");
    assert_eq!(vault.0.lock().unwrap().len(), 1);
    assert_eq!(
        f.core
            .backup_db(|db| Ok(db.query_row(
                "SELECT COUNT(*) FROM backup_probe_cleanup",
                [],
                |r| r.get::<_, i64>(0)
            )?))
            .unwrap(),
        1
    );
    f.store.0.lock().unwrap().fail_delete = false;
    crate::backup::spaces::cleanup_probes_with(&f.core, &vault, |_, _, _, _| Ok(f.store.clone()))
        .await
        .unwrap();
    assert_eq!(vault.0.lock().unwrap().len(), 0);
    assert!(f
        .store
        .0
        .lock()
        .unwrap()
        .objects
        .keys()
        .all(|key| !key.contains("probes/")));
    assert!(f
        .store
        .0
        .lock()
        .unwrap()
        .objects
        .contains_key("test/library.json"));
}

#[tokio::test]
async fn p3_cleanup_skips_a_live_connection_probe_and_its_vault_reference() {
    let (f, _a, _engine) = sync_fixture().await;
    let vault = Arc::new(Vault::default());
    let started = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    f.store.0.lock().unwrap().hold_probe = Some((started.clone(), release.clone()));
    let core = f.core.clone();
    let store = f.store.clone();
    let save_vault = vault.clone();
    let update = update_for(&f.space.endpoint, &f.space.bucket, &f.space.prefix, "r2");
    let save = tokio::spawn(async move {
        crate::backup::spaces::save_with(&core, update, save_vault.as_ref(), |_, _, _, _| Ok(store))
            .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(3), started.notified())
        .await
        .unwrap();
    crate::backup::spaces::cleanup_probes_with(&f.core, vault.as_ref(), |_, _, _, _| {
        Ok(f.store.clone())
    })
    .await
    .unwrap();
    assert_eq!(
        vault.0.lock().unwrap().len(),
        1,
        "live probe credentials must survive maintenance"
    );
    release.notify_one();
    let saved = save.await.unwrap().unwrap();
    assert!(settings::credentials(&f.core, &saved.space_id, vault.as_ref()).is_ok());
    assert_eq!(vault.0.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn p3_restart_does_not_seed_old_task_transcript_over_imported_cloud_text() {
    let (f, a, engine) = sync_fixture().await;
    f.core.verify_backup_asset(&f.id).await.unwrap();
    local_text(&f.core, &f.id, "old snapshot text");
    let task_id = uuid::Uuid::new_v4().to_string();
    let path = f.root.path().join("video.mp4");
    let source = crate::desktop_core::transcription::Source::read(&path).unwrap();
    let task = serde_json::json!({"id":task_id,"asset_id":f.id,"revision":1,"status":"completed","video":{"platform":"youtube","video_id":"12345678901","canonical_url":"https://www.youtube.com/watch?v=12345678901","title":"task title","kind":"video","resolver":"yt-dlp"},"bytes":24,"total_bytes":24,"speed_bps":null,"error":null,"path":path,"created_at":1,"completed_at":2,"canceling":false,"download_requested":true});
    let old = serde_json::json!({"task_id":task_id,"operation_id":"old","sequence":1,"state":"running","phase":null,"error_code":null,"updated_at":2,"has_result":true,"source_changed":false,"result":{"text":"old snapshot text","source":source,"model_revision":"m","pipeline_revision":"p","completed_at":1}});
    f.core
        .backup_db(|db| {
            db.execute(
                "INSERT INTO download_tasks VALUES(?1,?2)",
                params![task_id, task.to_string()],
            )?;
            db.execute(
                "INSERT INTO video_transcripts VALUES(?1,?2)",
                params![task_id, old.to_string()],
            )?;
            db.execute(
                "UPDATE media_assets SET task_id=?2 WHERE asset_id=?1",
                params![f.id, task_id],
            )?;
            Ok(())
        })
        .unwrap();
    let op = claim(&engine, &f.core);
    engine.execute_upload(&op).await.unwrap();
    let (_broot, bcore, _b, bengine) = second_device(&f).await;
    let bid = bcore.backup_assets().unwrap()[0].asset_id.clone();
    local_text(&bcore, &bid, "new remote text");
    let op = claim(&bengine, &bcore);
    bengine.execute_upload(&op).await.unwrap();
    engine.pull().await.unwrap();
    assert_eq!(
        crate::backup::sync_read::transcript(&f.core, a.as_ref(), &f.id)
            .await
            .unwrap()
            .unwrap()
            .text,
        "new remote text"
    );
    let Fixture {
        root,
        core,
        store,
        space,
        id,
    } = f;
    drop(engine);
    drop(core);
    drop(a);
    let reopened = Core::open(
        &root.path().join("app.sqlite"),
        root.path().join("downloads"),
        "unused".into(),
        vec![],
        |_| {},
    )
    .unwrap();
    assert_eq!(
        reopened.asset_transcript(&id).unwrap().unwrap().text,
        "new remote text"
    );
    let started = reopened.start_transcription(&task_id, true).unwrap();
    assert_eq!(
        reopened.asset_transcript(&id).unwrap().unwrap().text,
        "new remote text"
    );
    reopened
        .cancel_transcription(&task_id, &started.operation_id)
        .await
        .unwrap();
    assert_eq!(
        reopened.asset_transcript(&id).unwrap().unwrap().text,
        "new remote text"
    );
    let provider = Arc::new(S3Provider::new(
        store,
        space,
        capabilities(),
        reopened.backup_db(repository::device_id).unwrap(),
    ));
    SyncEngine::new(reopened.clone(), provider)
        .enqueue()
        .unwrap();
    assert!(scheduler::operations(&reopened)
        .unwrap()
        .iter()
        .all(|o| o.kind != "upload" || o.state == "completed"));
}

#[tokio::test]
async fn p3_maintenance_failure_is_local_to_its_journal_and_shutdown_cancels_wait() {
    let (f, _a, _engine) = sync_fixture().await;
    let vault = Vault::default();
    f.store.0.lock().unwrap().fail_delete = true;
    assert!(crate::backup::spaces::test_with(
        &f.core,
        update_for(&f.space.endpoint, &f.space.bucket, &f.space.prefix, "r2"),
        &vault,
        |_, _, _, _| Ok(f.store.clone())
    )
    .await
    .is_err());
    crate::backup::spaces::cleanup_probes_with(&f.core, &vault, |_, _, _, _| {
        Err::<Memory, _>(anyhow::anyhow!("BACKUP_ACCESS_DENIED"))
    })
    .await
    .unwrap();
    assert_eq!(
        f.core
            .backup_db(|db| Ok(db.query_row(
                "SELECT last_error FROM backup_probe_cleanup",
                [],
                |r| r.get::<_, String>(0)
            )?))
            .unwrap(),
        "BACKUP_ACCESS_DENIED"
    );
    assert!(crate::backup::spaces::get(&f.core)
        .unwrap()
        .unwrap()
        .last_error
        .is_none());
    let (stop, mut cancel) = tokio::sync::watch::channel(false);
    let work = tokio::spawn(async move {
        sync_engine::maintenance_once(&mut cancel, std::future::pending::<Result<()>>()).await
    });
    stop.send_replace(true);
    assert_eq!(
        tokio::time::timeout(std::time::Duration::from_millis(500), work)
            .await
            .unwrap()
            .unwrap()
            .unwrap_err()
            .to_string(),
        "CANCELED"
    );
}

#[tokio::test]
async fn p3_switch_cancels_old_pagination_before_discovering_colliding_id_in_new_space() {
    let (f, a, engine) = sync_fixture().await;
    let op = claim(&engine, &f.core);
    engine.execute_upload(&op).await.unwrap();
    let vault = Vault::default();
    let store_b = Memory::default();
    let bview = crate::backup::spaces::save_with(
        &f.core,
        update_for("https://s3.example.test", "new-bucket", "new/", "s3"),
        &vault,
        |_, _, _, _| Ok(store_b.clone()),
    )
    .await
    .unwrap();
    let mut bspace = f
        .core
        .backup_db(repository::spaces)
        .unwrap()
        .into_iter()
        .find(|s| s.space_id == bview.space_id)
        .unwrap();
    bspace.enabled = true;
    let b = Arc::new(S3Provider::new(
        store_b.clone(),
        bspace.clone(),
        capabilities(),
        f.core.backup_db(repository::device_id).unwrap(),
    ));
    let object = f
        .store
        .get(&format!("test/records/{}.json", f.id))
        .await
        .unwrap()
        .unwrap();
    let mut wire: crate::backup::s3_provider::WireRecord =
        serde_json::from_slice(&object.body).unwrap();
    wire.library_id = bspace.library_id.clone();
    wire.fields
        .insert("title".into(), serde_json::json!("B remote"));
    wire.media.as_mut().unwrap().key = wire
        .media
        .as_ref()
        .unwrap()
        .key
        .replacen("test/", "new/", 1);
    store_b.0.lock().unwrap().objects.insert(
        format!("new/records/{}.json", f.id),
        (serde_json::to_vec(&wire).unwrap(), None),
    );
    f.core
        .backup_db(|db| crate::backup::spaces::select_local(db, &f.space.space_id, true))
        .unwrap();
    let started = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    f.store.0.lock().unwrap().hold_list = Some((started.clone(), release));
    let a_id = f.space.space_id.clone();
    let b_id = bspace.space_id.clone();
    let b_resolve = b.clone();
    let (stop, cancel) = tokio::sync::watch::channel(false);
    let worker = tokio::spawn(sync_engine::run_with(
        f.core.clone(),
        cancel,
        Arc::new(move |_, id| {
            if id == a_id {
                Ok(a.clone())
            } else if id == b_id {
                Ok(b_resolve.clone())
            } else {
                bail!("wrong space")
            }
        }),
    ));
    tokio::time::timeout(std::time::Duration::from_secs(3), started.notified())
        .await
        .unwrap();
    f.core
        .backup_db(|db| crate::backup::spaces::select_local(db, &bspace.space_id, true))
        .unwrap();
    f.core.wake_backup();
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            if f.core
                .backup_db(|db| repository::local_asset_id(db, &bspace.space_id, &f.id))
                .unwrap()
                .is_some()
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    let local = f
        .core
        .backup_db(|db| repository::local_asset_id(db, &bspace.space_id, &f.id))
        .unwrap()
        .unwrap();
    assert_ne!(local, f.id);
    assert_eq!(
        crate::backup::sync_read::list(&f.core, &bspace.space_id).unwrap()[0]
            .asset
            .metadata["title"],
        "B remote"
    );
    assert!(crate::backup::spaces::get(&f.core)
        .unwrap()
        .unwrap()
        .last_error
        .is_none());
    stop.send_replace(true);
    f.core.wake_backup();
    tokio::time::timeout(std::time::Duration::from_secs(1), worker)
        .await
        .unwrap()
        .unwrap();
}

use crate::backup::sync_delete;
fn deletion_operation(core: &Core) -> String {
    scheduler::operations(core)
        .unwrap()
        .into_iter()
        .find(|o| o.kind == "delete" && o.state != "completed")
        .unwrap()
        .operation_id
}
async fn delete_pass(core: &Arc<Core>, provider: &dyn BackupProvider, id: &str) -> Result<()> {
    core.backup_db(|db| {
        db.execute(
            "UPDATE backup_operations SET state='running' WHERE operation_id=?",
            [id],
        )?;
        Ok(())
    })?;
    sync_delete::execute(core, provider, id).await
}
#[tokio::test]
async fn p4_tombstone_keeps_other_device_local_video_and_blocks_revival() {
    let (f, a, engine) = sync_fixture().await;
    let op = claim(&engine, &f.core);
    engine.execute_upload(&op).await.unwrap();
    let (_root, bcore, b, bengine) = second_device(&f).await;
    let id = bcore.backup_assets().unwrap()[0].asset_id.clone();
    let local = crate::backup::sync_read::ensure(&bcore, b.as_ref(), &id)
        .await
        .unwrap();
    sync_delete::delete(&f.core, &f.space.space_id, &f.id)
        .await
        .unwrap();
    assert!(!f.root.path().join("video.mp4").exists());
    let deletion = deletion_operation(&f.core);
    delete_pass(&f.core, a.as_ref(), &deletion).await.unwrap();
    bengine.pull().await.unwrap();
    assert!(local.is_file());
    assert_eq!(bcore.backup_assets().unwrap()[0].asset_state, "active");
    let (gets, writes) = {
        let s = f.store.0.lock().unwrap();
        (s.get_count, s.writes)
    };
    assert_eq!(
        crate::backup::sync_read::ensure(&bcore, b.as_ref(), &id)
            .await
            .unwrap(),
        local
    );
    assert_eq!(f.store.0.lock().unwrap().get_count, gets);
    assert_eq!(f.store.0.lock().unwrap().writes, writes);
    bengine.enqueue().unwrap();
    assert!(scheduler::operations(&bcore)
        .unwrap()
        .iter()
        .all(|o| o.kind != "upload"));
    assert!(a.read_asset(&f.id).await.unwrap().unwrap().snapshot.deleted);
}
#[tokio::test]
async fn p4_missing_non_uploader_preserves_projection_and_all_pending_data() {
    let (f, a, engine) = sync_fixture().await;
    let op = claim(&engine, &f.core);
    engine.execute_upload(&op).await.unwrap();
    let (_root, bcore, b, bengine) = second_device(&f).await;
    let id = bcore.backup_assets().unwrap()[0].asset_id.clone();
    local_text(&bcore, &id, "unsynced local transcript");
    edit(&bcore, &id, "title", serde_json::json!("unsynced title"));
    sync_delete::delete(&f.core, &f.space.space_id, &f.id)
        .await
        .unwrap();
    let op = deletion_operation(&f.core);
    delete_pass(&f.core, a.as_ref(), &op).await.unwrap();
    bengine.pull().await.unwrap();
    assert!(
        !crate::backup::sync_read::cached(&bcore, &b.scope().space_id, &id)
            .unwrap()
            .deleted,
        "background tombstone must retain the old synchronized projection"
    );
    let writes = f.store.0.lock().unwrap().writes;
    assert_eq!(
        crate::backup::sync_read::ensure(&bcore, b.as_ref(), &id)
            .await
            .unwrap_err()
            .to_string(),
        "BACKUP_ASSET_DELETED"
    );
    assert_eq!(
        f.store.0.lock().unwrap().writes,
        writes,
        "on-demand confirmation cannot delete remote objects or multipart sessions"
    );
    assert!(crate::backup::sync_read::cached(&bcore, &b.scope().space_id, &id).is_ok());
    assert_eq!(
        crate::backup::sync_read::cached_transcript(&bcore, &b.scope().space_id, &id)
            .unwrap()
            .unwrap()
            .text,
        "unsynced local transcript"
    );
    assert_eq!(bcore.backup_assets().unwrap().len(), 1);
    let (visible, pending): (bool, String) = bcore
        .backup_db(|db| {
            Ok(db.query_row(
                "SELECT cloud_visible,pending_json FROM backup_space_assets",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?)
        })
        .unwrap();
    assert!(!visible);
    assert!(
        !serde_json::from_str::<sync_engine::PendingChanges>(&pending)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        bcore
            .backup_db(|db| Ok(db.query_row(
                "SELECT COUNT(*) FROM backup_upload_proofs",
                [],
                |r| r.get::<_, i64>(0)
            )?))
            .unwrap(),
        0
    );
}
#[tokio::test]
async fn p4_proven_uploader_cleans_projection_only_on_actual_missing_read() {
    let (f, b, bengine) = sync_fixture().await;
    let op = claim(&bengine, &f.core);
    bengine.execute_upload(&op).await.unwrap();
    let (_root, acore, a, _aengine) = second_device(&f).await;
    let aid = acore.backup_assets().unwrap()[0].asset_id.clone();
    std::fs::remove_file(f.root.path().join("video.mp4")).unwrap();
    local_text(&f.core, &f.id, "local transcript after last upload");
    sync_delete::delete(&acore, &a.scope().space_id, &aid)
        .await
        .unwrap();
    let op = deletion_operation(&acore);
    delete_pass(&acore, a.as_ref(), &op).await.unwrap();
    bengine.pull().await.unwrap();
    assert!(crate::backup::sync_read::cached(&f.core, &f.space.space_id, &f.id).is_ok());
    let writes = f.store.0.lock().unwrap().writes;
    assert_eq!(
        crate::backup::sync_read::ensure(&f.core, b.as_ref(), &f.id)
            .await
            .unwrap_err()
            .to_string(),
        "BACKUP_ASSET_DELETED"
    );
    assert_eq!(f.store.0.lock().unwrap().writes, writes);
    assert!(crate::backup::sync_read::cached(&f.core, &f.space.space_id, &f.id).is_err());
    assert_eq!(
        f.core.asset_transcript(&f.id).unwrap().unwrap().text,
        "local transcript after last upload"
    );
    assert_eq!(
        crate::backup::sync_read::cached_transcript(&f.core, &f.space.space_id, &f.id)
            .unwrap()
            .unwrap()
            .text,
        "local transcript after last upload"
    );
    let (proof, tomb, asset): (i64, i64, i64) = f
        .core
        .backup_db(|db| {
            Ok((
                db.query_row("SELECT COUNT(*) FROM backup_upload_proofs", [], |r| {
                    r.get(0)
                })?,
                db.query_row("SELECT COUNT(*) FROM backup_tombstones", [], |r| r.get(0))?,
                db.query_row("SELECT COUNT(*) FROM media_assets", [], |r| r.get(0))?,
            ))
        })
        .unwrap();
    assert_eq!((proof, tomb, asset), (1, 1, 1));
    assert!(crate::backup::sync_read::ensure(&f.core, b.as_ref(), &f.id)
        .await
        .is_err());
}
#[tokio::test]
async fn p4_late_upload_after_delete_never_revives_and_initiator_reclaims_it() {
    let (f, a, engine) = sync_fixture().await;
    let op = claim(&engine, &f.core);
    engine.execute_upload(&op).await.unwrap();
    let (_root, bcore, b, bengine) = second_device(&f).await;
    let id = bcore.backup_assets().unwrap()[0].asset_id.clone();
    local_text(&bcore, &id, "new pending text");
    let op = claim(&bengine, &bcore);
    let started = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    f.store.0.lock().unwrap().hold_upload = Some((started.clone(), release.clone()));
    let upload = tokio::spawn(async move { bengine.execute_upload(&op).await });
    tokio::time::timeout(std::time::Duration::from_secs(3), started.notified())
        .await
        .unwrap();
    sync_delete::delete(&f.core, &f.space.space_id, &f.id)
        .await
        .unwrap();
    let deletion = deletion_operation(&f.core);
    delete_pass(&f.core, a.as_ref(), &deletion).await.unwrap();
    release.notify_one();
    assert_eq!(
        upload.await.unwrap().unwrap_err().to_string(),
        "BACKUP_ASSET_DELETED"
    );
    assert!(a.read_asset(&f.id).await.unwrap().unwrap().snapshot.deleted);
    assert_eq!(
        bcore.asset_transcript(&id).unwrap().unwrap().text,
        "new pending text"
    );
    assert!(f
        .store
        .0
        .lock()
        .unwrap()
        .objects
        .keys()
        .any(|k| k.rsplit('/').next().is_some_and(|name|name.starts_with("transcript-") && name.ends_with(".json"))));
    delete_pass(&f.core, a.as_ref(), &deletion).await.unwrap();
    assert!(f
        .store
        .0
        .lock()
        .unwrap()
        .objects
        .keys()
        .all(|k| !k.contains("media/") && !k.rsplit('/').next().is_some_and(|name|name.starts_with("transcript-") && name.ends_with(".json"))));
    assert!(b.read_asset(&f.id).await.unwrap().unwrap().snapshot.deleted);
}
#[tokio::test]
async fn p4_cleanup_covers_unindexed_versions_and_part_sessions_but_retains_tombstone() {
    let (f, a, engine) = sync_fixture().await;
    let op = claim(&engine, &f.core);
    engine.execute_upload(&op).await.unwrap();
    let old = format!("test/media/{}/old/version/video.mp4", f.id);
    let orphan = format!("test/media/{}/orphan/video.mp4", f.id);
    {
        let mut s = f.store.0.lock().unwrap();
        s.objects.insert(old, (b"old version".to_vec(), None));
        s.uploads.insert(
            "other-device-session".into(),
            (orphan, "digest".into(), Default::default()),
        );
        s.objects.insert(
            "test/media/unrelated/video.mp4".into(),
            (b"untouched".to_vec(), None),
        );
    }
    sync_delete::delete(&f.core, &f.space.space_id, &f.id)
        .await
        .unwrap();
    let deletion = deletion_operation(&f.core);
    f.store.0.lock().unwrap().fail_after = Some("put");
    delete_pass(&f.core, a.as_ref(), &deletion).await.unwrap();
    assert!(f.store.0.lock().unwrap().uploads.is_empty());
    for _ in 0..2 {
        delete_pass(&f.core, a.as_ref(), &deletion).await.unwrap();
    }
    let s = f.store.0.lock().unwrap();
    assert!(s
        .objects
        .contains_key(&format!("test/records/{}.json", f.id)));
    assert!(s.objects.contains_key("test/media/unrelated/video.mp4"));
    assert_eq!(
        s.objects
            .keys()
            .filter(|k| k.contains(&format!("media/{}/", f.id)))
            .count(),
        0
    );
}
#[tokio::test]
async fn p4_release_rejects_unsynced_unique_copy_and_preserves_remote_asset_when_allowed() {
    let (f, a, engine) = sync_fixture().await;
    assert_eq!(
        crate::backup::lifecycle::release(&f.core, &f.id)
            .await
            .unwrap_err()
            .to_string(),
        "BACKUP_UNSYNCED_LOCAL_CHANGES"
    );
    assert!(f.root.path().join("video.mp4").is_file());
    let op = claim(&engine, &f.core);
    engine.execute_upload(&op).await.unwrap();
    sync_delete::release_with(&f.core, a.as_ref(), &f.id)
        .await
        .unwrap();
    assert!(!f.root.path().join("video.mp4").exists());
    assert!(!a.read_asset(&f.id).await.unwrap().unwrap().snapshot.deleted);
}

#[tokio::test]
async fn p4_same_remote_id_in_another_space_does_not_receive_deletion_or_cleanup() {
    let (f, a, engine) = sync_fixture().await;
    let op = claim(&engine, &f.core);
    engine.execute_upload(&op).await.unwrap();
    let mut other = f.space.clone();
    other.space_id = uuid::Uuid::new_v4().to_string();
    other.library_id = uuid::Uuid::new_v4().to_string();
    other.prefix = "other/".into();
    other.role = Role::Member;
    f.core
        .backup_db(|db| {
            repository::save_space(db, &other)?;
            db.execute(
                "UPDATE backup_provider_config SET protocol=2 WHERE space_id=?",
                [&other.space_id],
            )?;
            crate::backup::spaces::select_local(db, &other.space_id, true)
        })
        .unwrap();
    let b = Arc::new(S3Provider::new(
        f.store.clone(),
        other.clone(),
        capabilities(),
        f.core.backup_db(repository::device_id).unwrap(),
    ));
    b.open_library(true).await.unwrap();
    let original = a.read_asset(&f.id).await.unwrap().unwrap().snapshot;
    let mut media = original.media.unwrap();
    media.scope = b.scope().clone();
    let MediaLocation::S3Object { locator } = &mut media.location else {
        panic!("expected s3")
    };
    let old = locator.clone();
    *locator = locator.replacen("test/", "other/", 1);
    {
        let mut s = f.store.0.lock().unwrap();
        let object = s.objects[&old].clone();
        s.objects.insert(locator.clone(), object);
    }
    let change = AssetChange {
        recreate: false,
        resolved_conflicts: vec![],
        asset_id: f.id.clone(),
        base: None,
        fields: original.fields,
        field_bases: Default::default(),
        media: Some(media),
        cover: ReferenceChange::Unchanged,
        transcript: ReferenceChange::Unchanged,
        delete: false,
        device_id: f.core.backup_db(repository::device_id).unwrap(),
        operation_id: uuid::Uuid::new_v4().to_string(),
        conflicts: vec![],
    };
    b.publish_change(change, None).await.unwrap();
    let be = SyncEngine::new(f.core.clone(), b.clone());
    be.pull().await.unwrap();
    let bid = f
        .core
        .backup_db(|db| repository::local_asset_id(db, &other.space_id, &f.id))
        .unwrap()
        .unwrap();
    assert_ne!(bid, f.id);
    edit(&f.core, &bid, "title", serde_json::json!("B pending title"));
    let before: BTreeMap<_, _> = f
        .store
        .0
        .lock()
        .unwrap()
        .objects
        .iter()
        .filter(|(k, _)| k.starts_with("other/"))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    f.core
        .backup_db(|db| crate::backup::spaces::select_local(db, &f.space.space_id, true))
        .unwrap();
    sync_delete::delete(&f.core, &f.space.space_id, &f.id)
        .await
        .unwrap();
    let deletion = deletion_operation(&f.core);
    delete_pass(&f.core, a.as_ref(), &deletion).await.unwrap();
    let after: BTreeMap<_, _> = f
        .store
        .0
        .lock()
        .unwrap()
        .objects
        .iter()
        .filter(|(k, _)| k.starts_with("other/"))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    assert_eq!(before, after);
    assert!(!b.read_asset(&f.id).await.unwrap().unwrap().snapshot.deleted);
    assert!(!f
        .core
        .backup_db(|db| Ok(db.query_row(
            "SELECT remote_deleted FROM backup_space_assets WHERE space_id=?",
            [&other.space_id],
            |r| r.get::<_, bool>(0)
        )?))
        .unwrap());
}
#[tokio::test]
async fn p4_completed_delete_rechecks_late_objects_after_restart_with_same_operation() {
    let (f, a, engine) = sync_fixture().await;
    let op = claim(&engine, &f.core);
    engine.execute_upload(&op).await.unwrap();
    sync_delete::delete(&f.core, &f.space.space_id, &f.id)
        .await
        .unwrap();
    let deletion = deletion_operation(&f.core);
    for _ in 0..3 {
        delete_pass(&f.core, a.as_ref(), &deletion).await.unwrap();
    }
    let late = format!("test/media/{}/late/video.mp4", f.id);
    f.store
        .0
        .lock()
        .unwrap()
        .objects
        .insert(late.clone(), (b"late object".to_vec(), None));
    let Fixture {
        root,
        core,
        store,
        space,
        id: _,
    } = f;
    drop(engine);
    drop(core);
    drop(a);
    let core = Core::open(
        &root.path().join("app.sqlite"),
        root.path().join("downloads"),
        "unused".into(),
        vec![],
        |_| {},
    )
    .unwrap();
    let op = scheduler::operations(&core)
        .unwrap()
        .into_iter()
        .find(|o| o.operation_id == deletion)
        .unwrap();
    assert_eq!(op.state, "pending");
    assert_eq!(op.phase, "cleanup_verify");
    let provider = S3Provider::new(
        store.clone(),
        space,
        capabilities(),
        core.backup_db(repository::device_id).unwrap(),
    );
    delete_pass(&core, &provider, &deletion).await.unwrap();
    assert!(!store.0.lock().unwrap().objects.contains_key(&late));
}
#[tokio::test]
async fn p4_missing_remote_media_does_not_allow_releasing_the_only_copy() {
    let (f, a, engine) = sync_fixture().await;
    let op = claim(&engine, &f.core);
    engine.execute_upload(&op).await.unwrap();
    let media = a
        .read_asset(&f.id)
        .await
        .unwrap()
        .unwrap()
        .snapshot
        .media
        .unwrap();
    let MediaLocation::S3Object { locator } = media.location else {
        panic!("s3 required")
    };
    f.store.0.lock().unwrap().objects.remove(&locator);
    assert_eq!(
        sync_delete::release_with(&f.core, a.as_ref(), &f.id)
            .await
            .unwrap_err()
            .to_string(),
        "BACKUP_OBJECT_NOT_FOUND"
    );
    assert!(f.root.path().join("video.mp4").is_file());
}

#[tokio::test]
async fn p4_release_rechecks_tombstone_space_and_delete_intent_after_waiting_for_lock() {
    for mode in ["tombstone", "switch", "intent"] {
        let (f, a, engine) = sync_fixture().await;
        let op = claim(&engine, &f.core);
        engine.execute_upload(&op).await.unwrap();
        let (_root, othercore, other, _) = second_device(&f).await;
        let otherid = othercore.backup_assets().unwrap()[0].asset_id.clone();
        let held = f.core.backup_write_lock.lock().await;
        let before = f.store.0.lock().unwrap().head_count;
        let core = f.core.clone();
        let provider = a.clone();
        let local = f.id.clone();
        let releasing = tokio::spawn(async move {
            sync_delete::release_with(&core, provider.as_ref(), &local).await
        });
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            loop {
                if f.store.0.lock().unwrap().head_count > before {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        if mode == "tombstone" {
            sync_delete::delete(&othercore, &other.scope().space_id, &otherid)
                .await
                .unwrap();
            let deletion = deletion_operation(&othercore);
            delete_pass(&othercore, other.as_ref(), &deletion)
                .await
                .unwrap();
            engine.pull().await.unwrap();
        } else if mode == "switch" {
            let mut space = f.space.clone();
            space.space_id = uuid::Uuid::new_v4().to_string();
            space.library_id = uuid::Uuid::new_v4().to_string();
            space.prefix = "switched/".into();
            f.core
                .backup_db(|db| {
                    repository::save_space(db, &space)?;
                    crate::backup::spaces::select_local(db, &space.space_id, true)
                })
                .unwrap();
        } else {
            sync_delete::intent(&f.core, &f.space.space_id, &f.id).unwrap();
        }
        drop(held);
        let error = releasing.await.unwrap().unwrap_err().to_string();
        assert!(
            matches!(
                error.as_str(),
                "BACKUP_ASSET_DELETED" | "BACKUP_SPACE_INACTIVE" | "BACKUP_DELETE_PENDING"
            ),
            "mode {mode}: {error}"
        );
        assert!(
            f.root.path().join("video.mp4").is_file(),
            "mode {mode} deleted the only local copy"
        );
    }
}
#[test]
fn p4_restart_with_multiple_terminal_or_active_delete_logs_is_idempotent() {
    for existing_active in [false, true] {
        let db = rusqlite::Connection::open_in_memory().unwrap();
        repository::init(&db).unwrap();
        db.execute("INSERT INTO backup_spaces VALUES('space','lib','https://example','bucket','prefix/','member','device',1,NULL,1)",[]).unwrap();
        db.execute(
            "INSERT INTO media_assets VALUES('asset',NULL,'{}','video.mp4',NULL,1,'active')",
            [],
        )
        .unwrap();
        for operation in ["terminal-a", "terminal-b"] {
            db.execute("INSERT INTO backup_operations(operation_id,space_id,asset_id,kind,state,phase,payload_json) VALUES(?1,'space','asset','delete','completed','completed','{\"scope\":{\"protocol\":2}}')",[operation]).unwrap();
        }
        if existing_active {
            db.execute("INSERT INTO backup_operations(operation_id,space_id,asset_id,kind,state,phase,payload_json) VALUES('active','space','asset','delete','pending','objects','{\"scope\":{\"protocol\":2}}')",[]).unwrap();
        }
        repository::init(&db).unwrap();
        repository::init(&db).unwrap();
        assert_eq!(db.query_row("SELECT COUNT(*) FROM backup_operations WHERE state NOT IN ('completed','canceled')",[],|r|r.get::<_,i64>(0)).unwrap(),1);
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM backup_operations", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            if existing_active { 3 } else { 2 }
        );
    }
}
#[tokio::test]
async fn p4_repeated_explicit_deletion_reuses_original_operation() {
    let (f, a, engine) = sync_fixture().await;
    let op = claim(&engine, &f.core);
    engine.execute_upload(&op).await.unwrap();
    let original = sync_delete::intent(&f.core, &f.space.space_id, &f.id).unwrap();
    for _ in 0..3 {
        delete_pass(&f.core, a.as_ref(), &original).await.unwrap();
    }
    assert_eq!(
        sync_delete::intent(&f.core, &f.space.space_id, &f.id).unwrap(),
        original
    );
    assert_eq!(
        f.core
            .backup_db(|db| Ok(db.query_row(
                "SELECT COUNT(*) FROM backup_operations WHERE kind='delete'",
                [],
                |r| r.get::<_, i64>(0)
            )?))
            .unwrap(),
        1
    );
}
