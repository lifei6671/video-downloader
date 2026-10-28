use super::*;
use crate::desktop_core::transcription::Source;
use crate::library::keyframes::{insert_frame, FrameBatch};
use serde_json::json;
use std::{fs, path::Path};

fn database() -> Connection {
    let mut db = Connection::open_in_memory().unwrap();
    initialize(&mut db, Path::new(":memory:")).unwrap();
    db
}
fn seed(db: &Connection, root: &Path) -> (String, String, Source, String) {
    let path = root.join("源😀.mp4");
    fs::write(&path, b"video").unwrap();
    let source = Source::read(&path).unwrap();
    let item = register_local(
        db,
        &source,
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
            file_id: item.file_id.clone(),
            title: "节奏".into(),
            reason: String::new(),
            start_ms: 0,
            end_ms: 1000,
            playable: ClipRange {
                start_ms: 0,
                end_ms: 1000,
            },
            document_id: None,
            quote: None,
        },
    )
    .unwrap();
    (item.asset_id, item.file_id, source, clip)
}
fn png() -> Vec<u8> {
    let mut body = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut body, 1, 1);
        encoder.set_color(png::ColorType::Rgb);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&[0, 0, 0]).unwrap();
    }
    body
}
#[test]
fn png_resources_require_decodable_pixels_and_a_complete_stream() {
    let good = png();
    validate_frame_image(&good, 1, 1).unwrap();
    let mut bad = good.clone();
    let index = bad.windows(4).position(|b| b == b"IDAT").unwrap() + 4;
    bad[index] ^= 0xff;
    for bytes in [good[..33].to_vec(), good[..good.len() - 12].to_vec(), bad] {
        assert!(validate_frame_image(&bytes, 1, 1).is_err());
    }
}

