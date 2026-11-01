use super::super::telegram_transfer_journal as db;
use super::*;
use rusqlite::Connection;
use std::{
    collections::HashMap,
    sync::atomic::{AtomicBool, AtomicUsize},
};

struct TestJournal(Mutex<Connection>);
impl TestJournal {
    fn open(path: &Path) -> Self {
        let db = Connection::open(path).unwrap();
        db.execute_batch(include_str!("telegram-schema.sql"))
            .unwrap();
        Self(Mutex::new(db))
    }
    fn entry(&self, source: &MediaSource) -> Entry {
        let raw: String = self
            .0
            .lock()
            .unwrap()
            .query_row(
                "SELECT entry_json FROM telegram_media_sends WHERE operation_id=?1 AND kind=?2",
                rusqlite::params![source.operation_id, source.kind],
                |r| r.get(0),
            )
            .unwrap();
        serde_json::from_str(&raw).unwrap()
    }
}
impl Journal for TestJournal {
    fn reserve(&self, entry: Entry) -> Result<Entry> {
        db::reserve(&self.0.lock().unwrap(), entry)
    }
    fn save(&self, entry: &Entry) -> Result<()> {
        db::save(&self.0.lock().unwrap(), entry)
    }
    fn cooldown(&self, account: &str) -> Result<i64> {
        db::cooldown(&self.0.lock().unwrap(), account)
    }
    fn failure(&self, account: &str, error: &str) -> Result<()> {
        db::failure(&self.0.lock().unwrap(), account, error)
    }
}
#[derive(Default)]
struct Mock {
    documents: Mutex<HashMap<i32, (RemoteDocument, Vec<u8>)>>,
    uploads: AtomicUsize,
    sends: AtomicUsize,
    inspects: AtomicUsize,
    downloads: AtomicUsize,
    lost_response: AtomicBool,
    hidden: AtomicBool,
    corrupt: AtomicBool,
    expired: AtomicBool,
    deny: AtomicBool,
    cancel_on_send: Mutex<Option<watch::Sender<bool>>>,
    source_edit: Mutex<Option<PathBuf>>,
    reject_once: Mutex<Option<String>>,
    random_ids: Mutex<Vec<i64>>,
    video_attributes: Mutex<Vec<Option<crate::backup::provider::VideoAttributes>>>,
    finds: AtomicUsize,
    limit: u64,
}
impl Mock {
    fn new() -> Self {
        Self {
            limit: u64::MAX,
            ..Default::default()
        }
    }
}
impl Transport for Mock {
    type Uploaded = Vec<u8>;
    fn max_bytes(&self) -> u64 {
        self.limit
    }
    fn can_upload(&self) -> bool {
        !self.deny.load(Ordering::SeqCst)
    }
    fn upload<'a>(
        &'a self,
        reader: &'a mut (dyn AsyncRead + Unpin + Send),
        _: u64,
        _: String,
    ) -> ProviderFuture<'a, Vec<u8>> {
        Box::pin(async move {
            self.uploads.fetch_add(1, Ordering::SeqCst);
            let mut body = vec![];
            reader.read_to_end(&mut body).await?;
            if let Some(path) = self.source_edit.lock().unwrap().take() {
                std::fs::write(path, b"changed during upload")?;
            }
            Ok(body)
        })
    }
    fn send<'a>(
        &'a self,
        body: Vec<u8>,
        caption: String,
        random_id: i64,
        video: Option<crate::backup::provider::VideoAttributes>,
        _io: &'a Io,
    ) -> ProviderFuture<'a, Option<i32>> {
        Box::pin(async move {
            let id = self.sends.fetch_add(1, Ordering::SeqCst) as i32 + 1;
            self.random_ids.lock().unwrap().push(random_id);
            self.video_attributes.lock().unwrap().push(video);
            if let Some(code) = self.reject_once.lock().unwrap().take() {
                bail!(code);
            }
            self.documents.lock().unwrap().insert(
                id,
                (
                    RemoteDocument {
                        message_id: id,
                        document_id: i64::from(id) + 1000,
                        bytes: body.len() as u64,
                        caption,
                    },
                    body,
                ),
            );
            if let Some(cancel) = self.cancel_on_send.lock().unwrap().as_ref() {
                cancel.send_replace(true);
            }
            if self.lost_response.load(Ordering::SeqCst) {
                bail!("TELEGRAM_CONNECTION_ERROR");
            }
            Ok(Some(id))
        })
    }
    fn inspect(&self, id: i32) -> ProviderFuture<'_, RemoteDocument> {
        Box::pin(async move {
            self.inspects.fetch_add(1, Ordering::SeqCst);
            self.documents
                .lock()
                .unwrap()
                .get(&id)
                .map(|d| d.0.clone())
                .context("TELEGRAM_MEDIA_MISSING")
        })
    }
    fn find<'a>(
        &'a self,
        envelope: &'a Envelope,
        _: &'a Io,
    ) -> ProviderFuture<'a, Option<RemoteDocument>> {
        Box::pin(async move {
            self.finds.fetch_add(1, Ordering::SeqCst);
            if self.hidden.load(Ordering::SeqCst) {
                return Ok(None);
            }
            let caption = envelope.caption()?;
            Ok(self
                .documents
                .lock()
                .unwrap()
                .values()
                .find(|d| d.0.caption == caption)
                .map(|d| d.0.clone()))
        })
    }
    fn download(
        &self,
        document: RemoteDocument,
    ) -> ProviderFuture<'_, Box<dyn AsyncRead + Unpin + Send>> {
        Box::pin(async move {
            self.downloads.fetch_add(1, Ordering::SeqCst);
            if self.expired.swap(false, Ordering::SeqCst) {
                return Ok(Box::new(ExpiredReader) as Box<dyn AsyncRead + Unpin + Send>);
            }
            let mut body = self
                .documents
                .lock()
                .unwrap()
                .get(&document.message_id)
                .context("TELEGRAM_MEDIA_MISSING")?
                .1
                .clone();
            if self.corrupt.load(Ordering::SeqCst) {
                body[0] ^= 1;
            }
            Ok(Box::new(std::io::Cursor::new(body)) as Box<dyn AsyncRead + Unpin + Send>)
        })
    }
}
struct ExpiredReader;
impl AsyncRead for ExpiredReader {
    fn poll_read(
        self: Pin<&mut Self>,
        _: &mut TaskContext<'_>,
        _: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Poll::Ready(Err(std::io::Error::other(
            "TELEGRAM_FILE_REFERENCE_EXPIRED",
        )))
    }
}
fn target() -> Target {
    Target {
        scope: Scope {
            space_id: "local-space".into(),
            library_id: "portable-library".into(),
            provider_type: ProviderType::Telegram,
            protocol: 2,
        },
        account_id: "123".into(),
        channel_id: "456".into(),
    }
}
fn source(root: &Path, kind: &str, body: &[u8]) -> MediaSource {
    let path = root.join(format!("input-{kind}"));
    std::fs::write(&path, body).unwrap();
    MediaSource {
        video: None,
        base_revision: None,
        recreate_revision: None,
        asset_id: "asset".into(),
        kind: kind.into(),
        path,
        bytes: body.len() as u64,
        sha256: format!("{:x}", Sha256::digest(body)),
        container: match kind {
            "video" => "mp4",
            "cover" => "png",
            _ => "json",
        }
        .into(),
        operation_id: "operation".into(),
        device_id: "device".into(),
        control: None,
    }
}
fn io() -> (watch::Sender<bool>, watch::Sender<bool>, Io) {
    let (c, rx) = watch::channel(false);
    let (s, srx) = watch::channel(false);
    (c, s, Io::new(rx, srx, None))
}

