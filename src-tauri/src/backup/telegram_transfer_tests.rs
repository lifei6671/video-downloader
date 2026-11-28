use super::super::telegram_transfer_journal as db;
use super::*;
use rusqlite::Connection;
use std::{
    collections::HashMap,
    sync::atomic::{AtomicBool, AtomicUsize},
};

struct TestJournal(Mutex<Connection>);
#[tokio::test]
async fn album_member_unconfirmed_edit_resumes_same_message_after_restart() {
    let root = tempfile::tempdir().unwrap();
    let core = crate::desktop_core::Core::open(
        &root.path().join("core.sqlite"),
        root.path().join("downloads"),
        "unused".into(),
        vec![],
        |_| {},
    )
    .unwrap();
    let path = root.path().join("journal.sqlite");
    let journal = TestJournal::open(&path);
    let net = Mock::new();
    let target = target();
    let (_cancel, _stop, io) = io();
    let mut photo = source(root.path(), "photo", b"original photo");
    photo.kind = "album_file".into();
    photo.container = "jpeg".into();
    let receipt = upload_album_member(&net, &journal, &target, photo, &io, None, &core)
        .await
        .unwrap();
    let message = target.message_id(&receipt.reference).unwrap();
    let mut clip = source(root.path(), "clip", b"original live photo video");
    clip.kind = "album_file".into();
    clip.operation_id = "clip-operation".into();
    clip.container = "mp4".into();
    *net.reject_once.lock().unwrap() = Some("TELEGRAM_RPC_ERROR".into());
    assert!(upload_album_member(
        &net,
        &journal,
        &target,
        clip.clone(),
        &io,
        Some(message),
        &core
    )
    .await
    .is_err());
    assert_eq!(journal.entry(&clip).state, SendState::Sending);
    // Simulate a pre-fix journal whose original RPC rejection was already overwritten.
    let mut historical = journal.entry(&clip);
    historical.first_error = None;
    historical.last_error = Some("TELEGRAM_SEND_UNCONFIRMED".into());
    journal.save(&historical).unwrap();
    drop(journal);
    let journal = TestJournal::open(&path);
    *net.rpc_reject_once.lock().unwrap() = Some(super::super::telegram_session::RejectedRpc {
        code: 400,
        name: "RICH_MESSAGE_INVALID".into(),
    });
    assert_eq!(
        upload_album_member(
            &net,
            &journal,
            &target,
            clip.clone(),
            &io,
            Some(message),
            &core
        )
        .await
        .unwrap_err()
        .to_string(),
        "TELEGRAM_RPC_ERROR"
    );
    assert_eq!(
        journal.entry(&clip).first_error.as_deref(),
        Some("TELEGRAM_RPC_ERROR: RICH_MESSAGE_INVALID (400)")
    );
    assert_eq!(
        journal.entry(&clip).last_error.as_deref(),
        Some("TELEGRAM_RPC_ERROR: RICH_MESSAGE_INVALID (400)")
    );
    // Keep the uncertain-send path to verify that a read failure cannot trigger a resend.
    let mut uncertain = journal.entry(&clip);
    uncertain.state = SendState::Sending;
    journal.save(&uncertain).unwrap();
    net.hidden.store(true, Ordering::SeqCst);
    assert_eq!(
        upload_album_member(
            &net,
            &journal,
            &target,
            clip.clone(),
            &io,
            Some(message),
            &core
        )
        .await
        .unwrap_err()
        .to_string(),
        "TELEGRAM_CONNECTION_ERROR"
    );
    assert_eq!(journal.entry(&clip).state, SendState::Sending);
    assert_eq!(
        net.uploads.load(Ordering::SeqCst),
        3,
        "a failed read must never prove absence"
    );
    net.hidden.store(false, Ordering::SeqCst);
    let receipt = upload_album_member(
        &net,
        &journal,
        &target,
        clip.clone(),
        &io,
        Some(message),
        &core,
    )
    .await
    .unwrap();
    assert_eq!(target.message_id(&receipt.reference).unwrap(), message);
    assert_eq!(journal.entry(&clip).state, SendState::Confirmed);
    assert_eq!(
        journal.entry(&clip).first_error.as_deref(),
        Some("TELEGRAM_RPC_ERROR: RICH_MESSAGE_INVALID (400)")
    );
    assert_eq!(net.album_posts.load(Ordering::SeqCst), 1);
    assert_eq!(net.album_edits.load(Ordering::SeqCst), 1);
    assert_eq!(
        net.finds.load(Ordering::SeqCst),
        0,
        "known album edits must not scan channel history"
    );
    core.shutdown().await;
}

#[tokio::test]
async fn album_member_lost_edit_reply_reconciles_by_id_without_reupload() {
    let root = tempfile::tempdir().unwrap();
    let core = crate::desktop_core::Core::open(
        &root.path().join("core.sqlite"),
        root.path().join("downloads"),
        "unused".into(),
        vec![],
        |_| {},
    )
    .unwrap();
    let path = root.path().join("journal.sqlite");
    let journal = TestJournal::open(&path);
    let net = Mock::new();
    let target = target();
    let (_cancel, _stop, io) = io();
    let mut photo = source(root.path(), "photo", b"original photo");
    photo.kind = "album_file".into();
    photo.container = "jpeg".into();
    let receipt = upload_album_member(&net, &journal, &target, photo, &io, None, &core)
        .await
        .unwrap();
    let message = target.message_id(&receipt.reference).unwrap();
    let mut clip = source(root.path(), "clip", b"original live photo video");
    clip.kind = "album_file".into();
    clip.operation_id = "clip-operation".into();
    clip.container = "mp4".into();
    net.lost_response.store(true, Ordering::SeqCst);
    assert!(upload_album_member(
        &net,
        &journal,
        &target,
        clip.clone(),
        &io,
        Some(message),
        &core
    )
    .await
    .is_err());
    drop(journal);
    let journal = TestJournal::open(&path);
    std::fs::remove_file(&clip.path).unwrap();
    net.lost_response.store(false, Ordering::SeqCst);
    let receipt = upload_album_member(
        &net,
        &journal,
        &target,
        clip.clone(),
        &io,
        Some(message),
        &core,
    )
    .await
    .unwrap();
    assert_eq!(target.message_id(&receipt.reference).unwrap(), message);
    assert_eq!(net.uploads.load(Ordering::SeqCst), 2);
    assert_eq!(net.sends.load(Ordering::SeqCst), 2);
    assert_eq!(net.finds.load(Ordering::SeqCst), 0);
    assert_eq!(
        journal.entry(&clip).first_error.as_deref(),
        Some("TELEGRAM_CONNECTION_ERROR")
    );
    assert!(journal.entry(&clip).last_error.is_none());
    core.shutdown().await;
}

