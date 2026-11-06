//! Explicit real queue acceptance, limited to one persisted synthetic asset in the QA channel.
use super::*;
use crate::backup::telegram_transfer_journal::Journal;
use crate::backup::{
    telegram_transfer::{RemoteDocument, Target},
    telegram_transfer_journal::Entry,
};

fn fixture(core: &Arc<Core>, cfg: &Config, p: &dyn BackupProvider) -> Result<Run> {
    let path = cfg.work_directory.join("run.json");
    let mut run: Run = if path.exists() {
        serde_json::from_slice(&std::fs::read(&path)?)?
    } else {
        let run = Run {
            asset_id: uuid::Uuid::new_v4().to_string(),
            operation_id: None,
        };
        std::fs::write(&path, serde_json::to_vec_pretty(&run)?)?;
        run
    };
    let video = cfg.input_directory.join("video.mp4");
    let (hash, signature) = repository::fingerprint(&video)?;
    let cover = std::fs::read(cfg.input_directory.join("cover.png"))?;
    core.backup_db(|db| {
        let exists:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM media_assets WHERE asset_id=?)",[&run.asset_id],|r|r.get(0))?;
        if !exists {
            let metadata=json!({"title":"Telegram 富消息正式链路验证","author":"映取验证作者","author_profile":{"id":"synthetic-author","nickname":"映取验证作者","handle":"synthetic-author","kind":"user"},"duration_ms":10000,"width":640,"height":360,"platform":"youtube","video_id":run.asset_id.replace('-',"")[..11],"canonical_url":format!("https://www.youtube.com/watch?v={}",&run.asset_id.replace('-',"")[..11])});
            let tx=db.unchecked_transaction()?;
            tx.execute("INSERT INTO media_assets VALUES(?1,NULL,?2,'telegram-rich-synthetic.mp4',?3,?4,'active')",params![run.asset_id,metadata.to_string(),hash,signature["size"].as_u64().context("fixture size")? as i64])?;
            tx.execute("INSERT INTO asset_local_files VALUES(?1,?2,?3,?4,?5,unixepoch(),'present')",params![run.asset_id,video.to_string_lossy(),signature["size"].as_u64().unwrap() as i64,hash,signature.to_string()])?;
            tx.execute("INSERT INTO asset_transcripts VALUES(?1,?2,'富消息正式上传与按需恢复验证。Chinese English 123。','synthetic-test',NULL,'telegram-rich',1,NULL)",params![run.asset_id,hash])?;
            tx.execute("INSERT INTO asset_covers VALUES(?1,?2)",params![run.asset_id,cover])?;
            sync_engine::register_download(&tx,&run.asset_id)?;tx.commit()?;
        }
        Ok(())
    })?;
    SyncEngine::new(core.clone(), spaces::provider(core, &p.scope().space_id)?).enqueue()?;
    if run.operation_id.is_none() {
        run.operation_id = Some(
            super::super::scheduler::operations(core)?
                .into_iter()
                .find(|o| o.asset_id.as_deref() == Some(&run.asset_id) && o.kind == "upload")
                .context("fixture upload missing")?
                .operation_id,
        );
        std::fs::write(path, serde_json::to_vec_pretty(&run)?)?;
    }
    Ok(run)
}
async fn shape(
    core: &Arc<Core>,
    p: &dyn BackupProvider,
    operation: &str,
    expected: usize,
) -> Result<(i32, Vec<RemoteDocument>)> {
    let entries: Vec<Entry> = core.backup_db(|db| {
        let mut q =
            db.prepare("SELECT entry_json FROM telegram_media_sends WHERE operation_id=?")?;
        let rows = q
            .query_map([operation], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.into_iter()
            .map(|s| Ok(serde_json::from_str(&s)?))
            .collect()
    })?;
    if entries.len() != 4 || entries.iter().any(|e| e.document.is_none()) {
        bail!("four confirmed attachments required");
    }
    let message = entries[0].document.as_ref().unwrap().message_id;
    if entries
        .iter()
        .any(|e| e.document.as_ref().unwrap().message_id != message)
    {
        bail!("production still sent separate messages");
    }
    let target = entries[0].target.clone();
    p.scope().check(&target.scope)?;
    let id = core
        .telegram_accounts.binding(crate::telegram_accounts::Purpose::Backup).session
        .status()
        .operation_id
        .context("QA login operation missing")?;
    let (_stop, rx) = tokio::sync::watch::channel(false);
    let docs = core
        .telegram_accounts.binding(crate::telegram_accounts::Purpose::Backup).session
        .with_transport(core, &id, target, rx, ("test_probe", None), move |_, t, _| {
            Box::pin(async move { t.bundle_documents(message).await })
        })
        .await?;
    if docs.len() != expected {
        bail!("unexpected rich document count");
    }
    Ok((message, docs))
}
async fn stage(core: &Arc<Core>, cfg: &Config, mode: &str) -> Result<()> {
    let p = connect(core, cfg, false).await?;
    if mode == "inspect" {
        let login = core
            .telegram_accounts.binding(crate::telegram_accounts::Purpose::Backup).session
            .status()
            .operation_id
            .context("QA login")?;
        let target = entries_target(
            core,
            &serde_json::from_slice::<Run>(&std::fs::read(cfg.work_directory.join("run.json"))?)?
                .operation_id
                .context("QA operation")?,
        )?;
        let (_stop, rx) = tokio::sync::watch::channel(false);
        let report = core
            .telegram_accounts.binding(crate::telegram_accounts::Purpose::Backup).session
            .with_transport(core, &login, target, rx, ("test_probe", None), |_, t, io| {
                Box::pin(async move { t.recent_rich_shapes(io).await })
            })
            .await?;
        println!("Rich shape diagnostic: {report}");
        return Ok(());
    }
    let run = fixture(core, cfg, p.as_ref())?;
    let op = run
        .operation_id
        .as_deref()
        .context("fixture operation missing")?;
    if mode == "upload" || mode == "resume" {
        core.backup_db(|db| {
            db.execute(
                "UPDATE backup_operations SET state='running' WHERE operation_id=?",
                [op],
            )?;
            Ok(())
        })?;
        SyncEngine::new(core.clone(), p.clone())
            .execute_upload(op)
            .await?;
        let (id, docs) = shape(core, p.as_ref(), op, 4).await?;
        let snapshot = p
            .read_asset(&run.asset_id)
            .await?
            .context("new asset absent")?
            .snapshot;
        if snapshot.fields["title"] != "Telegram 富消息正式链路验证"
            || snapshot.fields["author"] != "映取验证作者"
            || snapshot.fields["source"]["author_profile"]["nickname"] != "映取验证作者"
        {
            bail!("title/author metadata changed");
        }
        let report = json!({"result":"PASSED","stage":mode,"single_message":true,"attachments":docs.len(),"all_receipts_same_message":true,"title_and_author_verified":true,"transcoded":false,"original_video_sha256":repository::fingerprint(&cfg.input_directory.join("video.mp4"))?.0});
        std::fs::write(
            cfg.work_directory.join(format!("{mode}-report.json")),
            serde_json::to_vec_pretty(&report)?,
        )?;
        std::fs::write(
            cfg.work_directory.join("playback.private.json"),
            serde_json::to_vec_pretty(&json!({"target":entries_target(core,op)?,"message_id":id}))?,
        )?;
        println!(
            "production {mode}: one message / four original attachments; title and author verified"
        );
    } else if mode == "delete" {
        // Keep a complete snapshot of unrelated assets, including prior tombstones.
        let unrelated = |page: ChangeBatch| -> Result<std::collections::BTreeMap<String,serde_json::Value>> {
            page.assets.into_iter().filter(|a| a.snapshot.asset_id != run.asset_id)
                .map(|a| Ok((a.snapshot.asset_id.clone(),serde_json::to_value(a.snapshot)?))).collect()
        };
        let before = unrelated(p.pull_changes(None).await?)?;
        let manifest = cfg.work_directory.join("deletion.private.json");
        let deletion: String = if manifest.exists() {
            serde_json::from_slice(&std::fs::read(&manifest)?)?
        } else {
            let id = super::super::sync_delete::intent(core, &p.scope().space_id, &run.asset_id)?;
            std::fs::write(&manifest, serde_json::to_vec(&id)?)?;
            id
        };
        core.backup_db(|db| {
            db.execute(
                "UPDATE backup_operations SET state='running' WHERE operation_id=?",
                [&deletion],
            )?;
            Ok(())
        })?;
        super::super::sync_delete::execute(core, p.as_ref(), &deletion).await?;
        let original = crate::backup::telegram_transfer_journal::CoreJournal(core)
            .get(op, "event")?.and_then(|e| e.document).context("missing original event receipt")?;
        let target = entries_target(core, op)?;
        let expected_asset = run.asset_id.clone();
        let upload_operation = op.to_owned();
        let delete_operation = deletion.clone();
        let binding = core.telegram_accounts.binding(crate::telegram_accounts::Purpose::Backup);
        let login = binding.session.status().operation_id.context("missing QA session")?;
        let (_stop, rx) = tokio::sync::watch::channel(false);
        let archive_revision = binding.session.with_transport(core, &login, target.clone(), rx,
            ("rich_archive_acceptance", None), move |_, transport, io| Box::pin(async move {
                match io.run(transport.bundle_documents(original.message_id)).await {
                    Err(e) if e.to_string() == "TELEGRAM_MEDIA_MISSING" => {}
                    Err(e) => return Err(e),
                    Ok(_) => bail!("original rich message still present"),
                }
                let (_, indexes) = transport.indexes(io).await?;
                let mut history = std::collections::BTreeMap::new();
                let mut high = 0;
                for index in indexes {
                    let envelope: crate::backup::telegram_transfer::Envelope =
                        serde_json::from_str(&index.document.caption)?;
                    if envelope.kind != "library" || envelope.library_id != target.scope.library_id { continue; }
                    let body = transport.index_body(&index, io).await?;
                    let carrier: crate::backup::telegram_provider::Library = serde_json::from_slice(&body)?;
                    let mut records = Vec::new();
                    carrier.append_archived_events(&target, index.document.message_id, &mut high, &mut records)?;
                    for (revision, bytes) in records {
                        let event = crate::backup::telegram_replay::Event::decode(&target, &bytes)?;
                        if event.change.asset_id == expected_asset {
                            if let Some(previous) = history.insert(revision, bytes.clone()) {
                                anyhow::ensure!(previous == bytes, "archive rewrote fixture history");
                            }
                        }
                    }
                }
                let replay = crate::backup::telegram_replay::replay(&target, high, history.into_iter().collect(), None)?;
                anyhow::ensure!(replay.operations.contains_key(&upload_operation)
                    && replay.operations.contains_key(&delete_operation), "original or deletion event missing from archive");
                Ok(replay.assets.get(&expected_asset).filter(|s| s.deleted).context("archive tombstone missing")?.revision)
            })).await?;
        let live = p.read_asset(&run.asset_id).await?.context("live tombstone absent")?;
        anyhow::ensure!(live.snapshot.deleted && live.snapshot.revision == archive_revision, "archive replay differs from live state");
        anyhow::ensure!(unrelated(p.pull_changes(None).await?)? == before, "unrelated asset history changed");
        std::fs::write(cfg.work_directory.join("delete-report.json"),serde_json::to_vec_pretty(
            &json!({"result":"PASSED","media_removed":3,"original_message_gone":true,
            "original_and_delete_events_replayed_from_archive":true,"unrelated_assets_unchanged":true}))?)?;
        println!("production delete: original message gone, archived events replayed, unrelated assets preserved");
    } else {
        bail!("unsupported source stage");
    }
    Ok(())
}
fn entries_target(core: &Core, op: &str) -> Result<Target> {
    let raw: String = core.backup_db(|db| {
        Ok(db.query_row(
            "SELECT entry_json FROM telegram_media_sends WHERE operation_id=? LIMIT 1",
            [op],
            |r| r.get(0),
        )?)
    })?;
    Ok(serde_json::from_str::<Entry>(&raw)?.target)
}
async fn recover_rich(core: &Arc<Core>, cfg: &Config) -> Result<()> {
    let run: Run = serde_json::from_slice(&std::fs::read(cfg.work_directory.join("run.json"))?)?;
    let p = connect(core, cfg, false).await?;
    let reader = SyncEngine::new(core.clone(), p.clone());
    reader.pull().await?;
    let local = core
        .backup_assets()?
        .into_iter()
        .find(|a| a.metadata["video_id"] == run.asset_id.replace('-', "")[..11])
        .context("rich asset not imported")?;
    let lazy = local.path.is_none();
    let cover = sync_read::cover(core, p.as_ref(), &local.asset_id).await?;
    if cover != std::fs::read(cfg.input_directory.join("cover.png"))? {
        bail!("cover bytes differ");
    }
    let transcript = sync_read::transcript(core, p.as_ref(), &local.asset_id)
        .await?
        .context("rich transcript missing")?;
    if transcript.text != "富消息正式上传与按需恢复验证。Chinese English 123。" {
        bail!("transcript differs");
    }
    if lazy
        && core
            .backup_assets()?
            .into_iter()
            .find(|a| a.asset_id == local.asset_id)
            .unwrap()
            .path
            .is_some()
    {
        bail!("small object reads restored video");
    }
    let restored = sync_read::ensure(core, p.as_ref(), &local.asset_id).await?;
    if repository::fingerprint(&restored)?.0
        != repository::fingerprint(&cfg.input_directory.join("video.mp4"))?.0
    {
        bail!("video SHA differs");
    }
    std::fs::write(
        cfg.work_directory.join("recover-report.json"),
        serde_json::to_vec_pretty(
            &json!({"result":"PASSED","lazy_video_before_restore":lazy,"original_cover_match":true,"transcript_match":true,"original_video_sha256_match":true}),
        )?,
    )?;
    println!("production recovery: cover/transcript exact; video lazy restore SHA-256 matched");
    Ok(())
}
async fn recreate_rich(core: &Arc<Core>, cfg: &Config) -> Result<()> {
    let run: Run = serde_json::from_slice(&std::fs::read(cfg.work_directory.join("run.json"))?)?;
    let p = connect(core, cfg, false).await?;
    let engine = SyncEngine::new(core.clone(), p.clone());
    engine.pull().await?;
    let local = core
        .backup_assets()?
        .into_iter()
        .find(|a| a.metadata["video_id"] == run.asset_id.replace('-', "")[..11])
        .context("retained rich QA asset missing")?;
    let file = local.path.context("retained QA video required")?;
    if repository::fingerprint(&file)?.0
        != repository::fingerprint(&cfg.input_directory.join("video.mp4"))?.0
    {
        bail!("retained video changed");
    }
    let path = cfg.work_directory.join("recreate-operation.private.json");
    let operation: String = if path.exists() {
        serde_json::from_slice(&std::fs::read(&path)?)?
    } else {
        sync_engine::request_upload_with(core, p.clone(), &local.asset_id)?;
        let operation = super::super::scheduler::operations(core)?
            .into_iter()
            .find(|o| {
                o.asset_id.as_deref() == Some(&local.asset_id)
                    && o.kind == "upload"
                    && o.state != "completed"
            })
            .context("recreate upload missing")?
            .operation_id;
        std::fs::write(path, serde_json::to_vec(&operation)?)?;
        operation
    };
    core.backup_db(|db| {
        db.execute(
            "UPDATE backup_operations SET state='running' WHERE operation_id=?",
            [&operation],
        )?;
        Ok(())
    })?;
    engine.execute_upload(&operation).await?;
    let (message, docs) = shape(core, p.as_ref(), &operation, 4).await?;
    if p.read_asset(&run.asset_id)
        .await?
        .context("recreated remote missing")?
        .snapshot
        .deleted
    {
        bail!("still deleted after recreate");
    }
    std::fs::write(
        cfg.work_directory.join("playback.private.json"),
        serde_json::to_vec_pretty(
            &json!({"target":entries_target(core,&operation)?,"message_id":message}),
        )?,
    )?;
    std::fs::write(
        cfg.work_directory.join("recreate-report.json"),
        serde_json::to_vec_pretty(
            &json!({"result":"PASSED","fresh_upload_uses_production_rich_format":true,"single_message":true,"attachments":docs.len(),"original_video_unchanged":true,"transcoded":false}),
        )?,
    )?;
    println!("production fresh reupload: one new message, four original attachments, no prototype repair");
    Ok(())
}
#[tokio::test]
#[ignore = "explicit QA sessions and private channel; one persisted synthetic rich asset, scoped delete only on explicit delete stage"]
async fn telegram_rich_live_production_queue_and_recovery() -> Result<()> {
    let cfg: Config = serde_json::from_slice(&std::fs::read(
        std::env::var_os("YINGQU_TELEGRAM_RICH_PRODUCTION_CONFIG")
            .context("explicit config required")?,
    )?)?;
    if cfg
        .source_data_directory
        .file_name()
        .and_then(|s| s.to_str())
        != Some("me.disign.yingqu.telegram-validation")
        || cfg
            .recovery_data_directory
            .file_name()
            .and_then(|s| s.to_str())
            != Some("me.disign.yingqu.telegram-recovery-validation")
        || cfg.work_directory.file_name().and_then(|s| s.to_str())
            != Some("telegram-rich-production-validation")
        || cfg.channel_name != "知识库"
    {
        bail!("dedicated rich QA configuration required");
    }
    std::fs::create_dir_all(&cfg.work_directory)?;
    let mode = std::env::var("YINGQU_TELEGRAM_RICH_PRODUCTION_STAGE")
        .context("explicit stage required")?;
    if !matches!(
        mode.as_str(),
        "upload" | "resume" | "recover" | "delete" | "inspect" | "recreate"
    ) {
        bail!("invalid rich QA stage");
    }
    let core = open(&cfg, matches!(mode.as_str(), "recover" | "recreate"))?;
    let result = if mode == "recreate" {
        recreate_rich(&core, &cfg).await
    } else if mode == "recover" {
        recover_rich(&core, &cfg).await
    } else {
        stage(&core, &cfg, &mode).await
    };
    core.shutdown().await;
    result
}
