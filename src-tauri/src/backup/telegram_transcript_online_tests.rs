//! Explicit QA acceptance for saving an ASR result after the video backup is complete.
use super::*;
use crate::backup::{telegram_transfer::Target, telegram_transfer_journal::Entry};
use rusqlite::OptionalExtension;
use std::collections::BTreeMap;

#[derive(Serialize, Deserialize)]
struct MergeRun {
    asset: String,
    video_operation: Option<String>,
    message: Option<i32>,
    updates: BTreeMap<String, String>,
}
fn save_run(cfg: &Config, name: &str, run: &MergeRun) -> Result<()> {
    std::fs::write(
        cfg.work_directory.join(name),
        serde_json::to_vec_pretty(run)?,
    )?;
    Ok(())
}
fn entries(core: &Core, operation: &str) -> Result<Vec<Entry>> {
    core.backup_db(|db| {
        let mut q =
            db.prepare("SELECT entry_json FROM telegram_media_sends WHERE operation_id=?")?;
        let rows = q
            .query_map([operation], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.into_iter()
            .map(|s| Ok(serde_json::from_str(&s)?))
            .collect()
    })
}
async fn presentation(core: &Arc<Core>, target: Target, message: i32) -> Result<serde_json::Value> {
    let login = core
        .telegram_accounts.binding(crate::telegram_accounts::Purpose::Backup).session
        .status()
        .operation_id
        .context("QA login missing")?;
    let (_stop, rx) = tokio::sync::watch::channel(false);
    core.telegram_accounts.binding(crate::telegram_accounts::Purpose::Backup).session
        .with_transport(core, &login, target, rx, ("test_probe", None), move |_, t, _| {
            Box::pin(async move { t.bundle_presentation(message).await })
        })
        .await
}
fn open_current_qa(cfg: &Config, final_schema: bool) -> Result<Arc<Core>> {
    let original = rusqlite::Connection::open_with_flags(
        cfg.source_data_directory.join("yingqu.sqlite"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    let device = repository::device_id(&original)?;
    let root = cfg.work_directory.join(if final_schema {
        "source-business-final"
    } else {
        "source-business"
    });
    std::fs::create_dir_all(root.join("downloads"))?;
    let core = Core::open(
        &root.join("yingqu.sqlite"),
        root.join("downloads"),
        "unused-engine".into(),
        vec![],
        |_| {},
    )?;
    core.backup_db(|db| {
        db.execute("UPDATE backup_device SET device_id=? WHERE id=1", [device])?;
        Ok(())
    })?;
    core.set_directory(root.join("downloads"))?;
    Ok(core)
}
async fn source_stage(core: &Arc<Core>, cfg: &Config, mode: &str) -> Result<()> {
    let p = connect(core, cfg, false)
        .await
        .context("connect QA provider")?;
    let engine = SyncEngine::new(core.clone(), p.clone());
    let manifest = "run.private.json";
    let mut run: MergeRun = if cfg.work_directory.join(manifest).exists() {
        serde_json::from_slice(&std::fs::read(cfg.work_directory.join(manifest))?)?
    } else {
        let asset = uuid::Uuid::new_v4().to_string();
        let run = MergeRun {
            asset,
            video_operation: None,
            message: None,
            updates: BTreeMap::new(),
        };
        save_run(cfg, manifest, &run)?;
        run
    };
    {
        let path = cfg.work_directory.join(if mode == "recreate" {
            "retained-original.mp4"
        } else {
            "source-original.mp4"
        });
        if mode == "video" && !path.exists() {
            std::fs::copy(cfg.input_directory.join("video.mp4"), &path)?;
        }
        let (hash, sig) = repository::fingerprint(&path)?;
        if mode == "video" && !cfg.work_directory.join("retained-original.mp4").exists() {
            std::fs::copy(&path, cfg.work_directory.join("retained-original.mp4"))?;
        }
        let cover = std::fs::read(cfg.input_directory.join("cover.png"))?;
        core.backup_db(|db| {
            let exists:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM media_assets WHERE asset_id=?)",[&run.asset],|r|r.get(0))?;
            if !exists {
                let metadata=json!({"title":"Telegram 后提取文案合并验证","author":"映取验证作者","platform":"youtube","video_id":run.asset.replace('-',"")[..11],"canonical_url":format!("https://www.youtube.com/watch?v={}",&run.asset.replace('-',"")[..11]),"duration_ms":2000,"width":640,"height":360});
                let tx=db.unchecked_transaction()?;
                tx.execute("INSERT INTO media_assets VALUES(?1,NULL,?2,'telegram-later-transcript.mp4',?3,?4,'active')",params![run.asset,metadata.to_string(),hash,sig["size"].as_u64().unwrap() as i64])?;
                tx.execute("INSERT INTO asset_local_files VALUES(?1,?2,?3,?4,?5,unixepoch(),'present')",params![run.asset,path.to_string_lossy(),sig["size"].as_u64().unwrap() as i64,hash,sig.to_string()])?;
                tx.execute("INSERT INTO asset_covers VALUES(?1,?2)",params![run.asset,cover])?;
                sync_engine::register_download(&tx,&run.asset)?;tx.commit()?;
            } else if mode=="recreate" {
                db.execute("UPDATE asset_local_files SET path=?2,bytes=?3,sha256=?4,source_json=?5,local_state='present' WHERE asset_id=(SELECT local_asset_id FROM backup_space_assets WHERE space_id=?6 AND remote_asset_id=?1)",params![run.asset,path.to_string_lossy(),sig["size"].as_u64().unwrap() as i64,hash,sig.to_string(),p.scope().space_id])?;
            }Ok(())
        })?;
    }
    if mode == "video" {
        engine.enqueue()?;
        if run.video_operation.is_none() {
            run.video_operation = Some(
                super::super::scheduler::operations(core)?
                    .into_iter()
                    .find(|o| o.asset_id.as_deref() == Some(&run.asset) && o.kind == "upload")
                    .context("video upload missing")?
                    .operation_id,
            );
            save_run(cfg, manifest, &run)?;
        }
        let operation = run.video_operation.as_ref().unwrap();
        core.backup_db(|db| {
            db.execute(
                "UPDATE backup_operations SET state='running' WHERE operation_id=?",
                [operation],
            )?;
            Ok(())
        })?;
        engine.execute_upload(operation).await?;
        let evidence = entries(core, operation)?;
        if evidence.len() != 3
            || evidence
                .iter()
                .any(|e| e.envelope.kind == "transcript" || e.document.is_none())
        {
            bail!("video-first fixture must have exactly video/cover/event");
        }
        let message = evidence[0].document.as_ref().unwrap().message_id;
        if evidence
            .iter()
            .any(|e| e.document.as_ref().unwrap().message_id != message)
        {
            bail!("video-first split messages");
        }
        run.message = Some(message);
        save_run(cfg, manifest, &run)?;
        std::fs::write(
            cfg.work_directory.join("video-report.json"),
            serde_json::to_vec_pretty(
                &json!({"result":"PASSED","single_message":true,"initial_transcript_absent":true,"attachments":3}),
            )?,
        )?;
        println!("video-first backup complete: one message, no transcript");
        return Ok(());
    }
    if matches!(mode, "delete" | "recreate") {
        return lifecycle_stage(core, cfg, mode, p, &mut run, manifest).await;
    }
    if !matches!(mode, "extract1" | "extract2") {
        bail!("invalid source stage");
    }
    let snapshot = p
        .read_asset(&run.asset)
        .await
        .context("read existing QA backup")?
        .context("existing video backup missing")?
        .snapshot;
    let local_asset = core
        .backup_db(|db| repository::local_asset_id(db, &p.scope().space_id, &run.asset))?
        .context("local QA binding missing")?;
    let video = snapshot
        .media
        .as_ref()
        .context("existing remote video missing")?;
    let message = match &video.location {
        MediaLocation::TelegramMessage { message_id, .. } => i32::try_from(*message_id)?,
        _ => bail!("not Telegram"),
    };
    if run.message.is_some_and(|old| old != message) {
        bail!("original video message changed");
    }
    run.message = Some(message);
    let text = match mode {
        "extract1" => "先上传视频，后提取文案。第一版。",
        "extract2" => "先上传视频，后提取文案。第二版修订。",
        _ => bail!("invalid transcript stage"),
    };
    if !run.updates.contains_key(mode) {
        core.backup_db(|db| {
            let current:Option<String>=db.query_row("SELECT text FROM asset_transcripts WHERE asset_id=?",[&local_asset],|r|r.get(0)).optional()?;
            if current.as_deref()!=Some(text) {
                let tx=db.unchecked_transaction()?;
                tx.execute("INSERT OR REPLACE INTO asset_transcripts VALUES(?1,?2,?3,'synthetic-ASR-result',NULL,'telegram-later-transcript',?4,NULL)",params![local_asset,video.sha256,text,if mode=="extract2" {2} else {1}])?;
                sync_engine::mark_local(&tx,&local_asset,BTreeMap::new(),true)?;tx.commit()?;
            }Ok(())
        })?;
        engine.enqueue()?;
        let operation = super::super::scheduler::operations(core)?
            .into_iter()
            .find(|o| {
                o.asset_id.as_deref() == Some(&local_asset)
                    && o.kind == "upload"
                    && o.state != "completed"
            })
            .context("transcript update missing")?
            .operation_id;
        run.updates.insert(mode.into(), operation);
        save_run(cfg, manifest, &run)?;
    }
    let operation = &run.updates[mode];
    core.backup_db(|db| {
        db.execute(
            "UPDATE backup_operations SET state='running' WHERE operation_id=?",
            [operation],
        )?;
        Ok(())
    })?;
    engine
        .execute_upload(operation)
        .await
        .context("upload later QA transcript")?;
    let evidence = entries(core, operation)?;
    if evidence.len() != 2
        || evidence.iter().any(|e| {
            !matches!(e.envelope.kind.as_str(), "transcript" | "event")
                || e.document.as_ref().is_none_or(|d| d.message_id != message)
        })
    {
        bail!("later transcript did not merge into existing video message");
    }
    let current = p
        .read_asset(&run.asset)
        .await?
        .context("updated snapshot missing")?
        .snapshot;
    if current.media.as_ref() != Some(video) {
        bail!("video reference changed during transcript update");
    }
    let target = evidence[0].target.clone();
    let display = presentation(core, target.clone(), message)
        .await
        .context("fetch full QA rich presentation")?;
    let transcript = current
        .transcript
        .as_ref()
        .context("updated transcript absent")?;
    if display["visible_documents"]
        .as_array()
        .context("visible docs")?
        .iter()
        .filter(|d| d.as_i64() == transcript.document_id())
        .count()
        != 1
        || display["history_collapsed"] != true
    {
        bail!("latest transcript is not the single visible version");
    }
    let local = sync_read::transcript(core, p.as_ref(), &local_asset)
        .await?
        .context("updated text read missing")?;
    if local.text != text {
        bail!("updated transcript differs");
    }
    std::fs::write(
        cfg.work_directory.join(format!("{mode}-report.json")),
        serde_json::to_vec_pretty(
            &json!({"result":"PASSED","same_video_message":true,"no_video_reupload":true,"latest_transcript_match":true,"history_collapsed":true,"raw_documents":display["documents"],"anchor_chunks":display["anchors"]}),
        )?,
    )?;
    std::fs::write(
        cfg.work_directory.join("playback.private.json"),
        serde_json::to_vec_pretty(&json!({"target":target,"message_id":message}))?,
    )?;
    println!("{mode}: transcript and immutable event merged into original video message; latest text recovered");
    Ok(())
}
async fn lifecycle_stage(
    core: &Arc<Core>,
    cfg: &Config,
    mode: &str,
    p: std::sync::Arc<dyn BackupProvider>,
    run: &mut MergeRun,
    manifest: &str,
) -> Result<()> {
    let local = core
        .backup_db(|db| repository::local_asset_id(db, &p.scope().space_id, &run.asset))?
        .context("owned local QA asset missing")?;
    let engine = SyncEngine::new(core.clone(), p.clone());
    if !run.updates.contains_key(mode) {
        let operation = if mode == "delete" {
            super::super::sync_delete::intent(core, &p.scope().space_id, &local)?
        } else {
            sync_engine::request_upload_with(core, p.clone(), &local)?;
            engine.enqueue()?;
            super::super::scheduler::operations(core)?
                .into_iter()
                .find(|o| {
                    o.asset_id.as_deref() == Some(&local)
                        && o.kind == "upload"
                        && o.state != "completed"
                })
                .context("recreate operation missing")?
                .operation_id
        };
        run.updates.insert(mode.into(), operation);
        save_run(cfg, manifest, run)?;
    }
    let operation = &run.updates[mode];
    core.backup_db(|db| {
        db.execute(
            "UPDATE backup_operations SET state='running' WHERE operation_id=?",
            [operation],
        )?;
        Ok(())
    })?;
    if mode == "delete" {
        super::super::sync_delete::execute(core, p.as_ref(), operation).await?;
        let snapshot = p
            .read_asset(&run.asset)
            .await?
            .context("tombstone absent")?
            .snapshot;
        if !snapshot.deleted {
            bail!("virtual-revision delete did not converge");
        }
        let target = entries(
            core,
            run.video_operation
                .as_deref()
                .context("initial video evidence")?,
        )?[0]
            .target
            .clone();
        let message = run.message.context("owned message missing")?;
        let login = core
            .telegram_accounts.binding(crate::telegram_accounts::Purpose::Backup).session
            .status()
            .operation_id
            .context("QA login missing")?;
        let (_stop, rx) = tokio::sync::watch::channel(false);
        let docs = core
            .telegram_accounts.binding(crate::telegram_accounts::Purpose::Backup).session
            .with_transport(core, &login, target, rx, ("test_probe", None), move |_, t, _| {
                Box::pin(async move { t.bundle_documents(message).await })
            })
            .await?;
        if docs.len() != 3
            || docs.iter().any(|d| {
                serde_json::from_str::<crate::backup::telegram_transfer::Envelope>(&d.caption)
                    .is_ok_and(|e| e.kind != "event")
            })
        {
            bail!("delete failed to retain exactly the three immutable metadata versions");
        }
        std::fs::write(
            cfg.work_directory.join("delete-report.json"),
            serde_json::to_vec_pretty(
                &json!({"result":"PASSED","media_removed":true,"three_events_retained":true,"virtual_revision_delete_replayed":true}),
            )?,
        )?;
        println!("versioned-message delete: media removed, three immutable events and replayable tombstone retained");
    } else {
        engine.execute_upload(operation).await?;
        let evidence = entries(core, operation)?;
        if evidence.len() != 4 || evidence.iter().any(|e| e.document.is_none()) {
            bail!("recreate must have four confirmed attachments");
        }
        let message = evidence[0].document.as_ref().unwrap().message_id;
        if run.message == Some(message)
            || evidence
                .iter()
                .any(|e| e.document.as_ref().unwrap().message_id != message)
        {
            bail!("recreate must use one new message");
        }
        let snapshot = p
            .read_asset(&run.asset)
            .await?
            .context("recreated asset absent")?
            .snapshot;
        if snapshot.deleted {
            bail!("recreated asset still deleted");
        }
        run.message = Some(message);
        save_run(cfg, manifest, run)?;
        std::fs::write(
            cfg.work_directory.join("playback.private.json"),
            serde_json::to_vec_pretty(&json!({"target":evidence[0].target,"message_id":message}))?,
        )?;
        std::fs::write(
            cfg.work_directory.join("recreate-report.json"),
            serde_json::to_vec_pretty(
                &json!({"result":"PASSED","virtual_revision_recreation":true,"one_new_message":true,"four_original_attachments":true,"new_video_after_tombstone":true}),
            )?,
        )?;
        println!("virtual-revision recreation: original video and latest transcript uploaded in one new message");
    }
    Ok(())
}
async fn recover_fresh(core: &Arc<Core>, cfg: &Config, final_schema: bool) -> Result<()> {
    let run: MergeRun =
        serde_json::from_slice(&std::fs::read(cfg.work_directory.join("run.private.json"))?)?;
    let p = connect(core, cfg, false).await?;
    let freshdir = cfg.work_directory.join(if final_schema {
        "fresh-metadata-final"
    } else {
        "fresh-metadata"
    });
    std::fs::create_dir_all(&freshdir)?;
    let fresh = Core::open(
        &freshdir.join("yingqu.sqlite"),
        freshdir.clone(),
        "unused-engine".into(),
        vec![],
        |_| {},
    )?;
    let space = core
        .backup_db(repository::spaces)?
        .into_iter()
        .find(|s| s.space_id == p.scope().space_id)
        .context("QA space")?;
    let binding = crate::backup::telegram_spaces::binding(core, &space.space_id)?
        .context("QA channel binding")?
        .0;
    fresh.backup_db(|db| {
        repository::save_space(db,&space)?;
        db.execute("UPDATE backup_provider_config SET provider_type='telegram',profile='telegram',protocol=2,capabilities_json=?2 WHERE space_id=?1",params![space.space_id,serde_json::to_string(&p.capabilities())?])?;
        db.execute("INSERT INTO telegram_spaces(space_id,account_id,channel_id) VALUES(?1,?2,?3)",params![space.space_id,binding.account_id,binding.channel_id])?;
        db.execute("INSERT INTO telegram_space_labels VALUES(?1,?2)",params![space.space_id,cfg.channel_name])?;
        spaces::select_local(db,&space.space_id,true)
    })?;
    let result=async {
        SyncEngine::new(fresh.clone(),p.clone()).pull().await?;
        let local=fresh.backup_assets()?.into_iter().find(|a|a.metadata["video_id"]==run.asset.replace('-',"")[..11]).context("fresh reconstruction missing")?;
        if local.path.is_some() {bail!("fresh metadata replay downloaded video");}
        let transcript=sync_read::transcript(&fresh,p.as_ref(),&local.asset_id).await?.context("fresh transcript absent")?;
        if transcript.text!="先上传视频，后提取文案。第二版修订。" {bail!("fresh reconstruction selected old transcript");}
        let path=sync_read::ensure(&fresh,p.as_ref(),&local.asset_id).await?;
        if repository::fingerprint(&path)?.0!=repository::fingerprint(&cfg.input_directory.join("video.mp4"))?.0 {bail!("fresh video SHA mismatch");}
        std::fs::write(cfg.work_directory.join(if final_schema {"recovery-final-report.json"} else {"recovery-report.json"}),serde_json::to_vec_pretty(&json!({"result":"PASSED","fresh_business_database":true,"uses_existing_QA_authorization":true,"latest_transcript":true,"lazy_video_restore_sha256_match":true}))?)?;
        println!("fresh business database replay recovered latest transcript and lazy original video");
        Ok::<_,anyhow::Error>(())
    }.await;
    fresh.shutdown().await;
    result
}
#[tokio::test]
#[ignore = "explicit QA sessions/private channel; adds synthetic transcript edits to persisted owned QA video messages"]
async fn telegram_later_transcript_live_same_message_and_rebuild() -> Result<()> {
    let cfg: Config = serde_json::from_slice(&std::fs::read(
        std::env::var_os("YINGQU_TELEGRAM_TRANSCRIPT_TEST_CONFIG")
            .context("explicit transcript QA config")?,
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
            != Some("telegram-transcript-first-validation")
        || cfg.channel_name != "知识库"
    {
        bail!("dedicated QA paths/channel required");
    }
    std::fs::create_dir_all(&cfg.work_directory)?;
    let mode = std::env::var("YINGQU_TELEGRAM_TRANSCRIPT_TEST_STAGE")
        .context("explicit transcript stage")?;
    if !matches!(
        mode.as_str(),
        "video" | "extract1" | "extract2" | "recover" | "recover_final" | "delete" | "recreate"
    ) {
        bail!("invalid transcript stage");
    }
    let core = open_current_qa(&cfg, mode == "recover_final")?;
    let result = if matches!(mode.as_str(), "recover" | "recover_final") {
        recover_fresh(&core, &cfg, mode == "recover_final").await
    } else {
        source_stage(&core, &cfg, &mode).await
    };
    core.shutdown().await;
    result
}
