use super::*;
use crate::{desktop_core::transcription::Source, library::LocalMetadata};
use std::{
    path::Path,
    sync::{Arc, Mutex},
};

fn fixture(root: &Path) -> (Arc<Core>, Arc<Mutex<Vec<LibraryChange>>>) {
    let events = Arc::new(Mutex::new(Vec::new()));
    let captured = events.clone();
    let core = Core::open(
        &root.join("test.sqlite"),
        root.join("downloads"),
        "unused-fixture".into(),
        vec![],
        move |event| {
            if let TaskEvent::LibraryChanged(event) = event {
                captured.lock().unwrap().push(event);
            }
        },
    )
    .unwrap();
    (core, events)
}
fn seed(core: &Core, root: &Path, name: &str) -> library::ImportedVideo {
    let path = root.join(format!("{name}.mp4"));
    std::fs::write(&path, b"1234").unwrap();
    let source = Source::read(&path).unwrap();
    library::register_local(
        &core.inner.lock().unwrap().db,
        &source,
        &LocalMetadata {
            width: 64,
            height: 48,
            duration_ms: Some(1000),
        },
    )
    .unwrap()
}

#[tokio::test]
async fn clip_events_restart_and_historical_media_export_protection() {
    let root = tempfile::tempdir().unwrap(); let (core, events) = fixture(root.path());
    let item = seed(&core, root.path(), "原视频😀");
    let original = core.library_asset(&item.asset_id).unwrap().files.remove(0).path;
    let mut input = library::ClipInput { file_id: item.file_id.clone(), title: "开场".into(), reason: "节奏".into(), start_ms: 0, end_ms: 500,
        playable: library::ClipRange { start_ms: 0, end_ms: 1000 }, document_id: None, quote: None };
    let id = core.create_library_clip(&item.asset_id, &input).unwrap();
    assert_eq!(events.lock().unwrap().last().unwrap().scope, LibraryScope::Clips);
    let count = events.lock().unwrap().len(); input.end_ms = 2000;
    assert!(core.create_library_clip(&item.asset_id, &input).is_err());
    assert_eq!(events.lock().unwrap().len(), count);
    let replacement = root.path().join("替换.mp4"); std::fs::write(&replacement, b"replacement").unwrap();
    library::relink_file(&core.inner.lock().unwrap().db, &item.file_id, &replacement).unwrap();
    let document = core.save_library_document(&item.asset_id, "人工整理", 0, None).unwrap();
    assert_eq!(core.export_library_document(&item.asset_id, &document, &original).unwrap_err().to_string(), "TRANSCRIPTION_EXPORT_SOURCE");
    assert_eq!(core.export_library_content(&item.asset_id, &library::ContentSelection::Document(document), library::ContentFormat::Txt, &original).unwrap_err().to_string(), "TRANSCRIPTION_EXPORT_SOURCE");
    assert_eq!(std::fs::read(&original).unwrap(), b"1234");
    core.shutdown().await; drop(core);
    let (reopened, _) = fixture(root.path());
    assert_eq!(reopened.library_clips(&item.asset_id).unwrap()[0].id, id);
    assert_eq!(reopened.library_clip_playback(&item.asset_id, &id).unwrap_err().to_string(), "CLIP_SOURCE_CHANGED");
    reopened.delete_library_clip(&item.asset_id, &id).unwrap();
    assert!(reopened.library_clips(&item.asset_id).unwrap().is_empty());
    assert!(original.exists() && replacement.exists()); reopened.shutdown().await;
}

