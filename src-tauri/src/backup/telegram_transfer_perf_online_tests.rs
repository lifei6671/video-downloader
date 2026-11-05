//! Explicit QA only. Uses run-owned synthetic files and durable send identities; never scans/deletes old assets.
use super::*;
use crate::backup::{
    telegram_transfer::{self as transfer, Envelope, Io, RemoteDocument, Target, Transport},
    telegram_transfer_journal::CoreJournal,
    telegram_transport::{GrammersTransport, PreparedUpload},
};
use crate::telegram_transfer_engine::{
    self as engine, Chunk, ChunkSource, Download, Plan, Reader, Settings, Stats,
};
use grammers_client::{media::Downloadable, tl, Client};
use rusqlite::OptionalExtension;
use std::{
    collections::BTreeMap,
    sync::{atomic::Ordering, Mutex},
    time::Instant,
};
use tokio::io::{AsyncRead, AsyncReadExt};

struct Original(tl::types::Document);
impl Downloadable for Original {
    fn to_raw_input_location(&self) -> Option<tl::enums::InputFileLocation> {
        Some(
            tl::types::InputDocumentFileLocation {
                id: self.0.id,
                access_hash: self.0.access_hash,
                file_reference: self.0.file_reference.clone(),
                thumb_size: String::new(),
            }
            .into(),
        )
    }
    fn size(&self) -> Option<usize> {
        usize::try_from(self.0.size).ok()
    }
    fn dc_id(&self) -> Option<i32> {
        Some(self.0.dc_id)
    }
}
struct LegacySource(tokio::sync::Mutex<grammers_client::client::DownloadIter>);
impl ChunkSource for LegacySource {
    fn fetch(&self, _chunk: Chunk) -> ProviderFuture<'_, Vec<u8>> {
        Box::pin(async {
            self.0
                .lock()
                .await
                .next()
                .await
                .map_err(crate::backup::telegram_session::rpc_error)?
                .context("INCOMPLETE_MEDIA")
        })
    }
}
struct Measured<'a> {
    inner: &'a GrammersTransport,
    client: Client,
    legacy: bool,
    upload_seconds: Mutex<Option<f64>>,
}
impl Transport for Measured<'_> {
    type Uploaded = PreparedUpload;
    fn max_bytes(&self) -> u64 {
        self.inner.max_bytes()
    }
    fn can_upload(&self) -> bool {
        self.inner.can_upload()
    }
    fn can_delete(&self) -> bool {
        self.inner.can_delete()
    }
    fn upload<'a>(
        &'a self,
        reader: &'a mut (dyn AsyncRead + Unpin + Send),
        bytes: u64,
        name: String,
    ) -> ProviderFuture<'a, PreparedUpload> {
        self.upload_with_progress(reader, bytes, name, None, None)
    }
    fn upload_with_progress<'a>(
        &'a self,
        mut reader: &'a mut (dyn AsyncRead + Unpin + Send),
        bytes: u64,
        name: String,
        progress: Option<&'a std::sync::atomic::AtomicU64>,
        activity: Option<&'a Io>,
    ) -> ProviderFuture<'a, PreparedUpload> {
        Box::pin(async move {
            let start = Instant::now();
            let result = if self.legacy {
                let file = self
                    .client
                    .upload_stream(&mut reader, usize::try_from(bytes)?, name)
                    .await?;
                Ok(PreparedUpload::without_thumbnail(file))
            } else {
                self.inner
                    .upload_with_progress(reader, bytes, name, progress, activity)
                    .await
            };
            *self.upload_seconds.lock().unwrap() = Some(start.elapsed().as_secs_f64());
            result
        })
    }
    fn send<'a>(
        &'a self,
        uploaded: PreparedUpload,
        caption: String,
        random: i64,
        video: Option<VideoAttributes>,
        io: &'a Io,
    ) -> ProviderFuture<'a, Option<i32>> {
        self.inner.send(uploaded, caption, random, video, io)
    }
    fn inspect(&self, id: i32) -> ProviderFuture<'_, RemoteDocument> {
        self.inner.inspect(id)
    }
    fn inspect_document(&self, id: i32, doc: i64) -> ProviderFuture<'_, RemoteDocument> {
        self.inner.inspect_document(id, doc)
    }
    fn inspect_envelope<'a>(
        &'a self,
        id: i32,
        envelope: &'a Envelope,
    ) -> ProviderFuture<'a, RemoteDocument> {
        self.inner.inspect_envelope(id, envelope)
    }
    fn find<'a>(
        &'a self,
        envelope: &'a Envelope,
        io: &'a Io,
    ) -> ProviderFuture<'a, Option<RemoteDocument>> {
        self.inner.find(envelope, io)
    }
    fn download(
        &self,
        expected: RemoteDocument,
    ) -> ProviderFuture<'_, Box<dyn AsyncRead + Unpin + Send>> {
        Box::pin(async move {
            if !self.legacy {
                return self.inner.download(expected).await;
            }
            let raw = self.inner.test_document(&expected).await?;
            let iter = self.client.iter_download(&Original(raw));
            Ok(Box::new(Reader::new(Download::new(
                Arc::new(LegacySource(tokio::sync::Mutex::new(iter))),
                Plan::new(expected.bytes, engine::UPLOAD_PART, 1)?,
            ))) as Box<dyn AsyncRead + Unpin + Send>)
        })
    }
}

