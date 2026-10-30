use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::{fs, path::Path};
use yingqu_core::{
    backup::repository,
    desktop_core::{Core, Task},
};

fn old_task(id: &str, path: &Path, status: &str, kind: &str) -> Value {
    json!({"id":id,"revision":1,"status":status,"video":{"platform":"youtube","video_id":"12345678901","canonical_url":format!("https://www.youtube.com/watch?v={id}&token=secret"),"title":"示例","kind":kind,"resolver":"yt-dlp","cover_url":"https://example.com/?secret=1"},"bytes":16,"total_bytes":16,"speed_bps":null,"error":null,"path":path,"created_at":1,"completed_at":2,"canceling":false,"download_requested":true})
}

fn open(db: &Path, root: &Path) -> std::sync::Arc<Core> {
    Core::open(db, root.to_owned(), "unused-engine".into(), vec![], |_| {}).unwrap()
}

#[tokio::test]
async fn legacy_migration_preserves_video_text_and_asset_after_history_removal() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("video.mp4");
    fs::write(&path, b"\0\0\0\x10ftypisom1234").unwrap();
    let db_path = root.path().join("app.sqlite");
    let db = Connection::open(&db_path).unwrap();
    yingqu_core::parse_cache::init(&db).unwrap();
    db.execute_batch("CREATE TABLE download_tasks(id TEXT PRIMARY KEY,snapshot TEXT NOT NULL); CREATE TABLE video_transcripts(task_id TEXT PRIMARY KEY,snapshot TEXT NOT NULL);").unwrap();
    let id = uuid::Uuid::new_v4().to_string();
    let raw = old_task(&id, &path, "completed", "video");
    let legacy: Task = serde_json::from_value(raw.clone()).unwrap();
    assert!(legacy.asset_id.is_none());
    db.execute(
        "INSERT INTO download_tasks VALUES(?1,?2)",
        params![id, raw.to_string()],
    )
    .unwrap();
    let (hash, source) = repository::fingerprint(&path).unwrap();
    let transcript = json!({"task_id":id,"operation_id":"old","sequence":1,"state":"completed","phase":null,"error_code":null,"updated_at":2,"has_result":true,"source_changed":false,"result":{"text":"保留完整文案","source":source,"model_revision":"old-model","pipeline_revision":"old-pipeline","completed_at":2}});
    db.execute(
        "INSERT INTO video_transcripts VALUES(?1,?2)",
        params![id, transcript.to_string()],
    )
    .unwrap();
    drop(db);
    let core = open(&db_path, root.path());
    core.verify_backup_asset(&id).await.unwrap();
    let assets = core.backup_assets().unwrap();
    assert_eq!(assets.len(), 1);
    assert_eq!(assets[0].asset_id, id);
    assert_eq!(assets[0].content_sha256.as_deref(), Some(hash.as_str()));
    assert!(!assets[0].metadata.to_string().contains("secret"));
    assert_eq!(core.playback_path(&id).unwrap(), path);
    assert_eq!(core.transcript_text(&id).unwrap().text, "保留完整文案");
    assert_eq!(
        core.asset_transcript(&id).unwrap().unwrap().content_sha256,
        Some(hash)
    );
    drop(core);
    let db = Connection::open(&db_path).unwrap();
    yingqu_core::parse_cache::init(&db).unwrap();
    let device = repository::device_id(&db).unwrap();
    drop(db);
    let core = open(&db_path, root.path());
    assert_eq!(core.backup_assets().unwrap().len(), 1);
    core.remove_history(&id).unwrap();
    assert!(core.list().is_empty());
    let assets = core.backup_assets().unwrap();
    assert_eq!(assets.len(), 1);
    assert!(assets[0].task_id.is_none());
    let check=Connection::open(&db_path).unwrap();
    assert_eq!(check.query_row("SELECT COUNT(*) FROM asset_hidden_history WHERE asset_id=?",[&id],|r|r.get::<_,i64>(0)).unwrap(),1);
    drop(check);
    assert_eq!(
        core.asset_transcript(&id).unwrap().unwrap().text,
        "保留完整文案"
    );
    assert!(path.is_file());
    drop(core);
    let db = Connection::open(&db_path).unwrap();
    yingqu_core::parse_cache::init(&db).unwrap();
    assert_eq!(repository::device_id(&db).unwrap(), device);
    assert!(db
        .prepare("PRAGMA foreign_key_check")
        .unwrap()
        .query([])
        .unwrap()
        .next()
        .unwrap()
        .is_none());
    let other = Connection::open_in_memory().unwrap();
    repository::init(&other).unwrap();
    assert_ne!(repository::device_id(&other).unwrap(), device);
}

#[test]
fn albums_and_unfinished_downloads_do_not_create_assets() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("video.mp4");
    fs::write(&path, b"\0\0\0\x10ftypisom1234").unwrap();
    let db_path = root.path().join("app.sqlite");
    let db = Connection::open(&db_path).unwrap();
    yingqu_core::parse_cache::init(&db).unwrap();
    db.execute_batch("CREATE TABLE download_tasks(id TEXT PRIMARY KEY,snapshot TEXT NOT NULL);")
        .unwrap();
    for (id, status, kind) in [
        ("album", "completed", "image_album"),
        ("pending", "ready", "video"),
        ("failed", "failed", "video"),
    ] {
        db.execute(
            "INSERT INTO download_tasks VALUES(?1,?2)",
            params![id, old_task(id, &path, status, kind).to_string()],
        )
        .unwrap();
    }
    drop(db);
    let core = open(&db_path, root.path());
    assert!(core.backup_assets().unwrap().is_empty());
    assert_eq!(core.list().len(), 3);
}

