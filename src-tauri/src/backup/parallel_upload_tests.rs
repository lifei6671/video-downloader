//! Offline queue integration: real SQLite/state machine and in-memory S3 objects.
use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::{mpsc, Semaphore};

struct ParallelProvider {
    inner: Arc<S3Provider<Memory>>,
    entered: mpsc::Sender<String>,
    release: Semaphore,
    active: AtomicUsize,
    peak: AtomicUsize,
    calls: Mutex<BTreeMap<String, usize>>,
}
struct Active<'a>(&'a AtomicUsize);
impl Drop for Active<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}
impl BackupProvider for ParallelProvider {
    fn scope(&self) -> &Scope {
        self.inner.scope()
    }
    fn capabilities(&self) -> ProviderCapabilities {
        self.inner.capabilities()
    }
    fn open_library(&self, create: bool) -> ProviderFuture<'_, LibraryDescriptor> {
        self.inner.open_library(create)
    }
    fn pull_changes(&self, checkpoint: Option<Checkpoint>) -> ProviderFuture<'_, ChangeBatch> {
        self.inner.pull_changes(checkpoint)
    }
    fn read_asset(&self, id: &str) -> ProviderFuture<'_, Option<RemoteAsset>> {
        self.inner.read_asset(id)
    }
    fn publish_change(
        &self,
        change: AssetChange,
        base: Option<VersionToken>,
    ) -> ProviderFuture<'_, PublishReceipt> {
        self.inner.publish_change(change, base)
    }
    fn upload_media(
        &self,
        source: MediaSource,
        resume: Option<ResumeState>,
    ) -> ProviderFuture<'_, UploadReceipt> {
        Box::pin(async move {
            if source.kind == "video" {
                *self
                    .calls
                    .lock()
                    .unwrap()
                    .entry(source.operation_id.clone())
                    .or_default() += 1;
                let current = self.active.fetch_add(1, Ordering::SeqCst) + 1;
                self.peak.fetch_max(current, Ordering::SeqCst);
                let _active = Active(&self.active);
                self.entered
                    .send(source.operation_id.clone())
                    .await
                    .unwrap();
                loop {
                    (source.control.as_ref().unwrap().check)()?;
                    tokio::select! {
                        permit = self.release.acquire() => { permit.unwrap().forget(); break; }
                        _ = tokio::time::sleep(std::time::Duration::from_millis(10)) => {}
                    }
                }
                self.inner.upload_media(source, resume).await
            } else {
                self.inner.upload_media(source, resume).await
            }
        })
    }
    fn cancel_upload(&self, resume: ResumeState, operation: &str) -> ProviderFuture<'_, ()> {
        self.inner.cancel_upload(resume, operation)
    }
    fn download_media(&self, reference: MediaRef, offset: u64) -> ProviderFuture<'_, MediaStream> {
        self.inner.download_media(reference, offset)
    }
    fn verify_media(&self, reference: MediaRef) -> ProviderFuture<'_, ()> {
        self.inner.verify_media(reference)
    }
    fn delete_media(&self, reference: MediaRef, operation: &str) -> ProviderFuture<'_, ()> {
        self.inner.delete_media(reference, operation)
    }
    fn reconcile_deleted_asset(
        &self,
        asset: &str,
        operation: &str,
    ) -> ProviderFuture<'_, DeleteCleanup> {
        self.inner.reconcile_deleted_asset(asset, operation)
    }
    fn compact_deleted_records(
        &self,
        candidates: Vec<RemoteAsset>,
        days: u32,
        now: u64,
    ) -> ProviderFuture<'_, Vec<RemoteAsset>> {
        self.inner.compact_deleted_records(candidates, days, now)
    }
}

