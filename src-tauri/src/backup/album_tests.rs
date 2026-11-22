use super::*;
use crate::{
    backup::album,
    desktop_core::Task,
    files::{AlbumFile, AlbumVideoFile},
};

pub(crate) fn fixture_album(root: &std::path::Path) -> (std::path::PathBuf, Vec<AlbumFile>) {
    let path = root.join("完整图集");
    std::fs::create_dir(&path).unwrap();
    let image = b"\xff\xd8\xff-original-photo";
    let video = b"\0\0\0\x18ftypisom\0\0\0\0isommp42";
    let mut files = vec![];
    for index in 1..=2 {
        let filename = format!("{index:03}.jpeg");
        std::fs::write(path.join(&filename), image).unwrap();
        let clip = (index == 1).then(|| {
            let filename = format!("{index:03}.mp4");
            std::fs::write(path.join(&filename), video).unwrap();
            AlbumVideoFile {
                filename,
                bytes: video.len() as u64,
                sha256: format!("{:x}", Sha256::digest(video)),
            }
        });
        files.push(AlbumFile {
            filename,
            bytes: image.len() as u64,
            sha256: format!("{:x}", Sha256::digest(image)),
            width: Some(800),
            height: Some(600),
            quality_source: "original".into(),
            video: clip,
        });
    }
    (path, files)
}

async fn album_fixture() -> (Fixture, Arc<S3Provider<Memory>>, SyncEngine, Vec<AlbumFile>) {
    let (f, provider, engine) = sync_fixture().await;
    let (path, files) = fixture_album(f.root.path());
    let mut task: Task = serde_json::from_value(serde_json::json!({"id":f.id,"revision":0,"status":"completed","video":{"kind":"image_album","platform":"douyin","video_id":"123","title":"完整图集"},"bytes":60,"total_bytes":60,"speed_bps":null,"error":null,"path":path,"files":files,"created_at":1,"completed_at":2,"canceling":false})).unwrap();
    f.core
        .backup_db(|db| {
            db.execute("DELETE FROM backup_space_assets", [])?;
            db.execute("DELETE FROM asset_local_files", [])?;
            db.execute("DELETE FROM media_assets", [])?;
            repository::associate(db, &mut task)?;
            db.execute(
                "INSERT INTO download_tasks(id,snapshot) VALUES(?1,?2)",
                params![task.id, serde_json::to_string(&task)?],
            )?;
            Ok(())
        })
        .unwrap();
    assert_eq!(task.asset_id.as_deref(), Some(f.id.as_str()));
    (f, provider, engine, files)
}

#[tokio::test]
async fn album_backup_s3_second_device_restores_every_original_and_live_clip() {
    let (f, provider, engine, files) = album_fixture().await;
    engine
        .execute_upload(&claim(&engine, &f.core))
        .await
        .unwrap();
    let record = provider.read_asset(&f.id).await.unwrap().unwrap().snapshot;
    assert_eq!(record.media.as_ref().unwrap().container, "tar");
    assert_eq!(record.fields["source"]["kind"], "image_album");
    let (_root, other, remote, _) = second_device(&f).await;
    let asset = other.backup_assets().unwrap().remove(0);
    assert!(asset.path.is_none());
    let directory = other.settings().download_directory.join("完整图集");
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(directory.join("user.txt"), b"keep").unwrap();
    let restored = sync_read::ensure(&other, remote.as_ref(), &asset.asset_id)
        .await
        .unwrap();
    assert_ne!(restored, directory);
    assert!(restored.is_dir());
    crate::files::validate_album(&restored, &files).unwrap();
    for file in &files {
        assert_eq!(
            std::fs::read(restored.join(&file.filename)).unwrap(),
            std::fs::read(f.root.path().join("完整图集").join(&file.filename)).unwrap()
        );
        if let Some(video) = &file.video {
            assert_eq!(
                std::fs::read(restored.join(&video.filename)).unwrap(),
                std::fs::read(f.root.path().join("完整图集").join(&video.filename)).unwrap()
            );
        }
    }
    assert_eq!(
        other.image(&asset.asset_id, 0).unwrap(),
        std::fs::read(restored.join("001.jpeg")).unwrap()
    );
    assert_eq!(
        other.album_video_path(&asset.asset_id, 0).unwrap(),
        restored.join("001.mp4")
    );
    let clip = std::fs::read(restored.join("001.mp4")).unwrap();
    std::fs::write(restored.join("001.mp4"), []).unwrap();
    assert!(other.album_video_path(&asset.asset_id, 0).is_err());
    std::fs::write(restored.join("001.mp4"), &clip).unwrap();
    assert_eq!(
        sync_read::ensure(&other, remote.as_ref(), &asset.asset_id)
            .await
            .unwrap(),
        restored
    );
    assert_eq!(std::fs::read(directory.join("user.txt")).unwrap(), b"keep");
    std::fs::write(restored.join("extra.txt"), b"keep too").unwrap();
    super::super::super::lifecycle::remove_local(&other, &asset.asset_id)
        .await
        .unwrap();
    assert!(!restored.join("001.mp4").exists());
    assert_eq!(
        std::fs::read(restored.join("extra.txt")).unwrap(),
        b"keep too"
    );
    let restored_again = sync_read::ensure(&other, remote.as_ref(), &asset.asset_id)
        .await
        .unwrap();
    crate::files::validate_album(&restored_again, &files).unwrap();
}

