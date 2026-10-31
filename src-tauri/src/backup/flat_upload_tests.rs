use super::*;
use crate::backup::{sync_delete, sync_read};

fn key(reference: &MediaRef) -> &str {
    let MediaLocation::S3Object { locator } = &reference.location else {
        panic!("S3 reference expected");
    };
    locator
}
fn cover(core: &Core, space: &str, id: &str, body: &[u8]) {
    core.backup_db(|db| {
        db.execute("INSERT INTO asset_covers VALUES(?1,?2) ON CONFLICT(asset_id) DO UPDATE SET body=excluded.body",params![id,body])?;
        sync_engine::mark_local(db,id,Default::default(),false)?;
        let (raw,revision):(String,i64)=db.query_row("SELECT pending_json,local_revision FROM backup_space_assets WHERE space_id=?1 AND local_asset_id=?2",params![space,id],|r|Ok((r.get(0)?,r.get(1)?)))?;
        let mut pending:sync_engine::PendingChanges=serde_json::from_str(&raw)?;
        pending.cover=Some(revision as u64);
        db.execute("UPDATE backup_space_assets SET pending_json=?3 WHERE space_id=?1 AND local_asset_id=?2",params![space,id,serde_json::to_string(&pending)?])?;Ok(())
    }).unwrap();
}
const FIRST: &[u8] = b"\x89PNG\r\n\x1a\nfirst cover";
const SECOND: &[u8] = b"\x89PNG\r\n\x1a\nsecond cover";

#[tokio::test]
async fn flat_video_cover_and_transcript_share_one_asset_directory() {
    let (f, a, engine) = sync_fixture().await;
    f.core.verify_backup_asset(&f.id).await.unwrap();
    cover(&f.core, &f.space.space_id, &f.id, FIRST);
    local_text(&f.core, &f.id, "same directory transcript");
    engine
        .execute_upload(&claim(&engine, &f.core))
        .await
        .unwrap();
    let remote = a.read_asset(&f.id).await.unwrap().unwrap();
    let directory = format!("test/media/{}/", f.id);
    let device = f.core.backup_db(repository::device_id).unwrap();
    for (kind, reference) in [
        ("video", remote.snapshot.media.as_ref().unwrap()),
        ("cover", remote.snapshot.cover.as_ref().unwrap()),
        ("transcript", remote.snapshot.transcript.as_ref().unwrap()),
    ] {
        let name = key(reference)
            .strip_prefix(&directory)
            .expect("different asset directory");
        assert!(!name.contains('/'), "unexpected nested directory: {name}");
        assert!(name.starts_with(&format!("{kind}-")));
        assert!(!name.contains(&device));
        assert_eq!(
            key(reference),
            format!(
                "{directory}{kind}-{}.{}",
                reference.sha256, reference.container
            )
        );
        assert!(f.store.head(key(reference)).await.unwrap().is_some());
    }
    assert!(!a.capabilities().resumable_upload);
    assert!(!f
        .store
        .0
        .lock()
        .unwrap()
        .objects
        .keys()
        .any(|name| name.starts_with("test/transcripts/")));
    let (_root, other, b, _) = second_device(&f).await;
    let local = other.backup_assets().unwrap()[0].asset_id.clone();
    assert_eq!(
        sync_read::cover(&other, b.as_ref(), &local).await.unwrap(),
        FIRST
    );
    assert_eq!(
        sync_read::transcript(&other, b.as_ref(), &local)
            .await
            .unwrap()
            .unwrap()
            .text,
        "same directory transcript"
    );
    assert_eq!(
        f.store.0.lock().unwrap().video_gets,
        0,
        "listing or small metadata restored the video"
    );
    let restored = sync_read::ensure(&other, b.as_ref(), &local).await.unwrap();
    assert_eq!(
        std::fs::read(restored).unwrap(),
        std::fs::read(f.root.path().join("video.mp4")).unwrap()
    );
}

#[tokio::test]
async fn flat_historical_nested_references_still_restore_and_delete() {
    let (f, a, engine) = sync_fixture().await;
    f.core.verify_backup_asset(&f.id).await.unwrap();
    cover(&f.core, &f.space.space_id, &f.id, FIRST);
    local_text(&f.core, &f.id, "historical transcript");
    engine
        .execute_upload(&claim(&engine, &f.core))
        .await
        .unwrap();
    let record_key = format!("test/records/{}.json", f.id);
    let current = f.store.get(&record_key).await.unwrap().unwrap();
    let mut wire: serde_json::Value = serde_json::from_slice(&current.body).unwrap();
    let device = f.core.backup_db(repository::device_id).unwrap();
    let mut historical = Vec::new();
    for (field, folder, kind) in [
        ("media", "media", "video"),
        ("cover", "media", "cover"),
        ("transcript", "transcripts", "transcript"),
    ] {
        let old_key = wire[field]["key"].as_str().unwrap().to_owned();
        let destination = format!(
            "test/{folder}/{}/{}/{device}/{}/{kind}.{}",
            f.id,
            wire[field]["sha256"].as_str().unwrap(),
            uuid::Uuid::new_v4(),
            wire[field]["container"].as_str().unwrap()
        );
        let mut store = f.store.0.lock().unwrap();
        let object = store.objects.remove(&old_key).unwrap();
        store.objects.insert(destination.clone(), object);
        wire[field]["key"] = destination.clone().into();
        historical.push(destination);
    }
    f.store
        .write(Write::Put {
            key: record_key,
            body: serde_json::to_vec(&wire).unwrap(),
            condition: Condition::Match(current.etag),
        })
        .await
        .unwrap();
    let (_root, other, b, bengine) = second_device(&f).await;
    let local = other.backup_assets().unwrap()[0].asset_id.clone();
    assert_eq!(
        sync_read::cover(&other, b.as_ref(), &local).await.unwrap(),
        FIRST
    );
    assert_eq!(
        sync_read::transcript(&other, b.as_ref(), &local)
            .await
            .unwrap()
            .unwrap()
            .text,
        "historical transcript"
    );
    let path = sync_read::ensure(&other, b.as_ref(), &local).await.unwrap();
    assert!(path.is_file());
    assert!(f.store.0.lock().unwrap().video_gets > 0);
    // Reading the old record must not rewrite or upload any historical object.
    for name in &historical {
        assert!(f.store.head(name).await.unwrap().is_some());
    }
    assert_eq!(
        f.store
            .0
            .lock()
            .unwrap()
            .objects
            .keys()
            .filter(|name| name.starts_with(&format!("test/media/{}/", f.id)))
            .count(),
        2
    );
    sync_delete::delete(&other, &b.scope().space_id, &local)
        .await
        .unwrap();
    delete_pass(&other, b.as_ref(), &deletion_operation(&other))
        .await
        .unwrap();
    for name in &historical {
        assert!(f.store.head(name).await.unwrap().is_none());
    }
    bengine.pull().await.unwrap();
    assert!(a.read_asset(&f.id).await.unwrap().unwrap().snapshot.deleted);
}

