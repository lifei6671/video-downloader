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
fn seed(db: &Connection, root: &Path, name: &str) -> (String, String, String, String) {
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
    let document = save_edited(
        db,
        &item.asset_id,
        "这是参考原文😀，不能成为我的草稿。",
        0,
        None,
    )
    .unwrap();
    let clip = create_clip(
        db,
        &item.asset_id,
        &ClipInput {
            file_id: item.file_id,
            title: "参考开头😀".into(),
            reason: String::new(),
            start_ms: 100,
            end_ms: 900,
            playable: ClipRange {
                start_ms: 0,
                end_ms: 1000,
            },
            document_id: Some(document.clone()),
            quote: Some("参考原文😀".into()),
        },
    )
    .unwrap();
    let topic = create_research_topic(
        db,
        &TopicInput {
            title: "参考专题😀".into(),
            description: "参考研究内容".into(),
        },
    )
    .unwrap();
    add_topic_member(
        db,
        &topic,
        &TopicMemberInput {
            kind: TopicMemberKind::Clip,
            asset_id: item.asset_id.clone(),
            resource_id: clip.clone(),
        },
        1,
    )
    .unwrap();
    (item.asset_id, clip, topic, document)
}
fn refs(asset: &str, clip: &str, topic: &str) -> Vec<IdeaReferenceInput> {
    vec![
        IdeaReferenceInput {
            kind: IdeaReferenceKind::Video,
            asset_id: Some(asset.into()),
            resource_id: asset.into(),
        },
        IdeaReferenceInput {
            kind: IdeaReferenceKind::Clip,
            asset_id: Some(asset.into()),
            resource_id: clip.into(),
        },
        IdeaReferenceInput {
            kind: IdeaReferenceKind::Topic,
            asset_id: None,
            resource_id: topic.into(),
        },
    ]
}
fn input(references: Vec<IdeaReferenceInput>) -> IdeaInput {
    IdeaInput {
        title: "自己的选题😀".into(),
        viewpoint: "我的观点与立场".into(),
        body: "我的原创草稿😀\n第二段".into(),
        references,
    }
}
#[test]
fn ideas_keep_original_writing_separate_from_live_sources_and_workbench_summaries() {
    let root = tempfile::tempdir().unwrap();
    let db = database();
    let (asset, clip, topic, document) = seed(&db, root.path(), "甲");
    let id = create_idea(&db, &input(refs(&asset, &clip, &topic))).unwrap();
    let saved = library_idea(&db, &id).unwrap();
    assert_eq!(saved.references.len(), 3);
    assert_eq!(saved.idea.body, "我的原创草稿😀\n第二段");
    assert!(notes(&db, &asset).unwrap().is_empty());
    assert!(read_document(&db, &asset, &document)
        .unwrap()
        .text
        .contains("参考原文"));
    edit_research_topic(
        &db,
        &topic,
        &TopicInput {
            title: "新的专题标题".into(),
            description: "新的研究内容".into(),
        },
        2,
    )
    .unwrap();
    let current = library_idea(&db, &id).unwrap();
    assert_eq!(current.idea.body, saved.idea.body);
    assert_eq!(current.references[2].title, "新的专题标题");
    assert_eq!(current.references[2].topic_revision, Some(3));
    let workbench = workbench_library(&db).unwrap();
    assert_eq!(workbench.ideas.total, 1);
    assert_eq!(workbench.ideas.items[0].reference_count, 3);
    let json = serde_json::to_string(&workbench).unwrap();
    assert!(!json.contains("我的原创草稿"));
    assert!(!json.contains("我的观点"));
    assert!(!json.contains("参考原文"));
}
#[test]
fn idea_references_reject_foreign_owners_wrong_kinds_duplicates_and_unknown_new_sources() {
    let root = tempfile::tempdir().unwrap();
    let db = database();
    let (asset, clip, topic, _) = seed(&db, root.path(), "甲");
    let (other, _, _, _) = seed(&db, root.path(), "乙");
    for bad in [
        IdeaReferenceInput {
            kind: IdeaReferenceKind::Clip,
            asset_id: Some(other.clone()),
            resource_id: clip.clone(),
        },
        IdeaReferenceInput {
            kind: IdeaReferenceKind::Video,
            asset_id: Some(other),
            resource_id: asset.clone(),
        },
        IdeaReferenceInput {
            kind: IdeaReferenceKind::Topic,
            asset_id: Some(asset.clone()),
            resource_id: topic.clone(),
        },
        IdeaReferenceInput {
            kind: IdeaReferenceKind::Video,
            asset_id: None,
            resource_id: asset.clone(),
        },
        IdeaReferenceInput {
            kind: IdeaReferenceKind::Topic,
            asset_id: None,
            resource_id: "unknown".into(),
        },
    ] {
        assert!(prepare_idea_reference(&db, &bad).is_err());
        assert!(create_idea(&db, &input(vec![bad])).is_err());
    }
    let mut duplicate = refs(&asset, &clip, &topic);
    duplicate.push(duplicate[0].clone());
    assert!(create_idea(&db, &input(duplicate)).is_err());
    assert!(idea_summaries(&db).unwrap().is_empty());
    let mut too_long = input(vec![]);
    too_long.viewpoint = "长".repeat(65537);
    assert!(create_idea(&db, &too_long).is_err());
}
#[test]
fn idea_body_and_references_commit_atomically_with_revision_conflicts() {
    let root = tempfile::tempdir().unwrap();
    let db = database();
    let (asset, clip, topic, _) = seed(&db, root.path(), "甲");
    let mut content = input(refs(&asset, &clip, &topic));
    db.execute_batch("CREATE TRIGGER fail_ref BEFORE INSERT ON idea_references BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
    assert!(create_idea(&db, &content).is_err());
    assert!(idea_summaries(&db).unwrap().is_empty());
    db.execute_batch("DROP TRIGGER fail_ref;").unwrap();
    let id = create_idea(&db, &content).unwrap();
    let old = serde_json::to_value(library_idea(&db, &id).unwrap()).unwrap();
    content.body = "不应半保存".into();
    content.references.reverse();
    db.execute_batch("CREATE TRIGGER fail_ref BEFORE INSERT ON idea_references BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
    assert!(edit_idea(&db, &id, &content, 1).is_err());
    assert_eq!(
        serde_json::to_value(library_idea(&db, &id).unwrap()).unwrap(),
        old
    );
    db.execute_batch("DROP TRIGGER fail_ref;").unwrap();
    edit_idea(&db, &id, &content, 1).unwrap();
    assert!(edit_idea(&db, &id, &content, 1).is_err());
    assert_eq!(library_idea(&db, &id).unwrap().idea.revision, 2);
}
#[test]
fn removing_idea_references_does_not_delete_source_material() {
    let root = tempfile::tempdir().unwrap();
    let db = database();
    let (asset, clip, topic, doc) = seed(&db, root.path(), "甲");
    let id = create_idea(&db, &input(refs(&asset, &clip, &topic))).unwrap();
    edit_idea(&db, &id, &input(vec![]), 1).unwrap();
    assert!(library_idea(&db, &id).unwrap().references.is_empty());
    assert!(clip_playback(&db, &asset, &clip).is_ok());
    assert!(research_topic(&db, &topic).is_ok());
    assert!(read_document(&db, &asset, &doc).is_ok());
}
#[test]
fn archived_changed_and_missing_references_allow_original_draft_edits_without_rebinding() {
    let root = tempfile::tempdir().unwrap();
    let db = database();
    let (asset, clip, topic, _) = seed(&db, root.path(), "甲");
    let mut content = input(refs(&asset, &clip, &topic));
    let id = create_idea(&db, &content).unwrap();
    archive_asset(&db, &asset, true).unwrap();
    let file = asset_files(&db, &asset).unwrap().remove(0);
    let replacement = root.path().join("replacement.mp4");
    fs::write(&replacement, b"different source").unwrap();
    relink_file(&db, &file.id, &replacement).unwrap();
    let detail = library_idea(&db, &id).unwrap();
    assert!(detail.references[0].archived);
    assert_eq!(detail.references[1].file_state.as_deref(), Some("changed"));
    assert!(clip_playback(&db, &asset, &clip).is_err());
    delete_clip(&db, &asset, &clip).unwrap();
    db.execute("DELETE FROM research_topics WHERE id=?1", [&topic])
        .unwrap();
    content.body = "来源缺失后自己的新稿😀".into();
    edit_idea(&db, &id, &content, 1).unwrap();
    let detail = library_idea(&db, &id).unwrap();
    assert!(detail.references[1].missing && detail.references[2].missing);
    assert!(detail.references[1].title.contains("参考开头"));
    assert_eq!(detail.idea.body, content.body);
    assert!(create_idea(&db, &content).is_err());
    db.execute("DELETE FROM library_assets WHERE id=?1", [&asset])
        .unwrap();
    edit_idea(&db, &id, &content, 2).unwrap();
    assert!(library_idea(&db, &id)
        .unwrap()
        .references
        .iter()
        .all(|r| r.missing));
}
#[test]
fn original_drafts_roundtrip_twice_with_reference_order_and_unavailable_sources() {
    let root = tempfile::tempdir().unwrap();
    let db = database();
    let (asset, clip, topic, _) = seed(&db, root.path(), "甲");
    let mut content = input(refs(&asset, &clip, &topic));
    content.references.reverse();
    let id = create_idea(&db, &content).unwrap();
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
        assert_eq!(report.ideas, 1);
        let before = library_idea(&db, &id).unwrap();
        let after = library_idea(&restored, &id).unwrap();
        assert_eq!(
            serde_json::to_value(before.idea).unwrap(),
            serde_json::to_value(after.idea).unwrap()
        );
        assert_eq!(
            before
                .references
                .iter()
                .map(|r| &r.reference)
                .collect::<Vec<_>>(),
            after
                .references
                .iter()
                .map(|r| &r.reference)
                .collect::<Vec<_>>()
        );
        assert!(after.references[1].missing);
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
fn idea_only_libraries_are_nonempty_recent_summaries_are_bounded_and_bad_packages_fail() {
    use sha2::{Digest, Sha256};
    let root = tempfile::tempdir().unwrap();
    let db = database();
    for i in 0..8 {
        let mut value = input(vec![]);
        value.title = format!("选题{i}");
        create_idea(&db, &value).unwrap();
    }
    assert!(require_empty_library(&db).is_err());
    let workbench = workbench_library(&db).unwrap();
    assert_eq!(workbench.ideas.total, 8);
    assert_eq!(workbench.ideas.items.len(), 6);
    let (asset, clip, topic, _) = seed(&db, root.path(), "甲");
    let (other, _, _, _) = seed(&db, root.path(), "乙");
    create_idea(&db, &input(refs(&asset, &clip, &topic))).unwrap();
    let package = root.path().join("package");
    write_backup(capture_backup(&db, false).unwrap(), &package, &mut |_| {
        Ok(())
    })
    .unwrap();
    let original: serde_json::Value =
        serde_json::from_slice(&fs::read(package.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(original["version"], 9);
    for flaw in [
        "old",
        "owner",
        "order",
        "parent",
        "topic_asset",
        "duplicate",
        "title",
        "revision",
    ] {
        let mut bad = original.clone();
        match flaw {
            "old" => bad["version"] = json!(7),
            "owner" => {
                let r = bad["idea_references"]
                    .as_array_mut()
                    .unwrap()
                    .iter_mut()
                    .find(|r| r["kind"] == "clip")
                    .unwrap();
                r["asset_id"] = json!(other);
            }
            "order" => bad["idea_references"][0]["position"] = json!(20),
            "parent" => bad["idea_references"][0]["idea_id"] = json!("foreign"),
            "topic_asset" => {
                let r = bad["idea_references"]
                    .as_array_mut()
                    .unwrap()
                    .iter_mut()
                    .find(|r| r["kind"] == "topic")
                    .unwrap();
                r["asset_id"] = json!(asset);
            }
            "duplicate" => {
                let r = bad["idea_references"][0].clone();
                bad["idea_references"].as_array_mut().unwrap().push(r);
            }
            "title" => bad["ideas"][0]["title"] = json!(""),
            _ => bad["ideas"][0]["revision"] = json!(0),
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
fn schema_nine_upgrade_backs_up_and_retries_without_losing_topics() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("upgrade.sqlite3");
    let mut db = Connection::open(&path).unwrap();
    initialize(&mut db, &path).unwrap();
    let topic = create_research_topic(
        &db,
        &TopicInput {
            title: "保留专题".into(),
            description: String::new(),
        },
    )
    .unwrap();
    db.execute_batch("ALTER TABLE research_topics DROP COLUMN ai_source_json;DROP TABLE idea_references;DROP TABLE library_ideas;UPDATE library_schema SET version=9;CREATE VIEW library_ideas AS SELECT 1;").unwrap();
    assert!(initialize(&mut db, &path).is_err());
    assert_eq!(
        db.query_row("SELECT version FROM library_schema", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        9
    );
    db.execute_batch("DROP VIEW library_ideas;").unwrap();
    let backup = initialize(&mut db, &path).unwrap().unwrap();
    let old = Connection::open(backup).unwrap();
    assert_eq!(
        old.query_row("SELECT version FROM library_schema", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        9
    );
    assert_eq!(
        db.query_row("SELECT version FROM library_schema", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        11
    );
    assert_eq!(research_topic(&db, &topic).unwrap().topic.title, "保留专题");
}
