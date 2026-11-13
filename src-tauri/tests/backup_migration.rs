use rusqlite::{params, types::Value, Connection};
use serde_json::json;
use std::{
    fs,
    path::{Path, PathBuf},
};
use yingqu_core::{backup::repository, desktop_core::Core};

const MASTER: &str = include_str!("../../tests/fixtures/master-schema.sql");
const DEVELOPMENT: &str = include_str!("../../tests/fixtures/backup-v9.sql");

fn version(db: &Connection) -> i64 {
    db.query_row("SELECT version FROM backup_schema WHERE id=1", [], |r| {
        r.get(0)
    })
    .unwrap()
}

fn rows(db: &Connection, sql: &str) -> Vec<Vec<Value>> {
    let mut query = db.prepare(sql).unwrap();
    let columns = query.column_count();
    query
        .query_map([], |r| (0..columns).map(|i| r.get(i)).collect())
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

fn schema(db: &Connection) -> Vec<Vec<Value>> {
    rows(db, "SELECT type,name,tbl_name,sql FROM sqlite_master WHERE name NOT LIKE 'sqlite_%' ORDER BY type,name")
}

fn backups(root: &Path) -> Vec<PathBuf> {
    fs::read_dir(root)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| {
            p.file_name()
                .unwrap()
                .to_string_lossy()
                .contains(".before-backup-v")
                && p.extension().is_some_and(|e| e == "sqlite")
        })
        .collect()
}

