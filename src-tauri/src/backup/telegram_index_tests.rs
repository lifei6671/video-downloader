use super::*;
use sha2::{Digest, Sha256};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Mutex,
};

struct Downloads {
    body: Mutex<Vec<u8>>,
    calls: AtomicUsize,
    expired: AtomicBool,
}
impl Transport for Downloads {
    type Uploaded = ();
    fn max_bytes(&self) -> u64 {
        16 * 1024 * 1024
    }
    fn can_upload(&self) -> bool {
        false
    }
    fn upload<'a>(
        &'a self,
        _: &'a mut (dyn AsyncRead + Unpin + Send),
        _: u64,
        _: String,
    ) -> super::super::provider::ProviderFuture<'a, ()> {
        unreachable!("index reads must not upload")
    }
    fn send<'a>(
        &'a self,
        _: (),
        _: String,
        _: i64,
        _: Option<VideoAttributes>,
        _: &'a Io,
    ) -> super::super::provider::ProviderFuture<'a, Option<i32>> {
        unreachable!("index reads must not send")
    }
    fn inspect(&self, _: i32) -> super::super::provider::ProviderFuture<'_, RemoteDocument> {
        unreachable!("index download already has fresh message evidence")
    }
    fn find<'a>(
        &'a self,
        _: &'a Envelope,
        _: &'a Io,
    ) -> super::super::provider::ProviderFuture<'a, Option<RemoteDocument>> {
        unreachable!("index reads must not discover sends")
    }
    fn download(
        &self,
        _: RemoteDocument,
    ) -> super::super::provider::ProviderFuture<'_, Box<dyn AsyncRead + Unpin + Send>> {
        Box::pin(async {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.expired.swap(false, Ordering::SeqCst) {
                bail!("TELEGRAM_FILE_REFERENCE_EXPIRED");
            }
            Ok(
                Box::new(std::io::Cursor::new(self.body.lock().unwrap().clone()))
                    as Box<dyn AsyncRead + Unpin + Send>,
            )
        })
    }
}
fn document(body: &[u8]) -> RemoteDocument {
    let id = uuid::Uuid::new_v4().to_string();
    let envelope = Envelope {
        generation: None,
        yingqu_media: 2,
        library_id: id.clone(),
        asset_id: id.clone(),
        operation_id: id.clone(),
        device_id: id,
        kind: "event".into(),
        bytes: body.len() as u64,
        sha256: format!("{:x}", Sha256::digest(body)),
        container: "json".into(),
    };
    RemoteDocument {
        message_id: 10,
        document_id: 100,
        bytes: body.len() as u64,
        caption: envelope.caption().unwrap(),
    }
}

#[tokio::test]
async fn verified_index_reads_reuse_body_but_refresh_changed_document_evidence() {
    let body = b"verified original index";
    let source = Downloads {
        body: Mutex::new(body.to_vec()),
        calls: AtomicUsize::new(0),
        expired: AtomicBool::new(false),
    };
    let cache = IndexBodyCache::default();
    let (_cancel, rx) = tokio::sync::watch::channel(false);
    let (_stop, srx) = tokio::sync::watch::channel(false);
    let io = Io::new(rx, srx, None);
    let expected = document(body);
    for _ in 0..100 {
        assert_eq!(
            read_index_body(&source, &cache, expected.clone(), &io, None)
                .await
                .unwrap(),
            body
        );
    }
    assert_eq!(
        source.calls.load(Ordering::SeqCst),
        1,
        "100 fresh snapshots of the same immutable index need one body download"
    );
    let mut replaced = expected.clone();
    replaced.document_id += 1;
    read_index_body(&source, &cache, replaced, &io, None)
        .await
        .unwrap();
    let mut edited = expected;
    let mut envelope: Envelope = serde_json::from_str(&edited.caption).unwrap();
    envelope.operation_id = uuid::Uuid::new_v4().to_string();
    edited.caption = envelope.caption().unwrap();
    read_index_body(&source, &cache, edited, &io, None)
        .await
        .unwrap();
    assert_eq!(
        source.calls.load(Ordering::SeqCst),
        3,
        "replaced documents and edited captions must be verified again"
    );
}