#[test]
fn p1a_upgrade_learning_history_clear_and_material_restore_round_trip() {
    let root = tempfile::tempdir().unwrap();
    let project = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let python = project.join(if cfg!(windows) {
        ".venv/Scripts/python.exe"
    } else {
        ".venv/bin/python"
    });
    let data = root.path().join("profile");
    let output = root.path().join("evidence");
    let prepare = || {
        let result = std::process::Command::new(&python)
            .args(["-X", "utf8"])
            .arg(project.join("tests/fixtures/seed_p1a_profile.py"))
            .arg("--data")
            .arg(&data)
            .arg("--output")
            .arg(&output)
            .args(["--scenario", "upgrade"])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    };
    let open = || {
        Core::open(
            &data.join("yingqu.sqlite"),
            output.join("media"),
            "unused-fixture".into(),
            vec![],
            |_| {},
        )
        .unwrap()
    };
    prepare();
    let core = open();
    assert_eq!(core.list().len(), 5);
    let assets = core.search_library(Default::default()).unwrap();
    assert_eq!(assets.items.len(), 2); // The album is not a video asset.
    let asset = assets
        .items
        .iter()
        .find(|item| item.files.total == 2)
        .unwrap()
        .asset
        .id
        .clone();
    let original = core.library_documents(&asset).unwrap().remove(0).id;
    assert!(core
        .library_document(&asset, &original)
        .unwrap()
        .text
        .contains("中文迁移原文"));
    assert_eq!(
        core.library_document(&asset, &original)
            .unwrap()
            .source_changed,
        Some(false)
    );
    let edited = core
        .save_library_document(&asset, "中文灵感😀 100%_", 0, Some(&original))
        .unwrap();
    let note = core
        .create_library_note(
            &asset,
            NoteInput {
                kind: "note".into(),
                body: "镜头学习心得".into(),
                citations: vec![library::Citation {
                    asset_id: asset.clone(),
                    document_id: original.clone(),
                    quote: "开场结构".into(),
                }],
            },
        )
        .unwrap();
    let parent = core.create_library_category(None, "学习").unwrap();
    let child = core.create_library_category(Some(&parent), "口播").unwrap();
    let other = core.create_library_category(None, "制作").unwrap();
    for category in [&child, &other] {
        core.set_library_categories(std::slice::from_ref(&asset), category, true)
            .unwrap();
    }
    let tag = core.create_library_tag("节奏").unwrap();
    core.set_library_tags(std::slice::from_ref(&asset), &tag, true)
        .unwrap();
    let request = SearchRequest {
        query: "灵感😀 100%_".into(),
        category_ids: vec![parent],
        tag_ids: vec![tag],
        ..Default::default()
    };
    assert_eq!(
        core.search_library(request.clone()).unwrap().items[0]
            .asset
            .id,
        asset
    );
    let media = core.library_asset(&asset).unwrap().files[0].path.clone();
    let fingerprint = Source::read(&media).unwrap();
    drop(core);
    prepare(); // Resuming acceptance must not rewrite source media fingerprints.
    assert_eq!(Source::read(&media).unwrap(), fingerprint);
    let core = open();
    assert_eq!(core.search_library(request.clone()).unwrap().items.len(), 1);
    assert_eq!(core.library_asset(&asset).unwrap().files.len(), 2);
    assert_eq!(core.library_documents(&asset).unwrap().len(), 2);
    assert_eq!(core.clear_history(None).unwrap().len(), 5);
    assert!(core.list().is_empty());
    assert_eq!(
        core.library_document(&asset, &edited).unwrap().text,
        "中文灵感😀 100%_"
    );
    assert_eq!(
        core.library_note(&asset, &note).unwrap().citations[0].document_id,
        original
    );
    std::fs::remove_file(&media).unwrap();
    assert!(core
        .library_asset(&asset)
        .unwrap()
        .files
        .iter()
        .any(|file| file.missing));
    let package = root.path().join("material-package");
    core.backup_library(&package, false, &mut |_| Ok(()))
        .unwrap();
    let destination = tempfile::tempdir().unwrap();
    let (restored, _) = fixture(destination.path());
    let report = restored
        .restore_library(&package, &destination.path().join("restored"), &mut |_| {
            Ok(())
        })
        .unwrap();
    assert_eq!((report.assets, report.documents, report.notes), (2, 3, 1));
    assert_eq!(
        restored.search_library(request).unwrap().items[0].asset.id,
        asset
    );
    assert_eq!(
        restored.library_asset(&asset).unwrap().category_ids.len(),
        2
    );
    assert_eq!(
        restored.library_document(&asset, &edited).unwrap().text,
        "中文灵感😀 100%_"
    );
    assert_eq!(
        restored.library_note(&asset, &note).unwrap().citations[0].document_id,
        original
    );
}