#[tokio::test]
async fn album_backup_missing_or_changed_live_clip_cannot_publish() {
    let (f, provider, engine, _) = album_fixture().await;
    std::fs::write(f.root.path().join("完整图集/001.mp4"), b"corrupt").unwrap();
    assert!(engine
        .execute_upload(&claim(&engine, &f.core))
        .await
        .is_err());
    assert!(provider.read_asset(&f.id).await.unwrap().is_none());
}

#[tokio::test]
async fn album_backup_registers_valid_legacy_history_after_restart() {
    let (f, provider, engine, _) = album_fixture().await;
    f.core
        .backup_db(|db| {
            db.execute("DELETE FROM backup_space_assets", [])?;
            db.execute("DELETE FROM asset_local_files", [])?;
            db.execute("DELETE FROM media_assets", [])?;
            db.execute(
                "UPDATE download_tasks SET snapshot=json_set(snapshot,'$.asset_id',NULL)",
                [],
            )?;
            Ok(())
        })
        .unwrap();
    drop(engine);
    drop(provider);
    let Fixture {
        root,
        core,
        space,
        id,
        ..
    } = f;
    core.shutdown().await;
    drop(core);
    let reopened = Core::open(
        &root.path().join("app.sqlite"),
        root.path().join("downloads"),
        "unused".into(),
        vec![],
        |_| {},
    )
    .unwrap();
    assert_eq!(
        reopened.register_backup_history(&space.space_id).unwrap(),
        1
    );
    assert_eq!(reopened.list()[0].asset_id.as_deref(), Some(id.as_str()));
    assert_eq!(
        reopened.register_backup_history(&space.space_id).unwrap(),
        0
    );
    assert!(crate::backup::album::is_album(
        &reopened.backup_asset(&id).unwrap().unwrap()
    ));
}

#[test]
fn album_backup_archive_is_deterministic_and_rejects_corruption_cancel_and_foreign_members() {
    let root = tempfile::tempdir().unwrap();
    let (path, files) = fixture_album(root.path());
    let first = album::pack(&path, &files, || false).unwrap();
    let second = album::pack(&path, &files, || false).unwrap();
    let bytes = std::fs::read(first.path()).unwrap();
    assert_eq!(bytes, std::fs::read(second.path()).unwrap());
    let asset = crate::backup::model::Asset { asset_id:"album".into(),task_id:None,
        metadata:serde_json::json!({"kind":"image_album","album_files":files}),original_filename:"album".into(),
        content_sha256:None,content_bytes:0,asset_state:"active".into(),path:Some(path.clone()),local_state:"present".into() };
    let (hash, fingerprint)=album::fingerprint(&asset,||false).unwrap();
    assert_eq!(hash,format!("{:x}",Sha256::digest(&bytes)),"streaming identity must match historical TAR bytes");
    assert_eq!(fingerprint["size"],bytes.len() as u64);

    assert!(album::pack(&path, &files, || true).is_err());
    let target = tempfile::tempdir().unwrap();
    assert!(album::extract(first.path(), &files, target.path(), || true).is_err());
    let mut bad = files.clone();
    bad[0].filename = "../outside.jpeg".into();
    assert!(album::extract(first.path(), &bad, target.path(), || false).is_err());
    let mut bad_bytes = bytes;
    // First actual media member follows the manifest's padded body and header.
    let image = b"\xff\xd8\xff-original-photo";
    let position = bad_bytes
        .windows(image.len())
        .position(|v| v == image)
        .unwrap();
    bad_bytes[position + 4] ^= 1;
    let archive = root.path().join("corrupt.tar");
    std::fs::write(&archive, bad_bytes).unwrap();
    assert!(album::extract(&archive, &files, target.path(), || false).is_err());
}

#[tokio::test]
async fn album_backup_local_cleanup_retries_after_partial_delete_and_preserves_extras() {
    let (f, _, _, _) = album_fixture().await;
    let path = f.root.path().join("完整图集");
    std::fs::write(path.join("user.txt"), b"keep").unwrap();
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // Allow the verification read but deny deletion of this live clip.
        let held = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(3)
            .open(path.join("001.mp4"))
            .unwrap();
        assert!(crate::backup::lifecycle::remove_local(&f.core, &f.id)
            .await
            .is_err());
        assert!(!path.join("001.jpeg").exists());
        assert!(path.join("001.mp4").exists());
        drop(held);
    }
    #[cfg(not(windows))]
    std::fs::remove_file(path.join("001.jpeg")).unwrap();
    crate::backup::lifecycle::remove_local(&f.core, &f.id)
        .await
        .unwrap();
    assert!(!path.join("001.mp4").exists());
    assert!(!path.join("002.jpeg").exists());
    assert_eq!(std::fs::read(path.join("user.txt")).unwrap(), b"keep");
    assert!(f.core.backup_asset(&f.id).unwrap().unwrap().path.is_none());
}