#[derive(Serialize, Deserialize)]
struct Fixture {
    path: PathBuf,
    bytes: u64,
    sha256: String,
    video: serde_json::Value,
}
#[derive(Serialize, Deserialize)]
struct Sample {
    asset: String,
    operation: String,
    complete: bool,
}
#[derive(Serialize, Deserialize)]
struct Manifest {
    target: Target,
    samples: BTreeMap<String, Sample>,
}
fn metrics(stats: &Stats, bytes: u64, seconds: f64, legacy: bool) -> serde_json::Value {
    json!({"bytes":bytes,"seconds":seconds,"mib_per_second":bytes as f64/1048576.0/seconds,
        "attempts":if legacy {None} else {Some(stats.attempts.load(Ordering::Relaxed))},
        "failed_attempts":if legacy {None} else {Some(stats.failures.load(Ordering::Relaxed))},
        "retries":if legacy {None} else {Some(stats.retries.load(Ordering::Relaxed))},
        "peak_rpc":if legacy {None} else {Some(stats.peak_rpc.load(Ordering::Relaxed))},
        "rpc_seconds_sum":if legacy {None} else {Some(stats.rpc_nanos.load(Ordering::Relaxed) as f64/1e9)},
        "dedicated_media_sessions":0,"recent_chunks":if legacy {None} else {Some(stats.recent_parts())}})
}

