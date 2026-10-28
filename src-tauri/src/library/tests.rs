use super::*;
use crate::desktop_core::{
    transcription::{Snapshot, Source, TranscriptResult},
    Status, Task,
};
use rusqlite::params;
use serde_json::json;
use std::path::Path;

fn database() -> Connection {
    let mut db = Connection::open_in_memory().unwrap();
    initialize(&mut db, Path::new(":memory:")).unwrap();
    db.execute_batch("CREATE TABLE download_tasks(id TEXT PRIMARY KEY,snapshot TEXT NOT NULL); CREATE TABLE video_transcripts(task_id TEXT PRIMARY KEY,snapshot TEXT NOT NULL);").unwrap();
    db
}

/// Synthetic legacy records, shared by migration and lifecycle tests. No network,
/// model, credentials or real user download directories are involved.
fn task(db: &Connection, id: &str, video_id: &str, path: &Path) -> Task {
    let task: Task = serde_json::from_value(json!({
        "id":id,"revision":1,"status":"completed",
        "video":{"platform":"douyin","video_id":video_id,"kind":"video","resolver":"yt-dlp","title":"同名视频","author":"测试作者","canonical_url":format!("https://www.douyin.com/video/{video_id}")},
        "bytes":4,"total_bytes":4,"speed_bps":null,"error":null,"path":path,
        "created_at":1,"completed_at":2,"canceling":false
    })).unwrap();
    db.execute(
        "INSERT INTO download_tasks VALUES(?1,?2)",
        params![id, serde_json::to_string(&task).unwrap()],
    )
    .unwrap();
    task
}

fn recognized(path: &Path, text: &str, time: u64) -> TranscriptResult {
    TranscriptResult { timing: None,
        text: text.into(),
        source: Source {
            path: path.into(),
            size: 4,
            mtime_ns: "1".into(),
        },
        model_revision: "legacy-revision".into(),
        model_name: None,
        pipeline_revision: "legacy-pipeline".into(),
        backend: None,
        completed_at: time,
    }
}

fn legacy_transcript(db: &Connection, task: &Task, text: &str) {
    let snapshot: Snapshot = serde_json::from_value(json!({
        "task_id":task.id,"operation_id":"old-operation","sequence":1,"state":"completed","phase":null,
        "error_code":null,"updated_at":3,"result":recognized(task.path.as_ref().unwrap(),text,3),"has_result":true,"source_changed":false
    })).unwrap();
    db.execute(
        "INSERT INTO video_transcripts VALUES(?1,?2)",
        params![task.id, serde_json::to_string(&snapshot).unwrap()],
    )
    .unwrap();
}

#[test]
fn schema_upgrade_backs_up_wal_data_and_rejects_future_versions() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("application.sqlite3");
    let mut db = Connection::open(&path).unwrap();
    db.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE settings(value TEXT); INSERT INTO settings VALUES('synthetic-setting');").unwrap();
    let backup = initialize(&mut db, &path).unwrap().unwrap();
    let old = Connection::open(backup).unwrap();
    assert_eq!(
        old.query_row("SELECT value FROM settings", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "synthetic-setting"
    );
    assert!(!old
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='library_assets')",
            [],
            |r| r.get::<_, bool>(0)
        )
        .unwrap());
    assert!(initialize(&mut db, &path).unwrap().is_none());
    db.execute("UPDATE library_schema SET version=999", [])
        .unwrap();
    assert_eq!(
        initialize(&mut db, &path).unwrap_err().to_string(),
        "LIBRARY_SCHEMA_UNSUPPORTED"
    );
}

