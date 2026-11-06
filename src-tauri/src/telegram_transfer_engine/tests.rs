use super::*;
use grammers_client::tl::Serializable;
use tokio::io::AsyncReadExt;

#[test]
fn download_response_fits_grammers_011_server_message_limit() {
    let file = tl::types::upload::File {
        r#type: tl::types::storage::FileUnknown {}.into(),
        mtime: 0,
        bytes: vec![1; DOWNLOAD_PART as usize],
    };
    let rpc_result_overhead = 12;
    assert!(
        file.to_bytes().len() + rpc_result_overhead < 1_044_456 - 8,
        "grammers-mtproto 0.11 Message::deserialize rejects a 1 MiB payload plus TL/RPC headers"
    );
}

#[test]
fn plans_align_downloads_and_bound_workers_and_last_part() {
    for size in [
        1,
        1_048_575,
        1_048_576,
        5_242_880,
        10_485_760,
        10_485_761,
        100 * DOWNLOAD_PART,
        500 * DOWNLOAD_PART + 17,
    ] {
        for concurrency in [1, 2, 4, 8, 16] {
            let plan = Plan::new(size, DOWNLOAD_PART, concurrency).unwrap();
            assert!(plan.workers <= concurrency && plan.workers <= plan.parts as usize);
            let mut total = 0;
            for index in 0..plan.parts {
                let chunk = plan.chunk(index);
                assert_eq!(chunk.offset % 4096, 0);
                let request_end = chunk.offset + DOWNLOAD_PART - 1;
                assert_eq!(chunk.offset / 1_048_576, request_end / 1_048_576);
                total += chunk.length as u64;
            }
            assert_eq!(total, size);
        }
    }
    assert_eq!(Plan::new(1_048_575, DOWNLOAD_PART, 16).unwrap().workers, 1);
    assert_eq!(
        Plan::new(4 * DOWNLOAD_PART, DOWNLOAD_PART, 16)
            .unwrap()
            .workers,
        2
    );
    assert_eq!(
        Plan::new(10 * DOWNLOAD_PART, DOWNLOAD_PART, 16)
            .unwrap()
            .workers,
        4
    );
    assert_eq!(
        Plan::new(30 * 1_048_576, DOWNLOAD_PART, 16)
            .unwrap()
            .workers,
        8
    );
    assert!(Plan::new(0, DOWNLOAD_PART, 4).is_err());
    assert!(Plan::new(u64::MAX, DOWNLOAD_PART, 4).is_err());
    assert!(Plan::new(1, DOWNLOAD_PART, 17).is_err());
}

struct MockDownload {
    calls: AtomicUsize,
    active: AtomicUsize,
    peak: AtomicUsize,
    bad: Option<usize>,
}
impl ChunkSource for MockDownload {
    fn fetch(&self, chunk: Chunk) -> Work<'_, Vec<u8>> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::Relaxed);
            let active = self.active.fetch_add(1, Ordering::Relaxed) + 1;
            self.peak.fetch_max(active, Ordering::Relaxed);
            struct Guard<'a>(&'a AtomicUsize);
            impl Drop for Guard<'_> {
                fn drop(&mut self) {
                    self.0.fetch_sub(1, Ordering::Relaxed);
                }
            }
            let _guard = Guard(&self.active);
            tokio::time::sleep(Duration::from_millis(if chunk.index == 0 { 25 } else { 1 })).await;
            Ok(vec![
                chunk.index as u8;
                if self.bad == Some(chunk.index as usize) {
                    chunk.length.saturating_sub(1)
                } else {
                    chunk.length
                }
            ])
        })
    }
}
fn mock(bad: Option<usize>) -> Arc<MockDownload> {
    Arc::new(MockDownload {
        calls: AtomicUsize::new(0),
        active: AtomicUsize::new(0),
        peak: AtomicUsize::new(0),
        bad,
    })
}

struct HeldHead(tokio::sync::Semaphore);
impl ChunkSource for HeldHead {
    fn fetch(&self, chunk: Chunk) -> Work<'_, Vec<u8>> {
        Box::pin(async move {
            if chunk.index == 0 {
                self.0.acquire().await.unwrap().forget();
            }
            Ok(vec![chunk.index as u8; chunk.length])
        })
    }
}

