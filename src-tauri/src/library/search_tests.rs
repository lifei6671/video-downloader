use super::*;
use crate::library::*;
use rusqlite::params;
use std::path::Path;

fn database() -> Connection {
    let mut db = Connection::open_in_memory().unwrap();
    initialize(&mut db, Path::new(":memory:")).unwrap();
    db.execute_batch("CREATE TABLE download_tasks(id TEXT PRIMARY KEY,snapshot TEXT NOT NULL)")
        .unwrap();
    db
}
fn seed(db: &Connection, id: &str, title: &str, platform: Option<&str>, path: &Path) {
    db.execute("INSERT INTO library_assets(id,title,author,platform,video_id,created_at,updated_at) VALUES(?1,?2,'作者',?3,?4,10,10)",params![id,title,platform,platform.map(|_|id)]).unwrap();
    db.execute(
        "INSERT INTO asset_files(id,asset_id,path,size) VALUES(?1,?1,?2,4)",
        params![id, path.to_str().unwrap()],
    )
    .unwrap();
}
fn ids(page: SearchPage) -> Vec<String> {
    page.items.into_iter().map(|item| item.asset.id).collect()
}

#[test]
fn unicode_literal_search_all_text_surfaces_and_versioned_hits() {
    let db = database();
    let root = tempfile::tempdir().unwrap();
    seed(
        &db,
        "a",
        "中文职场100%_稿",
        Some("douyin"),
        &root.path().join("missing.mp4"),
    );
    seed(&db, "b", "其他标题", None, &root.path().join("other.mp4"));
    db.execute(
        "UPDATE library_assets SET author='English Author' WHERE id='b'",
        [],
    )
    .unwrap();
    let raw = crate::desktop_core::transcription::TranscriptResult { timing: None,
        text: format!("{}识别核心{}", "前".repeat(200), "后".repeat(200)),
        source: crate::desktop_core::transcription::Source {
            path: root.path().join("other.mp4"),
            size: 4,
            mtime_ns: "0".into(),
        },
        model_revision: "fixture".into(),
        model_name: None,
        pipeline_revision: "fixture".into(),
        backend: None,
        completed_at: 1,
    };
    let raw_id = save_recognized(&db, "b", Some("b"), &raw).unwrap();
    let edited = save_edited(&db, "b", "旧版本独有", 0, Some(&raw_id)).unwrap();
    save_edited(&db, "b", "新版本独有", 1, Some(&raw_id)).unwrap();
    let note_id = create_note(
        &db,
        "b",
        NewNote {
            kind: "note",
            body: "笔记灵感",
            origin: NoteOrigin::Human,
            model_name: None,
            citations: &[],
        },
    )
    .unwrap();
    for term in ["中", "职场", "%", "_", "100%_"] {
        assert_eq!(
            ids(search_assets(
                &db,
                &SearchRequest {
                    query: term.into(),
                    ..Default::default()
                }
            )
            .unwrap()),
            ["a"]
        );
    }
    for (term, field) in [
        ("english author", "author"),
        ("识别核心", "recognized"),
        ("旧版本独有", "edited"),
        ("笔记灵感", "note"),
    ] {
        let page = search_assets(
            &db,
            &SearchRequest {
                query: term.into(),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(page.items.len(), 1);
        let hit = page.items[0].hit.as_ref().unwrap();
        assert_eq!(hit.field, field);
        assert!(hit.excerpt.chars().count() <= 120);
        if field == "recognized" {
            assert_eq!(hit.document_id.as_deref(), Some(raw_id.as_str()));
            assert_eq!(hit.document_version, Some(1));
        }
        if field == "edited" {
            assert_eq!(hit.document_id.as_deref(), Some(edited.as_str()));
            assert_eq!(hit.document_version, Some(1));
        }
        if field == "note" {
            assert_eq!(hit.note_id.as_deref(), Some(note_id.as_str()));
        }
    }
    assert!(search_assets(
        &db,
        &SearchRequest {
            query: "' OR 1=1 --".into(),
            ..Default::default()
        }
    )
    .unwrap()
    .items
    .is_empty());
}

#[test]
fn flat_category_filters_and_all_tags_without_duplicates() {
    let db = database();
    let root = tempfile::tempdir().unwrap();
    for id in ["a", "b", "c"] {
        seed(
            &db,
            id,
            "口播",
            Some("douyin"),
            &root.path().join(format!("{id}.mp4")),
        );
    }
    let parent = create_category(&db, None, "职场").unwrap();
    let child = create_category(&db, Some(&parent), "成长").unwrap();
    set_category_membership(&db, &["a".into(), "b".into()], &child, true).unwrap();
    // Historical hierarchy metadata must not expand a first-level category filter.
    assert!(search_assets(&db, &SearchRequest { category_ids: vec![parent.clone()], ..Default::default() }).unwrap().items.is_empty());
    assert_eq!(ids(search_assets(&db, &SearchRequest { category_ids: vec![child.clone()], ..Default::default() }).unwrap()), ["b", "a"]);

    let t1 = create_tag(&db, "故事").unwrap();
    let t2 = create_tag(&db, "反问").unwrap();
    set_tag_membership(&db, &["a".into(), "b".into()], &t1, true).unwrap();
    set_tag_membership(&db, &["a".into()], &t2, true).unwrap();
    let request = SearchRequest {
        query: "口播".into(),
        category_ids: vec![parent.clone(), child],
        tag_ids: vec![t1, t2.clone(), t2],
        platform: Some("douyin".into()),
        file_state: FileFilter::Missing,
        ..Default::default()
    };
    assert_eq!(ids(search_assets(&db, &request).unwrap()), ["a"]);
    archive_asset(&db, "a", true).unwrap();
    assert!(search_assets(&db, &request).unwrap().items.is_empty());
    assert_eq!(
        ids(search_assets(
            &db,
            &SearchRequest {
                archived: true,
                ..request
            }
        )
        .unwrap()),
        ["a"]
    );
    assert_eq!(
        ids(search_assets(
            &db,
            &SearchRequest {
                uncategorized: true,
                ..Default::default()
            }
        )
        .unwrap()),
        ["c"]
    );
    assert_eq!(
        search_assets(
            &db,
            &SearchRequest {
                category_ids: vec![parent],
                uncategorized: true,
                ..Default::default()
            }
        )
        .unwrap_err()
        .to_string(),
        "INVALID_INPUT"
    );
}

#[test]
fn stable_keyset_pages_apply_file_filters_before_limits() {
    let db = database();
    let root = tempfile::tempdir().unwrap();
    for id in ["a", "b", "c", "d", "e"] {
        let path = root.path().join(format!("{id}.mp4"));
        if matches!(id, "a" | "c" | "e") {
            std::fs::write(&path, b"1234").unwrap();
        }
        seed(&db, id, "分页", None, &path);
    }
    let mut request = SearchRequest {
        file_state: FileFilter::Available,
        limit: 1,
        ..Default::default()
    };
    let first = search_assets(&db, &request).unwrap();
    assert_eq!(first.items[0].asset.id, "e");
    request.cursor = first.next_cursor;
    seed(&db, "z", "新入库", None, &root.path().join("new.mp4"));
    let second = search_assets(&db, &request).unwrap();
    assert_eq!(second.items[0].asset.id, "c");
    request.cursor = second.next_cursor;
    let third = search_assets(&db, &request).unwrap();
    assert_eq!(third.items[0].asset.id, "a");
    assert!(third.next_cursor.is_none());
    std::fs::write(root.path().join("c.mp4"), b"changed").unwrap();
    assert_eq!(
        ids(search_assets(
            &db,
            &SearchRequest {
                file_state: FileFilter::Changed,
                ..Default::default()
            }
        )
        .unwrap()),
        ["c"]
    );
    assert_eq!(
        ids(search_assets(
            &db,
            &SearchRequest {
                file_state: FileFilter::Missing,
                ..Default::default()
            }
        )
        .unwrap()),
        ["z", "d", "b"]
    );
}

#[test]
fn invalid_parameters_and_unknown_organization_have_explicit_errors() {
    let db = database();
    for request in [
        SearchRequest {
            limit: 0,
            ..Default::default()
        },
        SearchRequest {
            limit: 101,
            ..Default::default()
        },
        SearchRequest {
            query: "中".repeat(513),
            ..Default::default()
        },
        SearchRequest {
            platform: Some("unknown".into()),
            ..Default::default()
        },
    ] {
        assert_eq!(
            search_assets(&db, &request).unwrap_err().to_string(),
            "INVALID_INPUT"
        );
    }
    assert_eq!(
        search_assets(
            &db,
            &SearchRequest {
                category_ids: vec!["missing".into()],
                ..Default::default()
            }
        )
        .unwrap_err()
        .to_string(),
        "CATEGORY_NOT_FOUND"
    );
    assert_eq!(
        search_assets(
            &db,
            &SearchRequest {
                tag_ids: vec!["missing".into()],
                ..Default::default()
            }
        )
        .unwrap_err()
        .to_string(),
        "TAG_NOT_FOUND"
    );
    for json in [
        r#"{"limit":"20"}"#,
        r#"{"file_state":"broken"}"#,
        r#"{"sql":"DROP TABLE"}"#,
    ] {
        assert!(serde_json::from_str::<SearchRequest>(json).is_err());
    }
}

#[test]
fn representative_corpus_records_size_and_latency() {
    let mut db = database();
    let root = tempfile::tempdir().unwrap();
    let started = std::time::Instant::now();
    let tx = db.transaction().unwrap();
    let body = format!(
        "{}灵感检索样本{}",
        "中文口播结构。".repeat(450),
        "镜头学习。".repeat(450)
    );
    for n in 0..1000 {
        let id = format!("asset-{n:04}");
        seed(
            &tx,
            &id,
            "样本",
            None,
            &root.path().join(format!("{n}.mp4")),
        );
        save_edited(&tx, &id, &body, 0, None).unwrap();
    }
    tx.commit().unwrap();
    let seeded = started.elapsed();
    let mut elapsed = Vec::new();
    for _ in 0..3 {
        let start = std::time::Instant::now();
        let result = search_assets(
            &db,
            &SearchRequest {
                query: "灵感检索样本".into(),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(result.items.len(), 20);
        elapsed.push(start.elapsed().as_millis());
    }
    let bytes: i64 = db
        .query_row(
            "SELECT sum(length(CAST(text AS BLOB))) FROM asset_documents",
            [],
            |r| r.get(0),
        )
        .unwrap();
    eprintln!("library search corpus: assets=1000 documents=1000 text_bytes={bytes} query_ms={elapsed:?} seed_ms={}",seeded.as_millis());
}