#[tokio::test]
async fn telegram_streaming_flag_follows_original_mp4_layout_and_keeps_original_bytes() {
    fn atom(kind: &[u8;4], body: &[u8]) -> Vec<u8> {
        [((body.len()+8) as u32).to_be_bytes().as_slice(),kind,body].concat()
    }
    let ftyp=atom(b"ftyp",b"isom0000");
    let moov=atom(b"moov",b"metadata");
    let mdat=atom(b"mdat",&[37;131_113]);
    for (body, streaming) in [([ftyp.clone(),moov.clone(),mdat.clone()].concat(),true),([ftyp,mdat,moov].concat(),false)] {
        let root=tempfile::tempdir().unwrap();
        let journal=TestJournal::open(&root.path().join("journal.sqlite"));
        let net=Mock::new();
        let (_cancel,_stop,io)=io();
        let target=target();
        let mut source=source(root.path(),"video",&body);
        source.video=crate::backup::provider::VideoAttributes::from_metadata(&serde_json::json!({"duration_ms":2000,"width":640,"height":360}));
        let receipt=upload(&net,&journal,&target,source.clone(),&io).await.unwrap();
        assert_eq!(net.video_attributes.lock().unwrap()[0].as_ref().unwrap().supports_streaming,streaming);
        assert_eq!(receipt.reference.sha256,format!("{:x}",Sha256::digest(&body)));
        let restored=root.path().join("restored.mp4");
        restore(&net,&journal,&target,&receipt.reference,&restored,&io).await.unwrap();
        assert_eq!(std::fs::read(restored).unwrap(),body);
        assert_eq!(std::fs::read(source.path).unwrap(),body);
    }
}