#[tokio::test]
async fn album_backup_telegram_document_preserves_originals_and_live_video_without_transcoding() {
    use crate::files::{AlbumFile, AlbumVideoFile};
    let root = tempfile::tempdir().unwrap();
    let album = root.path().join("album");
    std::fs::create_dir(&album).unwrap();
    let still = b"\xff\xd8\xff-original-photo";
    let clip = b"\0\0\0\x18ftypisom\0\0\0\0isommp42";
    std::fs::write(album.join("001.jpeg"), still).unwrap();
    std::fs::write(album.join("001.mp4"), clip).unwrap();
    let files = vec![AlbumFile {
        filename: "001.jpeg".into(),
        bytes: still.len() as u64,
        sha256: format!("{:x}", Sha256::digest(still)),
        width: Some(800),
        height: Some(600),
        quality_source: "original".into(),
        video: Some(AlbumVideoFile {
            filename: "001.mp4".into(),
            bytes: clip.len() as u64,
            sha256: format!("{:x}", Sha256::digest(clip)),
        }),
    }];
    let archive = crate::backup::album::pack(&album, &files, || false).unwrap();
    let body = std::fs::read(archive.path()).unwrap();
    let mut media = source(root.path(), "video", &body);
    media.container = "tar".into();
    media.video = None;
    let net = Mock::new();
    let journal = TestJournal::open(&root.path().join("journal.sqlite"));
    let target = target();
    let (_cancel, _stop, io) = io();
    let receipt = upload(&net, &journal, &target, media.clone(), &io)
        .await
        .unwrap();
    assert!(net.video_attributes.lock().unwrap()[0].is_none());
    let again = upload(&net, &journal, &target, media, &io).await.unwrap();
    assert_eq!(receipt.reference, again.reference);
    assert_eq!(net.sends.load(Ordering::SeqCst), 1);
    let restored = root.path().join("restored.tar");
    restore(&net, &journal, &target, &receipt.reference, &restored, &io)
        .await
        .unwrap();
    assert_eq!(std::fs::read(&restored).unwrap(), body);
    let destination = tempfile::tempdir().unwrap();
    crate::backup::album::extract(&restored, &files, destination.path(), || false).unwrap();
    assert_eq!(
        std::fs::read(destination.path().join("001.jpeg")).unwrap(),
        still
    );
    assert_eq!(
        std::fs::read(destination.path().join("001.mp4")).unwrap(),
        clip
    );
}
#[tokio::test]
async fn preparation_failure_keeps_prepared_and_retry_publishes_once() {
    let root = tempfile::tempdir().unwrap();
    let journal = TestJournal::open(&root.path().join("journal.sqlite"));
    let net = Mock::new();
    let source = source(root.path(), "video", b"original bytes");
    let (_cancel, _stop, io) = io();
    net.prepare_failure.store(true, Ordering::SeqCst);
    assert_eq!(
        upload(&net, &journal, &target(), source.clone(), &io)
            .await
            .unwrap_err()
            .to_string(),
        "TELEGRAM_CONNECTION_ERROR"
    );
    assert_eq!(journal.entry(&source).state, SendState::Prepared);
    assert_eq!(net.sends.load(Ordering::SeqCst), 0);
    assert_eq!(net.uploads.load(Ordering::SeqCst), 0);
    upload(&net, &journal, &target(), source.clone(), &io)
        .await
        .unwrap();
    assert_eq!(journal.entry(&source).state, SendState::Confirmed);
    assert_eq!(net.sends.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn proxy_stop_is_durable_but_explicit_cancellation_takes_priority() {
    let (cancel, rx) = watch::channel(false);
    let (connection, stopped) = watch::channel(false);
    let (network, epoch) = watch::channel(());
    let io = Io::new(rx, stopped, None).with_network_stop(epoch);
    let pending = io.run(std::future::pending::<Result<()>>());
    tokio::pin!(pending);
    assert!(
        tokio::time::timeout(Duration::from_millis(10), &mut pending)
            .await
            .is_err()
    );
    network.send_replace(());
    connection.send_replace(true);
    assert_eq!(
        pending.await.unwrap_err().to_string(),
        "TELEGRAM_NETWORK_CHANGED"
    );
    assert_eq!(
        io.check().unwrap_err().to_string(),
        "TELEGRAM_NETWORK_CHANGED"
    );
    cancel.send_replace(true);
    assert_eq!(
        io.check().unwrap_err().to_string(),
        "TELEGRAM_TRANSFER_CANCELED"
    );
}

#[tokio::test]
async fn proxy_change_after_publication_reconciles_without_uploading_or_sending_twice() {
    let root = tempfile::tempdir().unwrap();
    let journal = TestJournal::open(&root.path().join("journal.sqlite"));
    let net = Mock::new();
    let source = source(root.path(), "video", b"original bytes");
    let (_cancel, _stop, first_io) = io();
    let (network, epoch) = watch::channel(());
    *net.network_on_send.lock().unwrap() = Some(network);
    let first_io = first_io.with_network_stop(epoch);
    let error = upload(&net, &journal, &target(), source.clone(), &first_io)
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), "TELEGRAM_NETWORK_CHANGED");
    let interrupted = journal.entry(&source);
    assert_eq!(interrupted.state, SendState::Sending);
    std::fs::remove_file(&source.path).unwrap();
    let (_cancel, _stop, retry_io) = io();
    let receipt = upload(&net, &journal, &target(), source.clone(), &retry_io)
        .await
        .unwrap();
    let confirmed = journal.entry(&source);
    assert_eq!(confirmed.state, SendState::Confirmed);
    assert_eq!(confirmed.random_id, interrupted.random_id);
    assert_eq!(receipt.reference.bytes, source.bytes);
    assert_eq!(net.uploads.load(Ordering::SeqCst), 1);
    assert_eq!(net.sends.load(Ordering::SeqCst), 1);
}
impl TestJournal {
    fn open(path: &Path) -> Self {
        let db = Connection::open(path).unwrap();
        db.execute_batch(include_str!("telegram-schema.sql"))
            .unwrap();
        Self(Mutex::new(db))
    }
    fn entry(&self, source: &MediaSource) -> Entry {
        self.get(&source.operation_id, &source.kind)
            .unwrap()
            .unwrap()
    }
}
impl Journal for TestJournal {
    fn get(&self, operation: &str, kind: &str) -> Result<Option<Entry>> {
        db::get(&self.0.lock().unwrap(), operation, kind)
    }
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
type MockDocument = (RemoteDocument, Vec<u8>);
#[derive(Default)]
struct Mock {
    album_message: std::sync::atomic::AtomicI32,
    album_documents: Mutex<HashMap<(i32, i64), MockDocument>>,
    album_posts: AtomicUsize,
    album_edits: AtomicUsize,
    filenames: Mutex<Vec<String>>,
    documents: Mutex<HashMap<i32, (RemoteDocument, Vec<u8>)>>,
    uploads: AtomicUsize,
    prepare_failure: AtomicBool,
    upload_gate: Mutex<Option<(tokio::sync::oneshot::Sender<()>, Arc<tokio::sync::Notify>)>>,
    sends: AtomicUsize,
    inspects: AtomicUsize,
    downloads: AtomicUsize,
    read_delay: Option<Duration>,
    lost_response: AtomicBool,
    hidden: AtomicBool,
    corrupt: AtomicBool,
    expired: AtomicBool,
    deny: AtomicBool,
    cancel_on_send: Mutex<Option<watch::Sender<bool>>>,
    network_on_send: Mutex<Option<watch::Sender<()>>>,
    source_edit: Mutex<Option<PathBuf>>,
    reject_once: Mutex<Option<String>>,
    rpc_reject_once: Mutex<Option<super::super::telegram_session::RejectedRpc>>,
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
    type Prepared = ();
    fn prepare<'a>(&'a self, _: &'a Envelope, _: &'a Io) -> ProviderFuture<'a, ()> {
        Box::pin(async move {
            if self.prepare_failure.swap(false, Ordering::SeqCst) {
                bail!("TELEGRAM_CONNECTION_ERROR");
            }
            Ok(())
        })
    }
    fn prepare_album_member<'a>(
        &'a self,
        envelope: &'a Envelope,
        _source: &'a MediaSource,
        message: Option<i32>,
        _core: &'a crate::desktop_core::Core,
        io: &'a Io,
    ) -> ProviderFuture<'a, Self::Prepared> {
        self.album_message
            .store(message.unwrap_or_default(), Ordering::SeqCst);
        self.prepare(envelope, io)
    }
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
        name: String,
    ) -> ProviderFuture<'a, Vec<u8>> {
        Box::pin(async move {
            self.uploads.fetch_add(1, Ordering::SeqCst);
            self.filenames.lock().unwrap().push(name);
            let mut body = vec![];
            let gate = self.upload_gate.lock().unwrap().take();
            if let Some((started, release)) = gate {
                let mut first = [0u8; 1];
                reader.read_exact(&mut first).await?;
                body.extend_from_slice(&first);
                let _ = started.send(());
                release.notified().await;
            }
            reader.read_to_end(&mut body).await?;
            if let Some(path) = self.source_edit.lock().unwrap().take() {
                std::fs::write(path, b"changed during upload")?;
            }
            Ok(body)
        })
    }
    fn send<'a>(
        &'a self,
        (body, _): (Vec<u8>, ()),
        caption: String,
        random_id: i64,
        video: Option<crate::backup::provider::VideoAttributes>,
        _io: &'a Io,
        before_publish: &'a mut (dyn FnMut() -> Result<()> + Send),
    ) -> ProviderFuture<'a, Option<i32>> {
        Box::pin(async move {
            before_publish()?;
            let id = self.sends.fetch_add(1, Ordering::SeqCst) as i32 + 1;
            self.random_ids.lock().unwrap().push(random_id);
            self.video_attributes.lock().unwrap().push(video);
            if let Some(rpc) = self.rpc_reject_once.lock().unwrap().take() {
                return Err(anyhow::Error::new(rpc).context("TELEGRAM_RPC_ERROR"));
            }
            if let Some(code) = self.reject_once.lock().unwrap().take() {
                bail!(code);
            }
            let envelope: Envelope = serde_json::from_str(&caption)?;
            let message_id = if envelope.kind == "album_file" {
                let anchor = self.album_message.load(Ordering::SeqCst);
                if anchor == 0 {
                    self.album_posts.fetch_add(1, Ordering::SeqCst);
                    id
                } else {
                    self.album_edits.fetch_add(1, Ordering::SeqCst);
                    anchor
                }
            } else {
                id
            };
            let document = RemoteDocument {
                message_id,
                document_id: i64::from(id) + 1000,
                bytes: body.len() as u64,
                caption,
            };
            if envelope.kind == "album_file" {
                self.album_documents
                    .lock()
                    .unwrap()
                    .insert((message_id, document.document_id), (document, body));
            } else {
                self.documents
                    .lock()
                    .unwrap()
                    .insert(message_id, (document, body));
            }
            if let Some(cancel) = self.cancel_on_send.lock().unwrap().as_ref() {
                cancel.send_replace(true);
            }
            if let Some(network) = self.network_on_send.lock().unwrap().as_ref() {
                network.send_replace(());
            }
            if self.lost_response.load(Ordering::SeqCst) {
                bail!("TELEGRAM_CONNECTION_ERROR");
            }
            Ok(Some(message_id))
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
    fn inspect_document(&self, id: i32, document_id: i64) -> ProviderFuture<'_, RemoteDocument> {
        Box::pin(async move {
            if let Some((d, _)) = self.album_documents.lock().unwrap().get(&(id, document_id)) {
                return Ok(d.clone());
            }
            let d = self.inspect(id).await?;
            if d.document_id != document_id {
                bail!("TELEGRAM_MEDIA_CHANGED");
            }
            Ok(d)
        })
    }
    fn inspect_envelope<'a>(
        &'a self,
        id: i32,
        envelope: &'a Envelope,
    ) -> ProviderFuture<'a, RemoteDocument> {
        Box::pin(async move {
            if let Some((d, _)) = self
                .album_documents
                .lock()
                .unwrap()
                .values()
                .find(|(d, _)| d.message_id == id && d.caption == envelope.caption().unwrap())
            {
                return Ok(d.clone());
            }
            let d = self.inspect(id).await?;
            d.check(envelope)?;
            Ok(d)
        })
    }
    fn find_album_member<'a>(
        &'a self,
        message: i32,
        envelope: &'a Envelope,
    ) -> ProviderFuture<'a, Option<RemoteDocument>> {
        Box::pin(async move {
            if self.hidden.load(Ordering::SeqCst) {
                bail!("TELEGRAM_CONNECTION_ERROR");
            }
            let documents = self.album_documents.lock().unwrap();
            if !documents.values().any(|(d, _)| d.message_id == message) {
                bail!("TELEGRAM_MEDIA_MISSING");
            }
            let caption = envelope.caption()?;
            Ok(documents
                .values()
                .find(|(d, _)| d.message_id == message && d.caption == caption)
                .map(|(d, _)| d.clone()))
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
            if let Some((d, _)) = self
                .album_documents
                .lock()
                .unwrap()
                .values()
                .find(|(d, _)| d.caption == caption)
            {
                return Ok(Some(d.clone()));
            }
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
            if let Some((_, body)) = self
                .album_documents
                .lock()
                .unwrap()
                .get(&(document.message_id, document.document_id))
            {
                return Ok(Box::new(std::io::Cursor::new(body.clone()))
                    as Box<dyn AsyncRead + Unpin + Send>);
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
            if let Some(delay) = self.read_delay {
                return Ok(Box::new(SlowFirstRead {
                    body: std::io::Cursor::new(body),
                    ready: Box::pin(tokio::time::sleep(delay)),
                }) as Box<dyn AsyncRead + Unpin + Send>);
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
        video: (kind == "video").then_some(crate::backup::provider::VideoAttributes {
            duration_ms: 2000,
            width: 640,
            height: 360,
            supports_streaming: false,
        }),
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
async fn encrypted_database_uses_standalone_document_and_round_trips() {
    let root = tempfile::tempdir().unwrap();
    let journal = TestJournal::open(&root.path().join("journal.sqlite"));
    let sqlite = root.path().join("source.sqlite");
    Connection::open(&sqlite)
        .unwrap()
        .execute_batch("CREATE TABLE example(value); INSERT INTO example VALUES(42);")
        .unwrap();
    let key = age::x25519::Identity::generate();
    let (encrypted, _, _) = crate::backup::database::encrypt(&sqlite, &key, || false).unwrap();
    let ciphertext = std::fs::read(encrypted.path()).unwrap();
    let mut source = source(root.path(), "database", &ciphertext);
    source.asset_id = key.to_public().to_string();
    source.container = "age".into();
    let net = Mock::new();
    let (_cancel, _stop, io) = io();
    let target = target();
    let receipt = upload(&net, &journal, &target, source.clone(), &io)
        .await
        .unwrap();
    let entry = journal.entry(&source);
    assert_eq!(entry.envelope.yingqu_media, 3);
    assert!(crate::backup::telegram_database::matches(
        &target,
        &source.asset_id,
        &entry.envelope
    ));
    assert!(!crate::backup::telegram_database::matches(
        &target,
        "other recipient",
        &entry.envelope
    ));
    let downloaded = root.path().join("downloaded.age");
    restore(
        &net,
        &journal,
        &target,
        &receipt.reference,
        &downloaded,
        &io,
    )
    .await
    .unwrap();
    let restored = root.path().join("restored.sqlite");
    assert!(crate::backup::database::decrypt(
        &downloaded,
        &restored,
        &age::x25519::Identity::generate()
    )
    .is_err());
    crate::backup::database::decrypt(&downloaded, &restored, &key).unwrap();
    let db = Connection::open(restored).unwrap();
    assert_eq!(
        db.query_row("SELECT value FROM example", [], |row| row.get::<_, u32>(0))
            .unwrap(),
        42
    );
}

#[tokio::test]
async fn telegram_streaming_flag_accepts_front_and_trailing_mp4_metadata_and_keeps_original_bytes()
{
    fn atom(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        [
            ((body.len() + 8) as u32).to_be_bytes().as_slice(),
            kind,
            body,
        ]
        .concat()
    }
    let ftyp = atom(b"ftyp", b"isom0000");
    let moov = atom(b"moov", b"metadata");
    let mdat = atom(b"mdat", &[37; 131_113]);
    for (container, body) in [
        ("mp4", [ftyp.clone(), moov.clone(), mdat.clone()].concat()),
        ("mp4", [ftyp.clone(), mdat.clone(), moov.clone()].concat()),
        ("mov", [ftyp.clone(), moov.clone(), mdat.clone()].concat()),
        ("mov", [ftyp, mdat, moov].concat()),
    ] {
        let root = tempfile::tempdir().unwrap();
        let journal = TestJournal::open(&root.path().join("journal.sqlite"));
        let net = Mock::new();
        let (_cancel, _stop, io) = io();
        let target = target();
        let mut source = source(root.path(), "video", &body);
        source.container = container.into();
        source.video = crate::backup::provider::VideoAttributes::from_metadata(
            &serde_json::json!({"duration_ms":2000,"width":640,"height":360}),
        );
        let receipt = upload(&net, &journal, &target, source.clone(), &io)
            .await
            .unwrap();
        assert!(
            net.video_attributes.lock().unwrap()[0]
                .as_ref()
                .unwrap()
                .supports_streaming,
            "{container} must allow streaming with either index layout"
        );
        assert_eq!(
            receipt.reference.sha256,
            format!("{:x}", Sha256::digest(&body))
        );
        let restored = root.path().join("restored.mp4");
        restore(&net, &journal, &target, &receipt.reference, &restored, &io)
            .await
            .unwrap();
        assert_eq!(std::fs::read(restored).unwrap(), body);
        assert_eq!(std::fs::read(source.path).unwrap(), body);
    }
}

#[tokio::test]
async fn telegram_streaming_layout_recognition_handles_extended_and_invalid_boxes() {
    fn atom(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        [
            ((body.len() + 8) as u32).to_be_bytes().as_slice(),
            kind,
            body,
        ]
        .concat()
    }
    let ftyp = atom(b"ftyp", b"isom0000");
    let moov = atom(b"moov", b"metadata");
    let data = atom(b"mdat", b"original");
    let wide_moov = [
        1u32.to_be_bytes().as_slice(),
        b"moov",
        24u64.to_be_bytes().as_slice(),
        b"metadata",
    ]
    .concat();
    let eof_data = [0u32.to_be_bytes().as_slice(), b"mdat", b"original"].concat();
    let invalid = [1000u32.to_be_bytes().as_slice(), b"moov", b"short"].concat();
    let empty_data = atom(b"mdat", b"");
    let wide_empty_data = [
        1u32.to_be_bytes().as_slice(),
        b"mdat",
        16u64.to_be_bytes().as_slice(),
    ]
    .concat();
    let cases = [
        (
            [ftyp.clone(), empty_data.clone(), moov.clone(), data.clone()].concat(),
            true,
        ),
        (
            [ftyp.clone(), moov.clone(), empty_data, data.clone()].concat(),
            true,
        ),
        (
            [ftyp.clone(), wide_empty_data, moov.clone(), data.clone()].concat(),
            true,
        ),
        ([ftyp.clone(), wide_moov, data.clone()].concat(), true),
        ([ftyp.clone(), moov.clone(), eof_data].concat(), true),
        ([ftyp.clone(), data.clone(), moov.clone()].concat(), true),
        (
            [ftyp.clone(), data.clone(), invalid.clone()].concat(),
            false,
        ),
        ([ftyp.clone(), moov.clone()].concat(), false),
        ([moov, data].concat(), false),
        ([ftyp.clone(), invalid].concat(), false),
        (
            [ftyp, 1u32.to_be_bytes().to_vec(), b"moov".to_vec()].concat(),
            false,
        ),
    ];
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("layout.mp4");
    let (_cancel, _stop, io) = io();
    for (body, expected) in cases {
        std::fs::write(&path, body).unwrap();
        assert_eq!(mp4_has_metadata(&path, &io).await.unwrap(), expected);
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
/// Inject a lost reply after the mock channel has committed the real transfer payload.
pub(in crate::backup) async fn send_archive_losing_reply(
    root: &Path,
    journal: &impl Journal,
    target: &Target,
    operation: &str,
    device: &str,
    body: &[u8],
    io: &Io,
) -> (RemoteDocument, Vec<u8>) {
    let net = Mock::new();
    net.sends.store(29, Ordering::SeqCst); // next channel message ID is 30
    net.lost_response.store(true, Ordering::SeqCst);
    let mut file = source(root, "library", body);
    file.asset_id.clone_from(&target.scope.library_id);
    file.operation_id = operation.into();
    file.device_id = device.into();
    assert_eq!(
        upload(&net, journal, target, file, io)
            .await
            .unwrap_err()
            .to_string(),
        "TELEGRAM_CONNECTION_ERROR"
    );
    assert_eq!(net.uploads.load(Ordering::SeqCst), 1);
    net.documents.into_inner().unwrap().remove(&30).unwrap()
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
    assert_eq!(
        journal.entry(&source).first_error.as_deref(),
        Some("TELEGRAM_CONNECTION_ERROR")
    );
    assert_eq!(
        journal.entry(&source).last_error.as_deref(),
        Some("TELEGRAM_SEND_UNCONFIRMED")
    );
    assert!(source.path.is_file());
}
#[tokio::test]
async fn telegram_transfer_rpc_rejection_preserves_first_diagnostic_after_restart_and_later_retries(
) {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("journal.sqlite");
    let journal = TestJournal::open(&path);
    let net = Mock::new();
    let (_cancel, _stop, io) = io();
    let source = source(root.path(), "cover", b"original bytes");
    *net.rpc_reject_once.lock().unwrap() = Some(super::super::telegram_session::RejectedRpc {
        code: 400,
        name: "RICH_MESSAGE_INVALID".into(),
    });
    assert!(upload(&net, &journal, &target(), source.clone(), &io)
        .await
        .is_err());
    assert_eq!(journal.entry(&source).state, SendState::Prepared);
    drop(journal);
    let journal = TestJournal::open(&path);
    *net.reject_once.lock().unwrap() = Some("TELEGRAM_CONNECTION_ERROR".into());
    assert!(upload(&net, &journal, &target(), source.clone(), &io)
        .await
        .is_err());
    assert_eq!(
        journal.entry(&source).last_error.as_deref(),
        Some("TELEGRAM_CONNECTION_ERROR")
    );
    upload(&net, &journal, &target(), source.clone(), &io)
        .await
        .unwrap_err();
    assert_eq!(
        journal.entry(&source).last_error.as_deref(),
        Some("TELEGRAM_SEND_UNCONFIRMED")
    );
    // The first RPC rejection remains available even when reconciliation fails later.
    assert_eq!(
        journal.entry(&source).first_error.as_deref(),
        Some("TELEGRAM_RPC_ERROR: RICH_MESSAGE_INVALID (400)")
    );
    assert_eq!(net.sends.load(Ordering::SeqCst), 2);
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
async fn missing_parts_after_upload_return_to_prepared_without_confirming_or_blind_resend() {
    let root = tempfile::tempdir().unwrap();
    let journal = TestJournal::open(&root.path().join("journal.sqlite"));
    let net = Mock::new();
    let (_cancel, _stop, io) = io();
    let target = target();
    let source = source(root.path(), "video", b"unchanged original bytes");
    *net.reject_once.lock().unwrap() = Some("TELEGRAM_UPLOAD_PART_MISSING".into());
    assert_eq!(
        upload(&net, &journal, &target, source.clone(), &io)
            .await
            .unwrap_err()
            .to_string(),
        "TELEGRAM_UPLOAD_PART_MISSING"
    );
    assert_eq!(journal.entry(&source).state, SendState::Prepared);
    assert!(journal.entry(&source).document.is_none());
    assert!(net.documents.lock().unwrap().is_empty());
    upload(&net, &journal, &target, source.clone(), &io)
        .await
        .unwrap();
    assert_eq!(net.uploads.load(Ordering::SeqCst), 2);
    assert_eq!(journal.entry(&source).state, SendState::Confirmed);
    let ids = net.random_ids.lock().unwrap();
    assert_eq!(ids.len(), 2);
    assert_eq!(ids[0], ids[1]);
    assert_eq!(net.documents.lock().unwrap().len(), 1);
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
    let deadline = journal.cooldown("123").unwrap();
    let (_cancel, _stop, io) = io();
    let source = source(root.path(), "cover", b"cover");
    let reason = upload(&net, &journal, &target(), source, &io)
        .await
        .unwrap_err()
        .to_string();
    assert!(reason.starts_with("TELEGRAM_COOLDOWN_WAIT:"));
    journal.failure("123", &reason).unwrap();
    assert_eq!(journal.cooldown("123").unwrap(), deadline);
    assert_eq!(net.uploads.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn telegram_transfer_history_progress_keeps_send_alive_and_still_times_out_when_stalled() {
    let (_cancel, _stop, mut io) = io();
    io.idle = Duration::from_millis(150);
    io.run(async {
        for _ in 0..4 {
            tokio::time::sleep(Duration::from_millis(80)).await;
            io.check()?;
            io.touch();
        }
        Ok(())
    })
    .await
    .unwrap();
    let error = io
        .run(async { std::future::pending::<Result<()>>().await })
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), "TELEGRAM_TRANSFER_TIMEOUT");
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

#[tokio::test]
async fn upload_io_observes_binding_and_login_stops_in_source_fingerprints() {
    for binding_changed in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let source = source(root.path(), "video", b"original bytes");
        let (_cancel, _stop, io) = io();
        let (login, login_stop) = watch::channel(false);
        let (binding, binding_stop) = watch::channel(());
        let io = io.with_session_stops(login_stop, binding_stop);
        if binding_changed {
            binding.send_replace(());
        } else {
            login.send_replace(true);
        }
        assert_eq!(
            source_unchanged(&source, &io)
                .await
                .unwrap_err()
                .to_string(),
            "TELEGRAM_TRANSFER_CANCELED"
        );
        assert_eq!(
            io.run(async { Ok(()) }).await.unwrap_err().to_string(),
            "TELEGRAM_TRANSFER_CANCELED"
        );
    }
}

#[tokio::test]
async fn proxy_change_stops_the_source_fingerprint_worker_before_scanning() {
    let root = tempfile::tempdir().unwrap();
    let source = source(root.path(), "video", b"original bytes");
    let (_cancel, _stop, mut io) = io();
    let (network, epoch) = watch::channel(());
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = calls.clone();
    io.control = Some(Arc::new(TransferControl {
        check: Box::new(move || {
            if observed.fetch_add(1, Ordering::SeqCst) == 0 {
                network.send_replace(());
            }
            Ok(())
        }),
        checkpoint: Box::new(|_| Ok(())),
        progress: Arc::new(AtomicU64::new(0)),
    }));
    let io = io.with_network_stop(epoch);
    assert_eq!(
        source_unchanged(&source, &io)
            .await
            .unwrap_err()
            .to_string(),
        "TELEGRAM_NETWORK_CHANGED"
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        2,
        "one worker check, then the final Io classification; no file scan checks"
    );
}

#[tokio::test]
async fn telegram_upload_disk_reads_do_not_advance_server_confirmed_progress() {
    let root = tempfile::tempdir().unwrap();
    let journal = TestJournal::open(&root.path().join("journal.sqlite"));
    let net = Mock::new();
    let (started, first_byte) = tokio::sync::oneshot::channel();
    let release = Arc::new(tokio::sync::Notify::new());
    *net.upload_gate.lock().unwrap() = Some((started, release.clone()));
    let mut source = source(root.path(), "video", b"original video bytes");
    let progress = Arc::new(AtomicU64::new(0));
    source.control = Some(Arc::new(TransferControl {
        check: Box::new(|| Ok(())),
        checkpoint: Box::new(|_| Ok(())),
        progress: progress.clone(),
    }));
    let (_cancel, _stop, io) = io();
    let target = target();
    let uploading = upload(&net, &journal, &target, source.clone(), &io);
    tokio::pin!(uploading);
    tokio::select! {
        result = first_byte => result.unwrap(),
        result = &mut uploading => panic!("transfer finished before first-byte observation: {result:?}"),
        _ = tokio::time::sleep(Duration::from_secs(3)) => panic!("upload never started"),
    }
    assert_eq!(progress.load(Ordering::Relaxed), 0);
    assert_eq!(net.sends.load(Ordering::SeqCst), 0);
    release.notify_one();
    let receipt = uploading.await.unwrap();
    assert_eq!(progress.load(Ordering::Relaxed), source.bytes);
    assert_eq!(receipt.reference.sha256, source.sha256);
}

#[tokio::test]
async fn telegram_all_video_containers_keep_attributes_and_original_bytes() {
    for container in ["mp4", "mov", "webm", "mkv", "avi", "m4v"] {
        let root = tempfile::tempdir().unwrap();
        let journal = TestJournal::open(&root.path().join("journal.sqlite"));
        let net = Mock::new();
        let (_cancel, _stop, io) = io();
        let mut media = source(root.path(), "video", b"original fixture bytes");
        media.container = container.into();
        let target = target();
        let receipt = upload(&net, &journal, &target, media.clone(), &io)
            .await
            .unwrap();
        let attributes = net.video_attributes.lock().unwrap()[0].clone().unwrap();
        assert_eq!(
            (attributes.duration_ms, attributes.width, attributes.height),
            (2000, 640, 360)
        );
        assert!(!attributes.supports_streaming);
        assert_eq!(receipt.reference.container, container);
        assert_eq!(receipt.reference.sha256, media.sha256);
        assert_eq!(
            net.documents.lock().unwrap()[&1].1,
            b"original fixture bytes"
        );
    }
}

#[tokio::test]
async fn telegram_missing_video_attributes_fail_instead_of_silently_sending_a_file() {
    let root = tempfile::tempdir().unwrap();
    let journal = TestJournal::open(&root.path().join("journal.sqlite"));
    let net = Mock::new();
    let (_cancel, _stop, io) = io();
    let mut media = source(root.path(), "video", b"original bytes");
    media.video = None;
    assert_eq!(
        upload(&net, &journal, &target(), media, &io)
            .await
            .unwrap_err()
            .to_string(),
        "VIDEO_METADATA_UNAVAILABLE"
    );
    assert_eq!(net.uploads.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn telegram_uncertain_send_does_not_probe_or_require_the_local_video_again() {
    let root = tempfile::tempdir().unwrap();
    let core = crate::desktop_core::Core::open(
        &root.path().join("qa.sqlite"),
        root.path().join("downloads"),
        "must-not-start".into(),
        vec![],
        |_| {},
    )
    .unwrap();
    let mut source = source(root.path(), "video", b"original bytes");
    let journal = super::super::telegram_transfer_journal::CoreJournal(&core);
    let target = target();
    let envelope = Envelope::source(&target, &source).unwrap();
    let mut entry = journal
        .reserve(Entry {
            target: target.clone(),
            random_id: envelope.random_id().unwrap(),
            envelope,
            state: SendState::Prepared,
            document: None,
            last_error: None,
            first_error: None,
        })
        .unwrap();
    entry.state = SendState::Sending;
    journal.save(&entry).unwrap();
    std::fs::remove_file(&source.path).unwrap();
    source.video = None;
    let (_cancel, _stop, io) = io();
    prepare_video(&core, &mut source, &io).await.unwrap();
    assert!(source.video.is_none());
    core.shutdown().await;
}

#[tokio::test]
async fn telegram_video_probe_respects_canceled_io_before_launch() {
    let root = tempfile::tempdir().unwrap();
    let core = crate::desktop_core::Core::open(
        &root.path().join("qa.sqlite"),
        root.path().join("downloads"),
        "python-must-not-run".into(),
        vec![],
        |_| {},
    )
    .unwrap();
    let mut source = source(root.path(), "video", b"original bytes");
    source.video = None;
    let (cancel, _stop, io) = io();
    cancel.send_replace(true);
    assert_eq!(
        prepare_video(&core, &mut source, &io)
            .await
            .unwrap_err()
            .to_string(),
        "TELEGRAM_TRANSFER_CANCELED"
    );
    core.shutdown().await;
}

#[tokio::test]
async fn lost_direct_archive_receipt_finds_and_rereads_before_confirming() {
    let root = tempfile::tempdir().unwrap();
    let journal = TestJournal::open(&root.path().join("journal.sqlite"));
    let net = Mock::new();
    let (_cancel, _stop, io) = io();
    let target = target();
    let source = source(root.path(), "library", b"archive");
    let publish = || {
        upload_with_timing(
            &net,
            &journal,
            &target,
            source.clone(),
            &io,
            Default::default(),
            ReceiptCheck::FreshDeleteArchive,
        )
    };
    net.lost_response.store(true, Ordering::SeqCst);
    assert_eq!(
        publish().await.unwrap_err().to_string(),
        "TELEGRAM_CONNECTION_ERROR"
    );
    assert_eq!(journal.entry(&source).state, SendState::Sending);
    net.hidden.store(true, Ordering::SeqCst);
    assert_eq!(
        publish().await.unwrap_err().to_string(),
        "TELEGRAM_SEND_UNCONFIRMED"
    );
    net.hidden.store(false, Ordering::SeqCst);
    publish().await.unwrap();
    assert_eq!(journal.entry(&source).state, SendState::Confirmed);
    assert_eq!(net.finds.load(Ordering::SeqCst), 2);
    assert_eq!(net.inspects.load(Ordering::SeqCst), 1);
    assert_eq!(net.sends.load(Ordering::SeqCst), 1);
    net.documents
        .lock()
        .unwrap()
        .get_mut(&1)
        .unwrap()
        .0
        .document_id += 1;
    assert_eq!(
        publish().await.unwrap_err().to_string(),
        "TELEGRAM_MEDIA_CHANGED"
    );
}

#[tokio::test]
async fn delete_index_transfer_timings_follow_parent_and_skip_reused_uploads() {
    use crate::{backup::delete_timing::IndexTiming, desktop_core::Core};
    let root = tempfile::tempdir().unwrap();
    let core = Core::open(
        &root.path().join("core.sqlite"),
        root.path().join("downloads"),
        "unused".into(),
        vec![],
        |_| {},
    )
    .unwrap();
    core.backup_db(|db| {
        db.execute("INSERT INTO backup_spaces(space_id,library_id,endpoint,bucket,prefix,enabled) VALUES('qa','library','','','',0)", [])?;
        db.execute("INSERT INTO backup_operations(operation_id,space_id,kind,state,phase,payload_json) VALUES('deletion','qa','delete','running','objects','{}')", [])?;
        Ok(())
    }).unwrap();
    let journal = TestJournal::open(&root.path().join("journal.sqlite"));
    let net = Mock::new();
    let (_cancel, _stop, io) = io();
    for (kind, stage, check, reads) in [
        ("event", "publish", ReceiptCheck::ReadBack, 2),
        ("library", "archive", ReceiptCheck::ReadBack, 2),
        ("library", "publish", ReceiptCheck::FreshDeleteArchive, 1),
    ] {
        let mut source = source(root.path(), kind, b"{}");
        source.operation_id = format!("{kind}-{stage}");
        let timing = IndexTiming::new(&core, "deletion", stage);
        let before = net.inspects.load(Ordering::SeqCst);
        upload_with_timing(
            &net,
            &journal,
            &target(),
            source.clone(),
            &io,
            timing,
            check,
        )
        .await
        .unwrap();
        assert_eq!(net.inspects.load(Ordering::SeqCst) - before, reads);
        let read = || {
            core.backup_db(|db| {
                let raw: String = db.query_row(
                    "SELECT payload_json FROM backup_operations WHERE operation_id='deletion'",
                    [],
                    |r| r.get(0),
                )?;
                Ok(serde_json::from_str::<serde_json::Value>(&raw)?)
            })
            .unwrap()
        };
        let first = read();
        for step in ["upload", "send", "receipt"] {
            let metric = &first["timings"]["foreground"][format!("{stage}_{step}")];
            assert_eq!(metric["outcome"], "ok");
            assert!(metric["elapsed_ms"].is_u64());
        }
        let steps = &first["timings"]["foreground"][format!("{stage}_receipt")]["details"]["steps"];
        assert_eq!(steps["receipt_by_id"]["calls"], reads);
        assert!(steps["receipt_by_id"]["elapsed_ms"].is_u64());
        core.backup_db(|db| {
            db.execute("UPDATE backup_operations SET payload_json=json_set(payload_json,'$.local_cleanup_done',json('true')) WHERE operation_id='deletion'", [])?;
            Ok(())
        }).unwrap();
        upload_with_timing(&net, &journal, &target(), source, &io, timing, check)
            .await
            .unwrap();
        assert_eq!(net.inspects.load(Ordering::SeqCst) - before, reads + 1);
        let retry = read();
        let maintenance = &retry["timings"]["maintenance"];
        assert_eq!(maintenance[format!("{stage}_receipt")]["outcome"], "ok");
        assert_eq!(
            maintenance[format!("{stage}_receipt")]["details"]["steps"]["receipt_by_id"]["calls"],
            1
        );
        assert!(maintenance.get(format!("{stage}_upload")).is_none());
        assert!(maintenance.get(format!("{stage}_send")).is_none());
        core.backup_db(|db| {
            db.execute("UPDATE backup_operations SET attempt=attempt+1,payload_json=json_remove(payload_json,'$.local_cleanup_done') WHERE operation_id='deletion'", [])?;
            Ok(())
        }).unwrap();
    }
    core.shutdown().await;
}

#[tokio::test]
async fn new_asset_attachments_reuse_confirmed_video_before_projection_exists() {
    use super::super::telegram_provider::upload_destination;
    let root = tempfile::tempdir().unwrap();
    let journal = TestJournal::open(&root.path().join("journal.sqlite"));
    let net = Mock::new();
    let video = source(root.path(), "video", b"original bytes");
    let (_cancel, _stop, io) = io();
    let receipt = upload(&net, &journal, &target(), video.clone(), &io)
        .await
        .unwrap();
    let id = target().message_id(&receipt.reference).unwrap();
    for kind in ["cover", "transcript"] {
        let mut attachment = video.clone();
        attachment.kind = kind.into();
        assert_eq!(
            upload_destination(&journal, &target(), &attachment, None).unwrap(),
            Some(id)
        );
        attachment.asset_id = "another-asset".into();
        assert_eq!(
            upload_destination(&journal, &target(), &attachment, None)
                .unwrap_err()
                .to_string(),
            "TELEGRAM_OPERATION_MISMATCH"
        );
    }
    let mut entry = journal.entry(&video);
    entry.state = SendState::Sending;
    journal.save(&entry).unwrap();
    let mut attachment = video;
    attachment.kind = "cover".into();
    assert_eq!(
        upload_destination(&journal, &target(), &attachment, None)
            .unwrap_err()
            .to_string(),
        "TELEGRAM_SEND_UNCONFIRMED"
    );
}

struct SlowFirstRead {
    body: std::io::Cursor<Vec<u8>>,
    ready: Pin<Box<tokio::time::Sleep>>,
}
impl AsyncRead for SlowFirstRead {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        use std::future::Future;
        if self.ready.as_mut().poll(cx).is_pending() {
            return Poll::Pending;
        }
        Pin::new(&mut self.body).poll_read(cx, buffer)
    }
}

#[tokio::test(start_paused = true)]
async fn full_media_verification_allows_seventy_second_chunk_and_cancels_promptly() {
    let root = tempfile::tempdir().unwrap();
    let journal = TestJournal::open(&root.path().join("journal.sqlite"));
    let net = Mock {
        read_delay: Some(Duration::from_secs(70)),
        ..Mock::new()
    };
    let source = source(root.path(), "video", b"original verified media bytes");
    let (cancel, _stop, io) = io();
    let receipt = upload(&net, &journal, &target(), source, &io)
        .await
        .unwrap();
    let started = tokio::time::Instant::now();
    super::super::telegram_lifecycle::verify(&net, &target(), &receipt.reference, &io)
        .await
        .unwrap();
    assert!(started.elapsed() >= Duration::from_secs(70));
    let destination = target();
    let mut reading = Box::pin(super::super::telegram_lifecycle::verify(
        &net,
        &destination,
        &receipt.reference,
        &io,
    ));
    assert!(tokio::time::timeout(Duration::from_secs(1), &mut reading)
        .await
        .is_err());
    cancel.send_replace(true);
    assert_eq!(
        tokio::time::timeout(Duration::from_millis(100), reading)
            .await
            .unwrap()
            .unwrap_err()
            .to_string(),
        "TELEGRAM_TRANSFER_CANCELED"
    );
}

#[tokio::test(start_paused = true)]
async fn control_requests_keep_the_short_timeout() {
    let (_cancel, _stop, io) = io();
    let start = tokio::time::Instant::now();
    let result = io
        .run(async {
            tokio::time::sleep(Duration::from_secs(70)).await;
            Ok(())
        })
        .await;
    assert_eq!(result.unwrap_err().to_string(), "TELEGRAM_TRANSFER_TIMEOUT");
    assert_eq!(start.elapsed(), Duration::from_secs(60));
}

#[tokio::test]
async fn album_backup_telegram_independent_originals_survive_lost_response_and_journal_restart() {
    use crate::files::{AlbumFile, AlbumVideoFile};
    let root = tempfile::tempdir().unwrap();
    let album = root.path().join("album");
    std::fs::create_dir(&album).unwrap();
    let still = b"\xff\xd8\xff-original-photo";
    let clip = b"\0\0\0\x18ftypisom\0\0\0\0isommp42";
    std::fs::write(album.join("001.jpeg"), still).unwrap();
    std::fs::write(album.join("001.mp4"), clip).unwrap();
    let files = vec![AlbumFile {
        filename: "001.jpeg".into(),
        bytes: still.len() as u64,
        sha256: format!("{:x}", Sha256::digest(still)),
        width: Some(800),
        height: Some(600),
        quality_source: "original".into(),
        video: Some(AlbumVideoFile {
            filename: "001.mp4".into(),
            bytes: clip.len() as u64,
            sha256: format!("{:x}", Sha256::digest(clip)),
        }),
    }];

    let asset = crate::backup::model::Asset {
        asset_id: "asset".into(),
        task_id: None,
        metadata: serde_json::json!({"kind":"image_album","album_files":files}),
        original_filename: "album".into(),
        content_sha256: None,
        content_bytes: 0,
        asset_state: "active".into(),
        path: Some(album.clone()),
        local_state: "present".into(),
    };
    let (hash, fingerprint) = crate::backup::album::fingerprint(&asset, || false).unwrap();
    let mut media = source(root.path(), "album_file", b"unused");
    media.path = album.clone();
    media.video = None;
    media.bytes = fingerprint["size"].as_u64().unwrap();
    media.sha256 = hash;
    media.container = "album".into();
    let core = crate::desktop_core::Core::open(
        &root.path().join("core.sqlite"),
        root.path().join("downloads"),
        "unused".into(),
        vec![],
        |_| {},
    )
    .unwrap();
    let net = Mock::new();
    let target = target();
    let (_cancel, _stop, io) = io();
    let journal_path = root.path().join("journal.sqlite");
    let journal = TestJournal::open(&journal_path);
    net.lost_response.store(true, Ordering::SeqCst);
    assert!(crate::backup::telegram_album::upload(
        &core,
        &net,
        &journal,
        &target,
        media.clone(),
        files.clone(),
        &io
    )
    .await
    .is_err());
    assert_eq!(net.sends.load(Ordering::SeqCst), 1);
    assert_eq!(
        net.finds.load(Ordering::SeqCst),
        0,
        "Prepared sends need no uncertain-send lookup"
    );
    drop(journal);
    net.lost_response.store(false, Ordering::SeqCst);
    let journal = TestJournal::open(&journal_path);
    let receipt = crate::backup::telegram_album::upload(
        &core,
        &net,
        &journal,
        &target,
        media.clone(),
        files.clone(),
        &io,
    )
    .await
    .unwrap();
    let again = crate::backup::telegram_album::upload(
        &core,
        &net,
        &journal,
        &target,
        media,
        files.clone(),
        &io,
    )
    .await
    .unwrap();
    assert_eq!(receipt.reference, again.reference);
    assert_eq!(net.sends.load(Ordering::SeqCst), 2);
    assert_eq!(net.uploads.load(Ordering::SeqCst), 2);
    assert_eq!(net.album_posts.load(Ordering::SeqCst), 1);
    assert_eq!(net.album_edits.load(Ordering::SeqCst), 1);
    assert!(
        net.finds.load(Ordering::SeqCst) >= 1,
        "Sending evidence still reconciles the lost reply"
    );
    assert_eq!(*net.filenames.lock().unwrap(), vec!["001.jpeg", "001.mp4"]);
    assert!(net
        .video_attributes
        .lock()
        .unwrap()
        .iter()
        .all(Option::is_none));
    let MediaLocation::TelegramAlbum { files: members } = &receipt.reference.location else {
        panic!("independent album reference expected")
    };
    assert!(members
        .iter()
        .all(|m| target.message_id(&m.reference).unwrap()
            == target.message_id(&members[0].reference).unwrap()));
    assert_eq!(
        members
            .iter()
            .map(|m| m.filename.as_str())
            .collect::<Vec<_>>(),
        vec!["001.jpeg", "001.mp4"]
    );
    for member in members {
        let destination = root.path().join(format!("restored-{}", member.filename));
        restore(
            &net,
            &journal,
            &target,
            &member.reference,
            &destination,
            &io,
        )
        .await
        .unwrap();
        assert_eq!(
            std::fs::read(destination).unwrap(),
            std::fs::read(album.join(&member.filename)).unwrap()
        );
    }
    crate::backup::telegram_lifecycle::verify(&net, &target, &receipt.reference, &io)
        .await
        .unwrap();
    let mut bad = receipt.reference.clone();
    if let MediaLocation::TelegramAlbum { files: members } = &mut bad.location {
        members.swap(0, 1);
    }
    assert!(crate::backup::album::validate_reference(&bad, &files).is_err());
}
