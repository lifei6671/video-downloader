use super::*;
use crate::desktop_core::transcription::{Source, TranscriptResult};
use rusqlite::params;
use std::{fs, path::Path};

fn sample(db: &Connection, root: &Path) -> (String, String, TranscriptResult) {
    let path = root.join("原视频😀.mp4");
    fs::write(&path, b"synthetic video").unwrap();
    let source = Source::read(&path).unwrap();
    let entry = register_local(
        db,
        &source,
        &LocalMetadata {
            width: 90,
            height: 160,
            duration_ms: Some(3000),
        },
    )
    .unwrap();
    let timing: TranscriptTiming = serde_json::from_value(serde_json::json!({
        "operation_id":"fixture-run","schema_version":1,"time_base":"media_zero","timing_source":"fsmn_vad","precision":"vad_segment",
        "sample_rate":16000,"frames":48001,"vad_revision":"6840bae4c5c92ee8c04faaf4db23dd0105098d7f",
        "segments":[{"start_ms":100,"end_ms":400,"text":"中文😀"},{"start_ms":1700,"end_ms":3000,"text":"English"}]
    })).unwrap();
    (
        entry.asset_id,
        entry.file_id,
        TranscriptResult {
            timing: Some(timing),
            text: "中文😀\nEnglish".into(),
            source,
            model_revision: "model".into(),
            model_name: Some("Nano".into()),
            pipeline_revision: "media-zero-v2".into(),
            backend: Some("cpu".into()),
            completed_at: 1,
        },
    )
}
fn db() -> Connection {
    let mut db = Connection::open_in_memory().unwrap();
    initialize(&mut db, Path::new(":memory:")).unwrap();
    db
}
fn save(
    db: &mut Connection,
    asset: &str,
    file: &str,
    result: &TranscriptResult,
) -> anyhow::Result<String> {
    let tx = db.transaction()?;
    let id = save_recognized(&tx, asset, Some(file), result)?;
    tx.commit()?;
    Ok(id)
}

#[test]
fn timing_is_bound_to_immutable_document_and_source_file() {
    let root = tempfile::tempdir().unwrap();
    let mut db = db();
    let (asset, file, mut result) = sample(&db, root.path());
    let first = save(&mut db, &asset, &file, &result).unwrap();
    assert_eq!(save(&mut db, &asset, &file, &result).unwrap(), first);
    let first_timing = result.timing.clone();
    // Same-second successful operations must remain distinct; re-saving one is idempotent.
    result.timing.as_mut().unwrap().operation_id = Some("second-operation".into());
    let second = save(&mut db, &asset, &file, &result).unwrap();
    assert_ne!(second, first);
    let original = read_document(&db, &asset, &first).unwrap();
    assert_eq!(original.summary.version, 1);
    assert_eq!(original.summary.file_id.as_deref(), Some(file.as_str()));
    assert_eq!(original.timing, first_timing);
    assert_eq!(
        read_document(&db, &asset, &second).unwrap().timing,
        result.timing
    );
    let edited = save_edited(&db, &asset, "改写并重排", 0, Some(&first)).unwrap();
    assert!(read_document(&db, &asset, &edited)
        .unwrap()
        .timing
        .is_none());
    assert_eq!(
        latest_recognition(&db, &file).unwrap().unwrap().timing,
        result.timing
    );
    fs::write(&result.source.path, b"changed video contents").unwrap();
    assert_eq!(
        read_document(&db, &asset, &first).unwrap().source_changed,
        Some(true)
    );
    assert_eq!(
        read_document(&db, &asset, &first).unwrap().timing,
        first_timing
    );
    let replacement = root.path().join("replacement.mp4");
    fs::write(&replacement, b"replacement").unwrap();
    relink_file(&db, &file, &replacement).unwrap();
    result.completed_at = 3;
    assert_eq!(
        save(&mut db, &asset, &file, &result)
            .unwrap_err()
            .to_string(),
        "TRANSCRIPTION_SOURCE_CHANGED"
    );
    assert_eq!(documents(&db, &asset).unwrap().len(), 3);
}

#[test]
fn segment_insert_failure_rolls_back_document_and_metadata_and_can_retry() {
    let root = tempfile::tempdir().unwrap();
    let mut db = db();
    let (asset, file, mut result) = sample(&db, root.path());
    let first = save(&mut db, &asset, &file, &result).unwrap();
    db.execute_batch("CREATE TRIGGER reject_timing BEFORE INSERT ON transcript_segments WHEN NEW.position=1 BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
    result.completed_at = 2;
    assert!(save(&mut db, &asset, &file, &result).is_err());
    assert_eq!(documents(&db, &asset).unwrap().len(), 1);
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM transcript_timings", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        read_document(&db, &asset, &first).unwrap().timing,
        result.timing
    );
    db.execute_batch("DROP TRIGGER reject_timing;").unwrap();
    assert_ne!(save(&mut db, &asset, &file, &result).unwrap(), first);
}