#[test]
fn library_export_uses_saved_edition_and_protects_current_and_original_media() {
    let root = tempfile::tempdir().unwrap();
    let (core, _) = fixture(root.path());
    let a = seed(&core, root.path(), "原视频");
    let other = seed(&core, root.path(), "其他资料");
    let original = core.library_playback_path(&a.asset_id, &a.file_id).unwrap();
    let result = crate::desktop_core::transcription::TranscriptResult { timing: None,
        text: "原文内容".into(),
        source: Source::read(&original).unwrap(),
        model_revision: "fixture".into(),
        model_name: None,
        pipeline_revision: "fixture".into(),
        backend: None,
        completed_at: 1,
    };
    let original_id = library::save_recognized(
        &core.inner.lock().unwrap().db,
        &a.asset_id,
        Some(&a.file_id),
        &result,
    )
    .unwrap();
    let edited = core
        .save_library_document(&a.asset_id, "中文整理稿😀\n第二段", 0, Some(&original_id))
        .unwrap();
    let replacement = root.path().join("新视频.mp4");
    std::fs::write(&replacement, b"new video").unwrap();
    library::relink_file(&core.inner.lock().unwrap().db, &a.file_id, &replacement).unwrap();
    let alias = root.path().join("别名.txt");
    std::fs::hard_link(&original, &alias).unwrap();
    for target in [&original, &replacement, &alias] {
        assert_eq!(
            core.export_library_document(&a.asset_id, &edited, target)
                .unwrap_err()
                .to_string(),
            "TRANSCRIPTION_EXPORT_SOURCE"
        );
    }
    let target = root.path().join("文案.txt");
    assert_eq!(
        core.export_library_document(&other.asset_id, &edited, &target)
            .unwrap_err()
            .to_string(),
        "DOCUMENT_NOT_FOUND"
    );
    assert!(!target.exists());
    core.export_library_document(&a.asset_id, &edited, &target)
        .unwrap();
    let bytes = std::fs::read(&target).unwrap();
    assert!(bytes.starts_with(b"\xef\xbb\xbf"));
    let text = std::str::from_utf8(&bytes[3..]).unwrap();
    assert_eq!(text.replace("\r\n", "\n"), "中文整理稿😀\n第二段");
    assert_eq!(std::fs::read(&original).unwrap(), b"1234");
    assert_eq!(std::fs::read(&replacement).unwrap(), b"new video");
}

#[test]
fn playback_ids_validate_ownership_and_unicode_long_paths_and_missing_files() {
    let root = tempfile::tempdir().unwrap();
    let (core, _) = fixture(root.path());
    let mut media = root.path().join("隔离媒体");
    for _ in 0..30 {
        media.push("中文视频文件路径");
    }
    std::fs::create_dir_all(&media).unwrap();
    let a = seed(&core, &media, "横屏样本");
    let b = seed(&core, root.path(), "另一份资料");
    let path = core.library_playback_path(&a.asset_id, &a.file_id).unwrap();
    assert!(path.to_string_lossy().chars().count() > 260);
    assert_eq!(std::fs::read(&path).unwrap(), b"1234");
    assert_eq!(
        core.library_playback_path(&b.asset_id, &a.file_id)
            .unwrap_err()
            .to_string(),
        "FILE_NOT_FOUND"
    );
    std::fs::remove_file(path).unwrap();
    assert_eq!(
        core.library_playback_path(&a.asset_id, &a.file_id)
            .unwrap_err()
            .to_string(),
        "FILE_MISSING"
    );
    let detail = core.library_asset(&a.asset_id).unwrap();
    assert!(detail.files[0].missing);
    assert_eq!(detail.asset.id, a.asset_id);
}

#[test]
fn typed_core_round_trip_exposes_summaries_and_checks_ownership_and_revisions() {
    let root = tempfile::tempdir().unwrap();
    let (core, events) = fixture(root.path());
    let a = seed(&core, root.path(), "one");
    let b = seed(&core, root.path(), "two");
    let parent = core.create_library_category(None, "职场").unwrap();
    let child = core.create_library_category(Some(&parent), "成长").unwrap();
    core.update_library_category(&child, Some(&parent), "学习", 2)
        .unwrap();
    core.set_library_categories(std::slice::from_ref(&a.asset_id), &child, true)
        .unwrap();
    let tag = core.create_library_tag("反问").unwrap();
    core.rename_library_tag(&tag, "故事").unwrap();
    core.set_library_tags(std::slice::from_ref(&a.asset_id), &tag, true)
        .unwrap();
    assert_eq!(core.library_categories().unwrap().len(), 2);
    assert_eq!(core.library_tags().unwrap()[0].name, "故事");
    let doc = core
        .save_library_document(&a.asset_id, "中文口播😀", 0, None)
        .unwrap();
    let note = core
        .create_library_note(
            &a.asset_id,
            NoteInput {
                kind: "note".into(),
                body: "学习笔记".into(),
                citations: vec![Citation {
                    asset_id: a.asset_id.clone(),
                    document_id: doc.clone(),
                    quote: "口播".into(),
                }],
            },
        )
        .unwrap();
    let detail = serde_json::to_value(core.library_asset(&a.asset_id).unwrap()).unwrap();
    assert_eq!(detail["documents"][0]["characters"], 5);
    assert!(detail["documents"][0].get("text").is_none());
    assert!(detail["notes"][0].get("body").is_none());
    assert_eq!(detail["category_ids"][0], child);
    assert_eq!(detail["tag_ids"][0], tag);
    assert_eq!(
        core.library_document(&a.asset_id, &doc).unwrap().text,
        "中文口播😀"
    );
    assert_eq!(core.library_documents(&a.asset_id).unwrap().len(), 1);
    assert_eq!(
        core.library_notes(&a.asset_id).unwrap()[0].citation_count,
        1
    );
    let before = events.lock().unwrap().len();
    assert_eq!(
        core.save_library_document(&a.asset_id, "过期修改", 0, None)
            .unwrap_err()
            .to_string(),
        "DOCUMENT_CONFLICT"
    );
    assert_eq!(
        core.edit_library_note(&b.asset_id, &note, "错误归属", 1)
            .unwrap_err()
            .to_string(),
        "NOTE_NOT_FOUND"
    );
    assert!(core.library_note(&b.asset_id, &note).is_err());
    assert!(core.library_document(&b.asset_id, &doc).is_err());
    assert_eq!(events.lock().unwrap().len(), before);
    core.edit_library_note(&a.asset_id, &note, "补充后的笔记", 1)
        .unwrap();
    assert_eq!(core.library_note(&a.asset_id, &note).unwrap().revision, 2);
    assert_eq!(
        core.edit_library_note(&a.asset_id, &note, "过期笔记", 1)
            .unwrap_err()
            .to_string(),
        "NOTE_CONFLICT"
    );
    core.archive_library_asset(&a.asset_id, true).unwrap();
    assert_eq!(
        core.search_library(SearchRequest::default())
            .unwrap()
            .items
            .len(),
        1
    );
    core.archive_library_asset(&a.asset_id, false).unwrap();
    assert_eq!(
        core.search_library(SearchRequest::default())
            .unwrap()
            .items
            .len(),
        2
    );
    core.delete_library_category(&child).unwrap();
    core.delete_library_category(&parent).unwrap();
    core.delete_library_tag(&tag).unwrap();
    assert!(core
        .library_asset(&a.asset_id)
        .unwrap()
        .category_ids
        .is_empty());
    assert!(core.library_asset(&a.asset_id).unwrap().tag_ids.is_empty());
    assert!(core
        .library_playback_path(&a.asset_id, &a.file_id)
        .unwrap()
        .is_file());
    assert!(events.lock().unwrap().iter().all(|e| {
        let value = serde_json::to_value(e).unwrap();
        value.get("scope").is_some() && value.get("text").is_none() && value.get("body").is_none()
    }));
}