fn seed_master(db: &Connection) {
    db.execute_batch(MASTER).unwrap();
    db.execute_batch("PRAGMA wal_autocheckpoint=0;
        INSERT INTO app_settings VALUES(1,'saved-downloads',2,'custom/{link_id}');
        INSERT INTO platform_settings VALUES('youtube','saved-agent',NULL,'fixture-cookie',NULL,10485760);
        INSERT INTO tdl_settings VALUES(1,1,'legacy-tool','legacy-session');
        INSERT INTO parse_cache VALUES('cached','youtube','fixture','yt-dlp','video','{}',1,'old',X'0102',X'0304');
        INSERT INTO parse_cache_images VALUES('cached',0,X'010203');
        INSERT INTO parse_cache_aliases VALUES('https://unit.invalid/video','youtube','fixture');").unwrap();
    let task = json!({"id":"history","revision":3,"status":"failed","bytes":0,
        "error":"original-error","created_at":1,"canceling":false});
    db.execute(
        "INSERT INTO download_tasks VALUES('history',?1)",
        [task.to_string()],
    )
    .unwrap();
    let transcript = json!({"task_id":"history","operation_id":"transcription","sequence":4,
        "state":"completed","updated_at":2,"has_result":false,"source_changed":false});
    db.execute(
        "INSERT INTO video_transcripts VALUES('history',?1)",
        [transcript.to_string()],
    )
    .unwrap();
}

fn open(path: &Path) -> std::sync::Arc<Core> {
    Core::open(
        path,
        path.parent().unwrap().join("downloads"),
        "unused-engine".into(),
        vec![],
        |_| {},
    )
    .unwrap()
}

#[test]
fn master_upgrades_once_to_all_final_tables_indexes_and_constraints_without_losing_data() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("app.sqlite");
    let db = Connection::open(&path).unwrap();
    seed_master(&db);
    let original_schema = schema(&db);
    let preserved = [
        ("app_settings", "*"),
        (
            "platform_settings",
            "platform,user_agent,proxy,cookie,credential_ref,http_chunk_size",
        ),
        ("tdl_settings", "*"),
        (
            "parse_cache",
            "cache_key,platform,video_id,resolver,kind,metadata,saved_at,revision,author_avatar",
        ),
        ("parse_cache_images", "*"),
        ("parse_cache_aliases", "*"),
        ("video_transcripts", "*"),
    ];
    let before: Vec<_> = preserved
        .iter()
        .map(|(table, columns)| rows(&db, &format!("SELECT {columns} FROM {table}")))
        .collect();
    let original_task = rows(&db, "SELECT * FROM download_tasks");
    // Keep this connection open: the migration's snapshot must include committed WAL pages.
    let core = open(&path);
    assert_eq!(version(&db), 1);
    assert_eq!(core.list().len(), 1);
    assert_eq!(core.list()[0].error.as_deref(), Some("original-error"));
    for ((table, columns), expected) in preserved.iter().zip(&before) {
        assert_eq!(
            rows(&db, &format!("SELECT {columns} FROM {table}")),
            *expected,
            "lost master data: {table}"
        );
    }
    assert_eq!(
        rows(&db, "SELECT bypass_proxy FROM platform_settings"),
        vec![vec![Value::Integer(0)]]
    );
    assert_eq!(
        rows(&db, "SELECT account_auth, login_cookie FROM platform_settings"),
        vec![vec![Value::Text("manual".into()), Value::Text(String::new())]]
    );
    assert_eq!(
        rows(&db, "SELECT b.body FROM cover_refs r JOIN cover_blobs b USING(digest) WHERE r.owner='cache' AND r.owner_id='cached'"),
        vec![vec![Value::Blob(vec![1, 2])]]
    );
    let snapshots = backups(root.path());
    assert_eq!(snapshots.len(), 1);
    let snapshot = Connection::open(&snapshots[0]).unwrap();
    assert_eq!(schema(&snapshot), original_schema);
    assert_eq!(
        rows(&snapshot, "SELECT cover FROM parse_cache"),
        vec![vec![Value::Blob(vec![1, 2])]]
    );
    assert_eq!(
        rows(&snapshot, "SELECT * FROM download_tasks"),
        original_task
    );
    for ((table, columns), expected) in preserved.iter().zip(&before) {
        assert_eq!(
            rows(&snapshot, &format!("SELECT {columns} FROM {table}")),
            *expected
        );
    }
    // Compare every branch table/index/trigger with the frozen pre-squash final schema,
    // so accidentally omitting one of the intermediate changes cannot pass this test.
    let expected = Connection::open_in_memory().unwrap();
    expected.execute_batch(MASTER).unwrap();
    expected.execute_batch(DEVELOPMENT).unwrap();
    expected
        .execute_batch(
            "ALTER TABLE platform_settings ADD COLUMN bypass_proxy INTEGER NOT NULL DEFAULT 0 CHECK(bypass_proxy IN (0,1));
             ALTER TABLE platform_settings ADD COLUMN login_cookie TEXT NOT NULL DEFAULT '';
             ALTER TABLE platform_settings ADD COLUMN account_auth TEXT CHECK(account_auth IN ('login','manual'));",
        )
        .unwrap();
    let objects: Vec<(String,String,String)> = expected.prepare(
        "SELECT type,name,tbl_name FROM sqlite_master WHERE name NOT LIKE 'sqlite_%' ORDER BY type,name"
    ).unwrap().query_map([], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap().collect::<rusqlite::Result<_>>().unwrap();
    for (kind, name, table) in objects {
        if matches!(
            table.as_str(),
            "app_settings" | "parse_cache" | "asset_covers"
        ) {
            continue;
        } // The settings constraint and cover storage intentionally change.
        let found: bool = db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type=?1 AND name=?2)",
                params![kind, name],
                |r| r.get(0),
            )
            .unwrap();
        assert!(found, "missing {kind} {name}");
        if kind == "table" {
            for pragma in ["table_info", "foreign_key_list"] {
                let query = format!("PRAGMA {pragma}({name})");
                assert_eq!(
                    rows(&db, &query),
                    rows(&expected, &query),
                    "{pragma} changed for {name}"
                );
            }
        }
    }
    db.execute("UPDATE app_settings SET max_downloads=10", [])
        .unwrap();
    assert!(db
        .execute("UPDATE app_settings SET max_downloads=11", [])
        .is_err());
    assert!(rows(&db, "PRAGMA foreign_key_check").is_empty());
    let current_schema = schema(&db);
    let device = repository::device_id(&db).unwrap();
    drop(core);
    let reopened = open(&path);
    assert_eq!(schema(&db), current_schema);
    assert_eq!(repository::device_id(&db).unwrap(), device);
    assert_eq!(backups(root.path()).len(), 1);
    assert_eq!(reopened.list().len(), 1);
}

#[test]
fn migration_failure_rolls_back_settings_all_new_objects_and_version_together() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("app.sqlite");
    let db = Connection::open(&path).unwrap();
    seed_master(&db);
    // SQLite rejects creating a table with this existing index name, after the
    // settings rebuild and initial backup DDL have run inside the transaction.
    db.execute(
        "CREATE INDEX backup_operation_captures ON download_tasks(id)",
        [],
    )
    .unwrap();
    let before = schema(&db);
    let settings = rows(&db, "SELECT * FROM app_settings");
    assert!(Core::open(&path, root.path().into(), "unused".into(), vec![], |_| {}).is_err());
    assert_eq!(schema(&db), before);
    assert_eq!(rows(&db, "SELECT * FROM app_settings"), settings);
    assert!(db
        .execute("UPDATE app_settings SET max_downloads=5", [])
        .is_err());
    assert_eq!(backups(root.path()).len(), 1);
    assert_eq!(
        schema(&Connection::open(&backups(root.path())[0]).unwrap()),
        before
    );
    db.execute("DROP INDEX backup_operation_captures", [])
        .unwrap();
    let core = open(&path);
    assert_eq!(version(&db), 1);
    assert_eq!(core.list().len(), 1);
    db.execute("UPDATE app_settings SET max_downloads=10", [])
        .unwrap();
}

