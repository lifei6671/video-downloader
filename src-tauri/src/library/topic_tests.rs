use super::*;
use crate::desktop_core::transcription::Source;
use serde_json::json;
use std::{fs, path::Path};

fn database() -> Connection {
    let mut db = Connection::open_in_memory().unwrap();
    initialize(&mut db, Path::new(":memory:")).unwrap();
    db.execute_batch("CREATE TABLE download_tasks(id TEXT PRIMARY KEY,snapshot TEXT);")
        .unwrap();
    db
}
fn seed(db: &Connection, root: &Path, name: &str) -> (String, String, String) {
    let path = root.join(format!("{name}😀.mp4"));
    fs::write(&path, name).unwrap();
    let item = register_local(
        db,
        &Source::read(&path).unwrap(),
        &LocalMetadata {
            width: 90,
            height: 160,
            duration_ms: Some(1000),
        },
    )
    .unwrap();
    let clip = create_clip(
        db,
        &item.asset_id,
        &ClipInput {
            file_id: item.file_id,
            title: format!("{name}片段😀"),
            reason: String::new(),
            start_ms: 100,
            end_ms: 900,
            playable: ClipRange {
                start_ms: 0,
                end_ms: 1000,
            },
            document_id: None,
            quote: None,
        },
    )
    .unwrap();
    let note = create_note(
        db,
        &item.asset_id,
        NewNote {
            kind: "note",
            body: "学习笔记😀",
            origin: NoteOrigin::Human,
            model_name: None,
            citations: &[],
        },
    )
    .unwrap();
    (item.asset_id, clip, note)
}
fn create(db: &Connection) -> String {
    create_research_topic(
        db,
        &TopicInput {
            title: "开场研究😀".into(),
            description: "对照表达与制作".into(),
        },
    )
    .unwrap()
}
fn member(kind: TopicMemberKind, asset: &str, id: &str) -> TopicMemberInput {
    TopicMemberInput {
        kind,
        asset_id: asset.into(),
        resource_id: id.into(),
    }
}
#[test]
fn topic_members_combine_sources_reorder_and_remove_without_deleting_originals() {
    let root = tempfile::tempdir().unwrap();
    let db = database();
    let (asset, clip, note) = seed(&db, root.path(), "甲");
    let topic = create(&db);
    let a = add_topic_member(
        &db,
        &topic,
        &member(TopicMemberKind::Video, &asset, &asset),
        1,
    )
    .unwrap();
    let b = add_topic_member(
        &db,
        &topic,
        &member(TopicMemberKind::Clip, &asset, &clip),
        2,
    )
    .unwrap();
    let c = add_topic_member(
        &db,
        &topic,
        &member(TopicMemberKind::Note, &asset, &note),
        3,
    )
    .unwrap();
    reorder_topic_members(&db, &topic, &[c.clone(), a.clone(), b.clone()], 4).unwrap();
    let detail = research_topic(&db, &topic).unwrap();
    assert_eq!(
        detail
            .members
            .iter()
            .map(|m| &m.member.id)
            .collect::<Vec<_>>(),
        vec![&c, &a, &b]
    );
    remove_topic_member(&db, &topic, &b, 5).unwrap();
    assert_eq!(clips(&db, &asset).unwrap().len(), 1);
    assert!(read_note(&db, &asset, &note).is_ok());
    assert!(clip_playback(&db, &asset, &clip).is_ok());
    let detail = research_topic(&db, &topic).unwrap();
    assert_eq!(detail.members[1].member.position, 1);
    assert_eq!(detail.topic.revision, 6);
}
#[test]
fn topic_rejects_wrong_owners_duplicates_bad_permutations_and_stale_edits_atomically() {
    let root = tempfile::tempdir().unwrap();
    let db = database();
    let (asset, clip, note) = seed(&db, root.path(), "甲");
    let (other, _, _) = seed(&db, root.path(), "乙");
    let topic = create(&db);
    for input in [
        member(TopicMemberKind::Clip, &other, &clip),
        member(TopicMemberKind::Note, &other, &note),
        member(TopicMemberKind::Video, &other, &asset),
    ] {
        assert!(add_topic_member(&db, &topic, &input, 1).is_err());
    }
    let id = add_topic_member(
        &db,
        &topic,
        &member(TopicMemberKind::Video, &asset, &asset),
        1,
    )
    .unwrap();
    assert!(add_topic_member(
        &db,
        &topic,
        &member(TopicMemberKind::Video, &asset, &asset),
        2
    )
    .is_err());
    for order in [vec![], vec![id.clone(), id.clone()], vec!["foreign".into()]] {
        assert!(reorder_topic_members(&db, &topic, &order, 2).is_err());
    }
    assert!(remove_topic_member(&db, &topic, &id, 1).is_err());
    assert!(remove_topic_member(&db, &topic, "foreign", 2).is_err());
    assert_eq!(research_topic(&db, &topic).unwrap().topic.revision, 2);
    db.execute_batch("CREATE TRIGGER fail_topic BEFORE INSERT ON research_topic_members BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
    assert!(add_topic_member(
        &db,
        &topic,
        &member(TopicMemberKind::Note, &asset, &note),
        2
    )
    .is_err());
    assert_eq!(research_topic(&db, &topic).unwrap().topic.revision, 2);
    edit_research_topic(
        &db,
        &topic,
        &TopicInput {
            title: "新标题".into(),
            description: String::new(),
        },
        2,
    )
    .unwrap();
    assert!(edit_research_topic(
        &db,
        &topic,
        &TopicInput {
            title: "冲突".into(),
            description: String::new()
        },
        2
    )
    .is_err());
}
#[test]
fn archived_missing_media_and_deleted_references_remain_visible() {
    let root = tempfile::tempdir().unwrap();
    let db = database();
    let (asset, clip, note) = seed(&db, root.path(), "甲");
    let topic = create(&db);
    for (revision, input) in [
        (1, member(TopicMemberKind::Video, &asset, &asset)),
        (2, member(TopicMemberKind::Clip, &asset, &clip)),
        (3, member(TopicMemberKind::Note, &asset, &note)),
    ] {
        add_topic_member(&db, &topic, &input, revision).unwrap();
    }
    archive_asset(&db, &asset, true).unwrap();
    fs::remove_file(root.path().join("甲😀.mp4")).unwrap();
    let detail = research_topic(&db, &topic).unwrap();
    assert!(detail.members.iter().all(|m| m.archived));
    assert_eq!(detail.members[0].file_state.as_deref(), Some("missing"));
    assert!(!detail.members[0].missing);
    delete_clip(&db, &asset, &clip).unwrap();
    db.execute("DELETE FROM asset_notes WHERE id=?1", [&note])
        .unwrap();
    let detail = research_topic(&db, &topic).unwrap();
    assert_eq!(detail.members.len(), 3);
    assert!(detail.members[1].missing && detail.members[2].missing);
    assert!(detail.members[1].title.contains("片段😀"));
    db.execute("DELETE FROM library_assets WHERE id=?1", [&asset])
        .unwrap();
    assert!(research_topic(&db, &topic)
        .unwrap()
        .members
        .iter()
        .all(|m| m.missing));
}
#[test]
fn topics_roundtrip_twice_with_member_order_and_unavailable_sources() {
    let root = tempfile::tempdir().unwrap();
    let db = database();
    let (asset, clip, note) = seed(&db, root.path(), "甲");
    let topic = create(&db);
    let a = add_topic_member(
        &db,
        &topic,
        &member(TopicMemberKind::Video, &asset, &asset),
        1,
    )
    .unwrap();
    let b = add_topic_member(
        &db,
        &topic,
        &member(TopicMemberKind::Clip, &asset, &clip),
        2,
    )
    .unwrap();
    let c = add_topic_member(
        &db,
        &topic,
        &member(TopicMemberKind::Note, &asset, &note),
        3,
    )
    .unwrap();
    reorder_topic_members(&db, &topic, &[c, b, a], 4).unwrap();
    delete_clip(&db, &asset, &clip).unwrap();
    archive_asset(&db, &asset, true).unwrap();
    for media in [false, true] {
        let package = root.path().join(format!("package-{media}"));
        write_backup(capture_backup(&db, media).unwrap(), &package, &mut |_| {
            Ok(())
        })
        .unwrap();
        let mut restored = database();
        let prepared = prepare_restore(
            &package,
            &root.path().join(format!("restored-{media}")),
            &mut |_| Ok(()),
        )
        .unwrap();
        let report = commit_restore(&mut restored, prepared).unwrap();
        assert_eq!(report.topics, 1);
        let before = research_topic(&db, &topic).unwrap();
        let after = research_topic(&restored, &topic).unwrap();
        assert_eq!(
            serde_json::to_value(&before.topic).unwrap(),
            serde_json::to_value(&after.topic).unwrap()
        );
        assert_eq!(
            before
                .members
                .iter()
                .map(|m| &m.member.id)
                .collect::<Vec<_>>(),
            after
                .members
                .iter()
                .map(|m| &m.member.id)
                .collect::<Vec<_>>()
        );
        assert!(after.members.iter().all(|m| m.archived));
        assert!(after.members[1].missing);
        let again = root.path().join(format!("again-{media}"));
        write_backup(
            capture_backup(&restored, media).unwrap(),
            &again,
            &mut |_| Ok(()),
        )
        .unwrap();
        prepare_restore(
            &again,
            &root.path().join(format!("again-restored-{media}")),
            &mut |_| Ok(()),
        )
        .unwrap();
    }
}
#[test]
fn topic_only_library_is_not_empty_and_old_or_tampered_packages_are_rejected() {
    use sha2::{Digest, Sha256};
    let root = tempfile::tempdir().unwrap();
    let db = database();
    let topic = create(&db);
    assert!(require_empty_library(&db).is_err());
    let package = root.path().join("empty-assets");
    write_backup(capture_backup(&db, false).unwrap(), &package, &mut |_| {
        Ok(())
    })
    .unwrap();
    let mut restored = database();
    let prepared =
        prepare_restore(&package, &root.path().join("restored"), &mut |_| Ok(())).unwrap();
    commit_restore(&mut restored, prepared).unwrap();
    assert_eq!(research_topics(&restored).unwrap().len(), 1);
    let (asset, clip, _) = seed(&db, root.path(), "甲");
    let (other, _, _) = seed(&db, root.path(), "乙");
    add_topic_member(
        &db,
        &topic,
        &member(TopicMemberKind::Clip, &asset, &clip),
        1,
    )
    .unwrap();
    let package = root.path().join("with-members");
    write_backup(capture_backup(&db, false).unwrap(), &package, &mut |_| {
        Ok(())
    })
    .unwrap();
    let original: serde_json::Value =
        serde_json::from_slice(&fs::read(package.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(original["version"], 9);
    for flaw in ["old", "owner", "order", "topic", "title", "duplicate"] {
        let mut bad = original.clone();
        match flaw {
            "old" => bad["version"] = json!(6),
            "owner" => bad["topic_members"][0]["asset_id"] = json!(other),
            "order" => bad["topic_members"][0]["position"] = json!(9),
            "topic" => bad["topic_members"][0]["topic_id"] = json!("foreign"),
            "title" => bad["research_topics"][0]["title"] = json!(""),
            _ => {
                let duplicate = bad["topic_members"][0].clone();
                bad["topic_members"].as_array_mut().unwrap().push(duplicate);
            }
        }
        let bytes = serde_json::to_vec(&bad).unwrap();
        fs::write(package.join("manifest.json"), &bytes).unwrap();
        fs::write(
            package.join("manifest.sha256"),
            format!("{:x}", Sha256::digest(&bytes)),
        )
        .unwrap();
        assert!(
            prepare_restore(
                &package,
                &root.path().join(format!("bad-{flaw}")),
                &mut |_| Ok(())
            )
            .is_err(),
            "{flaw}"
        );
    }
}
#[test]
fn schema_eight_migration_failure_rolls_back_and_retries_with_consistent_backup() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("upgrade.sqlite3");
    let mut db = Connection::open(&path).unwrap();
    initialize(&mut db, &path).unwrap();
    db.execute_batch("DROP TABLE idea_references;DROP TABLE library_ideas;DROP TABLE research_topic_members;DROP TABLE research_topics;UPDATE library_schema SET version=8;CREATE VIEW research_topics AS SELECT 1;").unwrap();
    assert!(initialize(&mut db, &path).is_err());
    assert_eq!(
        db.query_row("SELECT version FROM library_schema", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        8
    );
    db.execute_batch("DROP VIEW research_topics;").unwrap();
    let backup = initialize(&mut db, &path).unwrap().unwrap();
    let old = Connection::open(backup).unwrap();
    assert_eq!(
        old.query_row("SELECT version FROM library_schema", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        8
    );
    assert_eq!(
        db.query_row("SELECT version FROM library_schema", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        11
    );
}

#[test]
fn relinking_a_file_keeps_old_clip_source_changed_in_topics() {
    let root = tempfile::tempdir().unwrap();
    let db = database();
    let (asset, clip, _) = seed(&db, root.path(), "甲");
    let topic = create(&db);
    add_topic_member(
        &db,
        &topic,
        &member(TopicMemberKind::Clip, &asset, &clip),
        1,
    )
    .unwrap();
    let file = asset_files(&db, &asset).unwrap().remove(0);
    let replacement = root.path().join("新版本.mp4");
    fs::write(&replacement, b"different media source").unwrap();
    relink_file(&db, &file.id, &replacement).unwrap();
    assert!(!asset_files(&db, &asset).unwrap()[0].changed);
    assert!(clips(&db, &asset).unwrap()[0].source_changed);
    assert_eq!(
        research_topic(&db, &topic).unwrap().members[0]
            .file_state
            .as_deref(),
        Some("changed")
    );
    assert!(clip_playback(&db, &asset, &clip).is_err());
}
