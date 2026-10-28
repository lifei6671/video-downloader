use super::*;
use crate::desktop_core::transcription::{Source, TranscriptResult};
use rusqlite::params;
use std::{fs, path::Path};

fn database() -> Connection {
    let mut db = Connection::open_in_memory().unwrap();
    initialize(&mut db, Path::new(":memory:")).unwrap();
    db
}
fn seed(db: &Connection, root: &Path, name: &str) -> (String, String, TranscriptResult) {
    let path = root.join(name);
    fs::write(&path, b"synthetic video").unwrap();
    let source = Source::read(&path).unwrap();
    let item = register_local(
        db,
        &source,
        &LocalMetadata {
            width: 160,
            height: 90,
            duration_ms: Some(4000),
        },
    )
    .unwrap();
    (
        item.asset_id,
        item.file_id,
        TranscriptResult {
            timing: None,
            text: "中文😀原文\n值得学习的开场".into(),
            source,
            model_revision: "fixture".into(),
            model_name: None,
            pipeline_revision: "fixture".into(),
            backend: None,
            completed_at: 1,
        },
    )
}
fn input(file: &str) -> ClipInput {
    ClipInput {
        file_id: file.into(),
        title: "开场😀".into(),
        reason: "学习节奏".into(),
        start_ms: 5200,
        end_ms: 7000,
        playable: ClipRange {
            start_ms: 5000,
            end_ms: 8804,
        },
        document_id: None,
        quote: None,
    }
}
#[test]
fn manual_clip_uses_measured_range_and_never_crops_or_rebinds_source() {
    let root = tempfile::tempdir().unwrap();
    let db = database();
    let (asset, file, result) = seed(&db, root.path(), "源文件 中文.mp4");
    let first = create_clip(&db, &asset, &input(&file)).unwrap();
    // Container duration is not the renderer's real upper bound (positive-PTS MP4).
    assert_eq!(clips(&db, &asset).unwrap()[0].end_ms, 7000);
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
    assert_eq!(
        clip_playback(&db, &asset, &first).unwrap().1,
        result.source.path
    );
    fs::write(&result.source.path, b"new bytes changed").unwrap();
    assert!(clips(&db, &asset).unwrap()[0].source_changed);
    assert_eq!(
        clip_playback(&db, &asset, &first).unwrap_err().to_string(),
        "CLIP_SOURCE_CHANGED"
    );
    assert_eq!(
        create_clip(&db, &asset, &input(&file))
            .unwrap_err()
            .to_string(),
        "CLIP_SOURCE_CHANGED"
    );
    let replacement = root.path().join("replacement.mp4");
    fs::write(&replacement, b"replacement").unwrap();
    relink_file(&db, &file, &replacement).unwrap();
    assert_eq!(
        clip_playback(&db, &asset, &first).unwrap_err().to_string(),
        "CLIP_SOURCE_CHANGED"
    );
    let second = create_clip(&db, &asset, &input(&file)).unwrap();
    fs::remove_file(&replacement).unwrap();
    assert!(clips(&db, &asset).unwrap().iter().all(|c| c.missing));
    assert_eq!(
        clip_playback(&db, &asset, &second).unwrap_err().to_string(),
        "FILE_MISSING"
    );
    delete_clip(&db, &asset, &first).unwrap();
    assert!(result.source.path.exists());
    assert_eq!(clips(&db, &asset).unwrap().len(), 1);
}
#[test]
fn clip_citation_is_immutable_and_checks_asset_file_quote_and_edited_ancestry() {
    let root = tempfile::tempdir().unwrap();
    let db = database();
    let (asset, file, mut result) = seed(&db, root.path(), "old.mp4");
    let document = save_recognized(&db, &asset, Some(&file), &result).unwrap();
    let edited = save_edited(&db, &asset, "改写中文😀", 0, Some(&document)).unwrap();
    let mut request = input(&file);
    request.document_id = Some(document.clone());
    request.quote = Some("中文😀原文".into());
    let id = create_clip(&db, &asset, &request).unwrap();
    result.completed_at = 2;
    result.text = "新版内容".into();
    save_recognized(&db, &asset, Some(&file), &result).unwrap();
    let clip = clips(&db, &asset).unwrap().remove(0);
    assert_eq!(clip.document_id.as_ref(), Some(&document));
    assert_eq!(clip.document_version, Some(1));
    assert_eq!(clip.quote.as_deref(), Some("中文😀原文"));
    assert_eq!(
        read_document(&db, &asset, &document).unwrap().text,
        "中文😀原文\n值得学习的开场"
    );
    let (other_asset, other_file, _) = seed(&db, root.path(), "other.mp4");
    assert!(create_clip(&db, &other_asset, &request).is_err());
    assert!(delete_clip(&db, &other_asset, &id).is_err());
    assert!(clip_playback(&db, &other_asset, &id).is_err());
    // A second file version in the same asset still cannot borrow the first file's text.
    db.execute(
        "UPDATE asset_files SET asset_id=?1 WHERE id=?2",
        params![asset, other_file],
    )
    .unwrap();
    request.file_id = other_file;
    assert_eq!(
        create_clip(&db, &asset, &request).unwrap_err().to_string(),
        "CLIP_SOURCE_MISMATCH"
    );
    request.document_id = Some(edited);
    request.quote = Some("改写中文😀".into());
    assert_eq!(
        create_clip(&db, &asset, &request).unwrap_err().to_string(),
        "CLIP_SOURCE_MISMATCH"
    );
    request.file_id = file;
    request.quote = Some("不存在的内容".into());
    assert_eq!(
        create_clip(&db, &asset, &request).unwrap_err().to_string(),
        "INVALID_CITATION"
    );
    delete_clip(&db, &asset, &id).unwrap();
    assert_eq!(documents(&db, &asset).unwrap().len(), 3);
}
#[test]
fn clip_rejects_invalid_ranges_including_unplayable_head_and_tail() {
    let root = tempfile::tempdir().unwrap();
    let db = database();
    let (asset, file, _) = seed(&db, root.path(), "source.mp4");
    for (start, end) in [
        (0, 6000),
        (5200, 8805),
        (6000, 6000),
        (7000, 5200),
        (0, u64::MAX),
    ] {
        let mut value = input(&file);
        value.start_ms = start;
        value.end_ms = end;
        assert!(create_clip(&db, &asset, &value).is_err(), "{start} {end}");
    }
    let mut value = input(&file);
    value.reason = "字".repeat(6000);
    assert_eq!(
        create_clip(&db, &asset, &value).unwrap_err().to_string(),
        "CLIP_REASON_TOO_LONG"
    );
    assert!(clips(&db, &asset).unwrap().is_empty());
    assert!(serde_json::from_value::<ClipInput>(serde_json::json!({"file_id":file,"title":"x","reason":"","start_ms":0.5,"end_ms":2,"playable":{"start_ms":0,"end_ms":10}})).is_err());
}
#[test]
fn clips_restore_with_media_without_media_and_after_source_relink() {
    for media in [true, false] {
        for relink in [true, false] {
            let root = tempfile::tempdir().unwrap();
            let db = database();
            let (asset, file, result) = seed(&db, root.path(), "源😀.mp4");
            let doc = save_recognized(&db, &asset, Some(&file), &result).unwrap();
            let mut value = input(&file);
            value.document_id = Some(doc.clone());
            value.quote = Some("中文😀".into());
            let id = create_clip(&db, &asset, &value).unwrap();
            if relink {
                let path = root.path().join("new.mp4");
                fs::write(&path, b"new").unwrap();
                relink_file(&db, &file, &path).unwrap();
            }
            let package = root.path().join("backup");
            write_backup(capture_backup(&db, media).unwrap(), &package, &mut |_| {
                Ok(())
            })
            .unwrap();
            let manifest = fs::read_to_string(package.join("manifest.json")).unwrap();
            assert!(!manifest.contains(&root.path().to_string_lossy().replace('\\', "\\\\")));
            let mut restored = database();
            commit_restore(
                &mut restored,
                prepare_restore(&package, &root.path().join("restored"), &mut |_| Ok(())).unwrap(),
            )
            .unwrap();
            let clip = clips(&restored, &asset).unwrap().remove(0);
            assert_eq!(clip.id, id);
            assert_eq!(clip.document_version, Some(1));
            assert_eq!(clip.document_id.as_ref(), Some(&doc));
            assert_eq!(clip.source_changed, !media || relink);
            assert_eq!(clip.missing, !media);
            assert_eq!(
                clip_playback(&restored, &asset, &id).is_ok(),
                media && !relink
            );
            let again = root.path().join("second-backup");
            write_backup(
                capture_backup(&restored, media).unwrap(),
                &again,
                &mut |_| Ok(()),
            )
            .unwrap();
            let mut twice = database();
            commit_restore(
                &mut twice,
                prepare_restore(&again, &root.path().join("second-restore"), &mut |_| Ok(()))
                    .unwrap(),
            )
            .unwrap();
            let second = clips(&twice, &asset).unwrap().remove(0);
            assert_eq!(second.document_id, clip.document_id);
            assert_eq!(second.source_changed, clip.source_changed);
            assert_eq!(second.missing, clip.missing);
        }
    }
}
#[test]
fn v5_migration_backs_up_and_preserves_v4_on_failure_then_retries() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("v4.sqlite");
    let mut db = Connection::open(&path).unwrap();
    for sql in [
        include_str!("schema.sql"),
        include_str!("schema-v2.sql"),
        include_str!("schema-v3.sql"),
        include_str!("schema-v4.sql"),
    ] {
        db.execute_batch(sql).unwrap();
    }
    let (asset, file, result) = seed(&db, root.path(), "legacy.mp4");
    let doc = save_recognized(&db, &asset, Some(&file), &result).unwrap();
    db.execute_batch("CREATE TABLE asset_clips(synthetic TEXT)")
        .unwrap();
    assert!(initialize(&mut db, &path).is_err());
    assert_eq!(
        db.query_row("SELECT version FROM library_schema", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        4
    );
    db.execute_batch("DROP TABLE asset_clips").unwrap();
    let backup = initialize(&mut db, &path).unwrap().unwrap();
    assert_eq!(
        Connection::open(backup)
            .unwrap()
            .query_row("SELECT version FROM library_schema", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        4
    );
    assert_eq!(read_document(&db, &asset, &doc).unwrap().text, result.text);
    assert!(initialize(&mut db, &path).unwrap().is_none());
}

#[test]
fn restore_rejects_mixed_source_citation_empty_id_and_invalid_range() {
    use sha2::{Digest, Sha256};
    let root = tempfile::tempdir().unwrap();
    let db = database();
    let (asset, file, result) = seed(&db, root.path(), "source.mp4");
    let old = save_recognized(&db, &asset, Some(&file), &result).unwrap();
    let edited = save_edited(&db, &asset, "历史整理稿", 0, Some(&old)).unwrap();
    let replacement = root.path().join("new.mp4");
    fs::write(&replacement, b"new source").unwrap();
    relink_file(&db, &file, &replacement).unwrap();
    create_clip(&db, &asset, &input(&file)).unwrap();
    let package = root.path().join("package");
    write_backup(
        capture_backup(&db, true).unwrap(),
        &package,
        &mut |_| Ok(()),
    )
    .unwrap();
    let original: serde_json::Value =
        serde_json::from_slice(&fs::read(package.join("manifest.json")).unwrap()).unwrap();
    for flaw in ["source", "edited-source", "id", "range", "version"] {
        let mut value = original.clone();
        match flaw {
            "source" => {
                value["clips"][0]["document_id"] = old.clone().into();
                value["clips"][0]["document_version"] = 1.into();
                value["clips"][0]["quote"] = "中文😀".into();
            }
            "edited-source" => {
                value["clips"][0]["document_id"] = edited.clone().into();
                value["clips"][0]["document_version"] = 1.into();
                value["clips"][0]["quote"] = "历史整理稿".into();
            }
            "id" => value["clips"][0]["id"] = "".into(),
            "range" => value["clips"][0]["end_ms"] = 0.into(),
            _ => value["version"] = 2.into(),
        }
        let body = serde_json::to_vec(&value).unwrap();
        fs::write(package.join("manifest.json"), &body).unwrap();
        fs::write(
            package.join("manifest.sha256"),
            format!("{:x}", Sha256::digest(&body)),
        )
        .unwrap();
        let output = root.path().join(format!("restore-{flaw}"));
        assert!(
            prepare_restore(&package, &output, &mut |_| Ok(())).is_err(),
            "{flaw}"
        );
        assert!(!output.exists());
    }
}