fn add_asset(f: &Fixture) -> String {
    let id = uuid::Uuid::new_v4().to_string();
    let path = f.root.path().join(format!("{id}.mp4"));
    std::fs::copy(
        f.core
            .backup_assets()
            .unwrap()
            .into_iter()
            .find(|a| a.asset_id == f.id)
            .unwrap()
            .path
            .unwrap(),
        &path,
    )
    .unwrap();
    use std::io::Write;
    std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap()
        .write_all(id.as_bytes())
        .unwrap();
    let bytes = i64::try_from(std::fs::metadata(&path).unwrap().len()).unwrap();
    f.core.backup_db(|db| {
        db.execute("INSERT INTO media_assets SELECT ?1,NULL,metadata_json,original_filename,NULL,?3,asset_state FROM media_assets WHERE asset_id=?2",params![id,f.id,bytes])?;
        db.execute("INSERT INTO asset_local_files(asset_id,path,bytes,local_state) VALUES(?1,?2,?3,'present')",params![id,path.to_string_lossy(),bytes])?;
        sync_engine::register_download(db, &id)
    }).unwrap();
    id
}
fn gate(inner: Arc<S3Provider<Memory>>) -> (Arc<ParallelProvider>, mpsc::Receiver<String>) {
    let (entered, received) = mpsc::channel(8);
    (
        Arc::new(ParallelProvider {
            inner,
            entered,
            release: Semaphore::new(0),
            active: 0.into(),
            peak: 0.into(),
            calls: Mutex::default(),
        }),
        received,
    )
}
async fn entered(rx: &mut mpsc::Receiver<String>) -> String {
    tokio::time::timeout(std::time::Duration::from_secs(3), rx.recv())
        .await
        .unwrap()
        .unwrap()
}
async fn until(mut ready: impl FnMut() -> bool) {
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        while !ready() {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}
fn run(
    f: &Fixture,
    provider: Arc<ParallelProvider>,
) -> (
    tokio::sync::watch::Sender<bool>,
    tokio::task::JoinHandle<()>,
) {
    let (stop, cancel) = tokio::sync::watch::channel(false);
    (
        stop,
        tokio::spawn(sync_engine::run_with(
            f.core.clone(),
            cancel,
            Arc::new(move |_, _| Ok(provider.clone())),
        )),
    )
}
async fn stop(stop: tokio::sync::watch::Sender<bool>, worker: tokio::task::JoinHandle<()>) {
    stop.send_replace(true);
    tokio::time::timeout(std::time::Duration::from_secs(3), worker)
        .await
        .unwrap()
        .unwrap();
}
fn completed(core: &Core, count: usize) -> bool {
    scheduler::operations(core)
        .unwrap()
        .iter()
        .filter(|op| op.kind == "upload" && op.state == "completed")
        .count()
        == count
}

#[tokio::test]
async fn two_uploads_overlap_third_waits_and_each_asset_is_written_once() {
    let (f, provider, engine) = sync_fixture().await;
    add_asset(&f);
    add_asset(&f);
    engine.enqueue().unwrap();
    let (provider, mut rx) = gate(provider);
    let (shutdown, worker) = run(&f, provider.clone());
    let a = entered(&mut rx).await;
    let b = entered(&mut rx).await;
    assert_ne!(a, b);
    f.core.backup_db(|db| {
        let captured: i64 = db.query_row("SELECT COUNT(*) FROM backup_operation_captures", [], |r| r.get(0))?;
        let hot: i64 = db.query_row("SELECT COUNT(*) FROM backup_operations WHERE state='running' AND json_type(payload_json,'$.capture') IS NOT NULL", [], |r| r.get(0))?;
        assert_eq!((captured, hot), (2, 0));
        Ok(())
    }).unwrap();
    let operations = scheduler::operations(&f.core).unwrap();
    assert_eq!(
        operations.iter().filter(|op| op.state == "running").count(),
        2
    );
    assert_eq!(
        operations.iter().filter(|op| op.state == "pending").count(),
        1
    );
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(80), rx.recv())
            .await
            .is_err()
    );
    provider.release.add_permits(1);
    let c = entered(&mut rx).await;
    assert_ne!(c, a);
    assert_ne!(c, b);
    provider.release.add_permits(2);
    until(|| completed(&f.core, 3)).await;
    f.core
        .backup_db(|db| {
            let remaining: i64 =
                db.query_row("SELECT COUNT(*) FROM backup_operation_captures", [], |r| {
                    r.get(0)
                })?;
            assert_eq!(
                remaining, 0,
                "acknowledged operations reclaim immutable captures"
            );
            Ok(())
        })
        .unwrap();
    stop(shutdown, worker).await;
    assert_eq!(provider.peak.load(Ordering::SeqCst), 2);
    assert_eq!(provider.active.load(Ordering::SeqCst), 0);
    assert_eq!(
        provider
            .calls
            .lock()
            .unwrap()
            .values()
            .copied()
            .collect::<Vec<_>>(),
        vec![1, 1, 1]
    );
    for op in scheduler::operations(&f.core).unwrap() {
        let reference: UploadReceipt =
            serde_json::from_value(op.payload["video_receipt"].clone()).unwrap();
        let remote = provider
            .inner
            .read_asset(op.payload["capture"]["remote"].as_str().unwrap())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(remote.snapshot.media.as_ref(), Some(&reference.reference));
        let local = f
            .core
            .backup_assets()
            .unwrap()
            .into_iter()
            .find(|a| Some(a.asset_id.as_str()) == op.asset_id.as_deref())
            .unwrap();
        assert_eq!(
            local.content_sha256.as_deref(),
            Some(reference.reference.sha256.as_str())
        );
        assert_eq!(local.content_bytes, reference.reference.bytes);
        let mut stream = provider
            .inner
            .download_media(reference.reference.clone(), 0)
            .await
            .unwrap();
        let mut body = Vec::new();
        tokio::io::AsyncReadExt::read_to_end(&mut stream.reader, &mut body)
            .await
            .unwrap();
        assert_eq!(body, std::fs::read(local.path.unwrap()).unwrap());
        assert_eq!(
            format!("{:x}", Sha256::digest(&body)),
            reference.reference.sha256
        );
    }
    f.core.shutdown().await;
}

