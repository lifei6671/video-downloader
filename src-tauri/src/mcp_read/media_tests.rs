use super::super::tests::fixture;
use super::*;
use serde_json::json;

#[test]
fn saved_media_reads_preserve_pins_pts_images_and_boundaries_without_writes() {
    let root = tempfile::tempdir().unwrap();
    let mut db = Connection::open_in_memory().unwrap();
    library::initialize(&mut db, std::path::Path::new(":memory:")).unwrap();
    fixture::seed(&db, root.path()).unwrap();
    fixture::seed_media(&db, root.path()).unwrap();
    crate::settings::load(&db, root.path().join("downloads")).unwrap();
    fixture::seed_resource_boundary(&db).unwrap();
    let before = db.total_changes();
    let call = |method, params| super::super::dispatch(&db, method, params);
    let clips = call("mcp_get_clip", json!({"asset_id":"asset-a"})).unwrap();
    let clip = &clips["clips"]["items"][0];
    assert_eq!(clip["document_id"], "doc-a1");
    assert_eq!(clip["document_version"], 1);
    assert_eq!(clip["quote_excerpt"], fixture::TEXT);
    assert_eq!(clip["start_ms"], 100);
    let first = call("mcp_get_keyframes", json!({"asset_id":"asset-a","limit":1})).unwrap();
    let frame = &first["frames"]["items"][0];
    assert_eq!(frame["id"], "media-frame-0");
    assert_eq!(frame["sample_start_ms"], 100);
    assert_eq!(frame["sample_end_ms"], 900);
    assert_eq!(frame["pts"], 500);
    assert_eq!(frame["time_base_den"], 1000);
    assert_eq!(frame["clip_id"], clip["id"]);
    let image = STANDARD
        .decode(frame["image_base64"].as_str().unwrap())
        .unwrap();
    library::validate_frame_image(&image, 16, 16).unwrap();
    assert_eq!(
        image,
        library::keyframe_image(&db, "asset-a", "media-frame-0").unwrap()
    );
    let next = call("mcp_get_keyframes",json!({"asset_id":"asset-a","limit":1,"cursor":first["frames"]["next_cursor"],"include_images":false})).unwrap();
    assert_eq!(next["frames"]["items"][0]["id"], "media-frame-1");
    assert!(next["frames"]["items"][0].get("image_base64").is_none());
    assert!(next["frames"]["next_cursor"].is_null());
    assert_eq!(
        call(
            "mcp_get_keyframes",
            json!({"asset_id":"asset-a","start_ms":500,"end_ms":750})
        )
        .unwrap()["frames"]["total"],
        1
    );
    assert_eq!(
        call(
            "mcp_get_keyframes",
            json!({"asset_id":"asset-a","clip_id":clip["id"]})
        )
        .unwrap()["frames"]["total"],
        1
    );
    for (method, params, error) in [
        (
            "mcp_get_clip",
            json!({"asset_id":"asset-b","clip_id":clip["id"]}),
            ReadError::ClipNotFound,
        ),
        (
            "mcp_get_keyframes",
            json!({"asset_id":"asset-b","file_id":"file-a"}),
            ReadError::FileNotFound,
        ),
        (
            "mcp_get_keyframes",
            json!({"asset_id":"asset-b","clip_id":clip["id"]}),
            ReadError::ClipNotFound,
        ),
        (
            "mcp_get_keyframes",
            json!({"asset_id":"asset-a","media_type":"video"}),
            ReadError::MediaUnsupported,
        ),
        (
            "mcp_get_keyframes",
            json!({"asset_id":"asset-a","media_type":"audio"}),
            ReadError::MediaUnsupported,
        ),
        (
            "mcp_get_keyframes",
            json!({"asset_id":"asset-a","limit":10}),
            ReadError::InvalidInput,
        ),
        (
            "mcp_get_keyframes",
            json!({"asset_id":"asset-a","start_ms":10,"end_ms":10}),
            ReadError::InvalidInput,
        ),
        (
            "mcp_get_keyframes",
            json!({"asset_id":"asset-a","cursor":"unknown"}),
            ReadError::InvalidCursor,
        ),
        (
            "mcp_get_clip",
            json!({"asset_id":"asset-a","path":"private"}),
            ReadError::InvalidInput,
        ),
    ] {
        assert_eq!(call(method, params).unwrap_err(), error);
    }
    let wire = serde_json::to_string(&(clips, first, next)).unwrap();
    assert!(!wire.contains("PRIVATE_"));
    assert!(!wire.contains("source_json"));
    assert!(!wire.contains("SYNTHETIC_COOKIE"));
    assert_eq!(db.total_changes(), before);
    // 749.6 ms displays as 750 ms, but still belongs to the range ending at 750.
    db.execute("UPDATE asset_keyframes SET metadata=json_set(metadata,'$.pts',1874,'$.time_base_den',2500,'$.requested_ms',749,'$.sample_start_ms',749) WHERE id='media-frame-1'", []).unwrap();
    assert_eq!(
        call(
            "mcp_get_keyframes",
            json!({"asset_id":"asset-a","end_ms":750})
        )
        .unwrap()["frames"]["total"],
        2
    );
    assert_eq!(
        call(
            "mcp_get_keyframes",
            json!({"asset_id":"asset-a","start_ms":750})
        )
        .unwrap()["frames"]["total"],
        0
    );
    db.execute("UPDATE asset_keyframes SET metadata=json_set(metadata,'$.pts',750,'$.time_base_den',1000,'$.requested_ms',750,'$.sample_start_ms',750) WHERE id='media-frame-1'", []).unwrap();
    // Two valid, noisy PNGs each fit alone but exceed the per-page image budget.
    let mut raw = vec![0u8; 512 * 192 * 3];
    let mut random = 312u32;
    for byte in &mut raw {
        random ^= random << 13;
        random ^= random >> 17;
        random ^= random << 5;
        *byte = random as u8;
    }
    let mut png = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut png, 512, 192);
        encoder.set_color(png::ColorType::Rgb);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&raw)
            .unwrap();
    }
    assert!(png.len() > 256 * 1024 && png.len() < 512 * 1024);
    db.execute("UPDATE asset_keyframes SET body=?1,metadata=json_set(metadata,'$.width',512,'$.height',192)", [&png]).unwrap();
    let budget = call("mcp_get_keyframes", json!({"asset_id":"asset-a"})).unwrap();
    assert_eq!(budget["frames"]["items"][0]["image_status"], "included");
    assert_eq!(
        budget["frames"]["items"][1]["image_status"],
        "page_budget_exceeded"
    );
    let retry = call(
        "mcp_get_keyframes",
        json!({"asset_id":"asset-a","limit":1,"cursor":"media-frame-0"}),
    )
    .unwrap();
    assert_eq!(retry["frames"]["items"][0]["image_status"], "included");
    let mut large = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut large, 512, 384);
        encoder.set_color(png::ColorType::Rgb);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&raw.repeat(2))
            .unwrap();
    }
    assert!(large.len() > 512 * 1024);
    db.execute(
        "UPDATE asset_keyframes SET body=?1,metadata=json_set(metadata,'$.height',384)",
        [&large],
    )
    .unwrap();
    let oversized = call("mcp_get_keyframes", json!({"asset_id":"asset-a"})).unwrap();
    let preview = &oversized["frames"]["items"][0];
    assert_eq!(preview["image_status"], "included");
    assert_eq!(preview["image_is_preview"], true);
    assert_eq!(
        (preview["width"].as_u64(), preview["height"].as_u64()),
        (Some(512), Some(384))
    );
    assert_eq!(
        (
            preview["image_width"].as_u64(),
            preview["image_height"].as_u64()
        ),
        (Some(384), Some(288))
    );
    let preview_body = STANDARD
        .decode(preview["image_base64"].as_str().unwrap())
        .unwrap();
    assert!(preview_body.len() <= 512 * 1024);
    library::validate_frame_image(&preview_body, 384, 288).unwrap();
    assert_eq!(
        library::keyframe_image(&db, "asset-a", "media-frame-0").unwrap(),
        large
    );
    // Restore saved small images for the missing-source read below.
    db.execute(
        "UPDATE asset_keyframes SET body=?1,metadata=json_set(metadata,'$.width',16,'$.height',16)",
        [&image],
    )
    .unwrap();
    std::fs::remove_file(root.path().join("PRIVATE_MEDIA_PATH.mp4")).unwrap();
    let missing = call("mcp_get_keyframes", json!({"asset_id":"asset-a"})).unwrap();
    assert_eq!(missing["frames"]["items"][0]["missing"], true);
    assert_eq!(missing["frames"]["items"][0]["source_changed"], true);
    assert_eq!(missing["frames"]["items"][0]["image_status"], "included");
}