#[test]
fn final_development_database_is_adopted_without_replaying_old_migrations() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("app.sqlite");
    let db = Connection::open(&path).unwrap();
    seed_master(&db);
    db.execute_batch(DEVELOPMENT).unwrap();
    let device = uuid::Uuid::new_v4().to_string();
    db.execute("INSERT INTO backup_device VALUES(1,?1)", [&device])
        .unwrap();
    db.execute(
        "INSERT INTO telegram_account_bindings VALUES('backup',?1)",
        [&device],
    )
    .unwrap();
    db.execute_batch("INSERT INTO backup_spaces VALUES('space','library','https://unit.invalid','bucket','prefix',NULL,1);
        INSERT INTO backup_provider_config(space_id,provider_type,profile) VALUES('space','s3','r2');
        INSERT INTO backup_sync_preference VALUES(1,1);
        INSERT INTO media_assets VALUES('asset',NULL,'{}','video.mp4','hash',4,'active');
        INSERT INTO asset_covers VALUES('asset',X'0102');
        CREATE TABLE library_assets(id TEXT PRIMARY KEY);
        CREATE TABLE asset_resources(asset_id TEXT NOT NULL REFERENCES library_assets(id) ON DELETE CASCADE,kind TEXT NOT NULL CHECK(kind='cover'),body BLOB NOT NULL,PRIMARY KEY(asset_id,kind));
        INSERT INTO library_assets VALUES('legacy');
        INSERT INTO asset_resources VALUES('legacy','cover',X'0102');
        INSERT INTO asset_transcripts VALUES('asset','hash','saved transcript','model',NULL,'pipeline',1,NULL);
        INSERT INTO asset_deletion_hidden_history VALUES('space','asset');
        INSERT INTO backup_operations(operation_id,space_id,asset_id,kind,state,phase,payload_json) VALUES('upload','space','asset','upload','blocked','sending','{\"nonce\":123}');
        INSERT INTO backup_operation_captures VALUES('upload','{\"cover\":[1,2],\"transcript\":\"saved body\"}');
        INSERT INTO telegram_media_sends VALUES('upload','video','{\"random_id\":123,\"state\":\"sending\"}');").unwrap();
    let tables = [
        "backup_spaces",
        "backup_sync_preference",
        "media_assets",
        "asset_covers",
        "asset_transcripts",
        "asset_deletion_hidden_history",
        "backup_operations",
        "asset_resources",
        "telegram_media_sends",
        "telegram_account_bindings",
    ];
    let before: Vec<_> = tables
        .iter()
        .map(|t| rows(&db, &format!("SELECT * FROM {t}")))
        .collect();
    let core = open(&path);
    assert_eq!(version(&db), 1);
    assert_eq!(repository::device_id(&db).unwrap(), device);
    for (table, expected) in tables.iter().zip(before) {
        assert_eq!(
            rows(&db, &format!("SELECT * FROM {table}")),
            expected,
            "development data changed: {table}"
        );
    }
    assert_eq!(
        rows(
            &db,
            "SELECT count(*),count(DISTINCT digest) FROM cover_refs"
        ),
        vec![vec![Value::Integer(4), Value::Integer(1)]]
    );
    assert_eq!(
        rows(&db, "SELECT capture_json FROM backup_operation_captures"),
        vec![vec![Value::Text(
            json!({"transcript":"saved body"}).to_string()
        )]]
    );
    assert_eq!(
        version(&Connection::open(&backups(root.path())[0]).unwrap()),
        9
    );
    drop(core);
    let _reopened = open(&path);
    assert_eq!(backups(root.path()).len(), 1);
}

#[test]
fn fresh_database_matches_upgraded_schema_except_preserved_obsolete_master_table() {
    let root = tempfile::tempdir().unwrap();
    let upgraded_path = root.path().join("upgraded.sqlite");
    let upgraded = Connection::open(&upgraded_path).unwrap();
    upgraded.execute_batch(MASTER).unwrap();
    let _core = open(&upgraded_path);
    let fresh_path = root.path().join("fresh.sqlite");
    let _fresh = open(&fresh_path);
    let fresh = Connection::open(&fresh_path).unwrap();
    assert_eq!(version(&fresh), 1);
    let objects = |db: &Connection| {
        rows(db,"SELECT type,name FROM sqlite_master WHERE name NOT LIKE 'sqlite_%' AND tbl_name<>'tdl_settings' ORDER BY type,name")
    };
    assert_eq!(objects(&fresh), objects(&upgraded));
    for table in [
        "app_settings",
        "platform_settings",
        "backup_operations",
        "backup_operation_captures",
        "telegram_account_bindings",
    ] {
        assert_eq!(
            rows(&fresh, &format!("PRAGMA table_info({table})")),
            rows(&upgraded, &format!("PRAGMA table_info({table})"))
        );
    }
    assert_eq!(
        backups(root.path()).len(),
        1,
        "fresh database should not need a snapshot"
    );
}

