use super::*;
use grammers_client::media::Downloadable;
use crate::desktop_core::{Core, Status, Task};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};
use tokio::sync::Semaphore;

#[derive(Default)]
struct Fixture {
    gates: Mutex<std::collections::HashMap<u32, Arc<Semaphore>>>,
    active: AtomicUsize,
    peak: AtomicUsize,
    public_only: std::sync::atomic::AtomicBool,
    public_channel: AtomicUsize,
    auth_denied: std::sync::atomic::AtomicBool,
    inspect_failures: AtomicUsize,
    next_failures: AtomicUsize,
    inspect_error: Mutex<Option<String>>,
    inspected: Mutex<Vec<u32>>,
    cover: Mutex<Option<Vec<u8>>>,
    cover_reads: AtomicUsize,
    avatar: Mutex<Option<std::result::Result<Vec<u8>, &'static str>>>,
    avatar_reads: AtomicUsize,
    native_attributes: std::sync::atomic::AtomicBool,
    channel_title: Mutex<Option<String>>,
}
struct Fake {
    fixture: Arc<Fixture>,
    media: Option<Media>,
    delivered: bool,
    downloading: bool,
}
impl Drop for Fake {
    fn drop(&mut self) {
        if self.downloading {
            self.fixture.active.fetch_sub(1, Ordering::SeqCst);
        }
    }
}
fn media(message: u32) -> Media {
    Media {
        channel: 100,
        channel_title: None,
        message,
        document: message as i64,
        size: 4096,
        mime: "video/mp4".into(),
        name: format!("video-{message}.mp4"),
        container: "mp4".into(),
        width: None,
        height: None,
        duration_ms: None,
    }
}
fn body() -> Vec<u8> {
    let mut b = vec![0; 4096];
    b[4..8].copy_from_slice(b"ftyp");
    b
}
fn png(color: [u8; 3]) -> Vec<u8> {
    let mut encoded = std::io::Cursor::new(Vec::new());
    image::RgbImage::from_pixel(40, 60, image::Rgb(color))
        .write_to(&mut encoded, image::ImageFormat::Png)
        .unwrap();
    encoded.into_inner()
}

#[test]
fn metadata_fill_preserves_native_values_and_newer_cache_identity() {
    let link = message_link("https://t.me/c/100/9").unwrap();
    let mut native = preview(&media(9), &link);
    native["width"] = 1080.into();
    native["height"] = 1920.into();
    fill_missing_attributes(&mut native, &json!({"width":1920,"height":1080,"duration_ms":2500}));
    assert_eq!((native["width"].as_u64(), native["height"].as_u64(), native["duration_ms"].as_u64()), (Some(1080),Some(1920),Some(2500)));
    let mut db = rusqlite::Connection::open_in_memory().unwrap();
    crate::parse_cache::init(&db).unwrap();
    let mut fresh = preview(&media(9), &link);
    fresh["title"] = "new caption".into();
    crate::parse_cache::save(&mut db, &fresh, vec![], Some(png([10,20,30])), 1).unwrap();
    crate::parse_cache::update_telegram_metadata(&db, &native).unwrap();
    let cached: String = db.query_row("SELECT metadata FROM parse_cache", [], |row|row.get(0)).unwrap();
    let cached:Value=serde_json::from_str(&cached).unwrap();
    assert_eq!(cached["title"], "new caption");
    assert_eq!(cached["width"],1080);
    fresh["telegram_document_id"] = 99.into();
    let new_cover=png([90,80,70]);
    crate::parse_cache::save(&mut db, &fresh, vec![], Some(new_cover.clone()), 2).unwrap();
    crate::parse_cache::update_telegram_metadata(&db, &native).unwrap();
    let (cached,cover):(String,Vec<u8>)=db.query_row("SELECT metadata,cover FROM parse_cache", [], |row|Ok((row.get(0)?,row.get(1)?))).unwrap();
    let cached:Value=serde_json::from_str(&cached).unwrap();
    assert_eq!(cached["telegram_document_id"],99);
    assert!(cached["width"].is_null());
    assert_eq!(cached["title"],"new caption");
    assert_eq!(cover,new_cover);
}

#[test]
fn channel_names_are_display_metadata_and_cannot_change_document_identity() {
    let link = message_link("https://t.me/c/100/9").unwrap();
    let original = media(9);
    let mut renamed = original.clone();
    renamed.channel_title = Some("频道新名称".into());
    assert_eq!(original, renamed);
    let mut different = renamed.clone();
    different.document += 1;
    assert_ne!(original, different);
    let fresh = preview(&renamed, &link);
    assert_eq!(fresh["author_profile"]["id"], "100");
    assert_eq!(fresh["author_profile"]["nickname"], "频道新名称");
    let mut legacy = preview(&original, &link);
    legacy["author_profile"]["handle"] = "known_public_name".into();
    merge_attributes(&mut legacy, &fresh);
    assert_eq!(legacy["author"], "频道新名称");
    assert_eq!(legacy["author_profile"]["handle"], "known_public_name");
    let mut foreign = fresh;
    foreign["author_profile"]["id"] = "999".into();
    foreign["author_profile"]["nickname"] = "其它频道".into();
    merge_attributes(&mut legacy, &foreign);
    assert_eq!(legacy["author_profile"]["nickname"], "频道新名称");
}

#[tokio::test]
async fn channel_title_is_refreshed_on_download_and_persisted_without_renaming_the_file() {
    let root = tempfile::tempdir().unwrap();
    let fixture = Arc::new(Fixture::default());
    *fixture.channel_title.lock().unwrap() = Some("最初频道名".into());
    let app = app(root.path(), &fixture);
    let task = resolve(&app, 9).await;
    assert_eq!(task.video.as_ref().unwrap()["author_profile"]["nickname"], "最初频道名");
    *fixture.channel_title.lock().unwrap() = Some("改名后的频道".into());
    app.download(&task.id, false).unwrap();
    let done = wait(&app, &task.id, Status::Completed).await;
    let video = done.video.as_ref().unwrap();
    assert_eq!(video["author_profile"]["nickname"], "改名后的频道");
    assert_eq!(video["author_profile"]["id"], "100");
    assert_eq!(video["video_id"], "100_9");
    assert_eq!(std::fs::read(done.path.as_ref().unwrap()).unwrap(), body());
    let db = rusqlite::Connection::open(root.path().join("tasks.sqlite")).unwrap();
    let raw: String = db.query_row("SELECT metadata FROM parse_cache WHERE platform='telegram'", [], |r| r.get(0)).unwrap();
    assert_eq!(serde_json::from_str::<Value>(&raw).unwrap()["author_profile"]["nickname"], "改名后的频道");
    drop(db);
    app.shutdown().await;
    drop(app);
    let reopened = super::tests::app(root.path(), &fixture);
    let stored = reopened.list().into_iter().find(|t| t.id == task.id).unwrap();
    assert_eq!(stored.video.unwrap()["author_profile"]["nickname"], "改名后的频道");
    assert_eq!(stored.path, done.path);
    reopened.shutdown().await;
}