#[tokio::test]
async fn telegram_streaming_layout_recognition_handles_extended_and_invalid_boxes() {
    fn atom(kind: &[u8;4], body: &[u8]) -> Vec<u8> {
        [((body.len()+8) as u32).to_be_bytes().as_slice(),kind,body].concat()
    }
    let ftyp=atom(b"ftyp",b"isom0000");
    let moov=atom(b"moov",b"metadata");
    let data=atom(b"mdat",b"original");
    let wide_moov=[1u32.to_be_bytes().as_slice(),b"moov",24u64.to_be_bytes().as_slice(),b"metadata"].concat();
    let eof_data=[0u32.to_be_bytes().as_slice(),b"mdat",b"original"].concat();
    let invalid=[1000u32.to_be_bytes().as_slice(),b"moov",b"short"].concat();
    let empty_data=atom(b"mdat",b"");
    let wide_empty_data=[1u32.to_be_bytes().as_slice(),b"mdat",16u64.to_be_bytes().as_slice()].concat();
    let cases=[
        ([ftyp.clone(),empty_data.clone(),moov.clone(),data.clone()].concat(),true),
        ([ftyp.clone(),moov.clone(),empty_data,data.clone()].concat(),true),
        ([ftyp.clone(),wide_empty_data,moov.clone(),data.clone()].concat(),true),
        ([ftyp.clone(),wide_moov,data.clone()].concat(),true),
        ([ftyp.clone(),moov.clone(),eof_data].concat(),true),
        ([ftyp.clone(),data.clone(),moov.clone()].concat(),false),
        ([ftyp.clone(),moov.clone()].concat(),false),
        ([moov,data].concat(),false),
        ([ftyp.clone(),invalid].concat(),false),
        ([ftyp,1u32.to_be_bytes().to_vec(),b"moov".to_vec()].concat(),false),
    ];
    let root=tempfile::tempdir().unwrap();
    let path=root.path().join("layout.mp4");
    let (_cancel,_stop,io)=io();
    for (body,expected) in cases {
        std::fs::write(&path,body).unwrap();
        assert_eq!(mp4_has_front_metadata(&path,&io).await.unwrap(),expected);
    }
}