#[test]
fn standalone_cover_upgrade_snapshots_before_rewriting_and_reopens_once() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("app.sqlite");
    drop(open(&path));
    let db = Connection::open(&path).unwrap();
    db.execute_batch(
        "ALTER TABLE parse_cache ADD COLUMN cover BLOB;
         INSERT INTO parse_cache(cache_key,platform,video_id,resolver,kind,metadata,saved_at,revision,cover)
         VALUES('preview','douyin','123','yt-dlp','video','{}',1,'legacy',X'0102');",
    )
    .unwrap();
    let snapshots = || {
        fs::read_dir(root.path())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| {
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("app.sqlite.before-cover-store-v1-")
                    && path.extension().is_some_and(|ext| ext == "sqlite")
            })
            .collect::<Vec<_>>()
    };
    let core = open(&path);
    let saved = snapshots();
    assert_eq!(saved.len(), 1);
    let previous = Connection::open(&saved[0]).unwrap();
    assert_eq!(
        rows(&previous, "SELECT cover FROM parse_cache"),
        vec![vec![Value::Blob(vec![1, 2])]]
    );
    assert_eq!(
        rows(
            &db,
            "SELECT count(*) FROM pragma_table_info('parse_cache') WHERE name='cover'"
        ),
        vec![vec![Value::Integer(0)]]
    );
    assert_eq!(
        rows(&db, "SELECT body FROM cover_blobs"),
        vec![vec![Value::Blob(vec![1, 2])]]
    );
    drop(core);
    let _reopened = open(&path);
    assert_eq!(snapshots(), saved);
}

#[test]
fn startup_rejects_dangling_cover_and_other_foreign_keys() {
    for insert in [
        "INSERT INTO cover_refs VALUES('cache','orphan',printf('%064d',0))",
        "INSERT INTO asset_remote_objects(space_id,asset_id,remote_state)
         VALUES('missing-space','missing-asset','pending')",
    ] {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("app.sqlite");
        drop(open(&path));
        let db = Connection::open(&path).unwrap();
        db.pragma_update(None, "foreign_keys", false).unwrap();
        db.execute(insert, []).unwrap();
        assert!(!rows(&db, "PRAGMA foreign_key_check").is_empty());
        drop(db);

        let error = Core::open(&path, root.path().into(), "unused".into(), vec![], |_| {})
            .err()
            .unwrap();
        assert_eq!(error.to_string(), "BACKUP_SCHEMA_FOREIGN_KEY_ERROR");
    }
}

#[test]
fn unsupported_intermediate_and_unknown_schemas_are_rejected_before_startup_writes() {
    for old in [0, 2, 3, 4, 5, 6, 7, 8, 99] {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("app.sqlite");
        let db = Connection::open(&path).unwrap();
        seed_master(&db);
        db.execute_batch(DEVELOPMENT).unwrap();
        db.execute("UPDATE backup_schema SET version=?1", [old])
            .unwrap();
        let before = schema(&db);
        let error = Core::open(&path, root.path().into(), "unused".into(), vec![], |_| {})
            .err()
            .unwrap();
        assert_eq!(error.to_string(), "BACKUP_UNSUPPORTED_SCHEMA");
        assert_eq!(version(&db), old);
        assert_eq!(schema(&db), before);
        assert!(backups(root.path()).is_empty());
    }
    // Early development v1 has the same number but lacks the final capture schema.
    let db = Connection::open_in_memory().unwrap();
    repository::init(&db).unwrap();
    db.execute("DROP TABLE backup_operation_captures", [])
        .unwrap();
    let before = schema(&db);
    assert_eq!(
        repository::init(&db).unwrap_err().to_string(),
        "BACKUP_MIGRATION_INVALID_STRUCTURE"
    );
    assert_eq!(schema(&db), before);
}

#[test]
fn transcript_restart_recovery_still_runs_after_the_schema_transaction() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("app.sqlite");
    let db = Connection::open(&path).unwrap();
    seed_master(&db);
    db.execute(
        "UPDATE video_transcripts SET snapshot=json_set(snapshot,'$.state','running')",
        [],
    )
    .unwrap();
    let _core = open(&path);
    assert_eq!(
        db.query_row(
            "SELECT json_extract(snapshot,'$.state') FROM video_transcripts",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "interrupted"
    );
    assert_eq!(version(&db), 1);
}