fn data(asset: &str, file: &str, clip: Option<&str>) -> KeyframeData {
    KeyframeData {
        id: "frame-1".into(),
        asset_id: asset.into(),
        file_id: file.into(),
        clip_id: clip.map(str::to_owned),
        batch_id: "batch".into(),
        sample_start_ms: 0,
        sample_end_ms: clip.map(|_| 1000),
        sample_index: 0,
        sample_count: 1,
        requested_ms: if clip.is_some() { 500 } else { 0 },
        pts: 500,
        time_base_num: 1,
        time_base_den: 1000,
        width: 1,
        height: 1,
        black: true,
        created_at: 1,
    }
}
#[test]
fn images_survive_source_change_and_missing_media_and_delete_only_the_resource() {
    let root = tempfile::tempdir().unwrap();
    let db = database();
    let (asset, file, source, clip) = seed(&db, root.path());
    let frame = data(&asset, &file, Some(&clip));
    insert_frame(&db, &frame, &source, &png()).unwrap();
    assert_eq!(keyframes(&db, &asset).unwrap()[0].actual_ms, 500);
    fs::write(&source.path, b"changed").unwrap();
    assert!(keyframes(&db, &asset).unwrap()[0].source_changed);
    fs::remove_file(&source.path).unwrap();
    assert!(keyframes(&db, &asset).unwrap()[0].missing);
    assert_eq!(keyframe_image(&db, &asset, &frame.id).unwrap(), png());
    let unrelated = root.path().join("unrelated.mp4");
    fs::write(&unrelated, b"keep").unwrap();
    delete_keyframe(&db, &asset, &frame.id).unwrap();
    assert!(keyframes(&db, &asset).unwrap().is_empty());
    assert!(unrelated.exists());
    assert_eq!(documents(&db, &asset).unwrap().len(), 0);
    assert_eq!(clips(&db, &asset).unwrap().len(), 1);
}
#[test]
fn sampling_binding_and_whole_batch_rollback_reject_invalid_artifacts() {
    let root = tempfile::tempdir().unwrap();
    let db = database();
    let (asset, file, source, clip) = seed(&db, root.path());
    assert_eq!(sample_points(0, 1000, 3).unwrap(), [166, 500, 833]);
    for (start, end, count) in [(0, 0, 3), (4, 2, 1), (0, 4, 9), (0, 10, 0), (0, 10, 10)] {
        assert!(sample_points(start, end, count).is_err());
    }
    let stage = root.path().join("stage");
    fs::create_dir(&stage).unwrap();
    for i in 0..3 {
        fs::write(stage.join(format!("frame-{i:03}.png")), png()).unwrap();
    }
    let batch = FrameBatch {
        asset_id: asset.clone(),
        file_id: file.clone(),
        clip_id: Some(clip.clone()),
        id: "batch".into(),
        source: source.clone(),
        start_ms: 0,
        end_ms: Some(1000),
        points: sample_points(0, 1000, 3).unwrap(),
    };
    let value = json!({"frames":batch.points.iter().enumerate().map(|(i,p)|json!({"filename":format!("frame-{i:03}.png"),"requested_ms":p,"pts":p,"time_base_num":1,"time_base_den":1000,"width":1,"height":1,"black":true,"bytes":png().len()})).collect::<Vec<_>>()});
    for flaw in ["path", "pts", "base", "dimensions", "range", "count"] {
        let mut bad = value.clone();
        match flaw {
            "path" => bad["frames"][1]["filename"] = "../frame.png".into(),
            "pts" => bad["frames"][1]["pts"] = 0.into(),
            "base" => bad["frames"][1]["time_base_den"] = 0.into(),
            "dimensions" => bad["frames"][1]["width"] = 2.into(),
            "range" => bad["frames"][1]["pts"] = 1000.into(),
            _ => {
                bad["frames"].as_array_mut().unwrap().pop();
            }
        }
        let result = batch
            .read(&stage, &bad)
            .and_then(|rows| store_frames(&db, &rows));
        assert!(result.is_err(), "{flaw}");
        assert!(keyframes(&db, &asset).unwrap().is_empty());
    }
    let rows = batch.read(&stage, &value).unwrap();
    db.execute_batch("CREATE TRIGGER reject_second_frame BEFORE INSERT ON asset_keyframes WHEN json_extract(NEW.metadata,'$.sample_index')=1 BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
    assert!(store_frames(&db, &rows).is_err());
    assert!(keyframes(&db, &asset).unwrap().is_empty());
    db.execute_batch("DROP TRIGGER reject_second_frame")
        .unwrap();
    store_frames(&db, &rows).unwrap();
    assert_eq!(keyframes(&db, &asset).unwrap().len(), 3);
    assert!(keyframe_image(&db, "other", &rows[0].data.id).is_err());
    delete_clip(&db, &asset, &clip).unwrap();
    assert!(keyframes(&db, &asset).unwrap().is_empty());
    assert!(source.path.exists());
}
#[test]
fn frames_roundtrip_portable_backup_twice_with_and_without_media_and_relinked_source() {
    for media in [true, false] {
        for relink in [true, false] {
            let root = tempfile::tempdir().unwrap();
            let db = database();
            let (asset, file, source, clip) = seed(&db, root.path());
            insert_frame(&db, &data(&asset, &file, Some(&clip)), &source, &png()).unwrap();
            if relink {
                let replacement = root.path().join("new.mp4");
                fs::write(&replacement, b"replacement").unwrap();
                relink_file(&db, &file, &replacement).unwrap();
            }
            let mut current = db;
            for generation in 0..2 {
                let package = root.path().join(format!("package-{generation}"));
                let target = root.path().join(format!("restored-{generation}"));
                write_backup(
                    capture_backup(&current, media).unwrap(),
                    &package,
                    &mut |_| Ok(()),
                )
                .unwrap();
                let mut next = database();
                commit_restore(
                    &mut next,
                    prepare_restore(&package, &target, &mut |_| Ok(())).unwrap(),
                )
                .unwrap();
                let frames = keyframes(&next, &asset).unwrap();
                assert_eq!(frames.len(), 1);
                assert_eq!(frames[0].actual_ms, 500);
                assert_eq!(frames[0].source_changed, !media || relink);
                assert_eq!(frames[0].missing, !media);
                assert_eq!(keyframe_image(&next, &asset, "frame-1").unwrap(), png());
                current = next;
            }
        }
    }
}
#[test]
fn v6_migration_backup_and_retry_preserve_v5_material() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("v5.sqlite");
    let mut db = Connection::open(&path).unwrap();
    for sql in [
        include_str!("schema.sql"),
        include_str!("schema-v2.sql"),
        include_str!("schema-v3.sql"),
        include_str!("schema-v4.sql"),
        include_str!("schema-v5.sql"),
    ] {
        db.execute_batch(sql).unwrap();
    }
    let (asset, _, _, clip) = seed(&db, root.path());
    db.execute_batch("CREATE TABLE asset_keyframes(dummy TEXT)")
        .unwrap();
    assert!(initialize(&mut db, &path).is_err());
    assert_eq!(
        db.query_row("SELECT version FROM library_schema", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        5
    );
    db.execute_batch("DROP TABLE asset_keyframes").unwrap();
    let backup = initialize(&mut db, &path).unwrap().unwrap();
    assert_eq!(
        Connection::open(backup)
            .unwrap()
            .query_row("SELECT version FROM library_schema", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        5
    );
    assert_eq!(clips(&db, &asset).unwrap()[0].id, clip);
    assert!(initialize(&mut db, &path).unwrap().is_none());
}

#[test]
fn invalid_frame_package_metadata_or_payload_never_publishes_a_restore() {
    use sha2::{Digest, Sha256};
    let root = tempfile::tempdir().unwrap();
    let db = database();
    let (asset, file, source, clip) = seed(&db, root.path());
    insert_frame(&db, &data(&asset, &file, Some(&clip)), &source, &png()).unwrap();
    let package = root.path().join("package");
    write_backup(capture_backup(&db, false).unwrap(), &package, &mut |_| {
        Ok(())
    })
    .unwrap();
    let original: serde_json::Value =
        serde_json::from_slice(&fs::read(package.join("manifest.json")).unwrap()).unwrap();
    for flaw in [
        "timebase",
        "clip-range",
        "source",
        "owner",
        "old-version",
        "image-size",
    ] {
        let mut value = original.clone();
        match flaw {
            "timebase" => value["keyframes"][0]["data"]["time_base_den"] = 0.into(),
            "clip-range" => value["keyframes"][0]["data"]["sample_end_ms"] = 1001.into(),
            "source" => value["keyframes"][0]["source"]["mtime_ns"] = "wrong".into(),
            "owner" => value["keyframes"][0]["data"]["asset_id"] = "other".into(),
            "old-version" => value["version"] = 3.into(),
            _ => value["keyframes"][0]["data"]["width"] = 2.into(),
        }
        let bytes = serde_json::to_vec(&value).unwrap();
        fs::write(package.join("manifest.json"), &bytes).unwrap();
        fs::write(
            package.join("manifest.sha256"),
            format!("{:x}", Sha256::digest(&bytes)),
        )
        .unwrap();
        let target = root.path().join(format!("bad-{flaw}"));
        assert!(
            prepare_restore(&package, &target, &mut |_| Ok(())).is_err(),
            "{flaw}"
        );
        assert!(!target.exists());
    }
}