#[tokio::test]
async fn file_sink_keeps_working_while_the_first_chunk_is_waiting() {
    let source = Arc::new(HeldHead(tokio::sync::Semaphore::new(0)));
    let plan = Plan::new(16 * DOWNLOAD_PART + 17, DOWNLOAD_PART, 4).unwrap();
    let mut file = FileDownload::new(source.clone(), plan);
    let mut seen = HashSet::new();
    for _ in 1..plan.parts {
        let (chunk, bytes) = tokio::time::timeout(Duration::from_secs(1), file.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_ne!(chunk.index, 0);
        assert!(seen.insert(chunk.index));
        assert_eq!(bytes, vec![chunk.index as u8; chunk.length]);
        assert!(file.pending.len() < plan.workers && file.active.len() < plan.workers);
    }
    assert_eq!(file.active, HashSet::from([0]));
    source.0.add_permits(1);
    assert_eq!(file.next().await.unwrap().unwrap().0.index, 0);
    assert!(file.next().await.unwrap().is_none());
}

#[tokio::test]
async fn file_sink_rejects_bad_and_replayed_chunks_and_reclaims_requests() {
    let source = mock(Some(1));
    let plan = Plan::new(30 * DOWNLOAD_PART, DOWNLOAD_PART, 4).unwrap();
    let mut file = FileDownload::new(source.clone(), plan);
    assert_eq!(
        file.next().await.unwrap_err().to_string(),
        "INCOMPLETE_MEDIA"
    );
    assert!(file.pending.is_empty() && file.active.is_empty());
    assert_eq!(source.active.load(Ordering::Relaxed), 0);
    assert!(file.next().await.is_err());
    let mut file = FileDownload::new(Arc::new(HeldHead(tokio::sync::Semaphore::new(0))), plan);
    let (chunk, _) = file.next().await.unwrap().unwrap();
    // A stale completion cannot consume another request's slot or count twice.
    file.pending
        .push(Box::pin(async move { Ok((chunk, vec![1; chunk.length])) }));
    loop {
        if let Err(error) = file.next().await {
            assert_eq!(error.to_string(), "INCOMPLETE_MEDIA");
            break;
        }
    }
    assert!(file.pending.is_empty() && file.active.is_empty());
    let source = mock(None);
    let mut file = FileDownload::new(source.clone(), plan);
    file.next().await.unwrap().unwrap();
    assert!(source.active.load(Ordering::Relaxed) > 0);
    drop(file);
    assert_eq!(source.active.load(Ordering::Relaxed), 0, "cancel drops outstanding requests");
    let mut missing = FileDownload::new(source, plan);
    missing.scheduled = plan.parts;
    assert_eq!(missing.next().await.unwrap_err().to_string(), "INCOMPLETE_MEDIA");
}

#[tokio::test]
async fn out_of_order_download_is_contiguous_and_backpressured() {
    let source = mock(None);
    let plan = Plan::new(16 * DOWNLOAD_PART + 17, DOWNLOAD_PART, 4).unwrap();
    let mut download = Download::new(source.clone(), plan);
    let first = download.next().await.unwrap().unwrap();
    assert_eq!(first, vec![0; DOWNLOAD_PART as usize]);
    assert_eq!(source.calls.load(Ordering::Relaxed), 4);
    assert_eq!(source.peak.load(Ordering::Relaxed), 4);
    tokio::time::sleep(Duration::from_millis(10)).await;
    assert_eq!(source.calls.load(Ordering::Relaxed), 4);
    let mut reader = Reader::new(download);
    let mut bytes = vec![];
    reader.read_to_end(&mut bytes).await.unwrap();
    assert_eq!(bytes.len(), 15 * DOWNLOAD_PART as usize + 17);
    for index in 1..17 {
        let offset = (index - 1) * DOWNLOAD_PART as usize;
        assert!(
            bytes[offset..(offset + DOWNLOAD_PART as usize).min(bytes.len())]
                .iter()
                .all(|v| *v == index as u8)
        );
    }
}

#[tokio::test]
async fn missing_or_short_chunk_fails_without_new_work() {
    let source = mock(Some(1));
    let mut download = Download::new(
        source.clone(),
        Plan::new(30 * DOWNLOAD_PART, DOWNLOAD_PART, 4).unwrap(),
    );
    assert_eq!(
        download.next().await.unwrap_err().to_string(),
        "INCOMPLETE_MEDIA"
    );
    assert_eq!(source.active.load(Ordering::Relaxed), 0);
    assert_eq!(source.calls.load(Ordering::Relaxed), 4);
    assert!(download.next().await.is_err());
}

#[tokio::test]
async fn duplicate_or_missing_completions_never_emit_duplicate_or_partial_success() {
    for emitted in [0, 1] {
        let plan = Plan::new(2 * DOWNLOAD_PART, DOWNLOAD_PART, 2).unwrap();
        let mut download = Download::new(mock(None), plan);
        download.scheduled = plan.parts;
        download.emitted = emitted;
        let chunk = plan.chunk(1 - emitted);
        // Duplicate an out-of-order completion, or replay a previously emitted chunk.
        for _ in 0..2 {
            download.pending.push(Box::pin(async move { Ok((chunk, vec![7; chunk.length])) }));
        }
        assert_eq!(download.next().await.unwrap_err().to_string(), "INCOMPLETE_MEDIA");
        assert_eq!(download.emitted, emitted);
        assert!(download.pending.is_empty() && download.ready.is_empty());
    }
    let plan = Plan::new(2 * DOWNLOAD_PART, DOWNLOAD_PART, 2).unwrap();
    let mut missing = Download::new(mock(None), plan);
    missing.scheduled = plan.parts;
    assert_eq!(missing.next().await.unwrap_err().to_string(), "INCOMPLETE_MEDIA");
}

#[tokio::test]
async fn dropping_read_cancels_all_chunk_futures() {
    let source = mock(None);
    let mut reader = Reader::new(Download::new(
        source.clone(),
        Plan::new(30 * DOWNLOAD_PART, DOWNLOAD_PART, 4).unwrap(),
    ));
    let mut bytes = [0; 1];
    assert!(
        tokio::time::timeout(Duration::from_millis(5), reader.read(&mut bytes))
            .await
            .is_err()
    );
    assert!(source.active.load(Ordering::Relaxed) > 0);
    drop(reader);
    assert_eq!(source.active.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn account_budget_is_shared_and_limits_all_dcs() {
    let id = uuid::Uuid::new_v4().to_string();
    let budget = Budget::account(&id);
    let other = Budget::account(&id);
    assert!(Arc::ptr_eq(&budget, &other));
    let mut permits = vec![];
    for _ in 0..GLOBAL_REQUESTS {
        permits.push(budget.acquire().await.unwrap());
    }
    assert!(
        tokio::time::timeout(Duration::from_millis(5), other.acquire())
            .await
            .is_err()
    );
    drop(permits);
    let _permit = other.acquire().await.unwrap();
    budget.flood(60);
    assert!(other
        .acquire()
        .await
        .unwrap_err()
        .to_string()
        .starts_with("TELEGRAM_FLOOD_WAIT:"));
}

#[tokio::test]
async fn flood_cooldown_survives_the_last_transfer_lease() {
    let id = uuid::Uuid::new_v4().to_string();
    let budget = Budget::account(&id);
    budget.flood(60);
    drop(budget);
    assert!(Budget::account(&id)
        .acquire()
        .await
        .unwrap_err()
        .to_string()
        .starts_with("TELEGRAM_FLOOD_WAIT:"));
}

#[test]
fn part_reports_preserve_independent_state_without_unbounded_history() {
    let stats = Stats::default();
    for index in 0..100 {
        let chunk = Chunk {
            index,
            offset: index * DOWNLOAD_PART,
            length: 17,
        };
        let mut meter = PartMeter::new(&stats, chunk, DOWNLOAD_PART as usize);
        meter.finish(Ok(())).unwrap();
    }
    let reports = stats.recent_parts();
    assert_eq!(reports.len(), 64);
    assert_eq!(reports[0].index, 36);
    assert_eq!(reports[0].offset, 36 * DOWNLOAD_PART);
    assert_eq!(reports[0].length, 17);
    assert_eq!(reports[0].request_length, DOWNLOAD_PART as usize);
    assert!(reports.iter().all(|report| report.status == "complete"));
    let chunk = Chunk {
        index: 100,
        offset: 100 * DOWNLOAD_PART,
        length: 17,
    };
    drop(PartMeter::new(&stats, chunk, DOWNLOAD_PART as usize));
    assert_eq!(stats.recent_parts().last().unwrap().status, "canceled");
    let mut meter = PartMeter::new(&stats, chunk, DOWNLOAD_PART as usize);
    assert!(meter
        .finish::<()>(Err(anyhow::anyhow!("INCOMPLETE_MEDIA")))
        .is_err());
    drop(meter);
    let report = stats.recent_parts().pop().unwrap();
    assert_eq!(report.status, "failed");
    assert_eq!(report.last_error.as_deref(), Some("INCOMPLETE_MEDIA"));
}

#[tokio::test]
async fn canceled_request_retains_elapsed_time_and_releases_budget() {
    let stats = Stats::default();
    let budget = Budget::account(&uuid::Uuid::new_v4().to_string());
    let chunk = Chunk {
        index: 7,
        offset: 7 * DOWNLOAD_PART,
        length: DOWNLOAD_PART as usize,
    };
    let meter = PartMeter::new(&stats, chunk, DOWNLOAD_PART as usize);
    let result = tokio::time::timeout(
        Duration::from_millis(15),
        rpc(&budget, &stats, Route::File, Some(&meter), || {
            Box::pin(std::future::pending::<
                std::result::Result<(), InvocationError>,
            >())
        }),
    )
    .await;
    assert!(result.is_err());
    assert_eq!(stats.active.load(Ordering::Relaxed), 0);
    assert_eq!(budget.requests.available_permits(), GLOBAL_REQUESTS);
    drop(meter);
    let report = stats.recent_parts().pop().unwrap();
    assert_eq!(report.status, "canceled");
    assert_eq!(report.attempts, 1);
    assert!(report.rpc_ms >= 1);
    assert!(report.elapsed_ms >= report.rpc_ms);
}

#[tokio::test]
async fn covers_do_not_bypass_file_budget_or_server_cooldown() {
    let budget = Budget::account(&uuid::Uuid::new_v4().to_string());
    let polled = AtomicUsize::new(0);
    let mut permits = vec![];
    for _ in 0..GLOBAL_REQUESTS {
        permits.push(budget.acquire().await.unwrap());
    }
    let result = tokio::time::timeout(
        Duration::from_millis(5),
        cover_chunk(&budget, async {
            polled.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }),
    )
    .await;
    assert!(result.is_err());
    assert_eq!(polled.load(Ordering::Relaxed), 0);
    drop(permits);
    let error = cover_chunk::<()>(&budget, async { Err(protocol_error(420, "FLOOD_WAIT_30")) })
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), "TELEGRAM_FLOOD_WAIT:30");
    let error = cover_chunk(&budget, async {
        polled.fetch_add(1, Ordering::Relaxed);
        Ok(())
    })
    .await
    .unwrap_err();
    assert!(error.to_string().starts_with("TELEGRAM_FLOOD_WAIT:"));
    assert_eq!(polled.load(Ordering::Relaxed), 0);
}

fn protocol_error(code: i32, message: &str) -> InvocationError {
    InvocationError::Rpc(
        tl::types::RpcError {
            error_code: code,
            error_message: message.into(),
        }
        .into(),
    )
}
fn document() -> tl::types::Document {
    tl::types::Document {
        id: 123,
        access_hash: 456,
        file_reference: vec![1],
        date: 0,
        mime_type: "video/mp4".into(),
        size: 2 * DOWNLOAD_PART as i64 + 17,
        thumbs: None,
        video_thumbs: None,
        dc_id: 2,
        attributes: vec![],
    }
}
enum Reply {
    Error(i32, &'static str),
    Disconnect,
    Cdn,
    Short,
}
#[derive(Default)]
struct FileMock {
    replies: Mutex<std::collections::VecDeque<Reply>>,
    calls: Mutex<Vec<(i32, i64, Vec<u8>)>>,
    authorizations: AtomicUsize,
    home: i32,
    fail_auth: bool,
    auth_barrier: Option<Arc<tokio::sync::Barrier>>,
    authorized_dcs: Option<Mutex<HashSet<i32>>>,
    auth_started: Option<Arc<Semaphore>>,
    auth_release: Option<Arc<Semaphore>>,
}
impl FileClient for FileMock {
    fn get<'a>(
        &'a self,
        dc: i32,
        request: &'a tl::functions::upload::GetFile,
    ) -> RpcWork<'a, tl::enums::upload::File> {
        Box::pin(async move {
            let tl::enums::InputFileLocation::InputDocumentFileLocation(location) =
                &request.location
            else {
                panic!("original document only");
            };
            self.calls
                .lock()
                .unwrap()
                .push((dc, request.offset, location.file_reference.clone()));
            assert!(!request.cdn_supported);
            assert_eq!(request.limit, DOWNLOAD_PART as i32);
            let unauthorized = self.authorized_dcs.as_ref().is_some_and(|dcs| !dcs.lock().unwrap().contains(&dc));
            let reply = if unauthorized { Some(Reply::Error(401, "AUTH_KEY_UNREGISTERED")) }
                else { self.replies.lock().unwrap().pop_front() };
            match reply {
                Some(Reply::Error(code, message)) => {
                    if message == "AUTH_KEY_UNREGISTERED" {
                        if let Some(barrier) = &self.auth_barrier { barrier.wait().await; }
                    }
                    Err(protocol_error(code, message))
                },
                Some(Reply::Disconnect) => Err(InvocationError::Io(std::io::Error::from(
                    std::io::ErrorKind::ConnectionReset,
                ))),
                Some(Reply::Cdn) => Ok(tl::types::upload::FileCdnRedirect {
                    dc_id: 3,
                    file_token: vec![],
                    encryption_key: vec![],
                    encryption_iv: vec![],
                    file_hashes: vec![],
                }
                .into()),
                _ => {
                    let length = (document().size as u64 - request.offset as u64).min(DOWNLOAD_PART)
                        as usize;
                    Ok(tl::types::upload::File {
                        r#type: tl::types::storage::FileUnknown {}.into(),
                        mtime: 0,
                        bytes: vec![
                            (request.offset as u64 / DOWNLOAD_PART) as u8;
                            if matches!(reply, Some(Reply::Short)) {
                                length - 1
                            } else {
                                length
                            }
                        ],
                    }
                    .into())
                }
            }
        })
    }
    fn home_dc(&self) -> RpcWork<'_, i32> {
        Box::pin(async { Ok(self.home) })
    }
    fn export(&self, dc: i32) -> RpcWork<'_, tl::types::auth::ExportedAuthorization> {
        assert_ne!(dc, self.home);
        Box::pin(async {
            Ok(tl::types::auth::ExportedAuthorization {
                id: 1,
                bytes: vec![1],
            })
        })
    }
    fn import<'a>(
        &'a self,
        dc: i32,
        _exported: &'a tl::types::auth::ExportedAuthorization,
    ) -> RpcWork<'a, ()> {
        Box::pin(async move {
            self.authorizations.fetch_add(1, Ordering::Relaxed);
            if dc == 2 {
                if let Some(started) = &self.auth_started { started.add_permits(1); }
                if let Some(release) = &self.auth_release { release.acquire().await.unwrap().forget(); }
            }
            if self.fail_auth {
                Err(protocol_error(401, "AUTH_KEY_UNREGISTERED"))
            } else {
                if let Some(dcs) = &self.authorized_dcs { dcs.lock().unwrap().insert(dc); }
                Ok(())
            }
        })
    }
}
fn adapter(mock: Arc<FileMock>, fresh: tl::types::Document) -> DocumentSource {
    DocumentSource {
        authorization: Arc::default(),
        client: mock,
        document: tokio::sync::Mutex::new((document(), 0)),
        refresh: Arc::new(move || {
            let fresh = fresh.clone();
            Box::pin(async move { Ok(fresh) })
        }),
        budget: Budget::account(&uuid::Uuid::new_v4().to_string()),
        stats: Arc::default(),
    }
}
fn file_mock(replies: Vec<Reply>, home: i32, fail_auth: bool) -> Arc<FileMock> {
    Arc::new(FileMock {
        replies: Mutex::new(replies.into()),
        home,
        fail_auth,
        ..Default::default()
    })
}
#[tokio::test]
async fn native_adapter_refreshes_only_failed_part_and_rejects_changed_identity() {
    let mock = file_mock(vec![], 1, false);
    let mut fresh = document();
    fresh.file_reference = vec![2];
    let source = adapter(mock.clone(), fresh.clone());
    let plan = Plan::new(document().size as u64, DOWNLOAD_PART, 1).unwrap();
    assert_eq!(
        source.fetch(plan.chunk(0)).await.unwrap().len(),
        DOWNLOAD_PART as usize
    );
    mock.replies
        .lock()
        .unwrap()
        .push_back(Reply::Error(400, "FILE_REFERENCE_EXPIRED"));
    source.fetch(plan.chunk(1)).await.unwrap();
    let calls = mock.calls.lock().unwrap().clone();
    assert_eq!(
        calls.iter().map(|c| c.1).collect::<Vec<_>>(),
        vec![0, DOWNLOAD_PART as i64, DOWNLOAD_PART as i64]
    );
    assert_eq!(calls[2].2, vec![2]);
    let mock = file_mock(vec![Reply::Error(400, "FILE_REFERENCE_EXPIRED")], 1, false);
    fresh.id += 1;
    assert_eq!(
        adapter(mock.clone(), fresh)
            .fetch(plan.chunk(0))
            .await
            .unwrap_err()
            .to_string(),
        "TELEGRAM_MEDIA_CHANGED"
    );
    assert_eq!(mock.calls.lock().unwrap().len(), 1);
}
#[tokio::test]
async fn native_adapter_bounds_reference_refresh_and_never_restarts_the_stream() {
    let mock = file_mock(
        vec![
            Reply::Error(400, "FILE_REFERENCE_EXPIRED"),
            Reply::Error(400, "FILE_REFERENCE_EXPIRED"),
            Reply::Error(400, "FILE_REFERENCE_EXPIRED"),
        ],
        1,
        false,
    );
    let source = adapter(mock.clone(), document());
    let chunk = Plan::new(document().size as u64, DOWNLOAD_PART, 1)
        .unwrap()
        .chunk(1);
    assert_eq!(
        source.fetch(chunk).await.unwrap_err().to_string(),
        "TELEGRAM_RPC_ERROR"
    );
    assert!(mock
        .calls
        .lock()
        .unwrap()
        .iter()
        .all(|c| c.1 == DOWNLOAD_PART as i64));
}
#[tokio::test]
async fn native_adapter_routes_migration_and_imports_only_non_home_authorization() {
    let chunk = Plan::new(document().size as u64, DOWNLOAD_PART, 1)
        .unwrap()
        .chunk(0);
    let mock = file_mock(vec![Reply::Error(303, "FILE_MIGRATE_4")], 1, false);
    adapter(mock.clone(), document())
        .fetch(chunk)
        .await
        .unwrap();
    assert_eq!(
        mock.calls
            .lock()
            .unwrap()
            .iter()
            .map(|c| c.0)
            .collect::<Vec<_>>(),
        vec![2, 4]
    );
    for (home, fail_auth, repeated, success, imports) in [
        (1, false, false, true, 1),
        (2, false, false, false, 0),
        (1, true, false, false, 1),
        (1, false, true, false, 1),
    ] {
        let mut replies = vec![Reply::Error(401, "AUTH_KEY_UNREGISTERED")];
        if repeated {
            replies.push(Reply::Error(401, "AUTH_KEY_UNREGISTERED"));
        }
        let mock = file_mock(replies, home, fail_auth);
        let result = adapter(mock.clone(), document()).fetch(chunk).await;
        if success {
            assert!(result.is_ok());
        } else {
            assert_eq!(result.unwrap_err().to_string(), "TELEGRAM_AUTH_REQUIRED");
        }
        assert_eq!(mock.authorizations.load(Ordering::Relaxed), imports);
    }
}
#[tokio::test]
async fn native_adapter_rejects_cdn_and_malformed_length_without_retrying() {
    let chunk = Plan::new(document().size as u64, DOWNLOAD_PART, 1)
        .unwrap()
        .chunk(0);
    for (reply, expected) in [
        (Reply::Cdn, "TELEGRAM_CDN_UNSUPPORTED"),
        (Reply::Short, "INCOMPLETE_MEDIA"),
        (
            Reply::Error(403, "CHANNEL_PRIVATE"),
            "TELEGRAM_CHANNEL_FORBIDDEN",
        ),
    ] {
        let mock = file_mock(vec![reply], 1, false);
        let source = adapter(mock.clone(), document());
        assert_eq!(source.fetch(chunk).await.unwrap_err().to_string(), expected);
        assert_eq!(mock.calls.lock().unwrap().len(), 1);
        assert_eq!(source.stats.acknowledged.load(Ordering::Relaxed), 0);
    }
}
#[tokio::test]
async fn native_adapter_retries_disconnect_and_server_failure_but_not_flood_wait() {
    let chunk = Plan::new(document().size as u64, DOWNLOAD_PART, 1)
        .unwrap()
        .chunk(0);
    let mock = file_mock(
        vec![Reply::Disconnect, Reply::Error(500, "INTERNAL")],
        1,
        false,
    );
    let source = adapter(mock.clone(), document());
    source.fetch(chunk).await.unwrap();
    assert_eq!(mock.calls.lock().unwrap().len(), 3);
    assert_eq!(source.stats.retries.load(Ordering::Relaxed), 2);
    let reports = source.stats.recent_parts();
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].attempts, 3);
    assert_eq!(reports[0].retries, 2);
    assert!(reports[0].elapsed_ms >= 600);
    assert_eq!(reports[0].status, "complete");
    assert_eq!(reports[0].last_error.as_deref(), Some("TELEGRAM_RPC_ERROR"));
    for name in ["FLOOD_WAIT_30", "FLOOD_PREMIUM_WAIT_30"] {
        let mock = file_mock(vec![Reply::Error(420, name)], 1, false);
        let source = adapter(mock.clone(), document());
        assert_eq!(
            source.fetch(chunk).await.unwrap_err().to_string(),
            "TELEGRAM_FLOOD_WAIT:30"
        );
        assert!(source
            .fetch(chunk)
            .await
            .unwrap_err()
            .to_string()
            .starts_with("TELEGRAM_FLOOD_WAIT:"));
        assert_eq!(mock.calls.lock().unwrap().len(), 1);
    }
}