#[test]
fn fresh_schema_enforces_foreign_keys_and_failed_upgrade_is_atomic() {
    let db = database();
    assert_eq!(
        db.query_row("PRAGMA foreign_keys", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert!(db
        .execute(
            "INSERT INTO asset_categories VALUES('missing','missing')",
            []
        )
        .is_err());
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("broken.sqlite3");
    let mut broken = Connection::open(&path).unwrap();
    broken
        .execute_batch("CREATE TABLE library_tags(id TEXT);")
        .unwrap();
    assert!(initialize(&mut broken, &path).is_err());
    assert!(!broken
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='library_assets')",
            [],
            |r| r.get::<_, bool>(0)
        )
        .unwrap());
    assert_eq!(
        broken
            .query_row("SELECT count(*) FROM library_tags", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn migration_merges_identity_preserves_versions_and_skips_non_videos() {
    let mut db = database();
    let root = tempfile::tempdir().unwrap();
    let first = task(&db, "first", "100", &root.path().join("first.mp4"));
    let second = task(&db, "second", "100", &root.path().join("second.mp4"));
    let different = task(&db, "different", "200", &root.path().join("third.mp4"));
    let mut album = task(&db, "album", "300", &root.path().join("album"));
    album.video.as_mut().unwrap()["kind"] = "image_album".into();
    let mut failed = task(&db, "failed", "400", &root.path().join("missing.mp4"));
    failed.status = Status::Failed;
    let mut preview = task(&db, "preview", "500", &root.path().join("preview.mp4"));
    preview.status = Status::Ready;
    legacy_transcript(&db, &first, "旧版识别文案");
    let tasks = [first, second, different, album, failed, preview];
    backfill(&mut db, &tasks).unwrap();
    backfill(&mut db, &tasks).unwrap();
    let assets = list_assets(&db, false, 0, 100).unwrap();
    assert_eq!(assets.len(), 2);
    let id = asset_for_task(&db, "first").unwrap().unwrap();
    assert_eq!(Some(id.clone()), asset_for_task(&db, "second").unwrap());
    let files = asset_files(&db, &id).unwrap();
    assert_eq!(files.len(), 2);
    assert!(files.iter().all(|file| file.missing));
    let docs = documents(&db, &id).unwrap();
    assert_eq!(docs.len(), 1);
    assert_eq!(docs[0].text, "旧版识别文案");
    let original = docs[0].recognition.as_ref().unwrap();
    assert_eq!(original.model_revision, "legacy-revision");
    assert!(original.model_name.is_none());
    assert!(asset_for_task(&db, "album").unwrap().is_none());
    assert!(asset_for_task(&db, "failed").unwrap().is_none());
    assert!(asset_for_task(&db, "preview").unwrap().is_none());
}

#[test]
fn migration_error_rolls_back_all_assets_and_can_be_retried() {
    let mut db = database();
    let first = task(&db, "first", "100", Path::new("missing.mp4"));
    let broken = task(&db, "broken", "200", Path::new("missing2.mp4"));
    db.execute(
        "INSERT INTO video_transcripts VALUES('broken','invalid-json')",
        [],
    )
    .unwrap();
    assert!(backfill(&mut db, &[first.clone(), broken]).is_err());
    assert!(list_assets(&db, false, 0, 100).unwrap().is_empty());
    assert_eq!(
        db.query_row("SELECT count(*) FROM download_tasks", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        2
    );
    db.execute("DELETE FROM video_transcripts WHERE task_id='broken'", [])
        .unwrap();
    backfill(&mut db, &[first]).unwrap();
    assert_eq!(list_assets(&db, false, 0, 100).unwrap().len(), 1);
}

#[test]
fn root_names_cycles_multi_membership_and_atomic_batches() {
    let mut db = database();
    let task = task(&db, "first", "100", Path::new("missing.mp4"));
    backfill(&mut db, &[task]).unwrap();
    let asset = asset_for_task(&db, "first").unwrap().unwrap();
    let parent = create_category(&db, None, " 职场 ").unwrap();
    assert_eq!(
        create_category(&db, None, "职场").unwrap_err().to_string(),
        "CATEGORY_EXISTS"
    );
    let child = create_category(&db, Some(&parent), "个人成长").unwrap();
    let second = create_category(&db, None, "个人成长").unwrap();
    assert_eq!(
        update_category(&db, &parent, Some(&child), "职场", 0)
            .unwrap_err()
            .to_string(),
        "CATEGORY_CYCLE"
    );
    assert_eq!(
        delete_category(&db, &parent).unwrap_err().to_string(),
        "CATEGORY_HAS_CHILDREN"
    );
    assert!(
        set_category_membership(&db, &[asset.clone(), "unknown".into()], &child, true).is_err()
    );
    assert!(category_assets(&db, &parent).unwrap().is_empty());
    for category in [&child, &second] {
        set_category_membership(&db, std::slice::from_ref(&asset), category, true).unwrap();
        set_category_membership(&db, std::slice::from_ref(&asset), category, true).unwrap();
    }
    assert_eq!(
        category_assets(&db, &parent).unwrap().as_slice(),
        std::slice::from_ref(&asset)
    );
    update_category(&db, &child, None, "成长", 2).unwrap();
    assert!(category_assets(&db, &parent).unwrap().is_empty());
    let tag = create_tag(&db, " 反问开头 ").unwrap();
    assert_eq!(create_tag(&db, "反问开头").unwrap(), tag);
    set_tag_membership(&db, std::slice::from_ref(&asset), &tag, true).unwrap();
    delete_tag(&db, &tag).unwrap();
    delete_category(&db, &child).unwrap();
    assert_eq!(list_assets(&db, false, 0, 100).unwrap().len(), 1);
    archive_asset(&db, &asset, true).unwrap();
    assert!(list_assets(&db, false, 0, 100).unwrap().is_empty());
    assert_eq!(list_assets(&db, true, 0, 100).unwrap().len(), 1);
    archive_asset(&db, &asset, false).unwrap();
    assert_eq!(list_assets(&db, false, 0, 100).unwrap().len(), 1);
}

#[test]
fn immutable_document_editions_and_ai_notes_keep_old_citations() {
    let mut db = database();
    let task = task(&db, "first", "100", Path::new("missing.mp4"));
    backfill(&mut db, &[task]).unwrap();
    let asset = asset_for_task(&db, "first").unwrap().unwrap();
    let recognized = recognized(Path::new("missing.mp4"), "识别原文", 1);
    let raw_id = save_recognized(&db, &asset, None, &recognized).unwrap();
    assert_eq!(
        save_recognized(&db, &asset, None, &recognized).unwrap(),
        raw_id
    );
    let first = save_edited(&db, &asset, "人工整理一", 0, Some(&raw_id)).unwrap();
    assert_eq!(
        save_edited(&db, &asset, "过期修改", 0, Some(&raw_id))
            .unwrap_err()
            .to_string(),
        "DOCUMENT_CONFLICT"
    );
    save_edited(&db, &asset, "人工整理二", 1, Some(&raw_id)).unwrap();
    let citations = [Citation {
        asset_id: asset.clone(),
        document_id: first.clone(),
        quote: "整理一".into(),
    }];
    let note = create_note(
        &db,
        &asset,
        NewNote {
            kind: "summary",
            body: "AI 分析",
            origin: NoteOrigin::Ai,
            model_name: None,
            citations: &citations,
        },
    )
    .unwrap();
    edit_note(&db, &note, "人工补充分析", 1).unwrap();
    assert_eq!(
        edit_note(&db, &note, "过期编辑", 1)
            .unwrap_err()
            .to_string(),
        "NOTE_CONFLICT"
    );
    let notes = notes(&db, &asset).unwrap();
    assert_eq!(notes[0].revision, 2);
    assert!(notes[0].model_name.is_none());
    assert_eq!(notes[0].citations[0].document_id, first);
    let docs = documents(&db, &asset).unwrap();
    assert_eq!(docs.len(), 3);
    assert_eq!(
        docs.iter().find(|doc| doc.id == first).unwrap().text,
        "人工整理一"
    );
    let invalid = [Citation {
        asset_id: asset.clone(),
        document_id: raw_id,
        quote: "并不存在".into(),
    }];
    assert_eq!(
        create_note(
            &db,
            &asset,
            NewNote {
                kind: "summary",
                body: "内容",
                origin: NoteOrigin::Ai,
                model_name: None,
                citations: &invalid
            }
        )
        .unwrap_err()
        .to_string(),
        "INVALID_CITATION"
    );
}

#[test]
fn task_removal_keeps_documents_notes_categories_and_legacy_cover() {
    let mut db = database();
    let root = tempfile::tempdir().unwrap();
    let cover = root.path().join("legacy.cover.jpg");
    let jpeg = b"\xff\xd8\xff\xe0fake-jpeg";
    std::fs::write(&cover, jpeg).unwrap();
    let mut task = task(&db, "first", "100", &root.path().join("missing.mp4"));
    task.cover_path = Some(cover);
    legacy_transcript(&db, &task, "不会被历史清理删除");
    backfill(&mut db, &[task]).unwrap();
    let asset = asset_for_task(&db, "first").unwrap().unwrap();
    let category = create_category(&db, None, "职场").unwrap();
    set_category_membership(&db, std::slice::from_ref(&asset), &category, true).unwrap();
    create_note(
        &db,
        &asset,
        NewNote {
            kind: "note",
            body: "我的心得",
            origin: NoteOrigin::Human,
            model_name: None,
            citations: &[],
        },
    )
    .unwrap();
    db.execute("DELETE FROM video_transcripts", []).unwrap();
    db.execute("DELETE FROM download_tasks", []).unwrap();
    assert!(asset_for_task(&db, "first").unwrap().is_none());
    assert_eq!(
        documents(&db, &asset).unwrap()[0].text,
        "不会被历史清理删除"
    );
    assert_eq!(notes(&db, &asset).unwrap()[0].body, "我的心得");
    assert_eq!(
        category_assets(&db, &category).unwrap().as_slice(),
        std::slice::from_ref(&asset)
    );
    assert_eq!(asset_cover(&db, &asset).unwrap(), jpeg);
}

#[test]
fn changed_and_missing_files_are_observed_without_rewriting_original_documents() {
    let mut db = database();
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("video.mp4");
    std::fs::write(&path, b"1234").unwrap();
    let task = task(&db, "first", "100", &path);
    legacy_transcript(&db, &task, "原文");
    backfill(&mut db, &[task]).unwrap();
    let asset = asset_for_task(&db, "first").unwrap().unwrap();
    let files = asset_files(&db, &asset).unwrap();
    assert!(!files[0].missing && !files[0].changed);
    std::fs::write(&path, b"a different video").unwrap();
    assert!(asset_files(&db, &asset).unwrap()[0].changed);
    let moved = root.path().join("moved.mp4");
    std::fs::rename(&path, &moved).unwrap();
    assert!(asset_files(&db, &asset).unwrap()[0].missing);
    relink_file(&db, &files[0].id, &moved).unwrap();
    let files = asset_files(&db, &asset).unwrap();
    assert!(!files[0].missing && !files[0].changed);
    assert_eq!(
        documents(&db, &asset).unwrap()[0]
            .recognition
            .as_ref()
            .unwrap()
            .source
            .path,
        path
    );
}

#[test]
fn version_one_upgrade_migrates_legacy_body_then_removes_duplicate_copy() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("legacy.sqlite");
    let mut db = Connection::open(&path).unwrap();
    db.execute_batch(include_str!("schema.sql")).unwrap();
    db.execute_batch("CREATE TABLE download_tasks(id TEXT PRIMARY KEY,snapshot TEXT NOT NULL); CREATE TABLE video_transcripts(task_id TEXT PRIMARY KEY,snapshot TEXT NOT NULL);").unwrap();
    let old = task(&db, "old", "100", &root.path().join("missing.mp4"));
    legacy_transcript(&db, &old, "旧文案只保留一份");
    let backup = initialize(&mut db, &path).unwrap().unwrap();
    backfill(&mut db, &[old]).unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM video_transcripts", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    let json: String = db
        .query_row("SELECT snapshot FROM asset_transcriptions", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert!(serde_json::from_str::<serde_json::Value>(&json).unwrap()["result"].is_null());
    let file: String = db
        .query_row("SELECT file_id FROM asset_transcriptions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        latest_recognition(&db, &file).unwrap().unwrap().text,
        "旧文案只保留一份"
    );
    backfill(&mut db, &[]).unwrap();
    let old_db = Connection::open(backup).unwrap();
    assert_eq!(
        old_db
            .query_row("SELECT version FROM library_schema", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        old_db
            .query_row("SELECT count(*) FROM video_transcripts", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
}