#[tokio::test]
async fn flat_failed_metadata_publish_keeps_previous_cover_and_transcript_valid() {
    let (f, a, engine) = sync_fixture().await;
    f.core.verify_backup_asset(&f.id).await.unwrap();
    cover(&f.core, &f.space.space_id, &f.id, FIRST);
    local_text(&f.core, &f.id, "first published text");
    engine
        .execute_upload(&claim(&engine, &f.core))
        .await
        .unwrap();
    let before = a.read_asset(&f.id).await.unwrap().unwrap();
    let original_cover = before.snapshot.cover.clone().unwrap();
    let original_text = before.snapshot.transcript.clone().unwrap();
    cover(&f.core, &f.space.space_id, &f.id, SECOND);
    local_text(&f.core, &f.id, "unpublished second text");
    let operation = claim(&engine, &f.core);
    f.store.0.lock().unwrap().fail_record = true;
    assert!(engine.execute_upload(&operation).await.is_err());
    let current = a.read_asset(&f.id).await.unwrap().unwrap();
    assert_eq!(current.snapshot, before.snapshot);
    assert_eq!(
        f.store
            .get(key(&original_cover))
            .await
            .unwrap()
            .unwrap()
            .body,
        FIRST
    );
    let previous = f
        .store
        .get(key(&original_text))
        .await
        .unwrap()
        .unwrap()
        .body;
    assert!(String::from_utf8(previous)
        .unwrap()
        .contains("first published text"));
    f.store.0.lock().unwrap().fail_record = false;
    engine.execute_upload(&operation).await.unwrap();
    let updated = a.read_asset(&f.id).await.unwrap().unwrap();
    assert_ne!(updated.snapshot.cover.as_ref(), Some(&original_cover));
    assert_ne!(updated.snapshot.transcript.as_ref(), Some(&original_text));
    assert_eq!(
        f.store
            .get(key(updated.snapshot.cover.as_ref().unwrap()))
            .await
            .unwrap()
            .unwrap()
            .body,
        SECOND
    );
    sync_delete::delete(&f.core, &f.space.space_id, &f.id)
        .await
        .unwrap();
    delete_pass(&f.core, a.as_ref(), &deletion_operation(&f.core))
        .await
        .unwrap();
    assert!(!f
        .store
        .0
        .lock()
        .unwrap()
        .objects
        .keys()
        .any(|name| name.starts_with(&format!("test/media/{}/", f.id))));
}

#[tokio::test]
async fn flat_unknown_historical_session_is_cleaned_before_new_destination_upload() {
    let (f, a, _engine) = sync_fixture().await;
    f.core.verify_backup_asset(&f.id).await.unwrap();
    let path = f.root.path().join("video.mp4");
    let (sha256, _) = repository::fingerprint(&path).unwrap();
    let device = f.core.backup_db(repository::device_id).unwrap();
    let operation = uuid::Uuid::new_v4().to_string();
    let historical = format!(
        "test/media/{}/{sha256}/{device}/{operation}/video.mp4",
        f.id
    );
    f.store
        .write(Write::CreateMultipart {
            key: historical.clone(),
            sha256: sha256.clone(),
        })
        .await
        .unwrap();
    let prior=OpaqueState{scope:a.scope().clone(),value:serde_json::json!({"completed_bytes":0,"asset_id":f.id,"operation_id":operation,"device_id":device,"key":historical,"sha256":sha256,"bytes":24,"upload_id":null}).to_string()};
    let receipt = a
        .upload_media(
            MediaSource {
                recreate_revision: None,
                asset_id: f.id.clone(),
                kind: "video".into(),
                path,
                bytes: 24,
                sha256: sha256.clone(),
                container: "mp4".into(),
                operation_id: operation,
                device_id: device,
                control: None,
            },
            Some(prior),
        )
        .await
        .unwrap();
    assert!(
        f.store.0.lock().unwrap().uploads.is_empty(),
        "old deep destination's unknown upload was abandoned"
    );
    assert_eq!(
        key(&receipt.reference),
        format!("test/media/{}/video-{sha256}.mp4", f.id)
    );
    assert!(f
        .store
        .head(key(&receipt.reference))
        .await
        .unwrap()
        .is_some());
}