#[test]
fn v3_upgrade_is_backed_up_and_does_not_invent_legacy_times() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("old.sqlite");
    let mut db = Connection::open(&path).unwrap();
    db.execute_batch(include_str!("schema.sql")).unwrap();
    db.execute_batch(include_str!("schema-v2.sql")).unwrap();
    db.execute_batch(include_str!("schema-v3.sql")).unwrap();
    let (asset, file, mut result) = sample(&db, root.path());
    result.timing = None;
    let legacy = save(&mut db, &asset, &file, &result).unwrap();
    let backup = initialize(&mut db, &path).unwrap().unwrap();
    assert_eq!(
        Connection::open(backup)
            .unwrap()
            .query_row("SELECT version FROM library_schema", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        3
    );
    assert_eq!(
        db.query_row("SELECT version FROM library_schema", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        11
    );
    assert!(read_document(&db, &asset, &legacy)
        .unwrap()
        .timing
        .is_none());
    assert_eq!(save(&mut db, &asset, &file, &result).unwrap(), legacy);
    assert!(initialize(&mut db, &path).unwrap().is_none());
}

#[test]
fn timing_roundtrips_backup_with_and_without_media_and_deduplicates_after_rebase() {
    for media in [true, false] {
        let root = tempfile::tempdir().unwrap();
        let mut db = db();
        let (asset, file, result) = sample(&db, root.path());
        let id = save(&mut db, &asset, &file, &result).unwrap();
        let package = root.path().join("backup");
        write_backup(capture_backup(&db, media).unwrap(), &package, &mut |_| {
            Ok(())
        })
        .unwrap();
        let target = root.path().join("restored");
        let mut restored = self::db();
        commit_restore(
            &mut restored,
            prepare_restore(&package, &target, &mut |_| Ok(())).unwrap(),
        )
        .unwrap();
        let doc = read_document(&restored, &asset, &id).unwrap();
        assert_eq!(doc.timing, result.timing);
        assert_eq!(doc.source_changed, Some(!media));
        let loaded = latest_recognition(&restored, &file).unwrap().unwrap();
        assert_eq!(save(&mut restored, &asset, &file, &loaded).unwrap(), id);
        assert_eq!(
            restored
                .query_row("SELECT COUNT(*) FROM transcript_segments", params![], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            2
        );
    }
}

#[test]
fn artifact_rejects_wrong_version_clock_ranges_text_and_path() {
    let root = tempfile::tempdir().unwrap();
    let db = db();
    let (_, _, result) = sample(&db, root.path());
    let good = serde_json::to_value(result.timing.as_ref().unwrap()).unwrap();
    for flaw in [
        "version",
        "clock",
        "precision",
        "float",
        "bool",
        "range",
        "order",
        "text",
        "vad",
        "unknown",
    ] {
        let mut value = good.clone();
        match flaw {
            "version" => value["schema_version"] = 2.into(),
            "clock" => value["time_base"] = "audio_start".into(),
            "precision" => value["precision"] = "word".into(),
            "float" => value["segments"][0]["start_ms"] = serde_json::json!(1.5),
            "bool" => value["frames"] = true.into(),
            "range" => value["segments"][1]["end_ms"] = 3001.into(),
            "order" => value["segments"][1]["start_ms"] = 200.into(),
            "text" => value["segments"][0]["text"] = "other".into(),
            "vad" => value["vad_revision"] = "unknown".into(),
            _ => value["extra"] = 1.into(),
        }
        let bytes = serde_json::to_vec(&value).unwrap();
        fs::write(root.path().join("transcript-segments.json"), &bytes).unwrap();
        let descriptor =
            serde_json::json!({"filename":"transcript-segments.json","bytes":bytes.len()});
        assert!(
            TranscriptTiming::artifact(root.path(), &descriptor, &result.text).is_err(),
            "{flaw}"
        );
    }
    for descriptor in [
        serde_json::json!({"filename":"../transcript-segments.json","bytes":1}),
        serde_json::json!({"filename":"transcript-segments.json","bytes":16777217}),
        serde_json::json!({"filename":"transcript-segments.json","bytes":1}),
    ] {
        assert!(TranscriptTiming::artifact(root.path(), &descriptor, &result.text).is_err());
    }
}

#[test]
fn failed_v4_migration_keeps_v3_and_can_retry() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("old.sqlite");
    let mut db = Connection::open(&path).unwrap();
    db.execute_batch(include_str!("schema.sql")).unwrap();
    db.execute_batch(include_str!("schema-v2.sql")).unwrap();
    db.execute_batch(include_str!("schema-v3.sql")).unwrap();
    db.execute_batch("CREATE TABLE transcript_segments(synthetic TEXT);")
        .unwrap();
    assert!(initialize(&mut db, &path).is_err());
    assert_eq!(
        db.query_row("SELECT version FROM library_schema", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        3
    );
    assert!(!db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='transcript_timings')",
            [],
            |r| r.get::<_, bool>(0)
        )
        .unwrap());
    db.execute_batch("DROP TABLE transcript_segments;").unwrap();
    assert!(initialize(&mut db, &path).unwrap().is_some());
}

#[test]
fn previous_v1_backup_restores_full_text_without_fabricated_timing() {
    use sha2::{Digest, Sha256};
    let root = tempfile::tempdir().unwrap();
    let mut db = db();
    let (asset, file, mut result) = sample(&db, root.path());
    result.timing = None;
    let id = save(&mut db, &asset, &file, &result).unwrap();
    let package = root.path().join("old-package");
    write_backup(
        capture_backup(&db, true).unwrap(),
        &package,
        &mut |_| Ok(()),
    )
    .unwrap();
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(package.join("manifest.json")).unwrap()).unwrap();
    manifest["version"] = 1.into();
    let body = serde_json::to_vec(&manifest).unwrap();
    fs::write(package.join("manifest.json"), &body).unwrap();
    fs::write(
        package.join("manifest.sha256"),
        format!("{:x}", Sha256::digest(&body)),
    )
    .unwrap();
    let mut restored = self::db();
    commit_restore(
        &mut restored,
        prepare_restore(&package, &root.path().join("restored"), &mut |_| Ok(())).unwrap(),
    )
    .unwrap();
    let document = read_document(&restored, &asset, &id).unwrap();
    assert_eq!(document.text, result.text);
    assert!(document.timing.is_none());
}