#[tokio::test]
async fn telegram_channel_avatar_survives_download_and_restart() {
    let root = tempfile::tempdir().unwrap();
    let fixture = Arc::new(Fixture::default());
    let avatar = png([180, 40, 90]);
    let cover = png([40, 90, 180]);
    *fixture.avatar.lock().unwrap() = Some(Ok(avatar.clone()));
    *fixture.cover.lock().unwrap() = Some(cover.clone());
    let application = app(root.path(), &fixture);

    let private = resolve(&application, 9).await;
    let public = application.resolve("https://t.me/example_channel/10".into()).unwrap();
    let public = wait(&application, &public.id, Status::Ready).await;
    for task in [&private, &public] {
        assert_eq!(task.video.as_ref().unwrap()["author_profile"]["avatar_cached"], true);
        assert_eq!(application.author_avatar(&task.id).unwrap(), avatar);
        assert_eq!(application.cover(&task.id).unwrap(), cover);
    }

    application.download(&private.id, false).unwrap();
    let done = wait(&application, &private.id, Status::Completed).await;
    assert_eq!(done.video.unwrap()["author_profile"]["avatar_cached"], true);
    assert_eq!(fixture.avatar_reads.load(Ordering::SeqCst), 2);
    application.shutdown().await;
    drop(application);

    let reopened = app(root.path(), &fixture);
    let stored = reopened.list().into_iter().find(|task| task.id == private.id).unwrap();
    assert_eq!(stored.video.unwrap()["author_profile"]["avatar_cached"], true);
    assert_eq!(reopened.author_avatar(&private.id).unwrap(), avatar);
    reopened.shutdown().await;
}

#[tokio::test]
async fn telegram_channel_avatar_is_optional_when_missing_unavailable_or_invalid() {
    let root = tempfile::tempdir().unwrap();
    let fixture = Arc::new(Fixture::default());
    let application = app(root.path(), &fixture);
    let cases = [None, Some(Err("TELEGRAM_CONNECTION_ERROR")), Some(Ok(b"invalid image".to_vec()))];
    for (message, avatar) in (9..).zip(cases) {
        *fixture.avatar.lock().unwrap() = avatar;
        let task = resolve(&application, message).await;
        assert_eq!(task.video.unwrap()["author_profile"]["avatar_cached"], false);
        assert!(application.author_avatar(&task.id).is_err());
    }
    application.shutdown().await;
}

fn sdk_document(
    custom: Option<Vec<u8>>,
    thumbnail: Option<Vec<u8>>,
) -> grammers_client::media::Document {
    let size = |name: &str, body: Vec<u8>| {
        tl::types::PhotoCachedSize {
            r#type: name.into(),
            w: 40,
            h: 60,
            bytes: body,
        }
        .into()
    };
    let raw = tl::types::Document {
        id: 9,
        access_hash: 0,
        file_reference: vec![],
        date: 0,
        mime_type: "video/mp4".into(),
        size: 4096,
        thumbs: thumbnail.map(|body| vec![size("thumb", body)]),
        video_thumbs: None,
        dc_id: 1,
        attributes: vec![tl::types::DocumentAttributeVideo {
            round_message: false,
            supports_streaming: true,
            nosound: false,
            duration: 2.5,
            w: 1080,
            h: 1920,
            preload_prefix_size: None,
            video_start_ts: None,
            video_codec: None,
        }
        .into()],
    };
    grammers_client::media::Document::from_raw_media(tl::types::MessageMediaDocument {
        nopremium: false,
        spoiler: false,
        video: true,
        round: false,
        voice: false,
        document: Some(raw.into()),
        alt_documents: None,
        video_cover: custom.map(|body| {
            tl::types::Photo {
                has_stickers: false,
                id: 1,
                access_hash: 0,
                file_reference: vec![],
                date: 0,
                sizes: vec![size("custom", body)],
                video_sizes: None,
                dc_id: 1,
            }
            .into()
        }),
        video_timestamp: None,
        ttl_seconds: None,
    })
}

#[tokio::test]
async fn custom_cover_precedes_thumbnail_and_broken_or_missing_covers_fall_back() {
    let custom = png([210, 30, 40]);
    let thumbnail = png([30, 40, 210]);
    for (custom_body, expected, calls) in [
        (Some(custom.clone()), Some(custom), 1),
        (Some(b"broken".to_vec()), Some(thumbnail.clone()), 2),
        (None, Some(thumbnail.clone()), 1),
    ] {
        let doc = sdk_document(custom_body, Some(thumbnail.clone()));
        let candidates = cover_candidates(&doc);
        let reads = AtomicUsize::new(0);
        let actual = first_available_cover(&candidates, |size| {
            reads.fetch_add(1, Ordering::SeqCst);
            Box::pin(async move { size.to_data().context("COVER_UNAVAILABLE") })
        })
        .await
        .unwrap();
        assert_eq!(actual, expected);
        assert_eq!(reads.load(Ordering::SeqCst), calls);
    }
    let doc = sdk_document(None, None);
    assert!(
        first_available_cover(&cover_candidates(&doc), |_| Box::pin(async {
            bail!("must not read missing cover")
        }))
        .await
        .unwrap()
        .is_none()
    );
    let raw = match sdk_document(None, None).raw.document.unwrap() {
        tl::enums::Document::Document(raw) => raw,
        _ => unreachable!(),
    };
    let media = media_info(100, 9, &raw).unwrap();
    let metadata = preview(&media, &message_link("https://t.me/c/100/9").unwrap());
    assert_eq!(
        (
            metadata["width"].as_u64(),
            metadata["height"].as_u64(),
            metadata["duration_ms"].as_u64()
        ),
        (Some(1080), Some(1920), Some(2500))
    );
    assert_eq!(metadata["formats"][0]["height"], 1920);
}

