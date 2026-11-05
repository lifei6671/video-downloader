use rusqlite::Connection;
use std::{fs, path::Path};
use yingqu_core::backup::repository;

const LEGACY: &str = include_str!("../../tests/fixtures/backup-v5.sql");

#[test]
fn v6_deletion_hides_are_scoped_atomically_without_losing_explicit_hides() {
    let db = Connection::open_in_memory().unwrap();
    repository::init(&db).unwrap();
    db.execute_batch("INSERT INTO backup_spaces VALUES('space','library','https://unit.invalid','test','test/',NULL,1);
        INSERT INTO backup_provider_config(space_id,provider_type,profile,protocol) VALUES('space','s3','r2',2);
        INSERT INTO media_assets VALUES('deleted',NULL,'{}','video.mp4',NULL,4,'active');
        INSERT INTO media_assets VALUES('hidden',NULL,'{}','video.mp4',NULL,4,'active');
        INSERT INTO asset_hidden_history VALUES('deleted'),('hidden');
        INSERT INTO backup_operations(operation_id,space_id,asset_id,kind,state,phase,payload_json) VALUES('delete','space','deleted','delete','completed','completed','{\"local_cleanup_done\":true}');
        DROP TABLE asset_deletion_hidden_history;
        UPDATE backup_schema SET version=6;
        CREATE TRIGGER reject_v8 BEFORE UPDATE OF version ON backup_schema WHEN NEW.version=8 BEGIN SELECT RAISE(ABORT,'injected failure'); END;").unwrap();
    assert!(repository::init(&db).is_err());
    assert_eq!(version(&db), 6);
    assert_eq!(db.query_row("SELECT count(*) FROM asset_hidden_history", [], |r| r.get::<_, i64>(0)).unwrap(), 2);
    assert_eq!(db.query_row("SELECT count(*) FROM sqlite_master WHERE name='asset_deletion_hidden_history'", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
    db.execute("DROP TRIGGER reject_v8", []).unwrap();
    repository::init(&db).unwrap();
    assert_eq!(version(&db), 8);
    assert_eq!(db.query_row("SELECT asset_id FROM asset_hidden_history", [], |r| r.get::<_, String>(0)).unwrap(), "hidden");
    assert_eq!(db.query_row("SELECT space_id,asset_id FROM asset_deletion_hidden_history", [], |r| Ok((r.get::<_, String>(0)?,r.get::<_, String>(1)?))).unwrap(), ("space".into(),"deleted".into()));
    // Explicitly hiding a remaining membership after migration must survive
    // every subsequent startup; migration evidence is consumed only once.
    db.execute("INSERT INTO asset_hidden_history VALUES('deleted')", []).unwrap();
    repository::init(&db).unwrap();
    assert_eq!(db.query_row("SELECT count(*) FROM asset_hidden_history", [], |r| r.get::<_, i64>(0)).unwrap(), 2);
}

fn seed(db: &Connection) {
    db.pragma_update(None, "foreign_keys", true).unwrap();
    db.execute_batch(LEGACY).unwrap();
    db.execute(
        "INSERT INTO backup_device VALUES(1,'device-before-migration')",
        [],
    )
    .unwrap();
    db.execute("INSERT INTO backup_spaces VALUES('space','library','https://unit.invalid','test','test/','primary','writer',1,'vault-reference',1)", []).unwrap();
    db.execute("INSERT INTO backup_current_space VALUES(1,'space')", [])
        .unwrap();
    db.execute("INSERT INTO backup_provider_config(space_id,provider_type,profile,capabilities_json,protocol) VALUES('space','s3','r2','{}',2)", []).unwrap();
    db.execute("INSERT INTO backup_sync_preference VALUES(1,1)", [])
        .unwrap();
    db.execute(
        "INSERT INTO media_assets VALUES('asset',NULL,'{}','video.mp4','hash',4,'active')",
        [],
    )
    .unwrap();
    db.execute(
        "INSERT INTO asset_local_files VALUES('asset','video.mp4',4,'hash','{}',1,'present')",
        [],
    )
    .unwrap();
    db.execute(
        "INSERT INTO asset_covers VALUES('asset',?1)",
        [b"original-cover".as_slice()],
    )
    .unwrap();
    db.execute("INSERT INTO asset_transcripts VALUES('asset','hash','保留文案','model',NULL,'pipeline',1,NULL)", []).unwrap();
    db.execute("INSERT INTO asset_remote_objects VALUES('space','asset','record','etag',7,'video-key','cover-key','text-key','backed_up')", []).unwrap();
    db.execute(
        "INSERT INTO asset_remote_records VALUES('space','asset','{\"original\":true}')",
        [],
    )
    .unwrap();
    db.execute("INSERT INTO backup_space_assets(space_id,remote_asset_id,local_asset_id) VALUES('space','remote','asset')", []).unwrap();
    db.execute("INSERT INTO backup_operations(operation_id,space_id,asset_id,kind,state,phase,payload_json,last_error) VALUES('blocked','space','asset','upload','blocked','uploading_media','{\"nonce\":123}','original-error')", []).unwrap();
    db.execute("INSERT INTO telegram_media_sends VALUES('blocked','video','{\"random_id\":123,\"state\":\"sending\",\"original_reference\":true}')", []).unwrap();
}

fn version(db: &Connection) -> i64 {
    db.query_row("SELECT version FROM backup_schema", [], |row| row.get(0))
        .unwrap()
}

fn columns(db: &Connection, table: &str) -> usize {
    db.prepare(&format!("PRAGMA table_info({table})"))
        .unwrap()
        .query_map([], |_| Ok(()))
        .unwrap()
        .count()
}

fn backups(root: &Path) -> Vec<std::path::PathBuf> {
    fs::read_dir(root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .contains(".before-backup-v")
                && path
                    .extension()
                    .is_some_and(|extension| extension == "sqlite")
        })
        .collect()
}

#[test]
fn v5_migration_preserves_wal_snapshot_receipts_and_blocks_automatic_work() {
    let root = tempfile::tempdir().unwrap();
    let db = Connection::open(root.path().join("user.sqlite")).unwrap();
    db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0;")
        .unwrap();
    seed(&db);
    db.execute("INSERT INTO backup_operations(operation_id,space_id,kind,state,phase,payload_json) VALUES('active','space','probe','running','unknown','{\"nonce\":456}')", []).unwrap();
    repository::init(&db).unwrap();
    assert_eq!(version(&db), 8);
    assert_eq!(columns(&db, "backup_spaces"), 7);
    assert_eq!(columns(&db, "asset_remote_objects"), 4);
    assert_eq!(columns(&db, "backup_provider_config"), 7);
    assert_eq!(
        repository::device_id(&db).unwrap(),
        "device-before-migration"
    );
    let (credential, enabled): (String, bool) = db
        .query_row(
            "SELECT credential_ref,enabled FROM backup_spaces",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(credential, "vault-reference");
    assert!(!enabled);
    assert_eq!(
        db.query_row("SELECT enabled FROM backup_sync_preference", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        db.query_row("SELECT enabled FROM backup_legacy_v5_spaces", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        db.query_row(
            "SELECT media_key FROM backup_legacy_v5_remote_objects",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "video-key"
    );
    assert_eq!(
        db.query_row(
            "SELECT state FROM backup_legacy_v5_operations WHERE operation_id='active'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "running"
    );
    assert_eq!(
        db.query_row(
            "SELECT state FROM backup_operations WHERE operation_id='active'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "blocked"
    );
    let blocked: (String,String,String) = db.query_row("SELECT state,payload_json,last_error FROM backup_operations WHERE operation_id='blocked'", [], |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert_eq!(
        blocked,
        (
            "blocked".into(),
            "{\"nonce\":123}".into(),
            "original-error".into()
        )
    );
    assert_eq!(
        db.query_row("SELECT body FROM asset_covers", [], |r| r
            .get::<_, Vec<u8>>(0))
            .unwrap(),
        b"original-cover"
    );
    assert_eq!(
        db.query_row("SELECT text FROM asset_transcripts", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "保留文案"
    );
    assert_eq!(
        db.query_row("SELECT record_json FROM asset_remote_records", [], |r| {
            r.get::<_, String>(0)
        })
        .unwrap(),
        "{\"original\":true}"
    );
    let evidence: String = db
        .query_row("SELECT entry_json FROM telegram_media_sends", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(
        evidence,
        "{\"random_id\":123,\"state\":\"sending\",\"original_reference\":true}"
    );
    assert!(db
        .prepare("PRAGMA foreign_key_check")
        .unwrap()
        .query([])
        .unwrap()
        .next()
        .unwrap()
        .is_none());
    let files = backups(root.path());
    assert_eq!(files.len(), 1);
    let before = Connection::open(&files[0]).unwrap();
    assert_eq!(version(&before), 5);
    assert_eq!(columns(&before, "backup_spaces"), 10);
    assert_eq!(
        before
            .query_row("SELECT entry_json FROM telegram_media_sends", [], |r| {
                r.get::<_, String>(0)
            })
            .unwrap(),
        evidence
    );
    assert_eq!(
        before
            .query_row(
                "SELECT state FROM backup_operations WHERE operation_id='active'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        "running"
    );
    repository::init(&db).unwrap();
    assert_eq!(backups(root.path()).len(), 1);
}

#[test]
fn failed_migration_rolls_back_every_table_and_keeps_a_complete_backup() {
    let root = tempfile::tempdir().unwrap();
    let db = Connection::open(root.path().join("user.sqlite")).unwrap();
    seed(&db);
    db.execute_batch("CREATE TRIGGER reject_upgrade BEFORE UPDATE OF version ON backup_schema WHEN NEW.version=8 BEGIN SELECT RAISE(ABORT,'injected migration failure'); END;").unwrap();
    assert!(repository::init(&db)
        .unwrap_err()
        .to_string()
        .contains("injected migration failure"));
    assert_eq!(version(&db), 5);
    assert_eq!(columns(&db, "backup_spaces"), 10);
    assert_eq!(columns(&db, "asset_remote_objects"), 9);
    assert_eq!(columns(&db, "backup_provider_config"), 8);
    assert_eq!(
        db.query_row("SELECT enabled FROM backup_sync_preference", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE name LIKE 'backup_legacy_v5_%'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        version(&Connection::open(&backups(root.path())[0]).unwrap()),
        5
    );
    assert!(db.is_autocommit());
}

#[test]
fn current_v1_upgrades_once_without_pausing_current_format_work() {
    let root = tempfile::tempdir().unwrap();
    let db = Connection::open(root.path().join("current.sqlite")).unwrap();
    repository::init(&db).unwrap();
    db.execute("INSERT INTO backup_sync_preference VALUES(1,1)", [])
        .unwrap();
    db.execute("UPDATE backup_schema SET version=1", [])
        .unwrap();
    let device = repository::device_id(&db).unwrap();
    repository::init(&db).unwrap();
    assert_eq!(version(&db), 8);
    assert_eq!(repository::device_id(&db).unwrap(), device);
    assert_eq!(
        db.query_row("SELECT enabled FROM backup_sync_preference", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(backups(root.path()).len(), 1);
    repository::init(&db).unwrap();
    assert_eq!(backups(root.path()).len(), 1);
}

#[test]
fn incompatible_protocol_layout_and_pending_remote_upgrade_are_not_relabelled() {
    for case in 0..7 {
        let db = Connection::open_in_memory().unwrap();
        seed(&db);
        let expected = match case {
            0 => {
                db.execute("UPDATE backup_provider_config SET protocol=1", [])
                    .unwrap();
                "BACKUP_MIGRATION_UNSUPPORTED_PROTOCOL"
            }
            1 => {
                db.execute("ALTER TABLE backup_spaces ADD COLUMN unrecognized TEXT", [])
                    .unwrap();
                "BACKUP_MIGRATION_INVALID_STRUCTURE"
            }
            2 => {
                db.execute("UPDATE backup_schema SET version=1", [])
                    .unwrap();
                "BACKUP_MIGRATION_INVALID_STRUCTURE"
            }
            3 => {
                db.execute("UPDATE backup_provider_config SET migration_json='{}'", [])
                    .unwrap();
                "BACKUP_MIGRATION_REMOTE_UPGRADE_PENDING"
            }
            4 => {
                db.execute(
                    "INSERT INTO backup_probe_cleanup VALUES('probe','{}','vault',NULL)",
                    [],
                )
                .unwrap();
                "BACKUP_MIGRATION_PROBE_CLEANUP_PENDING"
            }
            5 => {
                db.execute(
                    "CREATE INDEX custom_provider_index ON backup_provider_config(profile)",
                    [],
                )
                .unwrap();
                "BACKUP_MIGRATION_INVALID_STRUCTURE"
            }
            _ => {
                db.execute("CREATE TABLE custom_child(space_id TEXT REFERENCES BACKUP_PROVIDER_CONFIG(space_id) ON DELETE CASCADE)", []).unwrap();
                db.execute("INSERT INTO custom_child VALUES('space')", [])
                    .unwrap();
                "BACKUP_MIGRATION_INVALID_STRUCTURE"
            }
        };
        assert_eq!(repository::init(&db).unwrap_err().to_string(), expected);
        assert_eq!(columns(&db, "asset_remote_objects"), 9);
        assert_eq!(
            db.query_row("SELECT enabled FROM backup_spaces", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            1
        );
        if case == 6 {
            assert_eq!(
                db.query_row("SELECT COUNT(*) FROM custom_child", [], |row| row
                    .get::<_, i64>(0))
                    .unwrap(),
                1
            );
        }
    }
}

#[test]
fn fresh_schema_and_unknown_version_have_explicit_boundaries() {
    let db = Connection::open_in_memory().unwrap();
    repository::init(&db).unwrap();
    assert_eq!(version(&db), 8);
    let device = repository::device_id(&db).unwrap();
    db.execute("UPDATE backup_schema SET version=99", [])
        .unwrap();
    assert_eq!(
        repository::init(&db).unwrap_err().to_string(),
        "BACKUP_UNSUPPORTED_SCHEMA"
    );
    assert_eq!(version(&db), 99);
    assert_eq!(repository::device_id(&db).unwrap(), device);
}

#[test]
#[ignore = "explicit run-owned copy path required; SQLite only, never loads a session or uses network"]
fn migrate_authorized_database_copy() {
    let path = std::path::PathBuf::from(
        std::env::var_os("YINGQU_MIGRATION_COPY_PATH").expect("explicit copy path required"),
    )
    .canonicalize()
    .unwrap();
    let run = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join(".local/backup-schema-migration")
        .canonicalize()
        .unwrap();
    assert!(path.starts_with(run));
    assert_eq!(path.file_name().unwrap(), "validation-copy.sqlite");
    let db = Connection::open(path).unwrap();
    db.pragma_update(None, "foreign_keys", true).unwrap();
    repository::init(&db).unwrap();
    assert_eq!(version(&db), 8);
    assert_eq!(
        db.query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "ok"
    );
    assert_eq!(
        db.query_row("SELECT enabled FROM backup_sync_preference", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM backup_spaces WHERE enabled<>0",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    drop(db);
}