#[tokio::test]
async fn telegram_transfer_three_document_kinds_roundtrip_original_bytes_and_rebind_space() {
    let root = tempfile::tempdir().unwrap();
    let journal = TestJournal::open(&root.path().join("journal.sqlite"));
    let net = Mock::new();
    let (_cancel, _stop, io) = io();
    let target = target();
    for (kind, body) in [
        ("video", vec![42; 131_113]),
        ("cover", vec![0, 1, 2, 255]),
        (
            "transcript",
            "中文文案 English 123\n第二段".as_bytes().to_vec(),
        ),
    ] {
        let source = source(root.path(), kind, &body);
        let receipt = upload(&net, &journal, &target, source.clone(), &io)
            .await
            .unwrap();
        assert_eq!(journal.entry(&source).state, SendState::Confirmed);
        let destination = root.path().join(format!("restored-{kind}"));
        restore(
            &net,
            &journal,
            &target,
            &receipt.reference,
            &destination,
            &io,
        )
        .await
        .unwrap();
        assert_eq!(std::fs::read(&destination).unwrap(), body);
        assert!(
            source.path.is_file(),
            "upload/restore must keep the original file"
        );
        let mut other = target.clone();
        other.scope.space_id = "new-device-space".into();
        let mut portable = receipt.reference.clone();
        portable.scope = other.scope.clone();
        restore(
            &net,
            &journal,
            &other,
            &portable,
            &root.path().join(format!("new-device-{kind}")),
            &io,
        )
        .await
        .unwrap();
    }
    assert_eq!(net.sends.load(Ordering::SeqCst), 3);
    assert_eq!(
        net.finds.load(Ordering::SeqCst),
        0,
        "successful sends read the returned message directly"
    );
    assert!(
        ProviderType::Telegram.require_implemented().is_ok(),
        "T3 supplies the Telegram library provider"
    );
}
#[tokio::test]
async fn telegram_transfer_uncertain_send_reconciles_after_restart_without_reading_or_sending_source_again(
) {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("journal.sqlite");
    let journal = TestJournal::open(&path);
    let net = Mock::new();
    let (_cancel, _stop, io) = io();
    let target = target();
    let source = source(root.path(), "video", b"original bytes");
    net.lost_response.store(true, Ordering::SeqCst);
    assert!(upload(&net, &journal, &target, source.clone(), &io)
        .await
        .is_err());
    let first = journal.entry(&source);
    assert_eq!(first.state, SendState::Sending);
    assert_ne!(first.random_id, 0);
    drop(journal);
    std::fs::remove_file(&source.path).unwrap();
    let restarted = TestJournal::open(&path);
    let receipt = upload(&net, &restarted, &target, source.clone(), &io)
        .await
        .unwrap();
    assert_eq!(receipt.reference.bytes, source.bytes);
    assert_eq!(restarted.entry(&source).random_id, first.random_id);
    assert_eq!(net.uploads.load(Ordering::SeqCst), 1);
    assert_eq!(net.sends.load(Ordering::SeqCst), 1);
    net.deny.store(true, Ordering::SeqCst);
    upload(&net, &restarted, &target, source, &io)
        .await
        .unwrap();
}
#[tokio::test]
async fn telegram_transfer_unknown_outcome_remains_unconfirmed_and_does_not_blindly_retry() {
    let root = tempfile::tempdir().unwrap();
    let journal = TestJournal::open(&root.path().join("journal.sqlite"));
    let net = Mock::new();
    let (_cancel, _stop, io) = io();
    let target = target();
    let source = source(root.path(), "cover", b"cover bytes");
    net.hidden.store(true, Ordering::SeqCst);
    net.lost_response.store(true, Ordering::SeqCst);
    assert!(upload(&net, &journal, &target, source.clone(), &io)
        .await
        .is_err());
    for _ in 0..2 {
        assert_eq!(
            upload(&net, &journal, &target, source.clone(), &io)
                .await
                .unwrap_err()
                .to_string(),
            "TELEGRAM_SEND_UNCONFIRMED"
        );
    }
    assert_eq!(net.sends.load(Ordering::SeqCst), 1);
    assert_eq!(journal.entry(&source).state, SendState::Sending);
    assert!(source.path.is_file());
}
#[tokio::test]
async fn telegram_transfer_definite_rejection_retries_after_durable_cooldown_with_same_random_id() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("journal.sqlite");
    let journal = TestJournal::open(&path);
    let net = Mock::new();
    let (_cancel, _stop, io) = io();
    let target = target();
    let source = source(root.path(), "video", b"unchanged bytes");
    *net.reject_once.lock().unwrap() = Some("TELEGRAM_FLOOD_WAIT:120".into());
    assert_eq!(
        upload(&net, &journal, &target, source.clone(), &io)
            .await
            .unwrap_err()
            .to_string(),
        "TELEGRAM_FLOOD_WAIT:120"
    );
    assert_eq!(journal.entry(&source).state, SendState::Prepared);
    assert!(net.documents.lock().unwrap().is_empty());
    drop(journal);
    let journal = TestJournal::open(&path);
    assert!(upload(&net, &journal, &target, source.clone(), &io)
        .await
        .is_err());
    assert_eq!(net.uploads.load(Ordering::SeqCst), 1);
    // Simulate deadline expiry, without delaying the test or changing the production clock.
    journal
        .0
        .lock()
        .unwrap()
        .execute("UPDATE telegram_transfer_cooldowns SET retry_at=0", [])
        .unwrap();
    upload(&net, &journal, &target, source, &io).await.unwrap();
    let ids = net.random_ids.lock().unwrap();
    assert_eq!(ids.len(), 2);
    assert_eq!(ids[0], ids[1]);
    assert_eq!(net.documents.lock().unwrap().len(), 1);
}
#[tokio::test]
async fn telegram_transfer_changed_source_and_size_limit_never_publish_a_message() {
    let root = tempfile::tempdir().unwrap();
    let journal = TestJournal::open(&root.path().join("journal.sqlite"));
    let mut net = Mock::new();
    net.limit = 2;
    let (_cancel, _stop, io) = io();
    let target = target();
    let source = source(root.path(), "transcript", b"original");
    assert_eq!(
        upload(&net, &journal, &target, source.clone(), &io)
            .await
            .unwrap_err()
            .to_string(),
        "TELEGRAM_FILE_TOO_LARGE"
    );
    assert_eq!(net.uploads.load(Ordering::SeqCst), 0);
    net.limit = u64::MAX;
    *net.source_edit.lock().unwrap() = Some(source.path.clone());
    assert_eq!(
        upload(&net, &journal, &target, source.clone(), &io)
            .await
            .unwrap_err()
            .to_string(),
        "BACKUP_SOURCE_CHANGED"
    );
    assert_eq!(net.sends.load(Ordering::SeqCst), 0);
    assert_eq!(
        std::fs::read(&source.path).unwrap(),
        b"changed during upload"
    );
}
#[tokio::test]
async fn telegram_transfer_cancel_after_send_keeps_uncertain_evidence_and_reconciles() {
    let root = tempfile::tempdir().unwrap();
    let journal = TestJournal::open(&root.path().join("journal.sqlite"));
    let net = Mock::new();
    let (cancel, _stop, control) = io();
    let target = target();
    let source = source(root.path(), "video", b"bytes");
    *net.cancel_on_send.lock().unwrap() = Some(cancel);
    assert_eq!(
        upload(&net, &journal, &target, source.clone(), &control)
            .await
            .unwrap_err()
            .to_string(),
        "TELEGRAM_TRANSFER_CANCELED"
    );
    assert_eq!(journal.entry(&source).state, SendState::Sending);
    let (_cancel2, _stop2, retry) = io();
    upload(&net, &journal, &target, source, &retry)
        .await
        .unwrap();
    assert_eq!(net.sends.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn telegram_transfer_restore_refreshes_expired_reference_and_rejects_corruption_without_overwrite(
) {
    let root = tempfile::tempdir().unwrap();
    let journal = TestJournal::open(&root.path().join("journal.sqlite"));
    let net = Mock::new();
    let (_cancel, _stop, io) = io();
    let target = target();
    let source = source(root.path(), "video", b"original");
    let receipt = upload(&net, &journal, &target, source, &io).await.unwrap();
    net.expired.store(true, Ordering::SeqCst);
    let destination = root.path().join("restored.mp4");
    restore(
        &net,
        &journal,
        &target,
        &receipt.reference,
        &destination,
        &io,
    )
    .await
    .unwrap();
    assert_eq!(net.downloads.load(Ordering::SeqCst), 2);
    net.corrupt.store(true, Ordering::SeqCst);
    let bad = root.path().join("bad.mp4");
    assert_eq!(
        restore(&net, &journal, &target, &receipt.reference, &bad, &io)
            .await
            .unwrap_err()
            .to_string(),
        "BACKUP_HASH_MISMATCH"
    );
    assert!(!bad.exists());
    assert_eq!(
        restore(
            &net,
            &journal,
            &target,
            &receipt.reference,
            &destination,
            &io
        )
        .await
        .unwrap_err()
        .to_string(),
        "TELEGRAM_DESTINATION_EXISTS"
    );
    assert_eq!(std::fs::read(destination).unwrap(), b"original");
    let files = std::fs::read_dir(root.path())
        .unwrap()
        .map(|v| v.unwrap().file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    assert!(
        files.iter().all(|name| name == "journal.sqlite"
            || name == "input-video"
            || name == "restored.mp4"),
        "only owned staging files must be removed: {files:?}"
    );
}
#[tokio::test]
async fn telegram_transfer_rejects_wrong_account_channel_and_operation_reuse_before_read_or_send() {
    let root = tempfile::tempdir().unwrap();
    let journal = TestJournal::open(&root.path().join("journal.sqlite"));
    let net = Mock::new();
    let (_cancel, _stop, io) = io();
    let target = target();
    let source = source(root.path(), "cover", b"cover");
    let receipt = upload(&net, &journal, &target, source.clone(), &io)
        .await
        .unwrap();
    let inspections = net.inspects.load(Ordering::SeqCst);
    for account in [true, false] {
        let mut wrong = target.clone();
        if account {
            wrong.account_id = "999".into();
        } else {
            wrong.channel_id = "888".into();
        }
        assert!(restore(
            &net,
            &journal,
            &wrong,
            &receipt.reference,
            &root.path().join("rejected"),
            &io
        )
        .await
        .is_err());
        assert!(upload(&net, &journal, &wrong, source.clone(), &io)
            .await
            .is_err());
    }
    assert_eq!(net.inspects.load(Ordering::SeqCst), inspections);
    assert_eq!(net.sends.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn telegram_transfer_flood_wait_is_durable_and_prevents_network_calls() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("journal.sqlite");
    let journal = TestJournal::open(&path);
    journal.failure("123", "TELEGRAM_FLOOD_WAIT:120").unwrap();
    drop(journal);
    let journal = TestJournal::open(&path);
    let net = Mock::new();
    let (_cancel, _stop, io) = io();
    let source = source(root.path(), "cover", b"cover");
    assert!(upload(&net, &journal, &target(), source, &io)
        .await
        .unwrap_err()
        .to_string()
        .starts_with("TELEGRAM_FLOOD_WAIT:"));
    assert_eq!(net.uploads.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn telegram_transfer_history_progress_keeps_send_alive_and_still_times_out_when_stalled() {
    let (_cancel,_stop,mut io)=io();
    io.idle=Duration::from_millis(150);
    io.run(async {
        for _ in 0..4 {tokio::time::sleep(Duration::from_millis(80)).await;io.check()?;io.touch();}
        Ok(())
    }).await.unwrap();
    let error=io.run(async {std::future::pending::<Result<()>>().await}).await.unwrap_err();
    assert_eq!(error.to_string(),"TELEGRAM_TRANSFER_TIMEOUT");
}
#[tokio::test]
async fn telegram_transfer_idle_timeout_and_abort_drop_the_request() {
    let (_cancel, _stop, mut io) = io();
    io.idle = Duration::from_millis(10);
    let dropped = Arc::new(AtomicBool::new(false));
    struct Pending(Arc<AtomicBool>);
    impl Drop for Pending {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }
    let pending = Pending(dropped.clone());
    let result = io
        .run(async move {
            let _pending = pending;
            std::future::pending::<Result<()>>().await
        })
        .await;
    assert_eq!(result.unwrap_err().to_string(), "TELEGRAM_TRANSFER_TIMEOUT");
    assert!(dropped.load(Ordering::SeqCst));
    io.abort_handle().send_replace(true);
    assert_eq!(
        io.run(async { Ok(()) }).await.unwrap_err().to_string(),
        "TELEGRAM_TRANSFER_CANCELED"
    );
}