#[tokio::test]
async fn cancel_one_upload_releases_its_lane_without_canceling_the_other() {
    let (f, provider, engine) = sync_fixture().await;
    add_asset(&f);
    add_asset(&f);
    engine.enqueue().unwrap();
    let (provider, mut rx) = gate(provider);
    let (shutdown, worker) = run(&f, provider.clone());
    let canceled = entered(&mut rx).await;
    let unaffected = entered(&mut rx).await;
    scheduler::cancel(&f.core, &canceled).unwrap();
    f.core.wake_backup();
    let next = entered(&mut rx).await;
    assert_ne!(next, canceled);
    assert_ne!(next, unaffected);
    assert_eq!(
        scheduler::operation(&f.core, &unaffected).unwrap().state,
        "running"
    );
    assert_eq!(
        scheduler::operation(&f.core, &canceled).unwrap().state,
        "canceled"
    );
    provider.release.add_permits(2);
    until(|| completed(&f.core, 2)).await;
    stop(shutdown, worker).await;
    assert_eq!(provider.peak.load(Ordering::SeqCst), 2);
    assert!(f.core.backup_assets().unwrap().iter().all(|asset| asset
        .path
        .as_ref()
        .unwrap()
        .is_file()));
    f.core.shutdown().await;
}

#[tokio::test]
async fn shutdown_drains_both_uploads_without_claiming_or_canceling_the_third() {
    let (f, provider, engine) = sync_fixture().await;
    add_asset(&f);
    add_asset(&f);
    engine.enqueue().unwrap();
    let (provider, mut rx) = gate(provider);
    let (shutdown, worker) = run(&f, provider.clone());
    entered(&mut rx).await;
    entered(&mut rx).await;
    stop(shutdown, worker).await;
    assert_eq!(provider.active.load(Ordering::SeqCst), 0);
    let operations = scheduler::operations(&f.core).unwrap();
    assert_eq!(
        operations.iter().filter(|op| op.state == "pending").count(),
        1
    );
    assert!(operations.iter().all(|op| op.state != "canceled"));
    assert_eq!(provider.calls.lock().unwrap().len(), 2);
    f.core.shutdown().await;
}

#[tokio::test]
async fn foreground_delete_completes_while_unrelated_uploads_are_held() {
    let (f, inner, engine) = sync_fixture().await;
    let initial = claim(&engine, &f.core);
    engine.execute_upload(&initial).await.unwrap();
    let a = add_asset(&f);
    let b = add_asset(&f);
    let c = add_asset(&f);
    engine.enqueue().unwrap();
    let (provider, mut rx) = gate(inner);
    let (shutdown, worker) = run(&f, provider.clone());
    entered(&mut rx).await;
    entered(&mut rx).await;
    let deletion = sync_delete::delete(&f.core, &f.space.space_id, &f.id)
        .await
        .unwrap();
    until(|| {
        let op = scheduler::operation(&f.core, &deletion).unwrap();
        op.state == "completed" || op.payload["local_cleanup_done"] == true
    })
    .await;
    assert_eq!(
        provider.active.load(Ordering::SeqCst),
        2,
        "deletion must not drain unrelated uploads"
    );
    provider.release.add_permits(2);
    entered(&mut rx).await;
    provider.release.add_permits(1);
    until(|| completed(&f.core, 4)).await;
    stop(shutdown, worker).await;
    for id in [a, b, c] {
        let asset = f
            .core
            .backup_assets()
            .unwrap()
            .into_iter()
            .find(|a| a.asset_id == id)
            .unwrap();
        assert!(
            asset.path.unwrap().is_file(),
            "unrelated source was deleted"
        );
    }
    assert_eq!(provider.peak.load(Ordering::SeqCst), 2);
    f.core.shutdown().await;
}
