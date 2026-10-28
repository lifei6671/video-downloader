use super::*;
use crate::desktop_core::transcription::Source;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{fs, path::Path};

fn database() -> Connection {
    let mut db = Connection::open_in_memory().unwrap();
    initialize(&mut db, Path::new(":memory:")).unwrap();
    db
}
fn seed(db: &Connection, root: &Path) -> (String, String, String, String) {
    let path = root.join("文案范围😀.mp4");
    fs::write(&path, b"fixture media").unwrap();
    let source = Source::read(&path).unwrap();
    let imported = register_local(
        db,
        &source,
        &LocalMetadata {
            width: 90,
            height: 160,
            duration_ms: Some(1000),
        },
    )
    .unwrap();
    let doc = save_edited(db, &imported.asset_id, "固定原文😀\n第二段", 0, None).unwrap();
    let clip = create_clip(
        db,
        &imported.asset_id,
        &ClipInput {
            file_id: imported.file_id.clone(),
            title: "开头范围".into(),
            reason: String::new(),
            start_ms: 100,
            end_ms: 900,
            playable: ClipRange {
                start_ms: 0,
                end_ms: 1000,
            },
            document_id: Some(doc.clone()),
            quote: Some("固定原文😀".into()),
        },
    )
    .unwrap();
    let mut body = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut body, 1, 1);
        encoder.set_color(png::ColorType::Rgb);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&[0, 0, 0])
            .unwrap();
    }
    keyframes::insert_frame(
        db,
        &KeyframeData {
            id: "analysis-frame".into(),
            asset_id: imported.asset_id.clone(),
            file_id: imported.file_id.clone(),
            clip_id: Some(clip.clone()),
            batch_id: "analysis-batch".into(),
            sample_start_ms: 100,
            sample_end_ms: Some(900),
            sample_index: 0,
            sample_count: 1,
            requested_ms: 500,
            pts: 500,
            time_base_num: 1,
            time_base_den: 1000,
            width: 1,
            height: 1,
            black: true,
            created_at: 1,
        },
        &source,
        &body,
    )
    .unwrap();
    (imported.asset_id, imported.file_id, doc, clip)
}
fn copy(text: &str) -> AnalysisInput {
    AnalysisInput::Copywriting {
        opening: text.into(),
        viewpoint: "观点".into(),
        evidence: String::new(),
        turn: String::new(),
        ending: String::new(),
        takeaways: "可借鉴点".into(),
    }
}
fn technique(clip: &str, frame: bool) -> AnalysisInput {
    AnalysisInput::Technique {
        observations: vec![ObservationInput {
            dimension: ObservationKind::Visual,
            evidence: if frame {
                ObservationEvidence::Keyframe
            } else {
                ObservationEvidence::Video
            },
            text: "画面观察😀".into(),
            clip_id: clip.into(),
            keyframe_id: frame.then(|| "analysis-frame".into()),
        }],
    }
}
fn input(content: AnalysisInput, ai: bool) -> NewAnalysis {
    NewAnalysis {
        content,
        origin: if ai {
            NoteOrigin::Ai
        } else {
            NoteOrigin::Human
        },
        model_name: ai.then(|| "合成模型".into()),
        citations: vec![],
    }
}