#[test]
fn failed_writes_emit_no_success_event_and_keep_stable_error_codes() {
    let root = tempfile::tempdir().unwrap();
    let (core, events) = fixture(root.path());
    let a = seed(&core, root.path(), "one");
    core.inner.lock().unwrap().db.execute_batch("CREATE TRIGGER reject_note BEFORE INSERT ON asset_notes BEGIN SELECT RAISE(ABORT,'synthetic private diagnostics'); END;").unwrap();
    let error = core
        .create_library_note(
            &a.asset_id,
            NoteInput {
                kind: "note".into(),
                body: "笔记".into(),
                citations: vec![],
            },
        )
        .unwrap_err();
    assert_eq!(error.to_string(), "DATABASE_ERROR");
    assert!(events.lock().unwrap().is_empty());
    assert!(core.library_notes(&a.asset_id).unwrap().is_empty());
    let raw = serde_json::json!({"kind":"note","body":"text","origin":"ai"});
    assert!(serde_json::from_value::<NoteInput>(raw).is_err());
    assert!(serde_json::from_str::<SearchRequest>(r#"{"limit":-1}"#).is_err());
}

#[test]
fn document_source_and_playback_follow_current_file_without_losing_old_text() {
    let root = tempfile::tempdir().unwrap();
    let (core, _) = fixture(root.path());
    let a = seed(&core, root.path(), "one");
    let original = core.library_playback_path(&a.asset_id, &a.file_id).unwrap();
    let source = Source::read(&original).unwrap();
    let id = library::save_recognized(
        &core.inner.lock().unwrap().db,
        &a.asset_id,
        Some(&a.file_id),
        &crate::desktop_core::transcription::TranscriptResult { timing: None,
            text: "保留原文".into(),
            source,
            model_revision: "fixture".into(),
            model_name: None,
            pipeline_revision: "fixture".into(),
            backend: None,
            completed_at: 1,
        },
    )
    .unwrap();
    assert_eq!(
        core.library_document(&a.asset_id, &id)
            .unwrap()
            .source_changed,
        Some(false)
    );
    let replacement = root.path().join("replacement.mp4");
    std::fs::write(&replacement, b"another video").unwrap();
    library::relink_file(&core.inner.lock().unwrap().db, &a.file_id, &replacement).unwrap();
    assert!(original.exists());
    assert_eq!(
        core.library_document(&a.asset_id, &id)
            .unwrap()
            .source_changed,
        Some(true)
    );
    std::fs::remove_file(replacement).unwrap();
    assert_eq!(
        core.library_playback_path(&a.asset_id, &a.file_id)
            .unwrap_err()
            .to_string(),
        "FILE_MISSING"
    );
    assert_eq!(
        core.library_document(&a.asset_id, &id).unwrap().text,
        "保留原文"
    );
}

#[test]
fn deleting_through_existing_download_command_invalidates_library_file_state() {
    let root = tempfile::tempdir().unwrap();
    let (core, events) = fixture(root.path());
    let path = root.path().join("download.mp4");
    std::fs::write(&path, b"1234").unwrap();
    let task:crate::desktop_core::Task=serde_json::from_value(serde_json::json!({
        "id":"download","revision":1,"status":"completed","video":{"platform":"douyin","video_id":"123","kind":"video","resolver":"yt-dlp","title":"视频","canonical_url":"https://www.douyin.com/video/123"},
        "bytes":4,"total_bytes":4,"speed_bps":null,"error":null,"path":path,"created_at":1,"completed_at":2,"canceling":false,"download_requested":true
    })).unwrap();
    super::super::publish(&mut core.inner.lock().unwrap(), task, &*core.emit).unwrap();
    let asset_id = core.search_library(SearchRequest::default()).unwrap().items[0]
        .asset
        .id
        .clone();
    events.lock().unwrap().clear();
    core.delete_local_file("download").unwrap();
    assert!(!path.exists());
    assert!(core.library_asset(&asset_id).unwrap().files[0].missing);
    let notifications = events.lock().unwrap();
    assert_eq!(notifications.len(), 1);
    assert_eq!(notifications[0].scope, LibraryScope::Assets);
    assert_eq!(notifications[0].asset_ids, [asset_id]);
}

#[test]
fn workbench_visits_and_pending_states_survive_restart() {
    let root = tempfile::tempdir().unwrap();
    let (core, events) = fixture(root.path());
    let a = seed(&core, root.path(), "未整理");
    let b = seed(&core, root.path(), "笔记");
    let c = seed(&core, root.path(), "整理稿");
    assert_eq!(core.workbench().unwrap().library.recent.total, 0);
    core.library_asset(&a.asset_id).unwrap();
    assert_eq!(
        core.workbench().unwrap().library.recent.total,
        0,
        "background reads are not visits"
    );
    core.record_library_visit(&a.asset_id).unwrap();
    core.record_library_visit(&b.asset_id).unwrap();
    core.record_library_visit(&a.asset_id).unwrap();
    let snapshot = core.workbench().unwrap();
    assert_eq!(snapshot.library.recent.total, 2);
    assert_eq!(snapshot.library.recent.items[0].id, a.asset_id);
    assert_eq!(snapshot.library.unorganized.total, 3);
    let category = core.create_library_category(None, "分类").unwrap();
    core.set_library_categories(std::slice::from_ref(&a.asset_id), &category, true)
        .unwrap();
    core.create_library_note(
        &b.asset_id,
        NoteInput {
            kind: "note".into(),
            body: "不应泄露的全文".into(),
            citations: vec![],
        },
    )
    .unwrap();
    core.save_library_document(&c.asset_id, "整理稿全文", 0, None)
        .unwrap();
    let raw = crate::desktop_core::transcription::TranscriptResult { timing: None,
        text: "只有识别原文仍待整理".into(),
        source: Source::read(&root.path().join("未整理.mp4")).unwrap(),
        model_revision: "fixture".into(),
        model_name: None,
        pipeline_revision: "fixture".into(),
        backend: None,
        completed_at: 1,
    };
    library::save_recognized(
        &core.inner.lock().unwrap().db,
        &a.asset_id,
        Some(&a.file_id),
        &raw,
    )
    .unwrap();
    let snapshot = core.workbench().unwrap();
    assert_eq!(snapshot.library.uncategorized.total, 2);
    assert_eq!(snapshot.library.unorganized.items[0].id, a.asset_id);
    assert_eq!(snapshot.library.unorganized.total, 1);
    assert!(!serde_json::to_string(&snapshot)
        .unwrap()
        .contains("不应泄露"));
    let page = core
        .search_library(SearchRequest {
            unorganized: true,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].asset.id, a.asset_id);
    core.archive_library_asset(&a.asset_id, true).unwrap();
    assert_eq!(core.workbench().unwrap().library.unorganized.total, 0);
    assert_eq!(core.workbench().unwrap().library.recent.total, 1);
    core.archive_library_asset(&a.asset_id, false).unwrap();
    let emitted = events.lock().unwrap().len();
    assert!(core.record_library_visit("missing").is_err());
    core.inner.lock().unwrap().db.execute_batch("CREATE TRIGGER reject_visit BEFORE UPDATE ON asset_visits BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
    assert_eq!(
        core.record_library_visit(&a.asset_id)
            .unwrap_err()
            .to_string(),
        "DATABASE_ERROR"
    );
    assert_eq!(events.lock().unwrap().len(), emitted);
    core.inner
        .lock()
        .unwrap()
        .db
        .execute_batch("DROP TRIGGER reject_visit;")
        .unwrap();
    drop(core);
    let (reopened, _) = fixture(root.path());
    assert_eq!(
        reopened.workbench().unwrap().library.recent.items[0].id,
        a.asset_id
    );
    assert_eq!(reopened.workbench().unwrap().library.unorganized.total, 1);
    reopened
        .set_library_categories(std::slice::from_ref(&a.asset_id), &category, false)
        .unwrap();
    assert_eq!(reopened.workbench().unwrap().library.uncategorized.total, 3);
}

#[test]
fn workbench_migrates_v2_and_rolls_back_partial_upgrade() {
    for fail in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("old.sqlite");
        let mut db = rusqlite::Connection::open(&path).unwrap();
        db.execute_batch("PRAGMA journal_mode=WAL;").unwrap();
        db.execute_batch(include_str!("library/schema.sql"))
            .unwrap();
        db.execute_batch(include_str!("library/schema-v2.sql"))
            .unwrap();
        db.execute("INSERT INTO library_assets(id,title,created_at,updated_at) VALUES('old','保留资料',1,1)",[]).unwrap();
        if fail {
            db.execute_batch("CREATE TABLE asset_visits(legacy TEXT);")
                .unwrap();
        }
        let result = library::initialize(&mut db, &path);
        if fail {
            assert!(result.is_err());
            assert_eq!(
                db.query_row("SELECT version FROM library_schema", [], |r| r
                    .get::<_, i64>(0))
                    .unwrap(),
                2
            );
        } else {
            let backup = result.unwrap().unwrap();
            let old = rusqlite::Connection::open(backup).unwrap();
            assert_eq!(
                old.query_row("SELECT version FROM library_schema", [], |r| r
                    .get::<_, i64>(0))
                    .unwrap(),
                2
            );
            assert_eq!(
                old.query_row("SELECT title FROM library_assets", [], |r| r
                    .get::<_, String>(0))
                    .unwrap(),
                "保留资料"
            );
            assert_eq!(
                db.query_row("SELECT version FROM library_schema", [], |r| r
                    .get::<_, i64>(0))
                    .unwrap(),
                11
            );
            assert_eq!(
                library::workbench_library(&db).unwrap().unorganized.total,
                1
            );
        }
    }
}

#[test]
fn workbench_operations_use_live_overrides_and_keep_precise_destinations() {
    let root = tempfile::tempdir().unwrap();
    let (core, _) = fixture(root.path());
    let a = seed(&core, root.path(), "操作所属资料");
    let snapshot: crate::desktop_core::transcription::Snapshot = serde_json::from_value(serde_json::json!({
        "task_id":"", "asset_id":a.asset_id,"file_id":a.file_id,"operation_id":"asr-op","sequence":1,
        "state":"failed","phase":null,"error_code":"IO_ERROR","updated_at":1,"result":null,"has_result":false,"source_changed":false
    })).unwrap();
    core.inner
        .lock()
        .unwrap()
        .db
        .execute(
            "INSERT INTO asset_transcriptions VALUES(?1,?2)",
            rusqlite::params![a.file_id, serde_json::to_string(&snapshot).unwrap()],
        )
        .unwrap();
    let state = core.workbench().unwrap();
    assert_eq!(state.operations.len(), 1);
    assert_eq!(state.operations[0].asset_id.as_ref(), Some(&a.asset_id));
    assert_eq!(state.operations[0].file_id.as_ref(), Some(&a.file_id));
    let mut completed = snapshot.clone();
    completed.state = "completed".into();
    core.inner
        .lock()
        .unwrap()
        .transcription_unsaved
        .insert(a.file_id.clone(), completed);
    assert!(
        core.workbench().unwrap().operations.is_empty(),
        "newer in-memory completion suppresses stale failed row"
    );
    let (cancel, _) = tokio::sync::watch::channel(false);
    let (_, done) = tokio::sync::watch::channel(false);
    let mut running = snapshot;
    running.state = "running".into();
    core.inner.lock().unwrap().transcription =
        Some(crate::desktop_core::transcription::ActiveTranscription {
            snapshot: running,
            cancel,
            done,
            source: Source::read(&root.path().join("操作所属资料.mp4")).unwrap(),
        });
    assert_eq!(core.workbench().unwrap().operations[0].state, "running");
    assert!(!serde_json::to_string(&core.workbench().unwrap())
        .unwrap()
        .contains("SyntheticVideos"));
}

#[test]
fn workbench_downloads_keep_old_queued_and_failed_task_ids() {
    let root = tempfile::tempdir().unwrap();
    let (core, _) = fixture(root.path());
    for (id, status, requested, error) in [
        ("old-queued", "queued", true, None),
        ("preview", "parsing", false, None),
        ("failed", "failed", true, Some("IO_ERROR")),
        ("done", "completed", true, None),
        ("ready", "ready", false, None),
    ] {
        let task: crate::desktop_core::Task=serde_json::from_value(serde_json::json!({"id":id,"revision":1,"status":status,"download_requested":requested,"video":null,"bytes":0,"error":error,"path":null,"created_at":1,"completed_at":null,"canceling":false,"speed_bps":null,"total_bytes":null})).unwrap();
        core.inner.lock().unwrap().tasks.push(task);
    }
    let operations = core.workbench().unwrap().operations;
    assert_eq!(operations.len(), 3);
    for id in ["old-queued", "preview", "failed"] {
        assert!(operations
            .iter()
            .any(|op| op.task_id.as_deref() == Some(id)));
    }
    assert_eq!(
        operations
            .iter()
            .find(|op| op.id == "failed")
            .unwrap()
            .state,
        "failed"
    );
}

#[test]
fn portable_content_exports_documents_and_notes_in_three_formats_without_mutating_versions() {
    use library::{ContentFormat, ContentSelection};
    let root = tempfile::tempdir().unwrap();
    let (core, events) = fixture(root.path());
    let a = seed(&core, root.path(), "导出对象");
    let other = seed(&core, root.path(), "引用对象");
    let doc = core
        .save_library_document(&a.asset_id, "中文😀\n文案正文", 0, None)
        .unwrap();
    let cited = core
        .save_library_document(&other.asset_id, "被引用的原句", 0, None)
        .unwrap();
    let note = core
        .create_library_note(
            &a.asset_id,
            NoteInput {
                kind: "note".into(),
                body: "学习笔记😀".into(),
                citations: vec![library::Citation {
                    asset_id: other.asset_id.clone(),
                    document_id: cited.clone(),
                    quote: "原句".into(),
                }],
            },
        )
        .unwrap();
    let before = events.lock().unwrap().len();
    for selection in [
        ContentSelection::Document(doc.clone()),
        ContentSelection::Note(note.clone()),
    ] {
        for (format, ext) in [
            (ContentFormat::Txt, "txt"),
            (ContentFormat::Markdown, "md"),
            (ContentFormat::Json, "json"),
        ] {
            let path = root.path().join(format!("export.{ext}"));
            core.export_library_content(&a.asset_id, &selection, format, &path)
                .unwrap();
            let bytes = std::fs::read(&path).unwrap();
            match format {
                ContentFormat::Json => {
                    assert!(!bytes.starts_with(b"\xef\xbb\xbf"));
                    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                    assert_eq!(value["asset"]["id"], a.asset_id);
                    if matches!(selection, ContentSelection::Note(_)) {
                        assert_eq!(value["references"][0]["document"]["id"], cited);
                        assert_eq!(value["references"][0]["asset"]["id"], other.asset_id);
                    }
                }
                ContentFormat::Markdown => {
                    let text = std::str::from_utf8(&bytes).unwrap();
                    assert!(text.contains("## 来源信息"));
                    assert!(text.contains("yingqu-content"));
                }
                ContentFormat::Txt => assert!(bytes.starts_with(b"\xef\xbb\xbf")),
            }
        }
    }
    assert_eq!(events.lock().unwrap().len(), before);
    assert_eq!(core.library_documents(&a.asset_id).unwrap().len(), 1);
    assert_eq!(core.library_note(&a.asset_id, &note).unwrap().revision, 1);
    let wrong = root.path().join("wrong.json");
    assert_eq!(
        core.export_library_content(
            &a.asset_id,
            &ContentSelection::Document(cited),
            ContentFormat::Json,
            &wrong
        )
        .unwrap_err()
        .to_string(),
        "DOCUMENT_NOT_FOUND"
    );
    assert!(!wrong.exists());
    let source = core
        .library_playback_path(&other.asset_id, &other.file_id)
        .unwrap();
    let alias = root.path().join("alias.json");
    std::fs::hard_link(&source, &alias).unwrap();
    for target in [source, alias] {
        assert_eq!(
            core.export_library_content(
                &a.asset_id,
                &ContentSelection::Note(note.clone()),
                ContentFormat::Json,
                &target
            )
            .unwrap_err()
            .to_string(),
            "TRANSCRIPTION_EXPORT_SOURCE"
        );
    }
    assert_eq!(
        std::fs::read(root.path().join("引用对象.mp4")).unwrap(),
        b"1234"
    );
}

#[test]
fn core_backup_restore_preserves_settings_and_survives_restart_and_rechecks_empty_state() {
    let source = tempfile::tempdir().unwrap();
    let (app, _) = fixture(source.path());
    let a = seed(&app, source.path(), "原始资料");
    let doc = app
        .save_library_document(&a.asset_id, "跨设备保存的文案", 0, None)
        .unwrap();
    app.record_library_visit(&a.asset_id).unwrap();
    let package = source.path().join("package");
    let mut phases = Vec::new();
    app.backup_library(&package, true, &mut |p| {
        phases.push(p.phase);
        Ok(())
    })
    .unwrap();
    assert!(phases.contains(&"copying") && phases.contains(&"committing"));
    let destination = tempfile::tempdir().unwrap();
    let (core, events) = fixture(destination.path());
    let settings = serde_json::to_value(core.settings()).unwrap();
    let restored = destination.path().join("restored");
    core.restore_library(&package, &restored, &mut |_| Ok(()))
        .unwrap();
    assert_eq!(
        core.library_document(&a.asset_id, &doc).unwrap().text,
        "跨设备保存的文案"
    );
    assert_eq!(serde_json::to_value(core.settings()).unwrap(), settings);
    assert!(core.list().is_empty());
    assert_eq!(events.lock().unwrap().len(), 1);
    assert_eq!(events.lock().unwrap()[0].scope, LibraryScope::Assets);
    let again = destination.path().join("again");
    assert_eq!(
        core.restore_library(&package, &again, &mut |_| Ok(()))
            .unwrap_err()
            .to_string(),
        "LIBRARY_NOT_EMPTY"
    );
    assert!(!again.exists());
    drop(core);
    let (core, _) = fixture(destination.path());
    assert_eq!(
        core.library_document(&a.asset_id, &doc).unwrap().text,
        "跨设备保存的文案"
    );
    assert_eq!(
        core.workbench().unwrap().library.recent.items[0].id,
        a.asset_id
    );
    let race = tempfile::tempdir().unwrap();
    let (core, events) = fixture(race.path());
    let target = race.path().join("concurrent");
    let mut inserted = false;
    let error = core
        .restore_library(&package, &target, &mut |p| {
            if p.phase == "committing" && !inserted {
                inserted = true;
                core.create_library_tag("恢复前新增的标签")?;
            }
            Ok(())
        })
        .unwrap_err();
    assert_eq!(error.to_string(), "LIBRARY_NOT_EMPTY");
    assert!(!target.exists());
    assert_eq!(events.lock().unwrap().len(), 1);
    assert!(core
        .search_library(Default::default())
        .unwrap()
        .items
        .is_empty());
    let canceled = race.path().join("cancel-backup");
    assert_eq!(
        app.backup_library(&canceled, true, &mut |_| anyhow::bail!("CANCELED"))
            .unwrap_err()
            .to_string(),
        "CANCELED"
    );
    assert!(!canceled.exists());
}

#[test]
fn edited_content_exports_trace_original_models_and_file_source_through_immutable_editions() {
    use library::{ContentFormat, ContentSelection};
    let root = tempfile::tempdir().unwrap();
    let (core, _) = fixture(root.path());
    let a = seed(&core, root.path(), "来源视频");
    let original = library::save_recognized(
        &core.inner.lock().unwrap().db,
        &a.asset_id,
        Some(&a.file_id),
        &crate::desktop_core::transcription::TranscriptResult { timing: None,
            text: "模型识别的原文".into(),
            source: Source::read(&root.path().join("来源视频.mp4")).unwrap(),
            model_revision: "preserved-model-revision".into(),
            model_name: Some("original-model".into()),
            pipeline_revision: "original-pipeline".into(),
            backend: Some("cpu".into()),
            completed_at: 1,
        },
    )
    .unwrap();
    let first = core
        .save_library_document(&a.asset_id, "第一稿", 0, Some(&original))
        .unwrap();
    let second = core
        .save_library_document(&a.asset_id, "第二稿", 1, Some(&first))
        .unwrap();
    for (format, extension) in [
        (ContentFormat::Json, "json"),
        (ContentFormat::Markdown, "md"),
    ] {
        let path = root.path().join(format!("edited.{extension}"));
        core.export_library_content(
            &a.asset_id,
            &ContentSelection::Document(second.clone()),
            format,
            &path,
        )
        .unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            text.contains("preserved-model-revision")
                && text.contains("original-pipeline")
                && text.contains("来源视频.mp4")
        );
        if matches!(format, ContentFormat::Json) {
            let value: serde_json::Value = serde_json::from_str(&text).unwrap();
            assert_eq!(value["references"].as_array().unwrap().len(), 2);
            assert_eq!(value["references"][1]["document"]["id"], original);
            assert_eq!(value["references"][1]["document"]["version"], 1);
            assert!(value["references"][1].get("text").is_none());
        }
    }
}
