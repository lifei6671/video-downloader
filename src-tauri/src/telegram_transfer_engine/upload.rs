use super::*;
use tokio::io::AsyncReadExt;

const BIG_FILE: u64 = 10 * 1024 * 1024;
trait PartSink: Sync {
    fn save(&self, index: i32, bytes: Vec<u8>) -> Work<'_, ()>;
}
trait PartClient: Sync {
    fn small<'a>(&'a self, request: &'a tl::functions::upload::SaveFilePart) -> RpcWork<'a, bool>;
    fn big<'a>(&'a self, request: &'a tl::functions::upload::SaveBigFilePart) -> RpcWork<'a, bool>;
}
impl PartClient for Client {
    fn small<'a>(&'a self, request: &'a tl::functions::upload::SaveFilePart) -> RpcWork<'a, bool> {
        Box::pin(self.invoke(request))
    }
    fn big<'a>(&'a self, request: &'a tl::functions::upload::SaveBigFilePart) -> RpcWork<'a, bool> {
        Box::pin(self.invoke(request))
    }
}
struct TelegramSink<'a> {
    client: &'a dyn PartClient,
    budget: &'a Budget,
    stats: &'a Stats,
    file_id: i64,
    parts: i32,
    big: bool,
}
impl PartSink for TelegramSink<'_> {
    fn save(&self, index: i32, bytes: Vec<u8>) -> Work<'_, ()> {
        Box::pin(async move {
            let chunk = Chunk {
                index: index as u64,
                offset: index as u64 * UPLOAD_PART,
                length: bytes.len(),
            };
            let mut part = PartMeter::new(self.stats, chunk, chunk.length);
            let result = async {
                let stored = if self.big {
                    let request = tl::functions::upload::SaveBigFilePart {
                        file_id: self.file_id,
                        file_part: index,
                        file_total_parts: self.parts,
                        bytes,
                    };
                    rpc(self.budget, self.stats, Route::Main, Some(&part), || {
                        self.client.big(&request)
                    })
                    .await?
                } else {
                    let request = tl::functions::upload::SaveFilePart {
                        file_id: self.file_id,
                        file_part: index,
                        bytes,
                    };
                    rpc(self.budget, self.stats, Route::Main, Some(&part), || {
                        self.client.small(&request)
                    })
                    .await?
                };
                ensure!(stored, "TELEGRAM_UPLOAD_PART_REJECTED");
                Ok(())
            }
            .await;
            part.finish(result)
        })
    }
}

struct UploadFile {
    id: i64,
    parts: i32,
    name: String,
    big: bool,
}
impl UploadFile {
    fn new(bytes: u64, limit: u64, name: String, parts: u64) -> Result<Self> {
        ensure!(bytes <= limit, "TELEGRAM_FILE_TOO_LARGE");
        let parts = i32::try_from(parts).context("TELEGRAM_FILE_TOO_LARGE")?;
        let id = i64::from_le_bytes(
            uuid::Uuid::new_v4().as_bytes()[..8]
                .try_into()
                .expect("uuid prefix"),
        );
        Ok(Self {
            id,
            parts,
            name,
            big: bytes > BIG_FILE,
        })
    }
    fn finish(self, md5: md5::Context) -> tl::enums::InputFile {
        if self.big {
            tl::types::InputFileBig {
                id: self.id,
                parts: self.parts,
                name: self.name,
            }
            .into()
        } else {
            tl::types::InputFile {
                id: self.id,
                parts: self.parts,
                name: self.name,
                md5_checksum: format!("{:x}", md5.finalize()),
            }
            .into()
        }
    }
}

fn confirm(result: Result<Chunk>, stats: &Stats, progress: &(dyn Fn(u64) + Sync)) -> Result<()> {
    let chunk = result?;
    let confirmed = stats
        .acknowledged
        .fetch_add(chunk.length as u64, Ordering::Relaxed)
        + chunk.length as u64;
    progress(confirmed);
    Ok(())
}

/// Read/hash in source order, retain at most `workers` part buffers, acknowledge once per part.
async fn pump(
    reader: &mut (dyn AsyncRead + Unpin + Send),
    sink: &impl PartSink,
    plan: Plan,
    stats: &Stats,
    progress: &(dyn Fn(u64) + Sync),
) -> Result<md5::Context> {
    let mut pending: FuturesUnordered<Work<'_, Chunk>> = FuturesUnordered::new();
    let mut md5 = md5::Context::new();
    for index in 0..plan.parts {
        while pending.len() >= plan.workers {
            confirm(
                pending.next().await.context("INCOMPLETE_MEDIA")?,
                stats,
                progress,
            )?;
        }
        let chunk = plan.chunk(index);
        let mut bytes = vec![0; chunk.length];
        {
            let read = reader.read_exact(&mut bytes);
            tokio::pin!(read);
            loop {
                tokio::select! { biased;
                    Some(result) = pending.next(), if !pending.is_empty() => confirm(result, stats, progress)?,
                    result = &mut read => { result.context("BACKUP_SOURCE_CHANGED")?; break; }
                }
            }
        }
        stats
            .read_bytes
            .fetch_add(chunk.length as u64, Ordering::Relaxed);
        if plan.bytes <= BIG_FILE {
            md5.consume(&bytes);
        }
        pending.push(Box::pin(async move {
            sink.save(chunk.index as i32, bytes).await?;
            Ok(chunk)
        }));
    }
    while let Some(result) = pending.next().await {
        confirm(result, stats, progress)?;
    }
    let mut tail = [0; 1];
    ensure!(
        reader
            .read(&mut tail)
            .await
            .context("BACKUP_SOURCE_CHANGED")?
            == 0,
        "BACKUP_SOURCE_CHANGED"
    );
    Ok(md5)
}

