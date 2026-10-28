use super::*;
use crate::desktop_core::transcription::Source;
use serde_json::json;
use std::{fs, path::Path};

fn database() -> Connection {
    let mut db = Connection::open_in_memory().unwrap();
    initialize(&mut db, Path::new(":memory:")).unwrap();
    db
}
fn seed(db: &Connection, root: &Path, name: &str) -> ComparisonPin {
    let path = root.join(format!("{name}.mp4"));
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
    let doc = save_edited(
        db,
        &item.asset_id,
        &format!("{name}的原文😀\n第二段"),
        0,
        None,
    )
    .unwrap();
    let id = create_analysis(
        db,
        &item.asset_id,
        &NewAnalysis {
            content: copy(name),
            origin: NoteOrigin::Ai,
            model_name: Some("合成模型".into()),
            citations: vec![],
        },
    )
    .unwrap();
    let clip = create_clip(
        db,
        &item.asset_id,
        &ClipInput {
            file_id: item.file_id,
            title: format!("{name}片段"),
            reason: String::new(),
            start_ms: 100,
            end_ms: 900,
            playable: ClipRange {
                start_ms: 0,
                end_ms: 1000,
            },
            document_id: Some(doc.clone()),
            quote: Some(format!("{name}的原文😀")),
        },
    )
    .unwrap();
    ComparisonPin {
        asset_id: item.asset_id,
        document_id: Some(doc),
        copywriting: Some(comparison::RevisionRef { id, revision: 1 }),
        technique: None,
        clip_ids: vec![clip],
    }
}
fn copy(text: &str) -> AnalysisInput {
    AnalysisInput::Copywriting {
        opening: text.into(),
        viewpoint: String::new(),
        evidence: String::new(),
        turn: String::new(),
        ending: String::new(),
        takeaways: String::new(),
    }
}
fn input(pins: &[ComparisonPin]) -> NewComparison {
    serde_json::from_value(json!({"title":"对比😀","asset_ids":pins.iter().map(|p|&p.asset_id).collect::<Vec<_>>(),"points":[{"text":"两种开头的区别","sources":pins}]})).unwrap()
}
#[test]
fn comparison_sources_keep_each_asset_and_unknown_dimension_separate() {
    let root = tempfile::tempdir().unwrap();
    let db = database();
    let a = seed(&db, root.path(), "甲");
    let mut b = seed(&db, root.path(), "乙");
    b.copywriting = None;
    b.document_id = None;
    b.clip_ids.clear();
    let id = create_comparison(&db, &a.asset_id, &input(&[a.clone(), b.clone()])).unwrap();
    let note = read_note(&db, &a.asset_id, &id).unwrap();
    let data = note.comparison.as_ref().unwrap();
    assert!(data.points[0].sources[1].document.is_none());
    assert!(data.points[0].sources[1].copywriting.is_none());
    assert_eq!(
        data.points[0].sources[0]
            .copywriting
            .as_ref()
            .unwrap()
            .structure
            .as_ref()
            .unwrap()[1],
        ""
    );
    assert!(note.citations.iter().all(|c| c.asset_id == a.asset_id));
    assert!(note.body.contains("未知"));
    assert!(note.body.contains("甲的原文😀"));
    comparison::validate_note(&db, &note).unwrap();
    let value = crate::mcp_read::dispatch(
        &db,
        "mcp_read_note",
        json!({"asset_id":a.asset_id,"note_id":id}),
    )
    .unwrap();
    assert!(value.to_string().contains("两种开头的区别"));
    assert!(!value.to_string().contains(root.path().to_str().unwrap()));
    assert!(edit_note(&db, &id, "绕过结构编辑", 1).is_err());
}
#[test]
fn comparison_rejects_foreign_ids_duplicate_members_and_stale_revisions_without_writing() {
    let root = tempfile::tempdir().unwrap();
    let db = database();
    let a = seed(&db, root.path(), "甲");
    let b = seed(&db, root.path(), "乙");
    for kind in ["doc", "note", "clip", "revision"] {
        let mut wrong = a.clone();
        match kind {
            "doc" => wrong.document_id = b.document_id.clone(),
            "note" => wrong.copywriting = b.copywriting.clone(),
            "clip" => wrong.clip_ids = b.clip_ids.clone(),
            _ => wrong.copywriting.as_mut().unwrap().revision = 2,
        }
        assert!(
            create_comparison(&db, &a.asset_id, &input(&[wrong, b.clone()])).is_err(),
            "{kind}"
        );
    }
    assert!(create_comparison(&db, &a.asset_id, &input(&[a.clone(), a.clone()])).is_err());
    let mut wrong = input(&[a.clone(), b.clone()]);
    wrong.points[0].sources.push(a.clone());
    assert!(create_comparison(&db, &a.asset_id, &wrong).is_err());
    edit_analysis(
        &db,
        &a.asset_id,
        &a.copywriting.as_ref().unwrap().id,
        &copy("更新了甲"),
        1,
    )
    .unwrap();
    assert_eq!(
        create_comparison(&db, &a.asset_id, &input(&[a.clone(), b]))
            .unwrap_err()
            .to_string(),
        "COMPARISON_SOURCE_CHANGED"
    );
    assert_eq!(
        notes(&db, &a.asset_id)
            .unwrap()
            .iter()
            .filter(|n| n.kind == "comparison")
            .count(),
        0
    );
}
#[test]
fn saved_comparison_edits_keep_old_source_snapshots_and_are_atomic() {
    let root = tempfile::tempdir().unwrap();
    let db = database();
    let a = seed(&db, root.path(), "甲");
    let b = seed(&db, root.path(), "乙");
    let id = create_comparison(&db, &a.asset_id, &input(&[a.clone(), b])).unwrap();
    let old = read_note(&db, &a.asset_id, &id)
        .unwrap()
        .comparison
        .unwrap()
        .points
        .remove(0)
        .sources;
    edit_analysis(
        &db,
        &a.asset_id,
        &a.copywriting.as_ref().unwrap().id,
        &copy("新的甲开头"),
        1,
    )
    .unwrap();
    delete_clip(&db, &a.asset_id, &a.clip_ids[0]).unwrap();
    let edit = ComparisonEdit {
        title: "新标题".into(),
        point_texts: vec!["新的结论".into()],
    };
    edit_comparison(&db, &a.asset_id, &id, &edit, 1).unwrap();
    let saved = read_note(&db, &a.asset_id, &id).unwrap();
    assert_eq!(
        serde_json::to_value(&old).unwrap(),
        serde_json::to_value(&saved.comparison.as_ref().unwrap().points[0].sources).unwrap()
    );
    comparison::validate_note(&db, &saved).unwrap();
    assert!(edit_comparison(&db, &a.asset_id, &id, &edit, 1).is_err());
    db.execute_batch("CREATE TRIGGER fail_compare BEFORE UPDATE ON note_comparisons BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
    assert!(edit_comparison(&db, &a.asset_id, &id, &edit, 2).is_err());
    assert_eq!(read_note(&db, &a.asset_id, &id).unwrap().revision, 2);
}
#[test]
fn comparison_backup_roundtrips_and_rejects_source_graph_tampering() {
    use sha2::{Digest, Sha256};
    let root = tempfile::tempdir().unwrap();
    let db = database();
    let a = seed(&db, root.path(), "甲");
    let b = seed(&db, root.path(), "乙");
    let id = create_comparison(&db, &a.asset_id, &input(&[a.clone(), b.clone()])).unwrap();
    edit_analysis(
        &db,
        &a.asset_id,
        &a.copywriting.as_ref().unwrap().id,
        &copy("修订后"),
        1,
    )
    .unwrap();
    for media in [false, true] {
        let package = root.path().join(format!("package-{media}"));
        write_backup(capture_backup(&db, media).unwrap(), &package, &mut |_| {
            Ok(())
        })
        .unwrap();
        let original: serde_json::Value =
            serde_json::from_slice(&fs::read(package.join("manifest.json")).unwrap()).unwrap();
        assert_eq!(original["version"], 9);
        let mut restored = database();
        let prepared = prepare_restore(
            &package,
            &root.path().join(format!("restored-{media}")),
            &mut |_| Ok(()),
        )
        .unwrap();
        commit_restore(&mut restored, prepared).unwrap();
        assert_eq!(
            read_note(&db, &a.asset_id, &id).unwrap().body,
            read_note(&restored, &a.asset_id, &id).unwrap().body
        );
        let again = root.path().join(format!("again-{media}"));
        write_backup(
            capture_backup(&restored, media).unwrap(),
            &again,
            &mut |_| Ok(()),
        )
        .unwrap();
        prepare_restore(
            &again,
            &root.path().join(format!("restore-again-{media}")),
            &mut |_| Ok(()),
        )
        .unwrap();
        for flaw in ["old", "owner", "doc", "clip", "revision", "body"] {
            let mut bad = original.clone();
            let note = bad["notes"]
                .as_array_mut()
                .unwrap()
                .iter_mut()
                .find(|n| n["id"] == id)
                .unwrap();
            match flaw {
                "old" => bad["version"] = json!(5),
                "owner" => {
                    note["comparison"]["points"][0]["sources"][0]["pin"]["asset_id"] =
                        json!(b.asset_id)
                }
                "doc" => {
                    note["comparison"]["points"][0]["sources"][0]["document"]["version"] = json!(88)
                }
                "clip" => {
                    note["comparison"]["points"][0]["sources"][0]["clips"][0]["file_id"] =
                        json!("foreign")
                }
                "revision" => {
                    note["comparison"]["points"][0]["sources"][0]["copywriting"]["revision"] =
                        json!(99)
                }
                _ => note["body"] = json!("forged"),
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
                    &root.path().join(format!("invalid-{media}-{flaw}")),
                    &mut |_| Ok(())
                )
                .is_err(),
                "{flaw}"
            );
        }
    }
}
#[test]
fn schema_seven_upgrade_creates_consistent_backup_and_comparison_table() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("upgrade.sqlite3");
    let mut db = Connection::open(&path).unwrap();
    initialize(&mut db, &path).unwrap();
    db.execute_batch("DROP TABLE idea_references;DROP TABLE library_ideas;DROP TABLE research_topic_members;DROP TABLE research_topics;DROP TABLE note_comparisons; UPDATE library_schema SET version=7;")
        .unwrap();
    let backup = initialize(&mut db, &path).unwrap().unwrap();
    let old = Connection::open(backup).unwrap();
    assert_eq!(
        old.query_row("SELECT version FROM library_schema", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        7
    );
    assert_eq!(
        db.query_row("SELECT version FROM library_schema", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        11
    );
    assert!(initialize(&mut db, &path).unwrap().is_none());
}

#[test]
fn historical_snapshots_reject_impossible_revisions_and_malformed_dimensions() {
    let root = tempfile::tempdir().unwrap();
    let db = database();
    let a = seed(&db, root.path(), "甲");
    let b = seed(&db, root.path(), "乙");
    let id = create_comparison(&db, &a.asset_id, &input(&[a.clone(), b])).unwrap();
    edit_analysis(
        &db,
        &a.asset_id,
        &a.copywriting.as_ref().unwrap().id,
        &copy("新版本"),
        1,
    )
    .unwrap();
    for flaw in ["zero", "empty_clip", "duplicates", "fields", "excerpt"] {
        let mut note = read_note(&db, &a.asset_id, &id).unwrap();
        let data = note.comparison.as_mut().unwrap();
        let source = &mut data.points[0].sources[0];
        match flaw {
            "zero" => {
                source.pin.copywriting.as_mut().unwrap().revision = 0;
                source.copywriting.as_mut().unwrap().revision = 0;
            }
            "empty_clip" => {
                source.pin.clip_ids[0] = String::new();
                source.clips[0].id = String::new();
            }
            "duplicates" => {
                source.pin.clip_ids.push(source.pin.clip_ids[0].clone());
                source.clips.push(source.clips[0].clone());
            }
            "fields" => source.copywriting.as_mut().unwrap().structure = Some(vec!["伪造".into()]),
            _ => source.copywriting.as_mut().unwrap().excerpt = "长".repeat(2001),
        }
        note.body = data.body();
        assert!(comparison::validate_note(&db, &note).is_err(), "{flaw}");
    }
}
