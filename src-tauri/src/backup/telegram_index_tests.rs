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
            read_index_body(&source, &cache, expected.clone(), &io)
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
    read_index_body(&source, &cache, replaced, &io)
        .await
        .unwrap();
    let mut edited = expected;
    let mut envelope: Envelope = serde_json::from_str(&edited.caption).unwrap();
    envelope.operation_id = uuid::Uuid::new_v4().to_string();
    edited.caption = envelope.caption().unwrap();
    read_index_body(&source, &cache, edited, &io).await.unwrap();
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
        read_index_body(&source, &cache, expected.clone(), &io)
            .await
            .unwrap_err()
            .to_string(),
        "TELEGRAM_INDEX_INVALID"
    );
    *source.body.lock().unwrap() = good.to_vec();
    assert_eq!(
        read_index_body(&source, &cache, expected.clone(), &io)
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
    assert!(read_index_body(&source, &cache, expected.clone(), &io)
        .await
        .is_err());
    assert_eq!(source.calls.load(Ordering::SeqCst), 2);
    cancel.send(false).unwrap();
    read_index_body(&source, &IndexBodyCache::default(), expected, &io)
        .await
        .unwrap();
    assert_eq!(
        source.calls.load(Ordering::SeqCst),
        3,
        "a new request must verify its own index bodies"
    );
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
    read_index_body(&source, &cache, document(body), &io)
        .await
        .unwrap();
    assert_eq!(source.calls.load(Ordering::SeqCst), 2);
}