pub(crate) async fn upload(
    client: &Client,
    budget: &Budget,
    reader: &mut (dyn AsyncRead + Unpin + Send),
    bytes: u64,
    limit: u64,
    name: String,
    progress: &(dyn Fn(u64) + Sync),
) -> Result<grammers_client::media::Uploaded> {
    let settings = settings();
    let plan = Plan::new(bytes, UPLOAD_PART, settings.concurrency)?;
    let file = UploadFile::new(bytes, limit, name, plan.parts)?;
    let stats = settings.stats;
    progress(0);
    let sink = TelegramSink {
        client,
        budget,
        stats: &stats,
        file_id: file.id,
        parts: file.parts,
        big: file.big,
    };
    let md5 = pump(reader, &sink, plan, &stats, progress).await?;
    Ok(grammers_client::media::Uploaded::from_raw(file.finish(md5)))
}

#[cfg(test)]
mod tests {
    use super::*;
    type RecordedPart = (i64, i32, Option<i32>, Vec<u8>);
    struct RpcMock {
        calls: Mutex<Vec<RecordedPart>>,
        fail_first: bool,
        reject: bool,
    }
    impl RpcMock {
        fn call(&self, id: i64, index: i32, total: Option<i32>, bytes: &[u8]) -> RpcWork<'_, bool> {
            let mut calls = self.calls.lock().unwrap();
            calls.push((id, index, total, bytes.to_vec()));
            let fail = self.fail_first && calls.len() == 1;
            Box::pin(async move {
                if fail {
                    Err(InvocationError::Io(std::io::Error::from(
                        std::io::ErrorKind::ConnectionReset,
                    )))
                } else {
                    Ok(!self.reject)
                }
            })
        }
    }
    impl PartClient for RpcMock {
        fn small<'a>(&'a self, r: &'a tl::functions::upload::SaveFilePart) -> RpcWork<'a, bool> {
            self.call(r.file_id, r.file_part, None, &r.bytes)
        }
        fn big<'a>(&'a self, r: &'a tl::functions::upload::SaveBigFilePart) -> RpcWork<'a, bool> {
            self.call(r.file_id, r.file_part, Some(r.file_total_parts), &r.bytes)
        }
    }
    #[tokio::test]
    async fn native_upload_retries_identical_part_and_false_never_acknowledges() {
        for big in [false, true] {
            for reject in [false, true] {
                let mock = RpcMock {
                    calls: Mutex::default(),
                    fail_first: !reject,
                    reject,
                };
                let budget = Budget::account(&uuid::Uuid::new_v4().to_string());
                let stats = Stats::default();
                let sink = TelegramSink {
                    client: &mock,
                    budget: &budget,
                    stats: &stats,
                    file_id: 123,
                    parts: 21,
                    big,
                };
                let result = sink.save(20, vec![7; 17]).await;
                assert_eq!(result.is_ok(), !reject);
                let report = stats.recent_parts().pop().unwrap();
                assert_eq!(report.index, 20);
                assert_eq!(report.offset, 20 * UPLOAD_PART);
                assert_eq!(report.request_length, 17);
                assert_eq!(report.status, if reject { "failed" } else { "complete" });
                assert_eq!(report.attempts, if reject { 1 } else { 2 });
                assert_eq!(report.retries, if reject { 0 } else { 1 });
                let calls = mock.calls.lock().unwrap();
                assert_eq!(calls.len(), if reject { 1 } else { 2 });
                assert_eq!(calls[0], (123, 20, big.then_some(21), vec![7; 17]));
                if !reject {
                    assert_eq!(calls[0], calls[1]);
                }
                assert_eq!(
                    stats.acknowledged.load(Ordering::Relaxed),
                    0,
                    "only the pump confirms a successful part once"
                );
            }
        }
    }
    struct Sink {
        parts: Mutex<BTreeMap<i32, Vec<u8>>>,
        fail: bool,
    }
    impl PartSink for Sink {
        fn save(&self, index: i32, bytes: Vec<u8>) -> Work<'_, ()> {
            Box::pin(async move {
                tokio::time::sleep(Duration::from_millis(if index == 0 { 15 } else { 1 })).await;
                ensure!(!self.fail, "TELEGRAM_UPLOAD_PART_REJECTED");
                assert!(self.parts.lock().unwrap().insert(index, bytes).is_none());
                Ok(())
            })
        }
    }
    #[test]
    fn upload_boundary_limits_md5_and_unique_identity() {
        for bytes in [BIG_FILE, BIG_FILE + 1] {
            let plan = Plan::new(bytes, UPLOAD_PART, 4).unwrap();
            let file = UploadFile::new(bytes, bytes, "test.mp4".into(), plan.parts).unwrap();
            let other = UploadFile::new(bytes, bytes, "test.mp4".into(), plan.parts).unwrap();
            assert_ne!(file.id, other.id);
            match file.finish(md5::Context::new()) {
                tl::enums::InputFile::File(file) => {
                    assert_eq!(bytes, BIG_FILE);
                    assert_eq!(file.parts, 20);
                    assert_eq!(file.md5_checksum, "d41d8cd98f00b204e9800998ecf8427e");
                }
                tl::enums::InputFile::Big(file) => {
                    assert_eq!(bytes, BIG_FILE + 1);
                    assert_eq!(file.parts, 21);
                }
                _ => panic!("unexpected input file"),
            }
            assert!(UploadFile::new(bytes, bytes - 1, "x".into(), plan.parts).is_err());
        }
        assert!(UploadFile::new(1, 1, "x".into(), i32::MAX as u64 + 1).is_err());
    }
    #[tokio::test]
    async fn upload_preserves_order_digest_tail_and_confirmed_progress() {
        let bytes: Vec<_> = (0..(6 * UPLOAD_PART + 17))
            .map(|n| (n % 251) as u8)
            .collect();
        let mut reader = std::io::Cursor::new(&bytes);
        let sink = Sink {
            parts: Mutex::default(),
            fail: false,
        };
        let stats = Stats::default();
        let progress = AtomicU64::new(0);
        let md5 = pump(
            &mut reader,
            &sink,
            Plan::new(bytes.len() as u64, UPLOAD_PART, 4).unwrap(),
            &stats,
            &|bytes| progress.store(bytes, Ordering::Relaxed),
        )
        .await
        .unwrap();
        assert_eq!(md5.finalize(), md5::compute(&bytes));
        assert_eq!(stats.read_bytes.load(Ordering::Relaxed), bytes.len() as u64);
        assert_eq!(progress.load(Ordering::Relaxed), bytes.len() as u64);
        let stored: Vec<_> = sink
            .parts
            .lock()
            .unwrap()
            .values()
            .flatten()
            .copied()
            .collect();
        assert_eq!(stored, bytes);
    }
    #[tokio::test]
    async fn rejected_parts_and_changed_source_never_succeed() {
        for (body, declared, fail) in [
            (vec![1; 10], 10, true),
            (vec![1; 9], 10, false),
            (vec![1; 11], 10, false),
        ] {
            let sink = Sink {
                parts: Mutex::default(),
                fail,
            };
            let stats = Stats::default();
            let result = pump(
                &mut std::io::Cursor::new(body),
                &sink,
                Plan::new(declared, UPLOAD_PART, 4).unwrap(),
                &stats,
                &|_| {},
            )
            .await;
            assert!(result.is_err());
            if fail {
                assert_eq!(stats.acknowledged.load(Ordering::Relaxed), 0);
            }
        }
    }

    #[tokio::test]
    async fn tail_part_confirmations_keep_an_idle_consumer_alive() {
        struct Delayed;
        impl PartSink for Delayed {
            fn save(&self, index: i32, _bytes: Vec<u8>) -> Work<'_, ()> {
                Box::pin(async move {
                    tokio::time::sleep(Duration::from_millis(30 + (index as u64 % 4) * 50)).await;
                    Ok(())
                })
            }
        }
        let stats = Stats::default();
        let activity = Mutex::new(Instant::now());
        let confirmed = |_: u64| {
            *activity.lock().unwrap() = Instant::now();
        };
        let size = 6 * 1024 * 1024;
        let mut reader = std::io::Cursor::new(vec![1; size]);
        let sink = Delayed;
        let upload = pump(
            &mut reader,
            &sink,
            Plan::new(size as u64, UPLOAD_PART, 4).unwrap(),
            &stats,
            &confirmed,
        );
        tokio::pin!(upload);
        let mut idle_check = tokio::time::interval(Duration::from_millis(20));
        let start = Instant::now();
        loop {
            tokio::select! {
                result=&mut upload=>{result.unwrap();break;},
                _=idle_check.tick()=>assert!(activity.lock().unwrap().elapsed()<Duration::from_millis(150),"server confirmations must refresh activity"),
            }
        }
        assert!(start.elapsed() > Duration::from_millis(150));
        assert_eq!(stats.acknowledged.load(Ordering::Relaxed), size as u64);
    }
}