#[tokio::test]
#[ignore = "explicit dedicated QA config and generated fixture manifest required; publishes and removes only run-owned synthetic media"]
async fn telegram_transfer_live_performance_matrix() -> Result<()> {
    let config: Config = serde_json::from_slice(&std::fs::read(
        std::env::var_os("YINGQU_TELEGRAM_TRANSFER_PERF_CONFIG")
            .context("explicit QA config required")?,
    )?)?;
    anyhow::ensure!(
        config
            .source_data_directory
            .file_name()
            .and_then(|s| s.to_str())
            == Some("me.disign.yingqu.telegram-validation"),
        "dedicated QA account required"
    );
    let work = PathBuf::from(
        std::env::var_os("YINGQU_TELEGRAM_TRANSFER_PERF_DIRECTORY")
            .context("new fixture directory required")?,
    );
    anyhow::ensure!(
        work.file_name().and_then(|s| s.to_str())
            == Some("telegram-transfer-performance-validation"),
        "dedicated run directory required"
    );
    let fixtures: Vec<Fixture> =
        serde_json::from_slice(&std::fs::read(work.join("fixtures.json"))?)?;
    let count: usize = std::env::var("YINGQU_TELEGRAM_TRANSFER_PERF_SAMPLES")
        .unwrap_or_else(|_| "10".into())
        .parse()?;
    anyhow::ensure!((1..=10).contains(&count), "invalid sample count");
    let size_filter = std::env::var("YINGQU_TELEGRAM_TRANSFER_PERF_BYTES")
        .ok()
        .map(|v| v.parse::<u64>())
        .transpose()?;
    let mode_filter = std::env::var("YINGQU_TELEGRAM_TRANSFER_PERF_MODE").ok();
    // The authorized session may come from an older QA database. Reuse only its vault binding
    // and network profile, in a fresh current-schema database; never migrate the old fixture DB.
    let database = work.join("qa.sqlite");
    if !database.exists() {
        let bootstrap = Core::open(
            &database,
            work.join("downloads"),
            "unused-engine".into(),
            vec![],
            |_| {},
        )?;
        bootstrap.shutdown().await;
        drop(bootstrap);
        let old = rusqlite::Connection::open_with_flags(
            config.source_data_directory.join("yingqu.sqlite"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?;
        let has_bindings:bool=old.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='telegram_account_bindings')",[],|r|r.get(0))?;
        let namespace: String = if has_bindings {
            old.query_row(
                "SELECT session_ref FROM telegram_account_bindings WHERE purpose='backup'",
                [],
                |r| r.get(0),
            )
            .optional()?
            .unwrap_or(repository::device_id(&old)?)
        } else {
            repository::device_id(&old)?
        };
        uuid::Uuid::parse_str(&namespace)?;
        let new = rusqlite::Connection::open(&database)?;
        for purpose in ["backup", "download"] {
            new.execute(
                "INSERT INTO telegram_account_bindings VALUES(?1,?2)",
                params![purpose, namespace],
            )?;
        }
        let profile:Option<(String,Option<String>,Option<String>)>=old.query_row("SELECT user_agent,proxy,credential_ref FROM platform_settings WHERE platform='telegram'",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
        if let Some((agent, proxy, credential)) = profile {
            new.execute("INSERT OR REPLACE INTO platform_settings(platform,user_agent,proxy,cookie,credential_ref,http_chunk_size) VALUES('telegram',?1,?2,'',?3,NULL)",params![agent,proxy,credential])?;
        }
    }
    let core = Core::open(
        &database,
        work.join("downloads"),
        "unused-engine".into(),
        vec![],
        |_| {},
    )?;
    let network = if core
        .settings()
        .profiles
        .iter()
        .any(|p| p.platform == "telegram" && p.proxy.is_some())
    {
        "proxy"
    } else {
        "direct"
    };
    std::fs::write(work.join("network-path.txt"), network)?;
    std::fs::write(work.join("process-id.txt"), std::process::id().to_string())?;
    let result=async {
        let binding=core.telegram_accounts.binding(crate::telegram_accounts::Purpose::Backup);
        let session=binding.session;
        let login=session.resume(binding.namespace).await?;
        anyhow::ensure!(matches!(login.phase,Phase::Ready),"dedicated QA login required");
        let id=login.operation_id.context("login operation required")?;
        let account=login.account.context("ready account required")?;
        let channels=session.channels(&id).await?;
        let matching:Vec<_>=channels.iter().filter(|c|c.title==config.channel_name&&c.can_post&&c.can_delete).collect();
        anyhow::ensure!(matching.len()==1,"one authorized writable private QA channel required");
        let manifest_path=work.join("run.private.json");
        let mut manifest:Manifest=if manifest_path.exists() {serde_json::from_slice(&std::fs::read(&manifest_path)?)?} else {
            Manifest {target:Target {scope:Scope {space_id:uuid::Uuid::new_v4().to_string(),library_id:uuid::Uuid::new_v4().to_string(),provider_type:ProviderType::Telegram,protocol:2},account_id:account.id.clone(),channel_id:matching[0].id.clone()},samples:BTreeMap::new()}
        };
        anyhow::ensure!(manifest.target.account_id==account.id&&manifest.target.channel_id==matching[0].id,"QA target changed");
        let device=core.telegram_storage_device_id()?;
        for fixture in fixtures {
            if size_filter.is_some_and(|size|size!=fixture.bytes) {continue;}
            anyhow::ensure!(fixture.bytes<=500*1048576 && fixture.path.canonicalize()?.starts_with(work.canonicalize()?),"run-owned fixture required");
            let (hash,signature)=repository::fingerprint(&fixture.path)?;
            anyhow::ensure!(hash==fixture.sha256&&signature["size"].as_u64()==Some(fixture.bytes),"fixture changed");
            for round in 0..count {
                // Alternate each round so a later network period does not systematically favor a mode.
                let mut modes=vec!["legacy","1","2","4","8","16"];
                if round%2==1 {modes.reverse();}
                for mode in modes {
                    if mode_filter.as_deref().is_some_and(|filter|filter!=mode) {continue;}
                    for attempt in 0..=1 {
                    let key=format!("{}-{round}-{mode}",fixture.bytes);
                    let sample=manifest.samples.entry(key.clone()).or_insert_with(||Sample {asset:uuid::Uuid::new_v4().to_string(),operation:uuid::Uuid::new_v4().to_string(),complete:false});
                    if sample.complete {break;}
                    let asset=sample.asset.clone(); let operation=sample.operation.clone();
                    std::fs::write(&manifest_path,serde_json::to_vec_pretty(&manifest)?)?;
                    std::fs::write(work.join("active-sample.txt"),&key)?;
                    let target=manifest.target.clone();
                    let source=MediaSource {video:Some(VideoAttributes::from_metadata(&fixture.video).context("fixture video metadata")?),base_revision:None,recreate_revision:None,asset_id:asset,kind:"video".into(),path:fixture.path.clone(),bytes:fixture.bytes,sha256:fixture.sha256.clone(),container:"mp4".into(),operation_id:operation,device_id:device.clone(),control:None};
                    let legacy=mode=="legacy";let concurrency=if legacy {1} else {mode.parse()?};
                    let (_cancel,rx)=tokio::sync::watch::channel(false);
                    let bound=target.clone();
                    let diagnostic_path=work.join("failed-chunks.redacted.ndjson");
                    let diagnostic_key=key.clone();
                    let sample_result=session.with_transport(&core,&id,target,rx,("upload_media",None),move |core,t,io|Box::pin(async move {
                        let up=Arc::new(Stats::default());
                        let down=Arc::new(Stats::default());
                        let outcome=async {
                        let measured=Measured {inner:t,client:t.test_client(),legacy,upload_seconds:Mutex::new(None)};
                        let video=source.video.clone().context("synthetic video attributes required")?;
                        let receipt=engine::with_settings(Settings {concurrency,stats:up.clone()},transfer::upload(&measured,&CoreJournal(core),&bound,source,io)).await?;
                        let upload_seconds=*measured.upload_seconds.lock().unwrap();
                        let expected=t.inspect_reference(&bound,&receipt.reference).await?;
                        let raw=t.test_document(&expected).await?;
                        anyhow::ensure!(raw.mime_type=="video/mp4"&&raw.attributes.iter().any(|attribute|matches!(attribute,tl::enums::DocumentAttribute::Video(v) if v.w==video.width as i32&&v.h==video.height as i32&&(v.duration-video.duration_ms as f64/1000.0).abs()<0.001)),"server changed video attributes");
                        let start=Instant::now();
                        let (bytes,hash)=engine::with_settings(Settings {concurrency,stats:down.clone()},async {
                            let mut reader=io.run(measured.download(expected.clone())).await?;
                            let mut hash=Sha256::new();let mut bytes=0;let mut buffer=vec![0;64*1024];
                            loop {
                                let n=io.run(async {Ok(reader.read(&mut buffer).await?)}).await?;
                                if n==0 {break;} io.touch();bytes+=n as u64;hash.update(&buffer[..n]);
                            }
                            Ok::<_,anyhow::Error>((bytes,format!("{:x}",hash.finalize())))
                        }).await?;
                        let download_seconds=start.elapsed().as_secs_f64();
                        anyhow::ensure!(bytes==receipt.reference.bytes&&hash==receipt.reference.sha256,"independent readback mismatch");
                        // Only this sample's exact receipt is eligible for cleanup.
                        io.run(t.remove(expected)).await?;
                        Ok(match upload_seconds {
                            Some(seconds)=>json!({"upload":metrics(&up,bytes,seconds,legacy),"download":metrics(&down,bytes,download_seconds,legacy),"sha256_match":true,"document_identity_verified":true,"video_attributes_verified":true}),
                            None=>json!({"excluded":true,"reason":"resumed_confirmed_upload","sha256_match":true,"scoped_cleanup":true}),
                        })
                        }.await;
                        if let Err(error)=&outcome {
                            use std::io::Write;
                            let report=json!({"sample":diagnostic_key,"error":error.to_string(),"upload_chunks":up.recent_parts(),"download_chunks":down.recent_parts()});
                            let written=(||->Result<()> {
                                let mut file=std::fs::OpenOptions::new().create(true).append(true).open(diagnostic_path)?;
                                writeln!(file,"{report}")?;Ok(())
                            })();
                            if written.is_err() {eprintln!("QA_DIAGNOSTIC_IO");}
                        }
                        outcome
                    })).await;
                    match sample_result {
                        Ok(result)=>{
                            use std::io::Write;
                            if result["excluded"]==true {
                                let mut output=std::fs::OpenOptions::new().create(true).append(true).open(work.join("excluded.redacted.ndjson"))?;
                                writeln!(output,"{}",json!({"sample":key,"result":result}))?;
                                manifest.samples.insert(key.clone(),Sample {asset:uuid::Uuid::new_v4().to_string(),operation:uuid::Uuid::new_v4().to_string(),complete:false});
                                std::fs::write(&manifest_path,serde_json::to_vec_pretty(&manifest)?)?;
                                anyhow::ensure!(attempt==0,"repeated resumed sample excluded");
                                continue;
                            }
                            let record=json!({"sample":key,"bytes":fixture.bytes,"round":round,"mode":mode,"engine_part_bytes":engine::DOWNLOAD_PART,"measurements":result});
                            let mut output=std::fs::OpenOptions::new().create(true).append(true).open(work.join("samples.redacted.ndjson"))?;
                            writeln!(output,"{}",serde_json::to_string(&record)?)?;
                            manifest.samples.get_mut(&key).unwrap().complete=true;
                            std::fs::write(&manifest_path,serde_json::to_vec_pretty(&manifest)?)?;
                            println!("sample {key}: {}",serde_json::to_string(&result)?);
                            break;
                        }
                        Err(error)=>{std::fs::write(work.join("failure.txt"),format!("{key}: {error}"))?;return Err(error);}
                    }
                    }
                }
            }
        }
        Ok::<_,anyhow::Error>(())
    }.await;
    core.shutdown().await;
    result
}