#[tokio::test]
async fn index_cache_does_not_keep_bad_hashes_or_ignore_cancellation_and_request_boundaries() {
    let good = b"good index";
    let expected = document(good);
    let source = Downloads {
        body: Mutex::new(b"bad! index".to_vec()),
        calls: AtomicUsize::new(0),
        expired: AtomicBool::new(false),
    };
    let cache = IndexBodyCache::default();
    let (cancel, rx) = tokio::sync::watch::channel(false);
    let (_stop, srx) = tokio::sync::watch::channel(false);
    let io = Io::new(rx, srx, None);
    assert_eq!(
        read_index_body(&source, &cache, expected.clone(), &io, None)
            .await
            .unwrap_err()
            .to_string(),
        "TELEGRAM_INDEX_INVALID"
    );
    *source.body.lock().unwrap() = good.to_vec();
    assert_eq!(
        read_index_body(&source, &cache, expected.clone(), &io, None)
            .await
            .unwrap(),
        good
    );
    assert_eq!(
        source.calls.load(Ordering::SeqCst),
        2,
        "a corrupt response cannot populate the cache"
    );
    cancel.send(true).unwrap();
    assert!(
        read_index_body(&source, &cache, expected.clone(), &io, None)
            .await
            .is_err()
    );
    assert_eq!(source.calls.load(Ordering::SeqCst), 2);
    cancel.send(false).unwrap();
    read_index_body(&source, &IndexBodyCache::default(), expected, &io, None)
        .await
        .unwrap();
    assert_eq!(
        source.calls.load(Ordering::SeqCst),
        3,
        "a new request must verify its own index bodies"
    );
}

#[tokio::test]
async fn scan_reference_reads_verified_bytes_and_refreshes_only_on_expiry() {
    struct Expired;
    impl AsyncRead for Expired {
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

    let body = b"index";
    let expected = document(body);
    let source = Downloads {
        body: Mutex::new(body.to_vec()),
        calls: AtomicUsize::new(0),
        expired: AtomicBool::new(false),
    };
    let (_cancel, rx) = tokio::sync::watch::channel(false);
    let (_stop, srx) = tokio::sync::watch::channel(false);
    let io = Io::new(rx, srx, None);
    let cache = IndexBodyCache::default();
    let bad = Box::new(std::io::Cursor::new(b"other"));
    let error = read_index_body(&source, &cache, expected.clone(), &io, Some(bad))
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), "TELEGRAM_INDEX_INVALID");
    let valid = Box::new(std::io::Cursor::new(body));
    assert_eq!(
        read_index_body(&source, &cache, expected.clone(), &io, Some(valid))
            .await
            .unwrap(),
        body
    );
    assert_eq!(source.calls.load(Ordering::SeqCst), 0);

    let cold = IndexBodyCache::default();
    assert_eq!(
        read_index_body(&source, &cold, expected, &io, Some(Box::new(Expired)))
            .await
            .unwrap(),
        body
    );
    assert_eq!(source.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn index_file_reference_refresh_remains_bounded() {
    let body = b"index";
    let source = Downloads {
        body: Mutex::new(body.to_vec()),
        calls: AtomicUsize::new(0),
        expired: AtomicBool::new(true),
    };
    let cache = IndexBodyCache::default();
    let (_cancel, rx) = tokio::sync::watch::channel(false);
    let (_stop, srx) = tokio::sync::watch::channel(false);
    let io = Io::new(rx, srx, None);
    read_index_body(&source, &cache, document(body), &io, None)
        .await
        .unwrap();
    assert_eq!(source.calls.load(Ordering::SeqCst), 2);
}

#[test]
fn by_id_lookup_only_accepts_the_requested_broadcast_channel() {
    use tl::{Deserializable, Serializable};
    // Minimal TL channel with only the broadcast flag and the mandatory fields.
    let mut wire = Vec::new();
    32u32.serialize(&mut wire);
    0u32.serialize(&mut wire);
    42i64.serialize(&mut wire);
    "fixture".to_string().serialize(&mut wire);
    tl::enums::ChatPhoto::Empty.serialize(&mut wire);
    0i32.serialize(&mut wire);
    let mut raw = tl::types::Channel::from_bytes(&wire).unwrap();
    assert_eq!(checked_channel_reply(42, vec![raw.clone().into()]).unwrap(), Some(raw.clone()));
    assert!(checked_channel_reply(43, vec![raw.clone().into()]).is_err());
    raw.min = true;
    assert!(checked_channel_reply(42, vec![raw.clone().into()]).unwrap().is_none());
    raw.broadcast = false;
    assert!(checked_channel_reply(42, vec![raw.into()]).is_err());
    assert!(checked_channel_reply(42, Vec::new()).unwrap().is_none());
    let forbidden = tl::types::ChannelForbidden {
        broadcast: true, megagroup: false, monoforum: false, id: 42, access_hash: 1,
        title: "fixture".into(), until_date: None,
    };
    assert!(checked_channel_reply(42, vec![forbidden.into()]).is_err());
}