#[tokio::test]
async fn downloaded_cover_survives_metadata_fill_and_restart_without_frame_extraction() {
    let root = tempfile::tempdir().unwrap();
    let fixture = Arc::new(Fixture::default());
    let original = png([30, 200, 50]);
    *fixture.cover.lock().unwrap() = Some(original.clone());
    let application = app(root.path(), &fixture);
    let task = resolve(&application, 9).await;
    assert!(task.cover_cached);
    assert_eq!(application.cover(&task.id).unwrap(), original);
    application.download(&task.id, false).unwrap();
    let done = wait(&application, &task.id, Status::Completed).await;
    assert_eq!(done.video.as_ref().unwrap()["width"], 1920);
    assert_eq!(done.video.as_ref().unwrap()["formats"][0]["height"], 1080);
    assert_eq!(application.cover(&task.id).unwrap(), original);
    assert!(!root.path().join("cover-started").exists());
    assert_eq!(std::fs::read(done.path.as_ref().unwrap()).unwrap(), body());
    application.shutdown().await;
    drop(application);
    let db = rusqlite::Connection::open(root.path().join("tasks.sqlite")).unwrap();
    db.execute("UPDATE download_tasks SET snapshot=json_set(snapshot,'$.video.width',NULL,'$.video.height',NULL,'$.video.duration_ms',NULL) WHERE id=?",[&task.id]).unwrap();
    let asset = done.asset_id.as_ref().unwrap();
    db.execute("UPDATE media_assets SET metadata_json=json_set(metadata_json,'$.width',NULL,'$.height',NULL,'$.duration_ms',NULL,'$.title','用户修改的标题') WHERE asset_id=?",[asset]).unwrap();
    db.execute("INSERT INTO backup_spaces VALUES('repair-space','repair-library','https://example.test','bucket','test/',NULL,0)",[]).unwrap();
    db.execute("INSERT INTO backup_space_assets(space_id,remote_asset_id,local_asset_id,pending_json) VALUES('repair-space',?1,?1,'{}')",[asset]).unwrap();
    drop(db);
    let reopened = app(root.path(), &fixture);
    reopened.start_backup_service();
    timeout(Duration::from_secs(5), async {
        loop {
            if reopened
                .list()
                .iter()
                .find(|t| t.id == task.id)
                .unwrap()
                .video
                .as_ref()
                .unwrap()["width"]
                == 1920
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|error| {
        panic!(
            "metadata repair: {error}; tasks={}",
            serde_json::to_string(&reopened.list()).unwrap()
        )
    });
    assert_eq!(reopened.cover(&task.id).unwrap(), original);
    reopened.backup_db(|db| {
        let raw: String = db.query_row("SELECT metadata_json FROM media_assets WHERE asset_id=?",[asset],|r|r.get(0))?;
        let metadata: serde_json::Value = serde_json::from_str(&raw)?;
        assert_eq!(metadata["title"],"用户修改的标题");
        assert_eq!(metadata["width"],1920);
        assert_eq!(metadata["height"],1080);
        assert!(metadata["duration_ms"].as_u64().unwrap()>0);
        let raw: String=db.query_row("SELECT pending_json FROM backup_space_assets WHERE local_asset_id=?",[asset],|r|r.get(0))?;
        let pending: crate::backup::sync_engine::PendingChanges=serde_json::from_str(&raw)?;
        for field in ["width","height","duration_ms"] {assert_eq!(pending.fields[field].value,metadata[field]);}
        assert_eq!(pending.fields.len(),3);
        Ok(())
    }).unwrap();
    assert_eq!(std::fs::read(done.path.unwrap()).unwrap(), body());
    reopened.shutdown().await;
}

#[tokio::test]
async fn optional_cover_write_failure_keeps_completed_video_and_cached_preview() {
    let root = tempfile::tempdir().unwrap();
    let fixture = Arc::new(Fixture::default());
    let original = png([30, 90, 200]);
    *fixture.cover.lock().unwrap() = Some(original.clone());
    let gate = Arc::new(Semaphore::new(0));
    fixture.gates.lock().unwrap().insert(9, gate.clone());
    let application = app(root.path(), &fixture);
    let task = resolve(&application, 9).await;
    application.download(&task.id, false).unwrap();
    timeout(Duration::from_secs(3), async {
        while fixture.active.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let db = rusqlite::Connection::open(root.path().join("tasks.sqlite")).unwrap();
    let stage: String = db
        .query_row(
            "SELECT path FROM task_staging WHERE task_id=?",
            [&task.id],
            |row| row.get(0),
        )
        .unwrap();
    drop(db);
    std::fs::create_dir(Path::new(&stage).join("media/cover")).unwrap();
    gate.add_permits(1);
    let done = wait(&application, &task.id, Status::Completed).await;
    assert_eq!(std::fs::read(done.path.unwrap()).unwrap(), body());
    assert_eq!(done.cover_error.as_deref(), Some("COVER_UNAVAILABLE"));
    assert_eq!(application.cover(&task.id).unwrap(), original);
    application.shutdown().await;
}

#[tokio::test]
async fn native_attributes_keep_downloaded_cover_without_metadata_probe() {
    let root = tempfile::tempdir().unwrap();
    let fixture = Arc::new(Fixture::default());
    fixture.native_attributes.store(true, Ordering::SeqCst);
    let original = png([90, 40, 180]);
    *fixture.cover.lock().unwrap() = Some(original.clone());
    let application = app(root.path(), &fixture);
    let task = resolve(&application, 9).await;
    application.download(&task.id, false).unwrap();
    let done = wait(&application, &task.id, Status::Completed).await;
    assert_eq!(done.video.as_ref().unwrap()["width"], 1080);
    assert_eq!(done.video.as_ref().unwrap()["height"], 1920);
    assert_eq!(done.video.as_ref().unwrap()["duration_ms"], 3000);
    assert_eq!(application.cover(&task.id).unwrap(), original);
    assert!(done.cover_cached);
    assert!(!root.path().join("metadata-probed").exists());
    assert!(!root.path().join("cover-started").exists());
    assert_eq!(std::fs::read(done.path.unwrap()).unwrap(), body());
    application.shutdown().await;
}

impl Transport for Fake {
    fn cover(&mut self) -> ProviderFuture<'_, Option<Vec<u8>>> { Box::pin(async { self.fixture.cover_reads.fetch_add(1, Ordering::SeqCst); Ok(self.fixture.cover.lock().unwrap().clone()) }) }
    fn avatar(&mut self) -> ProviderFuture<'_, Option<Vec<u8>>> {
        Box::pin(async {
            self.fixture.avatar_reads.fetch_add(1, Ordering::SeqCst);
            self.fixture.avatar.lock().unwrap().clone().transpose().map_err(anyhow::Error::msg)
        })
    }
    fn inspect<'a>(&'a mut self, link: &'a MessageLink) -> ProviderFuture<'a, Media> {
        Box::pin(async move {
            if self.fixture.auth_denied.load(Ordering::SeqCst) { bail!("TELEGRAM_AUTH_REQUIRED"); }
            self.fixture.inspected.lock().unwrap().push(link.message);
            if self.fixture.inspect_failures.try_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1)).is_ok() {
                bail!(self.fixture.inspect_error.lock().unwrap().clone().unwrap_or_else(|| "TELEGRAM_CONNECTION_ERROR".into()));
            }
            if self.fixture.public_only.load(Ordering::SeqCst) && link.channel.is_some() {
                bail!("TELEGRAM_CHANNEL_FORBIDDEN");
            }
            let mut value = media(link.message);
            value.channel_title = self.fixture.channel_title.lock().unwrap().clone();
            if self.fixture.native_attributes.load(Ordering::SeqCst) {
                value.width = Some(1080);
                value.height = Some(1920);
                value.duration_ms = Some(3000);
            }
            value.channel = link
                .channel
                .unwrap_or(self.fixture.public_channel.load(Ordering::SeqCst).max(100) as u64);
            self.media = Some(value.clone());
            self.delivered = false;
            Ok(value)
        })
    }
    fn next(&mut self, offset: u64) -> ProviderFuture<'_, Option<ReceivedPart>> {
        Box::pin(async move {
            if self.delivered {
                return Ok(None);
            }
            self.downloading = true;
            let count = self.fixture.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.fixture.peak.fetch_max(count, Ordering::SeqCst);
            let gate = self
                .fixture
                .gates
                .lock()
                .unwrap()
                .get(&self.media.as_ref().unwrap().message)
                .cloned();
            if let Some(gate) = gate {
                let _permit = gate.acquire().await.unwrap();
            }
            if self.fixture.next_failures.try_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1)).is_ok() { bail!("TELEGRAM_CONNECTION_ERROR"); }
            self.delivered = true;
            Ok(Some(ReceivedPart {offset, bytes: body()}))
        })
    }
}
fn app(root: &Path, fixture: &Arc<Fixture>) -> Arc<Core> {
    app_with_engine(root, fixture, "python-must-not-run")
}
fn app_with_engine(root: &Path, fixture: &Arc<Fixture>, program: &str) -> Arc<Core> {
    let project = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let app = Core::open(
        &root.join("tasks.sqlite"),
        root.join("downloads"),
        program.into(),
        vec![project
            .join("tests/fixtures/desktop_engine.py")
            .to_string_lossy()
            .into_owned()],
        |_| {},
    )
    .unwrap();
    *app.telegram_accounts.binding(crate::telegram_accounts::Purpose::Download).session.reconnect_probe.lock().unwrap() = Some(|| Ok(()));
    let fixture_root = root.to_path_buf();
    *app.local_media_fixture.lock().unwrap() = Some(Arc::new(move |operation, source| {
        if operation == "probe" {
            std::fs::write(fixture_root.join("metadata-probed"), "ready")?;
            Ok(json!({"width":1920,"height":1080,"duration_ms":2500}))
        } else {
            std::fs::write(fixture_root.join("cover-started"), "ready")?;
            let body=b"\x89PNG\r\n\x1a\nfixture";
            std::fs::write(source.parent().unwrap().join("cover"),body)?;
            Ok(json!({"cover_filename":"cover","cover_bytes":body.len()}))
        }
    }));
    let fixture = fixture.clone();
    *app.telegram_source.lock().unwrap() = Some(Arc::new(move || {
        Box::new(Fake {
            fixture: fixture.clone(),
            media: None,
            delivered: false,
            downloading: false,
        })
    }));
    app
}
async fn wait(app: &Core, id: &str, status: Status) -> Task {
    timeout(Duration::from_secs(15), async {
        loop {
            let task = app.list().into_iter().find(|t| t.id == id).unwrap();
            if task.status == status {
                return task;
            }
            if matches!(task.status, Status::Failed) && status != Status::Failed {
                panic!("unexpected failure: {:?}", task.error);
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("task did not reach expected state")
}
async fn resolve(app: &Arc<Core>, message: u32) -> Task {
    let task = app
        .resolve(format!("https://t.me/c/100/{message}"))
        .unwrap();
    wait(app, &task.id, Status::Ready).await
}
#[test]
fn links_reject_ambiguous_and_unsupported_targets() {
    for link in [
        "https://t.me/channel/1?single",
        "https://t.me/c/100/1/2",
        "https://t.me/channel/2147483648",
        "https://t.me/channel/0",
        "https://t.me/channel/1#x",
        "https://t.me/c/100/1 https://t.me/c/100/2",
        "https://t.me/s/channel/1",
    ] {
        assert!(message_link(link).is_err(), "{link}");
    }
    let public = message_link("分享 https://telegram.me/CHANNEL/9。").unwrap();
    assert_eq!(public_peer(&public).as_deref(), Some("channel"));
    assert!(validate_identity("100_9", "https://t.me/c/100/9"));
    assert!(!validate_identity("100_9", "https://t.me/c/101/9"));
}
#[tokio::test]
async fn metadata_parse_retries_transient_errors_with_three_downloads_in_progress() {
    let root = tempfile::tempdir().unwrap();
    let fixture = Arc::new(Fixture::default());
    let app = app(root.path(), &fixture);
    app.set_max_downloads(3).unwrap();
    for message in 1..=3 {
        fixture
            .gates
            .lock()
            .unwrap()
            .insert(message, Arc::new(Semaphore::new(0)));
        let task = resolve(&app, message).await;
        app.download(&task.id, false).unwrap();
    }
    timeout(Duration::from_secs(2), async {
        while fixture.active.load(Ordering::SeqCst) != 3 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    fixture.inspect_failures.store(2, Ordering::SeqCst);
    let parsed = resolve(&app, 4).await;
    assert_eq!(parsed.status, Status::Ready);
    assert_eq!(
        fixture
            .inspected
            .lock()
            .unwrap()
            .iter()
            .filter(|m| **m == 4)
            .count(),
        3
    );
    assert_eq!(fixture.active.load(Ordering::SeqCst), 3);
    assert_eq!(
        app.list()
            .iter()
            .filter(|t| t.status == Status::Downloading)
            .count(),
        3
    );
    app.shutdown().await;
}

#[tokio::test]
async fn metadata_parse_does_not_retry_fatal_errors() {
    for error in [
        "TELEGRAM_AUTH_REQUIRED",
        "TELEGRAM_CHANNEL_FORBIDDEN",
        "TELEGRAM_MEDIA_CHANGED",
        "TELEGRAM_FLOOD_WAIT:30",
    ] {
        let root = tempfile::tempdir().unwrap();
        let fixture = Arc::new(Fixture::default());
        fixture.inspect_failures.store(9, Ordering::SeqCst);
        *fixture.inspect_error.lock().unwrap() = Some(error.into());
        let app = app(root.path(), &fixture);
        let task = app.resolve("https://t.me/c/100/9".into()).unwrap();
        let failed = wait(&app, &task.id, Status::Failed).await;
        assert_eq!(failed.error.as_deref(), error.split(':').next());
        let attempts = if matches!(error, "TELEGRAM_CONNECTION_ERROR" | "TELEGRAM_TIMEOUT") {
            3
        } else {
            1
        };
        assert_eq!(fixture.inspected.lock().unwrap().len(), attempts);
        assert_eq!(app.list().len(), 1);
        app.shutdown().await;
    }
}

#[tokio::test]
async fn metadata_parse_cancel_stops_retry_backoff_without_canceling_other_downloads() {
    let root = tempfile::tempdir().unwrap();
    let fixture = Arc::new(Fixture::default());
    let app = app(root.path(), &fixture);
    fixture
        .gates
        .lock()
        .unwrap()
        .insert(1, Arc::new(Semaphore::new(0)));
    let downloading = resolve(&app, 1).await;
    app.download(&downloading.id, false).unwrap();
    timeout(Duration::from_secs(2), async {
        while fixture.active.load(Ordering::SeqCst) != 1 { tokio::task::yield_now().await; }
    }).await.unwrap();
    fixture.inspect_failures.store(9, Ordering::SeqCst);
    let task = app.resolve("https://t.me/c/100/9".into()).unwrap();
    timeout(Duration::from_secs(2), async {
        while !fixture.inspected.lock().unwrap().contains(&9) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    app.cancel(&task.id).await.unwrap();
    assert_eq!(
        app.list().iter().find(|t| t.id == task.id).unwrap().status,
        Status::Canceled
    );
    assert_eq!(
        fixture
            .inspected
            .lock()
            .unwrap()
            .iter()
            .filter(|m| **m == 9)
            .count(),
        1
    );
    assert_eq!(
        app.list()
            .iter()
            .find(|t| t.id == downloading.id)
            .unwrap()
            .status,
        Status::Downloading
    );
    app.shutdown().await;
}

#[tokio::test]
async fn metadata_parse_shutdown_stops_retry_backoff_as_interrupted() {
    let root = tempfile::tempdir().unwrap();
    let fixture = Arc::new(Fixture::default());
    fixture.inspect_failures.store(9, Ordering::SeqCst);
    let app = app(root.path(), &fixture);
    let task = app.resolve("https://t.me/c/100/9".into()).unwrap();
    timeout(Duration::from_secs(2), async {
        while fixture.inspected.lock().unwrap().is_empty() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    timeout(Duration::from_secs(1), app.shutdown())
        .await
        .unwrap();
    let interrupted = app.list().into_iter().find(|t| t.id == task.id).unwrap();
    assert_eq!(interrupted.status, Status::Failed);
    assert_eq!(interrupted.error.as_deref(), Some("INTERRUPTED"));
    assert_eq!(fixture.inspected.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn revoked_download_authorization_clears_the_shared_account_view() {
    use crate::{backup::telegram_session::Phase, telegram_accounts::Purpose};
    let root = tempfile::tempdir().unwrap();
    let fixture = Arc::new(Fixture::default());
    fixture.auth_denied.store(true, Ordering::SeqCst);
    let app = app(root.path(), &fixture);
    app.telegram_accounts.binding(Purpose::Download).session.set_test_view(Phase::Ready, "account-a");
    let task = app.resolve("https://t.me/c/100/9".into()).unwrap();
    let failed = wait(&app, &task.id, Status::Failed).await;
    assert_eq!(failed.error.as_deref(), Some("TELEGRAM_AUTH_REQUIRED"));
    for purpose in [Purpose::Download, Purpose::Backup] {
        let view = app.telegram_accounts.status(purpose);
        assert!(matches!(view.phase, Phase::SignedOut));
        assert!(view.account.is_none());
    }
    app.shutdown().await;
}
#[tokio::test]
async fn account_logout_cancels_download_and_queue_without_signing_out_backup() {
    use crate::{backup::telegram_session::Phase, telegram_accounts::Purpose};
    let root = tempfile::tempdir().unwrap();
    let fixture = Arc::new(Fixture::default());
    fixture.gates.lock().unwrap().insert(1, Arc::new(Semaphore::new(0)));
    let app = app(root.path(), &fixture);
    app.set_max_downloads(1).unwrap();
    let binding = app.telegram_accounts.binding(Purpose::Download);
    binding.session.set_test_view(Phase::Ready, "account-a");
    let a = resolve(&app, 1).await;
    let b = resolve(&app, 2).await;
    app.download(&a.id, false).unwrap();
    app.download(&b.id, false).unwrap();
    wait(&app, &a.id, Status::Downloading).await;
    wait(&app, &b.id, Status::Queued).await;
    timeout(Duration::from_secs(2), async {
        while fixture.active.load(Ordering::SeqCst) != 1 { tokio::task::yield_now().await; }
    }).await.unwrap();
    app.telegram_accounts.logout(&app, Purpose::Download, binding.session.status().operation_id.as_deref().unwrap()).await.unwrap();
    wait(&app, &a.id, Status::Canceled).await;
    wait(&app, &b.id, Status::Canceled).await;
    assert_eq!(fixture.active.load(Ordering::SeqCst), 0);
    assert_eq!(app.telegram_accounts.status(Purpose::Backup).account.unwrap().id, "account-a");
    app.shutdown().await;
}
#[tokio::test]
async fn proxy_change_retries_same_download_and_user_cancel_still_stops_it() {
    use crate::telegram_accounts::Purpose;
    let root = tempfile::tempdir().unwrap();
    let fixture = Arc::new(Fixture::default());
    let gate = Arc::new(Semaphore::new(0));
    fixture.gates.lock().unwrap().insert(1, gate.clone());
    let app = app(root.path(), &fixture);
    let task = resolve(&app, 1).await;
    app.download(&task.id, false).unwrap();
    let session = app.telegram_accounts.binding(Purpose::Download).session;
    for expected_inspections in [2, 3] {
        timeout(Duration::from_secs(3), async {
            while fixture.active.load(Ordering::SeqCst) != 1 { tokio::task::yield_now().await; }
        }).await.unwrap();
        let pause = session.pause_network().await.unwrap();
        drop(pause);
        timeout(Duration::from_secs(3), async {
            while fixture.inspected.lock().unwrap().len() < expected_inspections + 1
                || fixture.active.load(Ordering::SeqCst) != 1 {
                tokio::task::yield_now().await;
            }
        }).await.unwrap();
        let current = app.list().into_iter().find(|value| value.id == task.id).unwrap();
        assert_eq!(current.status, Status::Downloading);
        assert!(current.error.is_none());
    }
    gate.add_permits(1);
    let done = wait(&app, &task.id, Status::Completed).await;
    assert_eq!(std::fs::read(done.path.unwrap()).unwrap(), body());
    let another = resolve(&app, 2).await;
    fixture.gates.lock().unwrap().insert(2, Arc::new(Semaphore::new(0)));
    app.download(&another.id, false).unwrap();
    timeout(Duration::from_secs(3), async {
        while fixture.active.load(Ordering::SeqCst) != 1 { tokio::task::yield_now().await; }
    }).await.unwrap();
    let pause = session.pause_network().await.unwrap();
    app.cancel(&another.id).await.unwrap();
    drop(pause);
    wait(&app, &another.id, Status::Canceled).await;
    assert_eq!(fixture.active.load(Ordering::SeqCst), 0);
    assert_eq!(app.list().len(), 2, "proxy retries cannot create extra tasks");
    app.shutdown().await;
}
#[tokio::test]
async fn duplicate_telegram_in_progress_needs_confirmation_before_creating_another_transfer() {
    for queued in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let fixture = Arc::new(Fixture::default());
        let gate = Arc::new(Semaphore::new(0));
        fixture.gates.lock().unwrap().insert(9, gate.clone());
        let app = app(root.path(), &fixture);
        app.set_max_downloads(1).unwrap();
        let blocker_gate = Arc::new(Semaphore::new(0));
        if queued {
            fixture
                .gates
                .lock()
                .unwrap()
                .insert(8, blocker_gate.clone());
            let blocker = resolve(&app, 8).await;
            app.download(&blocker.id, false).unwrap();
            wait(&app, &blocker.id, Status::Downloading).await;
        }
        let first = resolve(&app, 9).await;
        app.download(&first.id, false).unwrap();
        let second = resolve(&app, 9).await;
        assert_eq!(
            app.download(&second.id, false).unwrap_err().to_string(),
            "ALREADY_DOWNLOADED"
        );
        assert_eq!(
            app.list()
                .iter()
                .find(|t| t.id == second.id)
                .unwrap()
                .status,
            Status::Ready
        );
        assert!(
            !app.list()
                .iter()
                .find(|t| t.id == second.id)
                .unwrap()
                .download_requested
        );
        app.download(&second.id, true).unwrap();
        blocker_gate.add_permits(1);
        gate.add_permits(2);
        let first = wait(&app, &first.id, Status::Completed).await;
        let second = wait(&app, &second.id, Status::Completed).await;
        assert_ne!(first.path, second.path);
        assert_eq!(std::fs::read(first.path.unwrap()).unwrap(), body());
        assert_eq!(std::fs::read(second.path.unwrap()).unwrap(), body());
        app.shutdown().await;
    }
}

#[tokio::test]
async fn original_bytes_cover_cache_noclobber_and_restart() {
    let root = tempfile::tempdir().unwrap();
    let fixture = Arc::new(Fixture::default());
    let app = app(root.path(), &fixture);
    let task = resolve(&app, 9).await;
    assert_eq!(task.video.as_ref().unwrap()["resolver"], "grammers");
    app.download(&task.id, false).unwrap();
    let done = wait(&app, &task.id, Status::Completed).await;
    assert_eq!(std::fs::read(done.path.as_ref().unwrap()).unwrap(), body());
    assert!(done.cover_cached);
    assert!(done.cover_path.is_none());
    assert_eq!(app.cover(&task.id).unwrap(), b"\x89PNG\r\n\x1a\nfixture");
    assert_eq!(
        std::fs::read_dir(root.path().join("downloads"))
            .unwrap()
            .count(),
        1
    );
    let again = resolve(&app, 9).await;
    assert_eq!(
        app.download(&again.id, false).unwrap_err().to_string(),
        "ALREADY_DOWNLOADED"
    );
    app.download(&again.id, true).unwrap();
    let other = wait(&app, &again.id, Status::Completed).await;
    assert_ne!(done.path, other.path);
    app.shutdown().await;
    drop(app);
    let reopened = super::tests::app(root.path(), &fixture);
    assert_eq!(
        reopened.cover(&task.id).unwrap(),
        b"\x89PNG\r\n\x1a\nfixture"
    );
    assert_eq!(std::fs::read(done.path.unwrap()).unwrap(), body());
    reopened.shutdown().await;
}
#[tokio::test]
async fn queue_parallelism_cancel_one_and_parse_other_channels() {
    let root = tempfile::tempdir().unwrap();
    let fixture = Arc::new(Fixture::default());
    for id in [1, 2, 3] {
        fixture
            .gates
            .lock()
            .unwrap()
            .insert(id, Arc::new(Semaphore::new(0)));
    }
    let app = app(root.path(), &fixture);
    app.set_max_downloads(2).unwrap();
    let a = resolve(&app, 1).await;
    let b = resolve(&app, 2).await;
    let c = resolve(&app, 3).await;
    app.download(&a.id, false).unwrap();
    app.download(&b.id, false).unwrap();
    app.download(&c.id, false).unwrap();
    timeout(Duration::from_secs(2), async {
        while fixture.active.load(Ordering::SeqCst) < 2 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        app.list()
            .into_iter()
            .find(|t| t.id == c.id)
            .unwrap()
            .status,
        Status::Queued
    );
    let extra = app.resolve("https://t.me/other_channel/4".into()).unwrap();
    wait(&app, &extra.id, Status::Ready).await;
    app.cancel(&a.id).await.unwrap();
    wait(&app, &a.id, Status::Canceled).await;
    wait(&app, &c.id, Status::Downloading).await;
    assert_eq!(
        app.list()
            .into_iter()
            .find(|t| t.id == b.id)
            .unwrap()
            .status,
        Status::Downloading
    );
    fixture
        .gates
        .lock()
        .unwrap()
        .get(&2)
        .unwrap()
        .add_permits(1);
    fixture
        .gates
        .lock()
        .unwrap()
        .get(&3)
        .unwrap()
        .add_permits(1);
    wait(&app, &b.id, Status::Completed).await;
    wait(&app, &c.id, Status::Completed).await;
    assert_eq!(fixture.peak.load(Ordering::SeqCst), 2);
    assert_eq!(fixture.active.load(Ordering::SeqCst), 0);
    assert_eq!(
        std::fs::read_dir(root.path().join("downloads/telegram"))
            .unwrap()
            .count(),
        2
    );
    app.shutdown().await;
}
#[tokio::test]
async fn shutdown_cancels_all_active_transfers_and_removes_only_staging() {
    let root = tempfile::tempdir().unwrap();
    let fixture = Arc::new(Fixture::default());
    for id in [1, 2] {
        fixture
            .gates
            .lock()
            .unwrap()
            .insert(id, Arc::new(Semaphore::new(0)));
    }
    let app = app(root.path(), &fixture);
    let a = resolve(&app, 1).await;
    let b = resolve(&app, 2).await;
    std::fs::create_dir_all(root.path().join("downloads")).unwrap();
    std::fs::write(root.path().join("downloads/user.txt"), b"keep").unwrap();
    app.download(&a.id, false).unwrap();
    app.download(&b.id, false).unwrap();
    timeout(Duration::from_secs(2), async {
        while fixture.active.load(Ordering::SeqCst) < 2 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    timeout(Duration::from_secs(3), app.shutdown())
        .await
        .unwrap();
    assert_eq!(fixture.active.load(Ordering::SeqCst), 0);
    assert_eq!(
        std::fs::read_dir(root.path().join("downloads"))
            .unwrap()
            .count(),
        1
    );
    assert_eq!(
        std::fs::read(root.path().join("downloads/user.txt")).unwrap(),
        b"keep"
    );
}
struct Script {
    media: Media,
    parts: VecDeque<Result<Option<Vec<u8>>>>,
    refreshes: usize,
}

#[test]
fn progress_reports_once_per_second_and_uses_real_five_second_bytes() {
    let now = Instant::now();
    let mut meter = DownloadProgress::new(now);
    assert!(meter
        .sample(now + Duration::from_millis(250), 900, 100_000)
        .is_none());
    assert_eq!(
        meter
            .sample(now + Duration::from_secs(1), 1000, 100_000)
            .unwrap()["speed_bps"],
        1000.0
    );
    // A burst changes the real rolling average, rather than displaying its instantaneous peak.
    assert_eq!(
        meter
            .sample(now + Duration::from_secs(2), 4000, 100_000)
            .unwrap()["speed_bps"],
        2000.0
    );
    for second in 3..=7 {
        meter
            .sample(now + Duration::from_secs(second), 4000, 100_000)
            .unwrap();
    }
    assert_eq!(
        meter
            .sample(now + Duration::from_secs(8), 4000, 100_000)
            .unwrap()["speed_bps"],
        0.0
    );
    assert!(
        meter
            .sample(now + Duration::from_millis(8250), 100_000, 100_000)
            .is_some(),
        "completion is immediate"
    );
}

struct PositionedScript(VecDeque<ReceivedPart>);
impl Transport for PositionedScript {
    fn inspect<'a>(&'a mut self, _: &'a MessageLink) -> ProviderFuture<'a, Media> {
        Box::pin(async { Ok(media(9)) })
    }
    fn next(&mut self, _: u64) -> ProviderFuture<'_, Option<ReceivedPart>> {
        Box::pin(async { Ok(self.0.pop_front()) })
    }
}

#[tokio::test]
async fn completed_chunks_write_their_offsets_and_commit_only_the_complete_file() {
    for complete in [true, false] {
        let root = tempfile::tempdir().unwrap();
        let link = message_link("https://t.me/c/100/9").unwrap();
        let mut data = body();
        data[2048..].fill(123);
        let mut source = PositionedScript(VecDeque::from([ReceivedPart {
            offset: 2048,
            bytes: data[2048..].to_vec(),
        }]));
        if complete {
            source.0.push_back(ReceivedPart {
                offset: 0,
                bytes: data[..2048].to_vec(),
            });
        }
        let result = transfer(
            &mut source,
            &link,
            Some(&preview(&media(9), &link)),
            Some(root.path()),
            |_| {},
        )
        .await;
        if complete {
            result.unwrap();
            assert_eq!(std::fs::read(root.path().join("video.mp4")).unwrap(), data);
        } else {
            assert_eq!(result.unwrap_err().to_string(), "INCOMPLETE_MEDIA");
            assert!(!root.path().join("video.mp4").exists());
            assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
        }
    }
}

impl Transport for Script {
    fn inspect<'a>(&'a mut self, _: &'a MessageLink) -> ProviderFuture<'a, Media> {
        Box::pin(async {
            self.refreshes += 1;
            Ok(self.media.clone())
        })
    }
    fn next(&mut self, offset: u64) -> ProviderFuture<'_, Option<ReceivedPart>> {
        Box::pin(async move { self.parts.pop_front().unwrap_or(Ok(None)).map(|part|part.map(|bytes|ReceivedPart {offset, bytes})) })
    }
}
#[tokio::test]
async fn reference_expiry_restarts_once_and_checks_full_original_bytes() {
    let root = tempfile::tempdir().unwrap();
    let link = message_link("https://t.me/c/100/9").unwrap();
    let mut source = Script {
        media: media(9),
        parts: VecDeque::from([
            Ok(Some(body()[..32].to_vec())),
            Err(anyhow::anyhow!("TELEGRAM_FILE_REFERENCE_EXPIRED")),
            Ok(Some(body())),
            Ok(None),
        ]),
        refreshes: 0,
    };
    let expected = preview(&source.media, &link);
    transfer(
        &mut source,
        &link,
        Some(&expected),
        Some(root.path()),
        |_| {},
    )
    .await
    .unwrap();
    assert_eq!(source.refreshes, 2);
    assert_eq!(
        std::fs::read(root.path().join("video.mp4")).unwrap(),
        body()
    );
}

#[tokio::test]
async fn interrupted_native_attempt_cleans_only_its_own_partial_before_retry() {
    let root = tempfile::tempdir().unwrap();
    let legacy = root.path().join("video.mp4.part");
    std::fs::write(&legacy, b"unowned previous part").unwrap();
    let link = message_link("https://t.me/c/100/9").unwrap();
    let mut source = Script {
        media: media(9),
        parts: VecDeque::from([Err(anyhow::anyhow!("TELEGRAM_NETWORK_CHANGED"))]),
        refreshes: 0,
    };
    let expected = preview(&source.media, &link);
    assert_eq!(transfer(&mut source, &link, Some(&expected), Some(root.path()), |_| {}).await.unwrap_err().to_string(), "TELEGRAM_NETWORK_CHANGED");
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
    source.parts = VecDeque::from([Ok(Some(body())), Ok(None)]);
    transfer(&mut source, &link, Some(&expected), Some(root.path()), |_| {}).await.unwrap();
    assert_eq!(std::fs::read(&legacy).unwrap(), b"unowned previous part");
    assert_eq!(std::fs::read(root.path().join("video.mp4")).unwrap(), body());
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 2);
}
#[tokio::test]
async fn rejects_truncation_invalid_header_and_changed_document_before_commit() {
    for (bytes, error) in [
        (vec![0; 32], "INCOMPLETE_MEDIA"),
        (vec![0; 4096], "INVALID_MEDIA_CONTENT"),
    ] {
        let root = tempfile::tempdir().unwrap();
        let link = message_link("https://t.me/c/100/9").unwrap();
        let mut source = Script {
            media: media(9),
            parts: VecDeque::from([Ok(Some(bytes)), Ok(None)]),
            refreshes: 0,
        };
        let expected = preview(&source.media, &link);
        assert_eq!(
            transfer(
                &mut source,
                &link,
                Some(&expected),
                Some(root.path()),
                |_| {}
            )
            .await
            .unwrap_err()
            .to_string(),
            error
        );
        assert!(!root.path().join("video.mp4").exists());
    }
    let mut changed = media(9);
    changed.document = 999;
    assert_eq!(
        check_expected(
            &changed,
            &preview(&media(9), &message_link("https://t.me/c/100/9").unwrap())
        )
        .unwrap_err()
        .to_string(),
        "TELEGRAM_MEDIA_CHANGED"
    );
}

#[tokio::test]
async fn public_channel_retry_keeps_username_and_rejects_changed_owner() {
    for change_owner in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let fixture = Arc::new(Fixture::default());
        fixture.public_only.store(true, Ordering::SeqCst);
        fixture
            .gates
            .lock()
            .unwrap()
            .insert(9, Arc::new(Semaphore::new(0)));
        let app = app(root.path(), &fixture);
        let task = app
            .resolve("https://t.me/unjoined_channel/9".into())
            .unwrap();
        wait(&app, &task.id, Status::Ready).await;
        app.download(&task.id, false).unwrap();
        wait(&app, &task.id, Status::Downloading).await;
        app.cancel(&task.id).await.unwrap();
        wait(&app, &task.id, Status::Canceled).await;
        if change_owner {
            fixture.public_channel.store(101, Ordering::SeqCst);
        }
        fixture
            .gates
            .lock()
            .unwrap()
            .get(&9)
            .unwrap()
            .add_permits(1);
        // Simulate a persisted legacy task without changing its public source link.
        app.backup_db(|db| {db.execute("UPDATE download_tasks SET snapshot=json_set(snapshot,'$.video.resolver','tdl') WHERE id=?",[&task.id])?;Ok(())}).unwrap();
        app.shutdown().await;
        drop(app);
        let reopened = super::tests::app(root.path(), &fixture);
        reopened.retry(&task.id).unwrap();
        if change_owner {
            let failed = wait(&reopened, &task.id, Status::Failed).await;
            assert_eq!(failed.error.as_deref(), Some(ALIAS_IDENTITY_MISMATCH));
            assert_eq!(failed.video.unwrap()["video_id"], "100_9");
        } else {
            let done = wait(&reopened, &task.id, Status::Completed).await;
            assert_eq!(done.video.unwrap()["resolver"], "grammers");
        }
        reopened.shutdown().await;
    }
}

#[tokio::test]
async fn network_failure_auto_recovers_same_download_without_partial_files() {
    let root = tempfile::tempdir().unwrap();
    let fixture = Arc::new(Fixture::default());
    let application = app(root.path(), &fixture);
    let task = resolve(&application, 9).await;
    fixture.next_failures.store(2, Ordering::SeqCst);
    application.download(&task.id, false).unwrap();
    let done = wait(&application, &task.id, Status::Completed).await;
    assert!(done.error.is_none());
    assert_eq!(std::fs::read(done.path.unwrap()).unwrap(), body());
    assert_eq!(application.list().len(), 1);
    let db = rusqlite::Connection::open(root.path().join("tasks.sqlite")).unwrap();
    assert_eq!(db.query_row("SELECT count(*) FROM task_staging", [], |r|r.get::<_,i64>(0)).unwrap(), 0);
    application.shutdown().await;
}

#[tokio::test]
async fn startup_recovers_old_network_failures_without_starting_parse_only_download() {
    let root = tempfile::tempdir().unwrap();
    let fixture = Arc::new(Fixture::default());
    let application = app(root.path(), &fixture);
    *fixture.inspect_error.lock().unwrap() = Some("TELEGRAM_CHANNEL_FORBIDDEN".into());
    fixture.inspect_failures.store(1, Ordering::SeqCst);
    let task = application.resolve("https://t.me/c/100/9".into()).unwrap();
    wait(&application, &task.id, Status::Failed).await;
    application.shutdown().await;
    drop(application);
    let db = rusqlite::Connection::open(root.path().join("tasks.sqlite")).unwrap();
    db.execute("UPDATE download_tasks SET snapshot=json_set(snapshot,'$.error','TELEGRAM_CONNECTION_ERROR')", []).unwrap();
    drop(db);
    let application = app(root.path(), &fixture);
    application.start_backup_service();
    timeout(Duration::from_secs(2), async {
        while application.list().iter().any(|t| t.id == task.id && t.status == Status::Failed) { tokio::task::yield_now().await; }
    }).await.unwrap();
    let done = wait(&application, &task.id, Status::Ready).await;
    assert!(!done.download_requested);
    assert!(done.error.is_none());
    assert_eq!(application.list().len(), 1);
    application.shutdown().await;
}

#[tokio::test]
async fn metadata_network_errors_keep_waiting_beyond_previous_retry_limit() {
    let root = tempfile::tempdir().unwrap();
    let fixture = Arc::new(Fixture::default());
    fixture.inspect_failures.store(6, Ordering::SeqCst);
    let application = app(root.path(), &fixture);
    let task = resolve(&application, 9).await;
    assert_eq!(fixture.inspected.lock().unwrap().len(), 7);
    assert!(task.error.is_none());
    assert_eq!(application.list().len(), 1);
    application.shutdown().await;
}

#[tokio::test]
async fn reconnect_releases_download_slot_for_other_platform_and_then_resumes() {
    let root = tempfile::tempdir().unwrap();
    let fixture = Arc::new(Fixture::default());
    let application = app_with_engine(root.path(), &fixture, "python");
    application.set_max_downloads(1).unwrap();
    let telegram = resolve(&application, 9).await;
    let queued = resolve(&application, 10).await;
    let other = application.resolve("valid".into()).unwrap();
    wait(&application, &other.id, Status::Ready).await;
    let session = application.telegram_accounts.binding(crate::telegram_accounts::Purpose::Download).session;
    *session.reconnect_probe.lock().unwrap() = Some(|| Err(anyhow::anyhow!("TELEGRAM_CONNECTION_ERROR")));
    fixture.next_failures.store(1, Ordering::SeqCst);
    application.download(&telegram.id, false).unwrap();
    timeout(Duration::from_secs(2), async {
        while !application.list().iter().any(|t|t.id==telegram.id && t.error.as_deref()==Some("TELEGRAM_RECONNECTING")) { tokio::task::yield_now().await; }
    }).await.unwrap();
    application.download(&queued.id, false).unwrap();
    application.download(&other.id, false).unwrap();
    wait(&application, &other.id, Status::Completed).await;
    assert_eq!(application.list().iter().find(|t|t.id==queued.id).unwrap().status, Status::Queued);
    assert_eq!(application.list().iter().find(|t|t.id==telegram.id).unwrap().error.as_deref(), Some("TELEGRAM_RECONNECTING"));
    *session.reconnect_probe.lock().unwrap() = Some(|| Ok(()));
    let done = wait(&application, &telegram.id, Status::Completed).await;
    assert_eq!(std::fs::read(done.path.unwrap()).unwrap(), body());
    wait(&application, &queued.id, Status::Completed).await;
    assert_eq!(fixture.peak.load(Ordering::SeqCst), 1);
    application.shutdown().await;
}

#[tokio::test]
async fn canceling_reconnect_owner_allows_queued_task_to_take_over_recovery() {
    let root = tempfile::tempdir().unwrap();
    let fixture = Arc::new(Fixture::default());
    let application = app(root.path(), &fixture);
    application.set_max_downloads(1).unwrap();
    let first = resolve(&application, 9).await;
    let queued = resolve(&application, 10).await;
    let session = application.telegram_accounts.binding(crate::telegram_accounts::Purpose::Download).session;
    *session.reconnect_probe.lock().unwrap() = Some(|| Err(anyhow::anyhow!("TELEGRAM_CONNECTION_ERROR")));
    fixture.next_failures.store(1, Ordering::SeqCst);
    application.download(&first.id, false).unwrap();
    timeout(Duration::from_secs(2), async {
        while !session.reconnect.waiting() { tokio::task::yield_now().await; }
    }).await.unwrap();
    application.download(&queued.id, false).unwrap();
    application.cancel(&first.id).await.unwrap();
    timeout(Duration::from_secs(2), async {
        while !application.list().iter().any(|t|t.id==queued.id && t.error.as_deref()==Some("TELEGRAM_RECONNECTING")) { tokio::task::yield_now().await; }
    }).await.unwrap();
    *session.reconnect_probe.lock().unwrap() = Some(|| Ok(()));
    wait(&application, &queued.id, Status::Completed).await;
    assert_eq!(application.list().iter().find(|t|t.id==first.id).unwrap().status,Status::Canceled);
    assert_eq!(fixture.peak.load(Ordering::SeqCst),1);
    application.shutdown().await;
}

#[tokio::test]
async fn startup_restores_interrupted_telegram_with_and_without_resolved_metadata() {
    for deferred in [false,true] {
        let root = tempfile::tempdir().unwrap();
        let fixture = Arc::new(Fixture::default());
        fixture.gates.lock().unwrap().insert(9,Arc::new(Semaphore::new(0)));
        let application = app(root.path(),&fixture);
        let task = resolve(&application,9).await;
        application.download(&task.id,false).unwrap();
        timeout(Duration::from_secs(2),async {
            while fixture.active.load(Ordering::SeqCst)==0 {tokio::task::yield_now().await;}
        }).await.unwrap();
        application.shutdown().await;
        drop(application);
        if deferred {
            let db = rusqlite::Connection::open(root.path().join("tasks.sqlite")).unwrap();
            db.execute("UPDATE download_tasks SET snapshot=json_set(snapshot,'$.video',NULL) WHERE id=?",[&task.id]).unwrap();
        }
        fixture.gates.lock().unwrap().clear();
        let application = app(root.path(),&fixture);
        application.start_backup_service();
        let done = wait(&application,&task.id,Status::Completed).await;
        assert_eq!(done.insertion_id,task.insertion_id);
        assert_eq!(std::fs::read(done.path.unwrap()).unwrap(),body());
        assert_eq!(application.list().len(),1);
        application.shutdown().await;
    }
}

#[tokio::test]
async fn startup_keeps_duplicate_consent_for_deferred_telegram_recovery() {
    for confirmed in [false,true] {
        let root = tempfile::tempdir().unwrap();
        let fixture = Arc::new(Fixture::default());
        let application = app(root.path(),&fixture);
        let first = resolve(&application,9).await;
        application.download(&first.id,false).unwrap();
        let first = wait(&application,&first.id,Status::Completed).await;
        let original = std::fs::read(first.path.as_ref().unwrap()).unwrap();
        let repeat = resolve(&application,9).await;
        if confirmed {
            fixture.gates.lock().unwrap().insert(9,Arc::new(Semaphore::new(0)));
            application.download(&repeat.id,true).unwrap();
            timeout(Duration::from_secs(2),async {while fixture.active.load(Ordering::SeqCst)==0 {tokio::task::yield_now().await;}}).await.unwrap();
        }
        application.shutdown().await;
        drop(application);
        let db = rusqlite::Connection::open(root.path().join("tasks.sqlite")).unwrap();
        db.execute("UPDATE download_tasks SET snapshot=json_set(snapshot,'$.video',NULL,'$.download_requested',json('true'),'$.status','failed','$.error','INTERRUPTED') WHERE id=?",[&repeat.id]).unwrap();
        drop(db);
        fixture.gates.lock().unwrap().clear();
        let application = app(root.path(),&fixture);
        application.start_backup_service();
        let status = if confirmed {Status::Completed} else {Status::Ready};
        let resumed = wait(&application,&repeat.id,status).await;
        assert_eq!(resumed.duplicate_confirmed,confirmed);
        assert_eq!(resumed.insertion_id,repeat.insertion_id);
        assert_eq!(std::fs::read(first.path.unwrap()).unwrap(),original);
        assert_eq!(application.list().iter().filter(|task|task.status==Status::Completed).count(),if confirmed {2}else{1});
        application.shutdown().await;
        drop(application);
        if !confirmed {
            let application = app(root.path(),&fixture);
            application.start_backup_service();
            let waiting = application.list().into_iter().find(|task|task.id==repeat.id).unwrap();
            assert_eq!(waiting.status,Status::Ready);
            assert!(!waiting.duplicate_confirmed);
            assert_eq!(fixture.active.load(Ordering::SeqCst),0);
            application.shutdown().await;
        }
    }
}