#[test]
fn templates_provenance_versions_and_manual_revision_remain_distinct() {
    let root = tempfile::tempdir().unwrap();
    let db = database();
    let (asset, _, doc, _) = seed(&db, root.path());
    let mut create = input(copy("开头😀"), true);
    create.citations = vec![Citation {
        asset_id: asset.clone(),
        document_id: doc.clone(),
        quote: "固定原文😀".into(),
    }];
    let id = create_analysis(&db, &asset, &create).unwrap();
    save_edited(&db, &asset, "新版本不改变引用", 1, Some(&doc)).unwrap();
    let note = read_note(&db, &asset, &id).unwrap();
    assert!(matches!(note.origin, NoteOrigin::Ai));
    assert_eq!(note.model_name.as_deref(), Some("合成模型"));
    assert!(!note.analysis.unwrap().human_revised);
    assert!(note.body.contains("## 论据\n未记录"));
    assert_eq!(note.citations[0].document_id, doc);
    assert_eq!(
        edit_note(&db, &id, "不能将结构化笔记静默改为散文", 1)
            .unwrap_err()
            .to_string(),
        "ANALYSIS_REQUIRED"
    );
    edit_analysis(&db, &asset, &id, &copy("人工修订"), 1).unwrap();
    let note = read_note(&db, &asset, &id).unwrap();
    assert_eq!(note.revision, 2);
    assert!(note.analysis.unwrap().human_revised);
    assert!(matches!(note.origin, NoteOrigin::Ai));
    assert_eq!(
        edit_analysis(&db, &asset, &id, &copy("过期"), 1)
            .unwrap_err()
            .to_string(),
        "NOTE_CONFLICT"
    );
    assert!(read_note(&db, &asset, &id)
        .unwrap()
        .body
        .contains("人工修订"));
    let human = create_analysis(&db, &asset, &input(copy("人工观察"), false)).unwrap();
    let human = read_note(&db, &asset, &human).unwrap();
    assert!(matches!(human.origin, NoteOrigin::Human));
    assert!(human.model_name.is_none());
}
#[test]
fn existing_mcp_read_views_keep_structured_body_unicode_origin_and_read_only_boundaries() {
    let root = tempfile::tempdir().unwrap();
    let db = database();
    let (asset, _, doc, _) = seed(&db, root.path());
    let mut data = input(copy("MCP结构😀"), true);
    data.citations = vec![Citation {
        asset_id: asset.clone(),
        document_id: doc,
        quote: "固定原文😀".into(),
    }];
    let id = create_analysis(&db, &asset, &data).unwrap();
    let value =
        crate::mcp_read::dispatch(&db, "mcp_read_note", json!({"asset_id":asset,"note_id":id}))
            .unwrap();
    assert_eq!(value["summary"]["origin"], "ai");
    assert_eq!(value["summary"]["kind"], "copywriting_analysis");
    assert!(value["content"]["text"]
        .as_str()
        .unwrap()
        .contains("MCP结构😀"));
    assert_eq!(value["citations"].as_array().unwrap().len(), 1);
    let text = read_note(&db, &asset, &id).unwrap().body;
    let offset = text.chars().position(|c| c == '😀').unwrap();
    let slice = crate::mcp_read::dispatch(
        &db,
        "mcp_read_note",
        json!({"asset_id":asset,"note_id":id,"offset":offset,"length":1}),
    )
    .unwrap();
    assert_eq!(slice["content"]["text"], "😀");
    assert_eq!(slice["reference"]["offset"], offset);
    assert!(value.get("analysis").is_none());
    assert!(!value.to_string().contains("source_json"));
    assert!(!value.to_string().contains("mtime_ns"));
    assert!(crate::mcp_read::dispatch(&db, "mcp_create_analysis", json!({})).is_err());
}
#[test]
fn model_evidence_and_empty_templates_are_validated_at_the_boundary() {
    let root = tempfile::tempdir().unwrap();
    let db = database();
    let (asset, _, _, clip) = seed(&db, root.path());
    let mut ai = input(copy("分析"), true);
    ai.model_name = None;
    assert_eq!(
        create_analysis(&db, &asset, &ai).unwrap_err().to_string(),
        "ANALYSIS_MODEL_REQUIRED"
    );
    let mut human = input(copy("分析"), false);
    human.model_name = Some("不能伪装模型".into());
    assert!(create_analysis(&db, &asset, &human).is_err());
    let empty:AnalysisInput=serde_json::from_value(json!({"kind":"copywriting","opening":"","viewpoint":"","evidence":"","turn":"","ending":"","takeaways":""})).unwrap();
    assert!(create_analysis(&db, &asset, &input(empty, false)).is_err());
    let mut sound = technique(&clip, true);
    if let AnalysisInput::Technique { observations } = &mut sound {
        observations[0].dimension = ObservationKind::Sound;
    }
    assert_eq!(
        create_analysis(&db, &asset, &input(sound, false))
            .unwrap_err()
            .to_string(),
        "ANALYSIS_EVIDENCE_INVALID"
    );
    assert!(serde_json::from_value::<AnalysisInput>(
        json!({"kind":"copywriting","shell":"echo bad"})
    )
    .is_err());
}
#[test]
fn references_preserve_time_and_remain_readable_when_media_or_resources_disappear() {
    let root = tempfile::tempdir().unwrap();
    let db = database();
    let (asset, _, _, clip) = seed(&db, root.path());
    let id = create_analysis(&db, &asset, &input(technique(&clip, true), false)).unwrap();
    fs::remove_file(root.path().join("文案范围😀.mp4")).unwrap();
    edit_analysis(&db, &asset, &id, &technique(&clip, true), 1).unwrap();
    delete_keyframe(&db, &asset, "analysis-frame").unwrap();
    edit_analysis(&db, &asset, &id, &technique(&clip, true), 2).unwrap();
    delete_clip(&db, &asset, &clip).unwrap();
    edit_analysis(&db, &asset, &id, &technique(&clip, true), 3).unwrap();
    let note = read_note(&db, &asset, &id).unwrap();
    if let AnalysisContent::Technique { observations } = &note.analysis.unwrap().content {
        assert_eq!(observations[0].start_ms, 100);
        assert_eq!(observations[0].end_ms, 900);
        assert_eq!(observations[0].frame_actual_ms, Some(500));
    } else {
        panic!()
    }
    assert!(create_analysis(&db, &asset, &input(technique(&clip, true), false)).is_err());
}
#[test]
fn references_reject_foreign_assets_wrong_ranges_and_changed_frame_sources() {
    let root = tempfile::tempdir().unwrap();
    let db = database();
    let (asset, _, _, clip) = seed(&db, root.path());
    let second = root.path().join("second");
    fs::create_dir(&second).unwrap();
    let source = Source::read(&root.path().join("文案范围😀.mp4")).unwrap();
    let path = second.join("other.mp4");
    fs::write(&path, b"other").unwrap();
    let other = register_local(
        &db,
        &Source::read(&path).unwrap(),
        &LocalMetadata {
            width: 1,
            height: 1,
            duration_ms: Some(1000),
        },
    )
    .unwrap();
    assert!(create_analysis(&db, &other.asset_id, &input(technique(&clip, true), false)).is_err());
    let mut altered = source.clone();
    altered.mtime_ns = "changed".into();
    db.execute(
        "UPDATE asset_keyframes SET source_json=?1",
        [serde_json::to_string(&altered).unwrap()],
    )
    .unwrap();
    assert_eq!(
        create_analysis(&db, &asset, &input(technique(&clip, true), false))
            .unwrap_err()
            .to_string(),
        "ANALYSIS_REFERENCE_INVALID"
    );
    db.execute(
        "UPDATE asset_keyframes SET source_json=?1",
        [serde_json::to_string(&source).unwrap()],
    )
    .unwrap();
    db.execute("UPDATE asset_clips SET end_ms=499 WHERE id=?1", [&clip])
        .unwrap();
    assert!(create_analysis(&db, &asset, &input(technique(&clip, true), false)).is_err());
}
#[test]
fn frame_references_use_original_pts_even_when_displayed_milliseconds_round_to_the_end() {
    let root = tempfile::tempdir().unwrap();
    let db = database();
    let (asset, _, _, clip) = seed(&db, root.path());
    let mut data: KeyframeData = serde_json::from_str(
        &db.query_row(
            "SELECT metadata FROM asset_keyframes WHERE id='analysis-frame'",
            [],
            |r| r.get::<_, String>(0),
        )
        .unwrap(),
    )
    .unwrap();
    data.id = "edge-frame".into();
    data.clip_id = None;
    data.sample_start_ms = 899;
    data.sample_end_ms = None;
    data.requested_ms = 899;
    data.pts = 8996;
    data.time_base_den = 10000;
    let source = Source::read(&root.path().join("文案范围😀.mp4")).unwrap();
    let body = keyframe_image(&db, &asset, "analysis-frame").unwrap();
    keyframes::insert_frame(&db, &data, &source, &body).unwrap();
    let mut content = technique(&clip, true);
    if let AnalysisInput::Technique { observations } = &mut content {
        observations[0].keyframe_id = Some("edge-frame".into());
    }
    let id = create_analysis(&db, &asset, &input(content, false)).unwrap();
    let note = read_note(&db, &asset, &id).unwrap();
    analysis::validate_note(&db, &note).unwrap();
    assert!(note.body.contains("关键帧：00:00.900"));
}
#[test]
fn note_and_structured_data_commit_atomically_and_failed_edits_keep_revision() {
    let root = tempfile::tempdir().unwrap();
    let db = database();
    let (asset, _, _, _) = seed(&db, root.path());
    db.execute_batch("CREATE TRIGGER fail_analysis BEFORE INSERT ON note_analyses BEGIN SELECT RAISE(ABORT,'fixture');END;").unwrap();
    assert!(create_analysis(&db, &asset, &input(copy("不能半保存"), false)).is_err());
    assert!(notes(&db, &asset).unwrap().is_empty());
    db.execute_batch("DROP TRIGGER fail_analysis;").unwrap();
    let id = create_analysis(&db, &asset, &input(copy("旧内容"), false)).unwrap();
    db.execute_batch("CREATE TRIGGER fail_analysis BEFORE UPDATE ON note_analyses BEGIN SELECT RAISE(ABORT,'fixture');END;").unwrap();
    assert!(edit_analysis(&db, &asset, &id, &copy("不能半修改"), 1).is_err());
    let saved = read_note(&db, &asset, &id).unwrap();
    assert_eq!(saved.revision, 1);
    assert!(saved.body.contains("旧内容"));
}
#[test]
fn structured_notes_roundtrip_twice_with_or_without_media_and_export_provenance() {
    let root = tempfile::tempdir().unwrap();
    let db = database();
    let (asset, _, _, clip) = seed(&db, root.path());
    let id = create_analysis(&db, &asset, &input(technique(&clip, true), true)).unwrap();
    edit_analysis(&db, &asset, &id, &technique(&clip, true), 1).unwrap();
    let value: serde_json::Value = serde_json::from_str(
        &export_content(
            &db,
            &asset,
            &ContentSelection::Note(id.clone()),
            ContentFormat::Json,
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(value["content"]["origin"], "ai");
    assert_eq!(value["content"]["analysis"]["human_revised"], true);
    for media in [false, true] {
        let package = root.path().join(format!("package-{media}"));
        write_backup(capture_backup(&db, media).unwrap(), &package, &mut |_| {
            Ok(())
        })
        .unwrap();
        let bytes = fs::read(package.join("manifest.json")).unwrap();
        let manifest: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(manifest["version"], 9);
        assert!(!String::from_utf8(bytes)
            .unwrap()
            .contains(&root.path().to_string_lossy().replace('\\', "\\\\")));
        let mut restored = database();
        let prepared = prepare_restore(
            &package,
            &root.path().join(format!("restored-{media}")),
            &mut |_| Ok(()),
        )
        .unwrap();
        commit_restore(&mut restored, prepared).unwrap();
        assert_eq!(
            read_note(&restored, &asset, &id).unwrap().body,
            read_note(&db, &asset, &id).unwrap().body
        );
        let second = root.path().join(format!("again-{media}"));
        write_backup(
            capture_backup(&restored, media).unwrap(),
            &second,
            &mut |_| Ok(()),
        )
        .unwrap();
        prepare_restore(
            &second,
            &root.path().join(format!("again-restore-{media}")),
            &mut |_| Ok(()),
        )
        .unwrap();
    }
}
#[test]
fn invalid_backup_metadata_cannot_publish_or_downgrade_structured_notes() {
    let root = tempfile::tempdir().unwrap();
    let db = database();
    let (asset, _, _, clip) = seed(&db, root.path());
    create_analysis(&db, &asset, &input(technique(&clip, true), false)).unwrap();
    let package = root.path().join("package");
    write_backup(capture_backup(&db, false).unwrap(), &package, &mut |_| {
        Ok(())
    })
    .unwrap();
    let original: serde_json::Value =
        serde_json::from_slice(&fs::read(package.join("manifest.json")).unwrap()).unwrap();
    for flaw in [
        "old",
        "range",
        "frame-time",
        "body",
        "provenance",
        "timebase",
    ] {
        let mut value = original.clone();
        let observation = &mut value["notes"][0]["analysis"]["content"]["observations"][0];
        match flaw {
            "old" => value["version"] = 4.into(),
            "range" => observation["end_ms"] = 999.into(),
            "frame-time" => observation["frame_actual_ms"] = 600.into(),
            "body" => value["notes"][0]["body"] = "伪造正文".into(),
            "provenance" => value["notes"][0]["origin"] = "ai".into(),
            _ => value["keyframes"][0]["data"]["time_base_den"] = 0.into(),
        }
        let bytes = serde_json::to_vec(&value).unwrap();
        fs::write(package.join("manifest.json"), &bytes).unwrap();
        fs::write(
            package.join("manifest.sha256"),
            format!("{:x}", Sha256::digest(&bytes)),
        )
        .unwrap();
        let target = root.path().join(flaw);
        assert!(
            prepare_restore(&package, &target, &mut |_| Ok(())).is_err(),
            "{flaw}"
        );
        assert!(!target.exists());
    }
}
#[test]
fn v6_upgrade_is_backed_up_and_failed_migration_can_retry_without_losing_notes() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("old.sqlite");
    let mut db = Connection::open(&path).unwrap();
    initialize(&mut db, &path).unwrap();
    let (asset, _, _, _) = seed(&db, root.path());
    let id = create_note(
        &db,
        &asset,
        NewNote {
            kind: "note",
            body: "旧笔记",
            origin: NoteOrigin::Human,
            model_name: None,
            citations: &[],
        },
    )
    .unwrap();
    db.execute_batch("DROP TABLE idea_references;DROP TABLE library_ideas;DROP TABLE research_topic_members;DROP TABLE research_topics;DROP TABLE note_comparisons;DROP TABLE note_analyses;UPDATE library_schema SET version=6;CREATE VIEW note_analyses AS SELECT 1;").unwrap();
    assert!(initialize(&mut db, &path).is_err());
    assert_eq!(
        db.query_row("SELECT version FROM library_schema", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        6
    );
    db.execute_batch("DROP VIEW note_analyses").unwrap();
    let backup = initialize(&mut db, &path).unwrap().unwrap();
    assert!(backup.exists());
    assert_eq!(read_note(&db, &asset, &id).unwrap().body, "旧笔记");
    assert!(read_note(&db, &asset, &id).unwrap().analysis.is_none());
    assert_eq!(
        db.query_row("SELECT version FROM library_schema", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        11
    );
}
