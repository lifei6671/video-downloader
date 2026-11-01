//! Opt-in synthetic integration; separate processes can be terminated at durable boundaries.
#[path = "telegram_bundle_online_tests.rs"]
mod bundle_tests;
#[path = "telegram_transcript_online_tests.rs"]
mod transcript_tests;
use super::{
    provider::*,
    repository, spaces,
    sync_engine::{self, SyncEngine},
    sync_read,
    telegram_session::Phase,
};
use crate::desktop_core::Core;
use anyhow::{bail, Context, Result};
use rusqlite::params;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{path::PathBuf, sync::Arc};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    channel_name: String,
    source_data_directory: PathBuf,
    recovery_data_directory: PathBuf,
    work_directory: PathBuf,
    input_directory: PathBuf,
}
#[derive(Serialize, Deserialize)]
struct Run {
    asset_id: String,
    operation_id: Option<String>,
}
fn open(config: &Config, recovery: bool) -> Result<Arc<Core>> {
    let data = if recovery {
        &config.recovery_data_directory
    } else {
        &config.source_data_directory
    };
    let root = config.work_directory.join(if recovery {
        "recovery-downloads"
    } else {
        "source-downloads"
    });
    std::fs::create_dir_all(&root)?;
    let core = Core::open(
        &data.join("yingqu.sqlite"),
        root.clone(),
        "unused-engine".into(),
        vec![],
        |_| {},
    )?;
    core.set_directory(root)?;
    Ok(core)
}
async fn connect(
    core: &Arc<Core>,
    config: &Config,
    create: bool,
) -> Result<Arc<dyn BackupProvider>> {
    let binding = core.telegram_accounts.binding(crate::telegram_accounts::Purpose::Backup);
    let view = binding.session
        .resume(binding.namespace)
        .await?;
    if !matches!(view.phase, Phase::Ready) {
        bail!("dedicated account must be logged in first");
    }
    let channels = core
        .telegram_accounts.binding(crate::telegram_accounts::Purpose::Backup).session
        .channels(
            view.operation_id
                .as_deref()
                .context("missing login operation")?,
        )
        .await?;
    let matching: Vec<_> = channels
        .iter()
        .filter(|c| c.title == config.channel_name)
        .collect();
    if matching.len() != 1 {
        bail!("expected exactly one requested private broadcast channel");
    }
    let saved = spaces::save(
        core,
        spaces::Update {
            telegram: Some(spaces::TelegramConfig {
                account_id: view.account.context("missing account")?.id,
                channel_id: matching[0].id.clone(),
            }),
            create_library: create,
            display_name: Some("Telegram T3 合成验证".into()),
            provider_type: ProviderType::Telegram,
            profile: "telegram".into(),
            endpoint: String::new(),
            bucket: String::new(),
            prefix: String::new(),
            region: "auto".into(),
            path_style: true,
            enabled: true,
            activate: true,
            credentials: None,
            cleanup_deleted_records: false,
            deleted_record_retention_days: 30,
        },
    )
    .await?;
    spaces::provider(core, &saved.space_id)
}
fn source(core: &Arc<Core>, config: &Config, provider: &dyn BackupProvider) -> Result<Run> {
    let manifest = config.work_directory.join("run.json");
    let mut run: Run = if manifest.exists() {
        serde_json::from_slice(&std::fs::read(&manifest)?)?
    } else {
        let value = Run {
            asset_id: uuid::Uuid::new_v4().to_string(),
            operation_id: None,
        };
        std::fs::write(&manifest, serde_json::to_vec_pretty(&value)?)?;
        value
    };
    let path = config.input_directory.join("video.mp4");
    let (hash, signature) = repository::fingerprint(&path)?;
    let cover = std::fs::read(config.input_directory.join("cover.png"))?;
    core.backup_db(|db|{
        let exists:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM media_assets WHERE asset_id=?)",[&run.asset_id],|r|r.get(0))?;
        if !exists {
            let tx=db.unchecked_transaction()?;
            tx.execute("INSERT INTO media_assets VALUES(?1,NULL,?2,'telegram-t3-synthetic.mp4',?3,?4,'active')",params![run.asset_id,json!({"title":"Telegram T3 合成恢复验证","platform":"youtube","video_id":"12345678901","canonical_url":"https://www.youtube.com/watch?v=12345678901"}).to_string(),hash,signature["size"].as_u64().context("missing fixture size")? as i64])?;
            tx.execute("INSERT INTO asset_local_files VALUES(?1,?2,?3,?4,?5,unixepoch(),'present')",params![run.asset_id,path.to_string_lossy(),signature["size"].as_u64().unwrap() as i64,hash,signature.to_string()])?;
            tx.execute("INSERT INTO asset_transcripts VALUES(?1,?2,'T3 全新数据库恢复验证。中文 English 123。','synthetic-test',NULL,'telegram-t3',1,NULL)",params![run.asset_id,hash])?;
            tx.execute("INSERT INTO asset_covers VALUES(?1,?2)",params![run.asset_id,cover])?;
            sync_engine::register_download(&tx,&run.asset_id)?;tx.commit()?;
        }
        Ok(())
    })?;
    let engine = SyncEngine::new(
        core.clone(),
        spaces::provider(core, &provider.scope().space_id)?,
    );
    engine.enqueue()?;
    let op = super::scheduler::operations(core)?
        .into_iter()
        .find(|o| o.asset_id.as_deref() == Some(&run.asset_id) && o.kind == "upload")
        .context("synthetic operation not queued")?;
    if run
        .operation_id
        .as_ref()
        .is_some_and(|id| id != &op.operation_id)
    {
        bail!("test operation identity changed");
    }
    run.operation_id = Some(op.operation_id);
    std::fs::write(manifest, serde_json::to_vec_pretty(&run)?)?;
    Ok(run)
}
async fn seed(core: &Arc<Core>, config: &Config, stage: &str) -> Result<()> {
    let provider = connect(core, config, true).await?;
    let run = source(core, config, provider.as_ref())?;
    let operation = run.operation_id.context("missing operation")?;
    core.backup_db(|db| {
        db.execute(
            "UPDATE backup_operations SET state='running',attempt=attempt+1 WHERE operation_id=?",
            [&operation],
        )?;
        Ok(())
    })?;
    if stage == "seed_media" {
        core.backup_db(|db|{db.execute_batch("CREATE TEMP TRIGGER telegram_test_stop BEFORE UPDATE ON backup_operations WHEN NEW.phase='publishing' BEGIN SELECT RAISE(ABORT,'synthetic media boundary'); END;")?;Ok(())})?;
    } else if stage == "seed_event" {
        core.backup_db(|db|{db.execute_batch("CREATE TEMP TRIGGER telegram_test_stop BEFORE UPDATE ON backup_operations WHEN NEW.state='completed' BEGIN SELECT RAISE(ABORT,'synthetic publication boundary'); END;")?;Ok(())})?;
    }
    let engine = SyncEngine::new(core.clone(), provider);
    let result = engine.execute_upload(&operation).await;
    let (confirmed,events):(i64,i64)=core.backup_db(|db|Ok(db.query_row("SELECT COUNT(*),SUM(kind='event') FROM telegram_media_sends WHERE operation_id=? AND json_extract(entry_json,'$.state')='confirmed'",[&operation],|r|Ok((r.get(0)?,r.get(1)?)))?))?;
    if stage == "seed_media" || stage == "seed_event" {
        if result.is_ok()
            || confirmed != if stage == "seed_media" { 3 } else { 4 }
            || events != if stage == "seed_media" { 0 } else { 1 }
        {
            bail!("synthetic durable boundary not reached");
        }
        // Parent verifies this owned helper PID and terminates it; do not gracefully close the Core.
        std::fs::write(
            config.work_directory.join(format!("barrier-{stage}.json")),
            serde_json::to_vec(&json!({"stage":stage,"confirmed":confirmed,"events":events}))?,
        )?;
        println!("Durable {stage} boundary reached; waiting for parent termination");
        std::future::pending::<()>().await;
    }
    result?;
    if confirmed != 4 || events != 1 {
        bail!("unexpected repeated media/event send evidence");
    }
    let ack: (i64, i64) = core.backup_db(|db| {
        Ok(db.query_row(
            "SELECT local_revision,ack_revision FROM backup_space_assets WHERE local_asset_id=?",
            [&run.asset_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?)
    })?;
    if ack.0 != ack.1
        || super::scheduler::operations(core)?
            .into_iter()
            .find(|o| o.operation_id == operation)
            .context("missing operation")?
            .state
            != "completed"
    {
        bail!("publication did not converge");
    }
    println!("Seed converged after process restarts; exactly 3 media documents and 1 event for the operation");
    Ok(())
}
async fn recover(core: &Arc<Core>, config: &Config) -> Result<()> {
    let before = core.backup_assets()?.len();
    if !config.work_directory.join("recovery-report.json").exists() && before != 0 {
        bail!("recovery acceptance must start from an empty asset database");
    }
    let provider = connect(core, config, false).await?;
    let engine = SyncEngine::new(core.clone(), provider.clone());
    engine.pull().await?;
    let assets = core.backup_assets()?;
    let matching: Vec<_> = assets
        .iter()
        .filter(|a| a.metadata["title"] == "Telegram T3 合成恢复验证")
        .collect();
    if matching.len() != 1 {
        bail!("expected one reconstructed synthetic asset");
    }
    let asset = matching[0];
    if before == 0 && asset.path.is_some() {
        bail!("listing prefetched a video");
    }
    let operations = super::scheduler::operations(core)?;
    if before == 0 && operations.iter().any(|o| o.kind == "restore") {
        bail!("listing scheduled a video restore");
    }
    println!("Fresh database replay: asset discovered, no video file or restore job created");
    let transcript = sync_read::transcript(core, provider.as_ref(), &asset.asset_id)
        .await?
        .context("missing recovered transcript")?;
    if transcript.text != "T3 全新数据库恢复验证。中文 English 123。" {
        bail!("transcript changed");
    }
    let cover = sync_read::cover(core, provider.as_ref(), &asset.asset_id).await?;
    if cover != std::fs::read(config.input_directory.join("cover.png"))? {
        bail!("cover changed");
    }
    if before == 0 && core.backup_assets()?.iter().any(|a| a.path.is_some()) {
        bail!("small-object reads prefetched video");
    }
    println!("Transcript and cover restored without video download");
    let path = sync_read::ensure(core, provider.as_ref(), &asset.asset_id).await?;
    let actual = repository::fingerprint(&path)?.0;
    let expected = repository::fingerprint(&config.input_directory.join("video.mp4"))?.0;
    if actual != expected {
        bail!("recovered video SHA-256 differs");
    }
    let again = sync_read::ensure(core, provider.as_ref(), &asset.asset_id).await?;
    if again != path {
        bail!("valid local file was not reused");
    }
    let restored = core
        .backup_assets()?
        .into_iter()
        .find(|a| a.asset_id == asset.asset_id)
        .context("missing local asset")?;
    let report = json!({"initial_assets":before,"reconstructed_assets":assets.len(),"video_bytes":restored.content_bytes,"video_sha256":actual,"cover_sha256":format!("{:x}",Sha256::digest(&cover)),"transcript_recovered":true,"list_prefetched_video":false,"existing_local_reused":true});
    std::fs::write(
        config.work_directory.join("recovery-report.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    println!(
        "On-demand video: {} bytes, SHA-256 matched; second use reused local file",
        restored.content_bytes
    );
    Ok(())
}

#[tokio::test]
#[ignore = "explicit original video and dedicated channel authorization; adds one streaming video message, never transcodes"]
async fn telegram_streaming_live_original_video_flag_and_hash() -> Result<()> {
    use super::telegram_transfer::{RestoreRequest, Target, UploadRequest};
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct StreamingConfig {
        channel_name: String,
        data_directory: PathBuf,
        work_directory: PathBuf,
        source: PathBuf,
        source_sha256: String,
        video_metadata: serde_json::Value,
    }
    #[derive(Serialize,Deserialize)]
    struct StreamingRun { target: Target, asset_id: String, operation_id: String, source_sha256: String }
    let config: StreamingConfig=serde_json::from_slice(&std::fs::read(std::env::var_os("YINGQU_TELEGRAM_STREAMING_TEST_CONFIG").context("explicit ignored streaming config required")?)?)?;
    if config.data_directory.file_name().and_then(|v|v.to_str()) != Some("me.disign.yingqu.telegram-validation") || config.channel_name.trim().is_empty() { bail!("existing dedicated session and channel required"); }
    let core=Core::open(&config.data_directory.join("yingqu.sqlite"),config.work_directory.clone(),"unused-engine".into(),vec![], |_| {})?;
    let result=async {
        let (hash,signature)=repository::fingerprint(&config.source)?;
        if hash != config.source_sha256 { bail!("original source hash changed"); }
        let bytes=signature["size"].as_u64().context("source size required")?;
        let video=VideoAttributes::from_metadata(&config.video_metadata).context("video dimensions and duration required")?;
        let device=core.telegram_storage_device_id()?;
        let binding=core.telegram_accounts.binding(crate::telegram_accounts::Purpose::Backup);
        let login=binding.session.resume(binding.namespace).await?;
        if !matches!(login.phase,Phase::Ready) { bail!("existing authorized session required"); }
        let id=login.operation_id.context("login operation required")?;
        let account=login.account.context("ready account required")?;
        let channels=core.telegram_accounts.binding(crate::telegram_accounts::Purpose::Backup).session.channels(&id).await?;
        let matching:Vec<_>=channels.iter().filter(|c|c.title==config.channel_name && c.can_post).collect();
        if matching.len()!=1 { bail!("one authorized writable private channel required"); }
        std::fs::create_dir_all(&config.work_directory)?;
        let manifest=config.work_directory.join("run.json");
        let run:StreamingRun=if manifest.exists() {serde_json::from_slice(&std::fs::read(&manifest)?)?} else {
            let run=StreamingRun {
                target:Target {scope:Scope {space_id:uuid::Uuid::new_v4().to_string(),library_id:uuid::Uuid::new_v4().to_string(),provider_type:ProviderType::Telegram,protocol:2},account_id:account.id.clone(),channel_id:matching[0].id.clone()},
                asset_id:uuid::Uuid::new_v4().to_string(),operation_id:uuid::Uuid::new_v4().to_string(),source_sha256:hash.clone(),
            };
            std::fs::write(&manifest,serde_json::to_vec_pretty(&run)?)?;
            run
        };
        if run.source_sha256!=hash || run.target.account_id!=account.id || run.target.channel_id!=matching[0].id { bail!("streaming fixture target/source changed"); }
        let (_cancel,rx)=tokio::sync::watch::channel(false);
        let receipt=core.telegram_accounts.binding(crate::telegram_accounts::Purpose::Backup).session.upload_media(&core,UploadRequest {
            login_id:id.clone(),target:run.target.clone(),source:MediaSource {
                video:Some(video),base_revision:None,recreate_revision:None,asset_id:run.asset_id.clone(),kind:"video".into(),path:config.source.clone(),bytes,sha256:hash.clone(),container:"mp4".into(),operation_id:run.operation_id.clone(),device_id:device,control:None,
            },
        },rx.clone()).await?;
        let MediaLocation::TelegramMessage {message_id,..}=&receipt.reference.location else {bail!("Telegram reference required");};
        let message_id=i32::try_from(*message_id)?;
        let streaming=core.telegram_accounts.binding(crate::telegram_accounts::Purpose::Backup).session.with_transport(&core,&id,run.target.clone(),rx.clone(),("test_probe", None),move |_,transport,io|Box::pin(async move {io.run(transport.streaming_video_flag(message_id)).await})).await?;
        if !streaming {bail!("server document has no streaming video flag");}
        let destination=config.work_directory.join("restored-original.mp4");
        if !destination.exists() {
            core.telegram_accounts.binding(crate::telegram_accounts::Purpose::Backup).session.restore_media(&core,RestoreRequest {login_id:id,target:run.target.clone(),reference:receipt.reference,destination:destination.clone()},rx).await?;
        }
        if repository::fingerprint(&destination)?.0!=hash || repository::fingerprint(&config.source)?.0!=hash {bail!("original/restored byte integrity failed");}
        std::fs::write(config.work_directory.join("playback-target.json"),serde_json::to_vec_pretty(&json!({"channel_id":run.target.channel_id,"message_id":message_id,"asset_id":run.asset_id}))?)?;
        std::fs::write(config.work_directory.join("streaming-report.json"),serde_json::to_vec_pretty(&json!({"bytes":bytes,"sha256":hash,"server_streaming_flag":streaming,"original_and_restored_hash_match":true,"transcoded":false}))?)?;
        println!("original video: {bytes} bytes, server streaming flag confirmed, original and restored SHA-256 matched; no transcoding");
        Ok::<(),anyhow::Error>(())
    }.await;
    core.shutdown().await;
    result
}

#[tokio::test]
#[ignore = "explicit existing dedicated recovery session; reads channel metadata without uploading or deleting"]
async fn telegram_busy_live_cold_resume_channel_reads_and_discovery() -> Result<()> {
    let config: Config = serde_json::from_slice(&std::fs::read(
        std::env::var_os("YINGQU_TELEGRAM_T3_TEST_CONFIG").context("explicit ignored test config required")?,
    )?)?;
    if config.recovery_data_directory.file_name().and_then(|v|v.to_str()) != Some("me.disign.yingqu.telegram-recovery-validation")
        || config.channel_name.trim().is_empty() { bail!("dedicated recovery paths/channel required"); }
    for cycle in 1..=3 {
        let core=Core::open(&config.recovery_data_directory.join("yingqu.sqlite"),config.work_directory.join("recovery-downloads"),"unused-engine".into(),vec![], |_| {})?;
        let result=async {
            let binding=core.telegram_accounts.binding(crate::telegram_accounts::Purpose::Backup);
            let device=binding.namespace;
            let session=binding.session;
            let (first, second, third)=tokio::join!(
                session.ready_for_backup(device.clone()),
                session.resume(device.clone()),
                session.ready_for_backup(device),
            );
            let first=first?;let second=second?;let third=third?;
            if !matches!(first.phase,Phase::Ready) || first.operation_id != second.operation_id || first.operation_id != third.operation_id { bail!("concurrent resume did not share a ready session"); }
            let id=first.operation_id.context("ready operation required")?;
            let space=spaces::get(&core)?.context("saved dedicated target required")?;
            if space.provider_type != ProviderType::Telegram { bail!("dedicated Telegram space required"); }
            let provider=spaces::provider(&core,&space.space_id)?;
            let engine=SyncEngine::new(core.clone(),provider);
            let (first,pull,second)=tokio::join!(
                session.channels(&id),
                engine.pull(),
                session.channels(&id),
            );
            let first=first?;pull?;let second=second?;
            for channels in [first,second] {
                if !channels.iter().any(|c|c.title==config.channel_name) { bail!("expected dedicated channel was not discovered"); }
            }
            if session.status().last_error.is_some() || spaces::get(&core)?.and_then(|s|s.last_error).is_some() { bail!("successful reads retained a stale error"); }
            println!("cold cycle {cycle}: concurrent resume, two channel reads and discovery passed; no retained error");
            Ok(())
        }.await;
        core.shutdown().await;
        result?;
    }
    Ok(())
}

#[tokio::test]
#[ignore = "requires explicit dedicated channel authorization; opt-in stage and independent recovery login"]
async fn telegram_t3_live_queue_process_recovery_and_fresh_database() -> Result<()> {
    let config: Config = serde_json::from_slice(&std::fs::read(
        std::env::var_os("YINGQU_TELEGRAM_T3_TEST_CONFIG")
            .context("explicit ignored test config required")?,
    )?)?;
    if config
        .source_data_directory
        .file_name()
        .and_then(|v| v.to_str())
        != Some("me.disign.yingqu.telegram-validation")
        || config
            .recovery_data_directory
            .file_name()
            .and_then(|v| v.to_str())
            != Some("me.disign.yingqu.telegram-recovery-validation")
        || config.work_directory.file_name().and_then(|v| v.to_str())
            != Some("telegram-t3-validation")
        || config.channel_name.trim().is_empty()
    {
        bail!("dedicated validation paths/channel required");
    }
    std::fs::create_dir_all(&config.work_directory)?;
    let stage =
        std::env::var("YINGQU_TELEGRAM_T3_TEST_STAGE").context("explicit test stage required")?;
    if !matches!(
        stage.as_str(),
        "seed_media" | "seed_event" | "seed_finish" | "recover"
    ) {
        bail!("unsupported stage");
    }
    let core = open(&config, stage == "recover")?;
    let result = if stage == "recover" {
        recover(&core, &config).await
    } else {
        seed(&core, &config, &stage).await
    };
    core.shutdown().await;
    result
}

#[tokio::test]
#[ignore = "explicit dedicated T4 config; creates and deletes only its new synthetic asset"]
async fn telegram_t4_live_release_delete_recreate_and_preserve_history() -> Result<()> {
    let config: Config = serde_json::from_slice(&std::fs::read(
        std::env::var_os("YINGQU_TELEGRAM_T4_TEST_CONFIG")
            .context("explicit T4 config required")?,
    )?)?;
    if config.work_directory.file_name().and_then(|v| v.to_str()) != Some("telegram-t4-validation")
        || config.input_directory.parent() != Some(config.work_directory.as_path())
        || config
            .source_data_directory
            .file_name()
            .and_then(|v| v.to_str())
            != Some("me.disign.yingqu.telegram-validation")
        || config
            .recovery_data_directory
            .file_name()
            .and_then(|v| v.to_str())
            != Some("me.disign.yingqu.telegram-recovery-validation")
    {
        bail!("dedicated T4 paths required");
    }
    let a = open(&config, false)?;
    let b = open(&config, true)?;
    let result=async {
        let p=connect(&a,&config,false).await?;let q=connect(&b,&config,false).await?;
        if !p.capabilities().can_delete_owned_media || !q.capabilities().can_delete_owned_media {bail!("dedicated channel delete permission required");}
        let stage=std::env::var("YINGQU_TELEGRAM_T4_TEST_STAGE").unwrap_or_else(|_|"start".into());
        if !matches!(stage.as_str(),"start"|"resume_delete"|"resume_recreated") {bail!("unsupported T4 stage");}
        let resume=stage!="start";
        let resume_recreated=stage=="resume_recreated";
        let run:Run=if resume {serde_json::from_slice(&std::fs::read(config.work_directory.join("run.json"))?)?} else {source(&a,&config,p.as_ref())?};
        let asset=run.asset_id;let operation=run.operation_id.context("missing T4 operation")?;
        let before:Vec<_>=p.pull_changes(None).await?.assets.into_iter().filter(|s|s.snapshot.asset_id!=asset).collect();
        let reader=SyncEngine::new(b.clone(),q.clone());
        let (path,hash)=if resume {
            let owned=super::scheduler::operations(&a)?.into_iter().any(|o|o.operation_id==operation&&o.asset_id.as_deref()==Some(&asset)&&o.space_id==p.scope().space_id&&o.kind=="upload"&&o.state=="completed");
            if !owned || !p.read_asset(&asset).await?.is_some_and(|s|s.snapshot.deleted!=resume_recreated) {bail!("resume requires this run's confirmed upload and expected incarnation");}
            let path=b.backup_assets()?.into_iter().find(|s|s.asset_id==asset).and_then(|s|s.path).context("resume requires retained B copy")?;
            let hash=repository::fingerprint(&path)?.0;
            println!("T4 resuming persisted owned deletion; retained B video verified");(path,hash)
        } else {
            if p.read_asset(&asset).await?.is_some() {bail!("start requires a new T4 synthetic asset");}
            a.backup_db(|db|{db.execute("UPDATE backup_operations SET state='running' WHERE operation_id=?",[&operation])?;Ok(())})?;
            let writer=SyncEngine::new(a.clone(),p.clone());writer.execute_upload(&operation).await?;
            println!("T4 new synthetic video, cover and transcript converged");
            reader.pull().await?;
            sync_read::cover(&b,q.as_ref(),&asset).await?;
            sync_read::transcript(&b,q.as_ref(),&asset).await?.context("missing T4 transcript")?;
            let path=sync_read::ensure(&b,q.as_ref(),&asset).await?;
            let hash=repository::fingerprint(&path)?.0;
            super::sync_delete::release_with(&b,q.as_ref(),&asset).await?;
            if path.exists(){bail!("T4 local release did not remove video");}
            let path=sync_read::ensure(&b,q.as_ref(),&asset).await?;
            if repository::fingerprint(&path)?.0!=hash {bail!("T4 restored SHA-256 differs");}
            std::fs::write(config.work_directory.join("release-report.json"),serde_json::to_vec(&json!({"full_hash_before_release":true,"sha256":hash}))?)?;
            println!("T4 full remote hash verified; local release and on-demand restore passed");(path,hash)
        };
        async fn pass(core:&Arc<Core>,provider:&dyn BackupProvider,operation:&str)->Result<()> {
            core.backup_db(|db|{db.execute("UPDATE backup_operations SET state='running' WHERE operation_id=?",[operation])?;Ok(())})?;
            super::sync_delete::execute(core,provider,operation).await
        }
        let delete=if resume_recreated {
            if !super::scheduler::operations(&b)?.iter().any(|o|o.asset_id.as_deref()==Some(&asset)&&o.kind=="upload"&&o.state=="completed") {bail!("resume requires B's confirmed recreation");}
            println!("T4 resuming after confirmed recreation");
            // Core requeues completed deletions for late-object checks on every boot.
            super::scheduler::operations(&a)?.into_iter().find(|o|o.asset_id.as_deref()==Some(&asset)&&o.kind=="delete"&&o.space_id==p.scope().space_id&&o.payload["cleanup_pass"].as_u64().is_some_and(|n|n>=3)&&matches!(o.phase.as_str(),"superseded"|"cleanup_verify")&&o.state!="canceled").context("missing completed original deletion evidence")?.operation_id
        } else {
            let delete=super::sync_delete::intent(&a,&p.scope().space_id,&asset)?;
            for _ in 0..3 {pass(&a,p.as_ref(),&delete).await?;}
            reader.pull().await?;
            if !q.read_asset(&asset).await?.context("missing tombstone")?.snapshot.deleted || repository::fingerprint(&path)?.0!=hash {bail!("T4 tombstone/local preservation failed");}
            println!("T4 confirmed tombstone and exact media cleanup; other device retained local video");
            sync_engine::request_upload_with(&b,q.clone(),&asset)?;
            let upload=super::scheduler::operations(&b)?.into_iter().find(|o|o.asset_id.as_deref()==Some(&asset)&&o.kind=="upload"&&o.state=="pending").context("missing T4 reupload")?;
            b.backup_db(|db|{db.execute("UPDATE backup_operations SET state='running' WHERE operation_id=?",[&upload.operation_id])?;Ok(())})?;
            reader.execute_upload(&upload.operation_id).await?;
            if q.read_asset(&asset).await?.context("missing recreated asset")?.snapshot.deleted {bail!("T4 recreate failed");}
            delete
        };
        // Original device restarts an already confirmed old delete; the new incarnation must survive.
        a.backup_db(|db|{db.execute("UPDATE backup_operations SET state='running',phase='tombstone' WHERE operation_id=?",[&delete])?;Ok(())})?;
        super::sync_delete::execute(&a,p.as_ref(),&delete).await?;
        let recreated=p.read_asset(&asset).await?.context("missing recreation")?;
        p.verify_media(recreated.snapshot.media.context("missing recreated video")?).await?;
        println!("T4 explicit reupload survived retry of the old delete");
        let second=super::sync_delete::intent(&b,&q.scope().space_id,&asset)?;
        if second==delete {bail!("T4 reused deletion operation");}
        for _ in 0..3 {pass(&b,q.as_ref(),&second).await?;}
        for original in before {
            let after=p.read_asset(&original.snapshot.asset_id).await?.context("unrelated history missing")?;
            if serde_json::to_value(after.snapshot)?!=serde_json::to_value(original.snapshot)? {bail!("unrelated history changed");}
        }
        std::fs::write(config.work_directory.join("lifecycle-report.json"),serde_json::to_vec_pretty(&json!({"full_hash_before_release":true,"release_restore_sha256":hash,"other_device_local_preserved":true,"recreate_survived_old_delete":true,"exact_owned_cleanup":true,"unrelated_history_preserved":true}))?)?;
        println!("T4 second delete completed; prior assets, events and library preserved");
        Ok(())
    }.await;
    a.shutdown().await;
    b.shutdown().await;
    result
}