#[test]
fn schema_rejects_dangling_objects_and_preserves_uncertain_operation_payload() {
    let db = Connection::open_in_memory().unwrap();
    db.pragma_update(None, "foreign_keys", true).unwrap();
    repository::init(&db).unwrap();
    assert!(db.execute("INSERT INTO asset_local_files(asset_id,bytes,local_state) VALUES('unknown',1,'present')",[]).is_err());
    db.execute("INSERT INTO backup_spaces VALUES('space','library','https://test.r2.cloudflarestorage.com','bucket','yingqu/v1/','primary','device',1,NULL,1)",[]).unwrap();
    db.execute("INSERT INTO backup_operations(operation_id,space_id,kind,state,phase,payload_json) VALUES('operation','space','probe','running','put','{\"key\":\"probe-1\"}')",[]).unwrap();
    repository::init(&db).unwrap();
    let (state, phase, payload): (String, String, String) = db
        .query_row(
            "SELECT state,phase,payload_json FROM backup_operations",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(state, "pending");
    assert_eq!(phase, "put");
    assert!(payload.contains("probe-1"));
}

#[test]
fn identical_asset_ids_have_independent_device_paths_and_spaces_cannot_be_retargeted() {
    use yingqu_core::backup::model::{Role, Space};
    let id = uuid::Uuid::new_v4().to_string();
    let mut devices = Vec::new();
    for _ in 0..2 {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("video.mp4");
        fs::write(&path, b"\0\0\0\x10ftypisom1234").unwrap();
        let db_path = root.path().join("app.sqlite");
        let db = Connection::open(&db_path).unwrap();
        yingqu_core::parse_cache::init(&db).unwrap();
        db.execute_batch(
            "CREATE TABLE download_tasks(id TEXT PRIMARY KEY,snapshot TEXT NOT NULL);",
        )
        .unwrap();
        db.execute(
            "INSERT INTO download_tasks VALUES(?1,?2)",
            params![id, old_task(&id, &path, "completed", "video").to_string()],
        )
        .unwrap();
        drop(db);
        let core = open(&db_path, root.path());
        let asset = core.backup_assets().unwrap().remove(0);
        assert_eq!(asset.asset_id, id);
        assert_eq!(asset.path.as_ref(), Some(&path));
        devices.push((root, core, asset));
    }
    assert_ne!(devices[0].2.path, devices[1].2.path);
    let db = Connection::open_in_memory().unwrap();
    repository::init(&db).unwrap();
    let mut space = Space {
        space_id: uuid::Uuid::new_v4().to_string(),
        library_id: uuid::Uuid::new_v4().to_string(),
        endpoint: "https://test.r2.cloudflarestorage.com".into(),
        bucket: "test".into(),
        prefix: "yingqu/v1/".into(),
        role: Role::Reader,
        writer_device_id: uuid::Uuid::new_v4().to_string(),
        writer_epoch: 1,
        credential_ref: None,
        enabled: true,
    };
    repository::save_space(&db, &space).unwrap();
    assert_eq!(
        db.query_row("SELECT role FROM backup_spaces", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "reader"
    );
    space.bucket = "another".into();
    assert_eq!(
        repository::save_space(&db, &space).unwrap_err().to_string(),
        "BACKUP_SPACE_CHANGED"
    );
    assert_eq!(
        db.query_row("SELECT bucket FROM backup_spaces", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "test"
    );
}

#[tokio::test]
async fn released_task_never_points_to_replacement_at_old_path() {
    let root=tempfile::tempdir().unwrap();let path=root.path().join("video.mp4");
    fs::write(&path,b"\0\0\0\x10ftypisom1234").unwrap();
    let db_path=root.path().join("app.sqlite");let db=Connection::open(&db_path).unwrap();
    yingqu_core::parse_cache::init(&db).unwrap();
    db.execute_batch("CREATE TABLE download_tasks(id TEXT PRIMARY KEY,snapshot TEXT NOT NULL);").unwrap();
    let id=uuid::Uuid::new_v4().to_string();
    db.execute("INSERT INTO download_tasks VALUES(?1,?2)",params![id,old_task(&id,&path,"completed","video").to_string()]).unwrap();drop(db);
    let core=open(&db_path,root.path());
    yingqu_core::backup::lifecycle::release(&core,&id).await.unwrap();
    fs::write(&path,b"different user file").unwrap();
    assert!(core.list()[0].file_missing);assert!(core.list()[0].path.is_none());assert!(core.playback_path(&id).is_err());
    drop(core);
    let core=open(&db_path,root.path());assert!(core.list()[0].file_missing);assert!(core.list()[0].path.is_none());assert!(core.playback_path(&id).is_err());
    assert_eq!(fs::read(&path).unwrap(),b"different user file");
}

#[tokio::test]
async fn release_before_first_backup_rejects_replaced_registered_source() {
    let root=tempfile::tempdir().unwrap();let path=root.path().join("video.mp4");
    fs::write(&path,b"\0\0\0\x10ftypisom1234").unwrap();
    let db_path=root.path().join("app.sqlite");let db=Connection::open(&db_path).unwrap();
    yingqu_core::parse_cache::init(&db).unwrap();db.execute_batch("CREATE TABLE download_tasks(id TEXT PRIMARY KEY,snapshot TEXT NOT NULL);").unwrap();
    let id=uuid::Uuid::new_v4().to_string();db.execute("INSERT INTO download_tasks VALUES(?1,?2)",params![id,old_task(&id,&path,"completed","video").to_string()]).unwrap();drop(db);
    let core=open(&db_path,root.path());
    fs::write(&path,b"\0\0\0\x10ftypisomreplacement file").unwrap();
    assert_eq!(yingqu_core::backup::lifecycle::release(&core,&id).await.unwrap_err().to_string(),"BACKUP_LOCAL_CHANGED");
    assert!(path.exists());
}
