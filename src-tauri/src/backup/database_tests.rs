use super::{Fixture, Vault};
use crate::backup::{
    database, repository, s3_provider::S3Provider, settings::Vault as _, spaces,
    store::ObjectStore, sync_read,
};
use crate::desktop_core::Core;
use age::{secrecy::ExposeSecret, x25519::Identity};
use rusqlite::{params, Connection};
use uuid::Uuid;
use zeroize::Zeroizing;

const PASSWORD: &str = "换机恢复 synthetic backup";

#[tokio::test]
async fn database_backup_restores_telegram_binding_with_destination_session() {
    let f = Fixture::new(24).await;
    f.upload().await.unwrap();
    let mut space = f.space.clone();
    space.endpoint.clear();
    space.bucket.clear();
    space.prefix.clear();
    let binding = spaces::TelegramConfig {
        account_id: "101".into(),
        channel_id: "202".into(),
    };
    f.core.backup_db(|db| {
        db.execute("UPDATE backup_spaces SET endpoint='',bucket='',prefix='' WHERE space_id=?", [&space.space_id])?;
        db.execute("UPDATE backup_provider_config SET provider_type='telegram',profile='telegram' WHERE space_id=?", [&space.space_id])?;
        db.execute("INSERT INTO telegram_spaces VALUES(?1,?2,?3,0)", params![space.space_id,binding.account_id,binding.channel_id])?;
        db.execute("INSERT INTO telegram_account_bindings VALUES('backup','old-session')", [])?;
        Ok(())
    }).unwrap();
    let key = Identity::generate();
    let (cipher, _, _) =
        database::encrypt(&f.root.path().join("app.sqlite"), &key, || false).unwrap();
    let restored = f.root.path().join("telegram-restored.sqlite");
    database::decrypt(cipher.path(), &restored, &key).unwrap();
    let config = spaces::config(&f.core, &space.space_id).unwrap();
    database::prepare_restore(
        &restored,
        &space,
        &config,
        &Uuid::new_v4().to_string(),
        f.root.path(),
        "new-key",
        Some((&binding, "new-session")),
    )
    .unwrap();
    let db = Connection::open(restored).unwrap();
    assert_eq!(
        db.query_row(
            "SELECT session_ref FROM telegram_account_bindings WHERE purpose='backup'",
            [],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        "new-session"
    );
    assert!(db
        .query_row(
            "SELECT enabled FROM backup_spaces WHERE space_id=?",
            [&space.space_id],
            |row| row.get::<_, bool>(0)
        )
        .unwrap());
}

#[tokio::test]
async fn database_backup_requires_a_password_each_time_and_copies_only_generated_passwords() {
    let f = Fixture::new(24).await;
    f.upload().await.unwrap();
    let id = &f.space.space_id;
    let vault = Vault::default();
    assert!(!database::status(&f.core, id).unwrap().enabled);
    assert!(database::request_backup(&f.core, id).is_err());
    assert!(
        database::set_enabled_with(&f.core, id, true, None, false, &vault)
            .await
            .is_err()
    );
    assert!(
        !database::set_enabled_with(
            &f.core,
            id,
            true,
            Some(Zeroizing::new(PASSWORD.into())),
            false,
            &vault
        )
        .await
        .unwrap()
        .generated_password
    );
    let saved_ref: String = f
        .core
        .backup_db(|db| {
            Ok(db.query_row(
                "SELECT key_ref FROM backup_databases WHERE space_id=?",
                [id],
                |row| row.get(0),
            )?)
        })
        .unwrap();
    assert!(!vault.read(&saved_ref).unwrap().contains(PASSWORD));
    assert!(database::generated_recovery_password_with(&f.core, id, &vault).is_err());
    assert!(
        !database::set_enabled_with(&f.core, id, false, None, false, &vault)
            .await
            .unwrap()
            .enabled
    );
    assert!(database::request_backup(&f.core, id).is_err());
    assert!(
        database::set_enabled_with(&f.core, id, true, None, false, &vault)
            .await
            .is_err()
    );
    let generated = "aB3eF6hI9kL2nO5qR8tU1wX4";
    let enabled = database::set_enabled_with(
        &f.core,
        id,
        true,
        Some(Zeroizing::new(generated.into())),
        true,
        &vault,
    )
    .await
    .unwrap();
    assert!(enabled.enabled && enabled.generated_password && !enabled.queued);
    assert!(enabled.next_attempt_at.unwrap() >= chrono::Utc::now().timestamp() + 24 * 3600 - 1);
    let after_one_hour = database::set_interval(&f.core, id, 1).unwrap();
    assert_eq!(
        after_one_hour.next_attempt_at,
        enabled.next_attempt_at.map(|deadline| deadline - 23 * 3600)
    );
    assert!(database::request_backup(&f.core, id).unwrap().queued);
    assert_eq!(
        database::generated_recovery_password_with(&f.core, id, &vault)
            .unwrap()
            .as_str(),
        generated
    );
    assert_eq!(vault.0.lock().unwrap().len(), 1);
    assert!(!vault.0.lock().unwrap().contains_key(&saved_ref));
}

#[tokio::test]
async fn database_backup_password_reopens_on_another_device() {
    let f = Fixture::new(24).await;
    f.upload().await.unwrap();
    let id = &f.space.space_id;
    let vault = Vault::default();
    database::set_enabled_with(
        &f.core,
        id,
        true,
        Some(Zeroizing::new(PASSWORD.into())),
        false,
        &vault,
    )
    .await
    .unwrap();
    let key = {
        let credentials = vault.0.lock().unwrap();
        let secret: serde_json::Value =
            serde_json::from_str(credentials.values().next().unwrap()).unwrap();
        secret["key"].as_str().unwrap().parse::<Identity>().unwrap()
    };
    let (snapshot, _, _) =
        database::encrypt(&f.root.path().join("app.sqlite"), &key, || false).unwrap();
    let correct =
        database::password_identity(Zeroizing::new(PASSWORD.into()), f.space.library_id.clone());
    let wrong = database::password_identity(
        Zeroizing::new("different recovery password".into()),
        f.space.library_id.clone(),
    );
    let (correct, wrong) = tokio::join!(correct, wrong);
    let output = f.root.path().join("password-restore.sqlite");
    assert!(database::decrypt(snapshot.path(), &output, &wrong.unwrap()).is_err());
    database::decrypt(snapshot.path(), &output, &correct.unwrap()).unwrap();
    assert!(Connection::open(output)
        .unwrap()
        .query_row("SELECT count(*) FROM download_tasks", [], |r| r
            .get::<_, i64>(0))
        .is_ok());
}

fn seed_backup(core: &Core, id: &str) {
    core.backup_db(|db| {
        db.execute(
            "INSERT INTO backup_databases(space_id,key_ref,enabled) VALUES(?1,'fixture',1)",
            [id],
        )?;
        Ok(())
    })
    .unwrap();
}

#[tokio::test]
async fn database_backup_schedule_and_last_telegram_link_are_kept_per_space() {
    let f = Fixture::new(24).await;
    f.upload().await.unwrap();
    let id = &f.space.space_id;
    let vault = Vault::default();
    assert_eq!(database::status(&f.core, id).unwrap().interval_hours, 24);
    assert!(database::set_interval(&f.core, id, 6).is_err());
    seed_backup(&f.core, id);
    let queued = database::set_interval(&f.core, id, 6).unwrap();
    assert!(queued.queued);
    assert!(database::set_interval(&f.core, id, 0).is_err());
    f.core
        .backup_db(|db| {
            db.execute("INSERT INTO telegram_spaces VALUES(?1,'101','202',0)", [id])?;
            database::record_success(db, id, Some(123))
        })
        .unwrap();
    let completed = database::status(&f.core, id).unwrap();
    assert_eq!(
        completed.last_message_link.as_deref(),
        Some("https://t.me/c/202/123")
    );
    let success = completed.last_success_at.unwrap();
    assert_eq!(completed.next_attempt_at, Some(success + 6 * 3600));
    for hours in [1, 3, 24] {
        let scheduled = database::set_interval(&f.core, id, hours).unwrap();
        assert_eq!(
            scheduled.next_attempt_at,
            Some(success + i64::from(hours) * 3600)
        );
    }
    assert!(database::set_interval(&f.core, id, 168).is_err());
    assert!(database::request_backup(&f.core, id).unwrap().queued);
    assert_eq!(
        database::request_backup(&f.core, id)
            .err()
            .unwrap()
            .to_string(),
        "DATABASE_BACKUP_IN_PROGRESS"
    );
    assert!(database::set_interval(&f.core, id, 24).unwrap().queued);
    let canceled = database::cancel_backup(&f.core, id).unwrap();
    assert!(!canceled.queued && canceled.last_canceled);
    assert!(canceled.next_attempt_at.unwrap() >= success + 24 * 3600);
    assert_eq!(canceled.last_message_link, completed.last_message_link);
    assert!(database::request_backup(&f.core, id).unwrap().queued);
    let disabled = database::set_enabled_with(&f.core, id, false, None, false, &vault)
        .await
        .unwrap();
    assert!(!disabled.enabled);
    assert_eq!(disabled.last_success_at, Some(success));
    assert_eq!(disabled.last_message_link, completed.last_message_link);
}

#[test]
fn database_backup_prerelease_configuration_resets_without_changing_other_tables() {
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch(
        "CREATE TABLE backup_spaces(space_id TEXT PRIMARY KEY);
        INSERT INTO backup_spaces VALUES('existing');
        CREATE TABLE backup_databases(space_id TEXT PRIMARY KEY,key_ref TEXT,enabled INTEGER);
        INSERT INTO backup_databases VALUES('existing','key',1);",
    )
    .unwrap();
    database::init_schema(&db).unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM backup_databases", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    db.execute("INSERT INTO backup_databases(space_id,key_ref,enabled,interval_hours) VALUES('existing','new',1,1)", []).unwrap();
    database::init_schema(&db).unwrap();
    assert_eq!(
        db.query_row(
            "SELECT interval_hours FROM backup_databases JOIN backup_spaces USING(space_id)",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
}

#[tokio::test]
async fn database_backup_running_cancel_keeps_the_worker_claim_until_it_stops() {
    let f = Fixture::new(24).await;
    f.upload().await.unwrap();
    let id = &f.space.space_id;
    seed_backup(&f.core, id);
    database::set_interval(&f.core, id, 1).unwrap();
    let transfer = database::claim(&f.core, id).unwrap().unwrap();
    let mut worker = transfer.stop.subscribe();
    assert!(!*worker.borrow());
    assert!(database::request_backup(&f.core, id).is_err());
    let canceled = database::cancel_backup(&f.core, id).unwrap();
    assert_eq!(
        serde_json::to_value(canceled).unwrap()["progress"]["phase"],
        "canceling"
    );
    worker.changed().await.unwrap();
    assert!(*worker.borrow());
    assert!(database::request_backup(&f.core, id).is_err());
    assert!(database::claim(&f.core, id).unwrap().is_none());
    let before = chrono::Utc::now().timestamp();
    database::finish_attempt(&f.core, id, Err(anyhow::anyhow!("CANCELED")), true).unwrap();
    let status = database::status(&f.core, id).unwrap();
    assert!(status.progress.is_none() && status.last_canceled);
    assert!(!status.queued && status.last_error.is_none());
    assert!((before..=chrono::Utc::now().timestamp())
        .contains(&(status.next_attempt_at.unwrap() - 3600)));
    assert!(database::request_backup(&f.core, id).unwrap().queued);
    assert!(database::claim(&f.core, id).unwrap().is_some());
    let before = chrono::Utc::now().timestamp();
    database::finish_attempt(&f.core, id, Err(anyhow::anyhow!("FAILED")), false).unwrap();
    let failed = database::status(&f.core, id).unwrap();
    assert_eq!(failed.last_error.as_deref(), Some("FAILED"));
    assert!(!failed.queued && !failed.last_canceled);
    assert!((before..=chrono::Utc::now().timestamp())
        .contains(&(failed.next_attempt_at.unwrap() - 3600)));
}

#[tokio::test]
async fn database_backup_encrypted_wal_snapshot_and_cross_device_restore() {
    let f = Fixture::new(24).await;
    f.upload().await.unwrap();
    let source = f.root.path().join("app.sqlite");
    let original_device = f.core.backup_db(repository::device_id).unwrap();
    f.core
        .backup_db(|db| {
            db.execute(
                "INSERT INTO download_tasks VALUES(?1, ?2)",
                params![
                    "pending-download",
                    serde_json::json!({
                        "id":"pending-download", "revision":1, "status":"downloading",
                        "bytes":0, "created_at":1, "canceling":false,
                        "path":"/old-machine/media.mp4", "download_requested":true
                    })
                    .to_string()
                ],
            )?;
            db.execute(
                "INSERT INTO platform_settings(platform,user_agent,cookie) VALUES('youtube','fixture','synthetic-cookie')",
                [],
            )?;
            Ok(())
        })
        .unwrap();
    assert!(
        source
            .with_extension("sqlite-wal")
            .metadata()
            .unwrap()
            .len()
            > 0
    );
    let key = Identity::generate();
    let (file, length, hash) = database::encrypt(&source, &key, || false).unwrap();
    let encoded = std::fs::read(file.path()).unwrap();
    assert!(encoded.starts_with(b"age-encryption.org/v1\n"));
    assert!(!encoded
        .windows(b"synthetic-cookie".len())
        .any(|w| w == b"synthetic-cookie"));
    let prefix = database::prefix(&f.space, &key);
    let device_prefix = format!("{prefix}{original_device}/");
    let other_device = format!(
        "{prefix}{}/000000000001-{}.sqlite.age",
        Uuid::new_v4(),
        Uuid::new_v4()
    );
    for timestamp in 1..5 {
        let old = format!(
            "{device_prefix}{timestamp:012}-{}.sqlite.age",
            Uuid::new_v4()
        );
        database::publish(
            &f.store,
            &f.space,
            &old,
            file.path(),
            length,
            hash.clone(),
            None,
        )
        .await
        .unwrap();
    }
    database::publish(
        &f.store,
        &f.space,
        &other_device,
        file.path(),
        length,
        hash.clone(),
        None,
    )
    .await
    .unwrap();
    // A clock rollback can place a successful new upload before older objects.
    let object = format!("{device_prefix}000000000000-{}.sqlite.age", Uuid::new_v4());
    database::publish(
        &f.store,
        &f.space,
        &object,
        file.path(),
        length,
        hash.clone(),
        None,
    )
    .await
    .unwrap();
    database::prune(&f.store, &device_prefix, &original_device, &object)
        .await
        .unwrap();
    assert!(f.store.head(&object).await.unwrap().is_some());
    assert!(f.store.head(&other_device).await.unwrap().is_some());
    assert_eq!(
        f.store
            .0
            .lock()
            .unwrap()
            .objects
            .keys()
            .filter(|name| name.starts_with(&device_prefix))
            .count(),
        3
    );

    let root = tempfile::tempdir().unwrap();
    let restored_path = root.path().join("app.sqlite");
    let restored_dir = root.path().join("downloads");
    std::fs::create_dir(&restored_dir).unwrap();
    let destination = Core::open(
        &restored_path,
        restored_dir.clone(),
        "unused".into(),
        vec![],
        |_| {},
    )
    .unwrap();
    let destination_id = destination.backup_db(repository::device_id).unwrap();
    assert_ne!(destination_id, original_device);
    destination.backup_db(|db| {
        db.execute_batch("CREATE TABLE local_before_restore(value TEXT); INSERT INTO local_before_restore VALUES('kept');")?;
        Ok(())
    }).unwrap();
    drop(destination);

    let downloaded = root.path().join("encrypted");
    database::download(&f.store, &object, &downloaded)
        .await
        .unwrap();
    let wrong = Identity::generate();
    let pending = database::pending(&restored_path);
    assert!(database::decrypt(&downloaded, &pending, &wrong).is_err());
    assert!(!pending.exists());
    let mut corrupted = encoded;
    *corrupted.last_mut().unwrap() ^= 0x80;
    std::fs::write(root.path().join("corrupt"), corrupted).unwrap();
    assert!(database::decrypt(
        &root.path().join("corrupt"),
        &root.path().join("bad.sqlite"),
        &key
    )
    .is_err());
    let imported_key: Identity = key.to_string().expose_secret().parse().unwrap();
    database::decrypt(&downloaded, &pending, &imported_key).unwrap();
    let config = spaces::config(&f.core, &f.space.space_id).unwrap();
    let mut target = f.space.clone();
    target.space_id = Uuid::new_v4().to_string();
    target.credential_ref = Some("destination-only-ref".into());
    database::prepare_restore(
        &pending,
        &target,
        &config,
        &destination_id,
        &restored_dir,
        "new-key-ref",
        None,
    )
    .unwrap();
    let staged_copy = std::fs::read(&pending).unwrap();
    let restored = Core::open(
        &restored_path,
        restored_dir.clone(),
        "unused".into(),
        vec![],
        |_| {},
    )
    .unwrap();
    assert!(!pending.exists());
    assert_eq!(
        restored.backup_db(repository::device_id).unwrap(),
        destination_id
    );
    assert_eq!(restored.settings().download_directory, restored_dir);
    assert_eq!(
        restored.list()[0].error.as_deref(),
        Some("DATABASE_RESTORE_INTERRUPTED")
    );
    assert_eq!(restored.list()[0].path, None);
    assert!(restored
        .backup_asset(&f.id)
        .unwrap()
        .unwrap()
        .path
        .is_none());
    let space = spaces::saved_space(&restored, &f.space.space_id).unwrap();
    assert_eq!(
        space.credential_ref.as_deref(),
        Some("destination-only-ref")
    );
    assert!(database::discovery_paused(&restored, &space.space_id).unwrap());
    assert!(
        !database::status(&restored, &space.space_id)
            .unwrap()
            .enabled
    );
    assert_eq!(
        restored
            .backup_db(|db| Ok(db.query_row(
                "SELECT cookie FROM platform_settings WHERE platform='youtube'",
                [],
                |row| row.get::<_, String>(0)
            )?))
            .unwrap(),
        "synthetic-cookie"
    );
    assert_eq!(f.store.0.lock().unwrap().video_gets, 0);
    let provider = S3Provider::new(
        f.store.clone(),
        space,
        config.capabilities.unwrap(),
        destination_id,
    );
    let media = sync_read::ensure(&restored, &provider, &f.id)
        .await
        .unwrap();
    assert_eq!(
        std::fs::read(media).unwrap(),
        std::fs::read(f.root.path().join("video.mp4")).unwrap()
    );
    assert_eq!(f.store.0.lock().unwrap().video_gets, 1);
    assert!(database::discovery_paused(&restored, &f.space.space_id).unwrap());
    drop(restored);
    // A staged file left behind after commit must not replay over new data.
    std::fs::write(&pending, staged_copy).unwrap();
    let restored = Core::open(
        &restored_path,
        restored_dir,
        "unused".into(),
        vec![],
        |_| {},
    )
    .unwrap();
    assert!(restored
        .backup_asset(&f.id)
        .unwrap()
        .unwrap()
        .path
        .is_some());
    assert!(!pending.exists());
    let local_backup = std::fs::read_dir(root.path())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|file| file.to_string_lossy().contains(".before-database-restore-"))
        .unwrap();
    let db = Connection::open(local_backup).unwrap();
    assert_eq!(
        db.query_row("SELECT value FROM local_before_restore", [], |row| row
            .get::<_, String>(
            0
        ))
        .unwrap(),
        "kept"
    );
}