#[tokio::test]
async fn timeout_reduces_shared_actual_rpc_concurrency_including_existing_waiters() {
    let budget = Budget::account(&uuid::Uuid::new_v4().to_string());
    let other = Budget::account(&uuid::Uuid::new_v4().to_string());
    let mut held = vec![];
    for _ in 0..GLOBAL_REQUESTS { held.push(budget.acquire().await.unwrap()); }
    let queued = budget.acquire(); tokio::pin!(queued);
    assert!(tokio::time::timeout(Duration::from_millis(10), &mut queued).await.is_err());
    budget.failed(held[0].generation, true);
    drop(held);
    let first = queued.await.unwrap();
    let mut held = vec![first];
    for _ in 1..GLOBAL_REQUESTS/2 { held.push(budget.acquire().await.unwrap()); }
    assert!(tokio::time::timeout(Duration::from_millis(10), budget.acquire()).await.is_err());
    assert!(tokio::time::timeout(Duration::from_millis(10), other.acquire()).await.is_ok());
    drop(held);
    let stats = Stats::default(); let calls = AtomicUsize::new(0);
    rpc(&budget, &stats, Route::File, None, || Box::pin(async {
        if calls.fetch_add(1, Ordering::Relaxed) == 0 {
            Err(InvocationError::Io(std::io::ErrorKind::TimedOut.into()))
        } else { Ok(()) }
    })).await.unwrap();
    let mut held = vec![];
    for _ in 0..GLOBAL_REQUESTS/4 { held.push(budget.acquire().await.unwrap()); }
    assert!(tokio::time::timeout(Duration::from_millis(10), budget.acquire()).await.is_err());
    assert_eq!(stats.retries.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn concurrent_dc_auth_failures_share_one_export_import() {
    let mock = Arc::new(FileMock {
        replies: Mutex::new((0..8).map(|_| Reply::Error(401, "AUTH_KEY_UNREGISTERED")).collect()),
        home: 1, auth_barrier: Some(Arc::new(tokio::sync::Barrier::new(8))), ..Default::default()
    });
    let source = Arc::new(adapter(mock.clone(), document()));
    let mut tasks = tokio::task::JoinSet::new();
    for _ in 0..8 {
        let source = source.clone();
        tasks.spawn(async move { source.fetch(Chunk { index:0,offset:0,length:DOWNLOAD_PART as usize }).await });
    }
    tokio::time::timeout(Duration::from_secs(3), async {
        while let Some(result) = tasks.join_next().await { assert_eq!(result.unwrap().unwrap().len(), DOWNLOAD_PART as usize); }
    }).await.unwrap();
    assert_eq!(mock.authorizations.load(Ordering::Relaxed), 1);
}


#[tokio::test]
async fn broadcast_timeout_reduces_one_generation_only_and_recovers_across_files() {
    let id = uuid::Uuid::new_v4().to_string();
    let budget = Budget::account(&id);
    let barrier = Arc::new(tokio::sync::Barrier::new(4));
    let mut tasks = tokio::task::JoinSet::new();
    for _ in 0..4 {
        let budget = budget.clone(); let barrier = barrier.clone();
        tasks.spawn(async move {
            let stats = Stats::default(); let mut calls = 0;
            rpc(&budget, &stats, Route::File, None, || {
                calls += 1; let first = calls == 1; let barrier = barrier.clone();
                Box::pin(async move {
                    if first {
                        barrier.wait().await;
                        Err(InvocationError::Io(std::io::ErrorKind::TimedOut.into()))
                    } else { Ok(()) }
                })
            }).await
        });
    }
    tokio::time::timeout(Duration::from_secs(3), async {
        while let Some(result) = tasks.join_next().await { result.unwrap().unwrap(); }
    }).await.unwrap();
    assert_eq!(budget.window.lock().unwrap().weight, 2);
    // Later independent requests belong to new generations and can reduce again.
    for weight in [4, 8] {
        assert!(cover_chunk::<()>(&budget, async {
            Err(InvocationError::Io(std::io::ErrorKind::TimedOut.into()))
        }).await.is_err());
        assert_eq!(budget.window.lock().unwrap().weight, weight);
    }
    let next_file = Budget::account(&id);
    assert!(Arc::ptr_eq(&budget, &next_file));
    // Hold an old result through a complete down/up cycle to detect generation ABA.
    let stale = next_file.acquire().await.unwrap();
    let stale_generation = stale.generation;
    next_file.failed(stale_generation, true);
    drop(stale);
    tokio::time::pause();
    for weight in [8, 4, 2, 1] {
        for _ in 0..31 { cover_chunk(&next_file, async { Ok(()) }).await.unwrap(); }
        assert_eq!(next_file.window.lock().unwrap().weight, weight * 2);
        tokio::time::advance(Duration::from_secs(30)).await;
        cover_chunk(&next_file, async { Ok(()) }).await.unwrap();
        assert_eq!(next_file.window.lock().unwrap().weight, weight);
        if weight == 8 {
            next_file.failed(stale_generation, true);
            assert_eq!(next_file.window.lock().unwrap().weight, 8);
        }
    }
    next_file.failed(stale_generation, true);
    assert_eq!(next_file.window.lock().unwrap().weight, 1);
    let mut held = vec![];
    for _ in 0..16 { held.push(next_file.acquire().await.unwrap()); }
    assert!(tokio::time::timeout(Duration::from_millis(1), next_file.acquire()).await.is_err());
}

#[tokio::test]
async fn independent_sessions_import_separately_and_share_only_account_budget() {
    let barrier = Arc::new(tokio::sync::Barrier::new(8));
    let budget = Budget::account(&uuid::Uuid::new_v4().to_string());
    let mut clients = vec![]; let mut tasks = tokio::task::JoinSet::new();
    for _ in 0..2 {
        let client = Arc::new(FileMock {
            home: 1, authorized_dcs: Some(Mutex::new(HashSet::new())),
            auth_barrier: Some(barrier.clone()), ..Default::default()
        });
        let authorization = Arc::new(Authorization::default());
        clients.push(client.clone());
        // Two documents per independent session must still share its authorization.
        for _ in 0..2 {
            let mut source = adapter(client.clone(), document());
            source.budget = budget.clone(); source.authorization = authorization.clone();
            let source = Arc::new(source);
            for _ in 0..2 {
                let source = source.clone();
                tasks.spawn(async move { source.fetch(Chunk {index:0, offset:0, length:DOWNLOAD_PART as usize}).await });
            }
        }
    }
    tokio::time::timeout(Duration::from_secs(3), async {
        while let Some(result) = tasks.join_next().await { assert_eq!(result.unwrap().unwrap().len(), DOWNLOAD_PART as usize); }
    }).await.unwrap();
    for client in clients {
        assert_eq!(client.authorizations.load(Ordering::Relaxed), 1);
        assert!(client.authorized_dcs.as_ref().unwrap().lock().unwrap().contains(&2));
    }
}

#[tokio::test]
async fn slow_dc_import_does_not_block_an_authorized_dc() {
    let started = Arc::new(Semaphore::new(0)); let release = Arc::new(Semaphore::new(0));
    let client = Arc::new(FileMock {
        home: 1, authorized_dcs: Some(Mutex::new(HashSet::from([3]))),
        auth_started: Some(started.clone()), auth_release: Some(release.clone()), ..Default::default()
    });
    let source = Arc::new(adapter(client.clone(), document()));
    let chunk = Chunk {index:0,offset:0,length:DOWNLOAD_PART as usize};
    let blocked = { let source = source.clone(); tokio::spawn(async move { source.fetch(chunk).await }) };
    tokio::time::timeout(Duration::from_secs(2), started.acquire()).await.unwrap().unwrap().forget();
    let mut other = adapter(client.clone(), document());
    other.authorization = source.authorization.clone(); other.budget = source.budget.clone();
    other.document.lock().await.0.dc_id = 3;
    assert_eq!(tokio::time::timeout(Duration::from_secs(1), other.fetch(chunk)).await.unwrap().unwrap().len(), DOWNLOAD_PART as usize);
    assert!(!blocked.is_finished());
    release.add_permits(1);
    assert!(tokio::time::timeout(Duration::from_secs(2), blocked).await.unwrap().unwrap().is_ok());
    assert_eq!(client.authorizations.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn recovery_requires_both_stable_time_and_consecutive_file_successes() {
    let budget = Budget::account(&uuid::Uuid::new_v4().to_string());
    tokio::time::pause();
    assert!(cover_chunk::<()>(&budget, async {
        Err(InvocationError::Io(std::io::ErrorKind::TimedOut.into()))
    }).await.is_err());
    for _ in 0..32 { cover_chunk(&budget, async { Ok(()) }).await.unwrap(); }
    assert_eq!(budget.window.lock().unwrap().weight, 2, "success burst alone must not restore concurrency");
    assert!(cover_chunk::<()>(&budget, async {
        Err(InvocationError::Io(std::io::ErrorKind::ConnectionReset.into()))
    }).await.is_err());
    tokio::time::advance(Duration::from_secs(30)).await;
    for _ in 0..31 { cover_chunk(&budget, async { Ok(()) }).await.unwrap(); }
    assert_eq!(budget.window.lock().unwrap().weight, 2, "a failure must reset the success streak");
    cover_chunk(&budget, async { Ok(()) }).await.unwrap();
    assert_eq!(budget.window.lock().unwrap().weight, 1);
}
