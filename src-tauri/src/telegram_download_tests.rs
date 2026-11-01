use super::*;
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
    inspected: Mutex<Vec<u32>>,
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
        message,
        document: message as i64,
        size: 4096,
        mime: "video/mp4".into(),
        name: format!("video-{message}.mp4"),
        container: "mp4".into(),
    }
}
fn body() -> Vec<u8> {
    let mut b = vec![0; 4096];
    b[4..8].copy_from_slice(b"ftyp");
    b
}
impl Transport for Fake {
    fn inspect<'a>(&'a mut self, link: &'a MessageLink) -> ProviderFuture<'a, Media> {
        Box::pin(async move {
            if self.fixture.auth_denied.load(Ordering::SeqCst) { bail!("TELEGRAM_AUTH_REQUIRED"); }
            self.fixture.inspected.lock().unwrap().push(link.message);
            if self.fixture.public_only.load(Ordering::SeqCst) && link.channel.is_some() {
                bail!("TELEGRAM_CHANNEL_FORBIDDEN");
            }
            let mut value = media(link.message);
            value.channel = link
                .channel
                .unwrap_or(self.fixture.public_channel.load(Ordering::SeqCst).max(100) as u64);
            self.media = Some(value.clone());
            self.delivered = false;
            Ok(value)
        })
    }
    fn next(&mut self) -> ProviderFuture<'_, Option<Vec<u8>>> {
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
            self.delivered = true;
            Ok(Some(body()))
        })
    }
}
fn app(root: &Path, fixture: &Arc<Fixture>) -> Arc<Core> {
    let project = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let app = Core::open(
        &root.join("tasks.sqlite"),
        root.join("downloads"),
        project
            .join(if cfg!(windows) {
                ".venv/Scripts/python.exe"
            } else {
                ".venv/bin/python"
            })
            .to_string_lossy()
            .into_owned(),
        vec![project
            .join("tests/fixtures/desktop_engine.py")
            .to_string_lossy()
            .into_owned()],
        |_| {},
    )
    .unwrap();
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
impl Transport for Script {
    fn inspect<'a>(&'a mut self, _: &'a MessageLink) -> ProviderFuture<'a, Media> {
        Box::pin(async {
            self.refreshes += 1;
            Ok(self.media.clone())
        })
    }
    fn next(&mut self) -> ProviderFuture<'_, Option<Vec<u8>>> {
        Box::pin(async { self.parts.pop_front().unwrap_or(Ok(None)) })
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
