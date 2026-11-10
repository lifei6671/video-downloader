//! Opt-in sampled deletes on newly generated fixture IDs, sequentially on one authenticated session.
use super::*;
use rusqlite::OptionalExtension;
use tokio::io::AsyncReadExt;

// Respect explicit server cooldowns inside a sample. Keep their elapsed time in the durable result.
async fn wait_flood<T, F: std::future::Future<Output = Result<T>>>(
    stage: &str,
    marker: &std::path::Path,
    mut call: impl FnMut() -> F,
) -> Result<T> {
    for attempt in 0..=3 {
        match call().await {
            Err(error) if attempt < 3 => {
                let code = error.to_string();
                let Some(seconds) = code
                    .strip_prefix("TELEGRAM_FLOOD_WAIT:")
                    .and_then(|s| s.parse::<u64>().ok())
                else {
                    return Err(error);
                };
                std::fs::write(marker, format!("{stage} {seconds}"))?;
                if seconds > 900 {
                    return Err(error);
                }
                println!("explicit server cooldown: stage={stage} seconds={seconds}");
                tokio::time::sleep(std::time::Duration::from_secs(seconds + 1)).await;
            }
            result => return result,
        }
    }
    unreachable!("bounded retry")
}

#[tokio::test]
#[ignore = "explicit QA private channel and a new synthetic working directory; publishes and deletes ten new fixtures"]
async fn telegram_delete_perf_live_warm_registered() -> Result<()> {
    let cfg: Config = serde_json::from_slice(&std::fs::read(
        std::env::var_os("YINGQU_TELEGRAM_DELETE_PERF_CONFIG")
            .context("explicit QA config required")?,
    )?)?;
    if cfg.work_directory.file_name().and_then(|n| n.to_str())
        != Some("telegram-delete-perf-validation")
        || cfg
            .source_data_directory
            .file_name()
            .and_then(|n| n.to_str())
            != Some("me.disign.yingqu.telegram-validation")
        || cfg.input_directory.parent() != Some(&cfg.work_directory)
    {
        bail!("new, dedicated performance fixture directory required");
    }
    let case = std::env::var("YINGQU_TELEGRAM_DELETE_PERF_CASE").unwrap_or_else(|_| "warm".into());
    anyhow::ensure!(
        matches!(
            case.as_str(),
            "warm" | "cold" | "missing-map" | "unregistered" | "concurrent-read"
        ),
        "invalid perf case"
    );
    let samples = cfg.work_directory.join("samples");
    if samples.exists() {
        anyhow::ensure!(
            std::env::var("YINGQU_TELEGRAM_DELETE_PERF_RESUME").as_deref() == Ok("1"),
            "explicit resume required"
        );
    } else {
        std::fs::create_dir(&samples)?;
    }
    let core = open(&cfg, false)?;
    let result = async {
        let provider = wait_flood("connect", &cfg.work_directory.join("connect-retry.txt"), || connect(&core, &cfg, false)).await?;
        if !provider.capabilities().can_delete_owned_media {
            bail!("dedicated channel delete permission required");
        }
        let mut clean = 0;
        for sample in 1..=30 {
            if clean == 10 { break; }
            let dir = samples.join(format!("{sample:02}"));
            let input = dir.join("input");
            let timing = dir.join("timings.json");
            let marker = dir.join("retry.txt");
            if timing.exists() {
                let saved: serde_json::Value = serde_json::from_slice(&std::fs::read(&timing)?)?;
                anyhow::ensure!(saved["sample"] == sample && saved["case"].as_str().unwrap_or("warm") == case, "mixed fixture cases");
                if saved["recovered"].as_bool() != Some(true) { clean += 1; }
                continue;
            }
            let resumed = dir.exists();
            std::fs::create_dir_all(&input)?;
            if resumed { std::fs::write(&marker,"resumed incomplete sample")?; }
            if !dir.join("run.json").exists() {
                for name in ["video.mp4", "cover.png"] {
                    std::fs::copy(cfg.input_directory.join(name), input.join(name))?;
                }
            }
            let fixture = Config {
                channel_name: cfg.channel_name.clone(),
                source_data_directory: cfg.source_data_directory.clone(),
                recovery_data_directory: cfg.recovery_data_directory.clone(),
                work_directory: dir.clone(),
                input_directory: input.clone(),
            };
            let manifest = dir.join("run.json");
            let run: Run = if manifest.exists() {
                let saved = serde_json::from_slice::<Run>(&std::fs::read(&manifest)?)?;
                if saved.operation_id.is_none() { source(&core,&fixture,provider.as_ref())? } else { saved }
            } else { source(&core,&fixture,provider.as_ref())? };
            let operation = run.operation_id.context("new fixture upload missing")?;
            let remote = sync_read::remote_id(&core, &provider.scope().space_id, &run.asset_id)?;
            let ops = crate::backup::scheduler::operations(&core)?;
            let scoped = |o: &&crate::backup::scheduler::Operation| o.asset_id.as_deref() == Some(&run.asset_id)
                && o.space_id == provider.scope().space_id;
            let upload = ops.iter().filter(scoped).find(|o| o.operation_id == operation && o.kind=="upload")
                .context("manifest upload does not match this fixture in its QA database")?;
            let mut deletes = ops.iter().filter(scoped).filter(|o| o.kind=="delete");
            let existing_delete = deletes.next();
            anyhow::ensure!(deletes.next().is_none(),"ambiguous fixture deletion");
            let done = existing_delete.is_some_and(|d| d.payload["local_cleanup_done"] == true);
            if !done {
                let path: Option<String> = core.backup_db(|db| Ok(db.query_row(
                    "SELECT path FROM asset_local_files WHERE asset_id=?",[&run.asset_id],|r|r.get::<_,Option<String>>(0)).optional()?.flatten()))?;
                let expected = input.join("video.mp4");
                let local_finish = resumed && existing_delete.is_some_and(|d| matches!(d.phase.as_str(), "local_cleanup" | "finalizing"));
                let owned = match path {
                    Some(path) => std::path::Path::new(&path) == expected,
                    None => local_finish && !expected.exists(),
                };
                anyhow::ensure!(owned, "only this sample's file may be deleted");
            }
            if !resumed && wait_flood("preflight", &marker, || provider.read_asset(&remote)).await?.is_some() {
                bail!("generated asset must not already exist on the channel");
            }
            if upload.state != "completed" {
                core.backup_db(|db| {
                    db.execute("UPDATE backup_operations SET state='running',attempt=attempt+1 WHERE operation_id=?", [&operation])?;
                    Ok(())
                })?;
                let engine = SyncEngine::new(core.clone(), provider.clone());
                wait_flood("upload", &marker, || engine.execute_upload(&operation)).await?;
            }
            let mut active_download = None;
            let mut concurrent_metrics = serde_json::Value::Null;
            if existing_delete.is_none() && case != "unregistered" {
                let saved=wait_flood("registered", &marker, || provider.read_asset(&remote)).await?.context("registered asset missing")?;
                anyhow::ensure!(!saved.snapshot.deleted && saved.snapshot.media.is_some(),"registered reference required");
                if case == "concurrent-read" {
                    let reference = saved.snapshot.media.context("run-owned video required")?;
                    anyhow::ensure!(reference.bytes > 65_537, "concurrent fixture must exceed the stream buffer");
                    let started = Instant::now();
                    let mut stream = provider.download_media(reference.clone(), 0).await?;
                    // The first media byte witnesses an active transfer on the read lane.
                    let first = stream.reader.read_u8().await?;
                    active_download = Some((stream.reader, reference, first, started));
                }
            }
            let stray_receipt = dir.join("unregistered-reference.private.json");
            let stray: Option<MediaRef> = if stray_receipt.exists() {
                Some(serde_json::from_slice(&std::fs::read(&stray_receipt)?)?)
            } else if case == "unregistered" && existing_delete.is_none() {
                let saved = wait_flood("registered", &marker, || provider.read_asset(&remote)).await?.context("newly uploaded asset not found")?;
                anyhow::ensure!(!saved.snapshot.deleted && saved.snapshot.media.is_some(), "registered reference required");
                // This new upload belongs to this run's asset but is deliberately never published as a reference.
                let path = input.join("video.mp4");
                let (sha256, signature) = repository::fingerprint(&path)?;
                let id_path = dir.join("unregistered-operation.private.txt");
                let id = if id_path.exists() { std::fs::read_to_string(&id_path)? } else {
                    let id = uuid::Uuid::new_v4().to_string();
                    std::fs::write(&id_path,&id)?; id
                };
                let device = core.telegram_storage_device_id()?;
                let bytes = signature["size"].as_u64().context("fixture size missing")?;
                let receipt = wait_flood("unregistered_upload", &marker, || provider.upload_media(MediaSource {
                    video: None, base_revision: Some(saved.snapshot.revision), recreate_revision: None,
                    asset_id: remote.clone(), kind: "video".into(), path: path.clone(),
                    bytes, sha256: sha256.clone(),
                    container: "mp4".into(), operation_id: id.clone(),
                    device_id: device.clone(), control: None,
                }, None)).await?;
                anyhow::ensure!(saved.snapshot.media.as_ref() != Some(&receipt.reference), "orphan must have a separate location");
                std::fs::write(&stray_receipt,serde_json::to_vec(&receipt.reference)?)?;
                Some(receipt.reference)
            } else { None };
            let history: (i64,i64) = core.backup_db(|db| Ok(db.query_row(
                "SELECT COUNT(DISTINCT message_id),COALESCE(SUM(length(index_body)),0) FROM telegram_mapped_documents WHERE space_id=?",
                [&provider.scope().space_id], |r| Ok((r.get(0)?,r.get(1)?)),
            )?))?;
            if case == "missing-map" && existing_delete.is_none() {
                core.backup_db(|db| {
                    let removed = db.execute("DELETE FROM telegram_message_map WHERE space_id=?", [&provider.scope().space_id])?;
                    anyhow::ensure!(removed == 1 || resumed, "expected a populated map");
                    Ok(())
                })?;
            }
            if case == "cold" && existing_delete.is_none() {
                let binding = core.telegram_accounts.binding(crate::telegram_accounts::Purpose::Backup);
                let login = binding.session.status().operation_id.context("expected warm fixture upload session")?;
                let cleared = binding.session.cancel(&login).await?;
                anyhow::ensure!(matches!(cleared.phase, Phase::SignedOut), "sender pool must close before cold delete");
            }
            let delete = match existing_delete {
                Some(op) => op.operation_id.clone(),
                None => {
                    crate::backup::sync_delete::intent(&core, &provider.scope().space_id, &run.asset_id)?
                }
            };
            if !done {
                core.backup_db(|db| {
                    let claimed = db.execute("UPDATE backup_operations SET state='running',attempt=attempt+1 WHERE operation_id=? AND state IN ('running','pending','retry_wait','blocked')", [&delete])?;
                    anyhow::ensure!(claimed == 1, "runnable, run-owned delete expected");
                    Ok(())
                })?;
                let deleting = wait_flood("delete", &marker, || crate::backup::sync_delete::execute(&core, provider.as_ref(), &delete));
                if let Some((mut reader, reference, first, started)) = active_download {
                    let drain = async {
                        let mut hash = Sha256::new();
                        hash.update([first]);
                        let mut bytes = 1u64;
                        let mut buffer = vec![0u8; 65_536];
                        let failure = loop {
                            match reader.read(&mut buffer).await {
                                Ok(0) => break None,
                                Ok(count) => { hash.update(&buffer[..count]); bytes += count as u64; }
                                Err(error) => break Some(error.to_string()),
                            }
                        };
                        (bytes, format!("{:x}", hash.finalize()), failure, Instant::now())
                    };
                    let delete_done = async { deleting.await?; Ok::<_, anyhow::Error>(Instant::now()) };
                    let (deleted, (bytes, digest, error, finished)) = tokio::join!(delete_done, drain);
                    let deleted = deleted?;
                    let verified = error.is_none() && bytes == reference.bytes && digest == reference.sha256;
                    concurrent_metrics = json!({"bytes": bytes, "expected_bytes": reference.bytes, "elapsed_ms": finished.duration_since(started).as_millis(),
                        "sha256_verified":verified, "read_error":error, "delete_finished_before_transfer":deleted < finished});
                    std::fs::write(dir.join("concurrent-read.json"), serde_json::to_vec_pretty(&concurrent_metrics)?)?;
                    if !verified {
                        anyhow::ensure!(matches!(error.as_deref(), Some("TELEGRAM_MEDIA_MISSING" | "TELEGRAM_FILE_REFERENCE_EXPIRED")),
                            "same-asset read failed integrity or transport verification");
                        anyhow::ensure!(provider.verify_media(reference).await.is_err_and(|e| e.to_string()=="TELEGRAM_MEDIA_MISSING"),
                            "read invalidation needs a fresh missing-message confirmation");
                    }
                } else {
                    deleting.await?;
                }
            }
            let after = wait_flood("readback", &marker, || provider.read_asset(&remote)).await?.context("missing tombstone")?;
            if !after.snapshot.deleted || input.join("video.mp4").exists() {
                bail!("new fixture deletion did not converge");
            }
            if let Some(reference) = stray {
                match provider.verify_media(reference).await {
                    Err(error) if error.to_string()=="TELEGRAM_MEDIA_MISSING" => {}
                    Err(error) => return Err(error),
                    Ok(()) => bail!("run-owned unregistered video survived deletion"),
                }
            }
            let row: serde_json::Value = core.backup_db(|db| {
                let raw: String = db.query_row("SELECT payload_json FROM backup_operations WHERE operation_id=?", [&delete], |r| r.get(0))?;
                Ok(serde_json::from_str(&raw)?)
            })?;
            std::fs::write(&timing, serde_json::to_vec_pretty(&json!({
                "sample":sample, "case":case, "recovered":marker.exists(), "mapped_messages_before":history.0, "mapped_index_bytes_before":history.1, "initial_queue_ms":row["initial_queue_ms"],
                "foreground_elapsed_ms":row["foreground_elapsed_ms"],
                "phases":row["timings"]["foreground"], "concurrent_read":concurrent_metrics
            }))?)?;
            if !marker.exists() { clean += 1; }
            println!("{case} synthetic delete {sample} complete ({clean}/10 clean)");
        }
        anyhow::ensure!(clean == 10, "could not obtain ten clean samples in thirty attempts");
        Ok(())
    }.await;
    core.shutdown().await;
    result
}

// Queue-backed contention samples use a second, unrelated asset and the production backup service.
use crate::backup::{scheduler, sync_delete};
use std::{
    path::Path,
    time::{Duration, Instant},
};
use tokio::io::AsyncRead;

fn contention_fixture(
    core: &Arc<Core>,
    cfg: &Config,
    provider: &dyn BackupProvider,
    dir: &Path,
    media: &str,
) -> Result<Run> {
    std::fs::create_dir(dir)?; // A run only introduces new asset IDs. Never adopt an unrelated directory.
    let input = dir.join("input");
    std::fs::create_dir(&input)?;
    std::fs::copy(cfg.input_directory.join(media), input.join("video.mp4"))?;
    std::fs::copy(
        cfg.input_directory.join("cover.png"),
        input.join("cover.png"),
    )?;
    source(
        core,
        &Config {
            work_directory: dir.into(),
            input_directory: input,
            channel_name: cfg.channel_name.clone(),
            source_data_directory: cfg.source_data_directory.clone(),
            recovery_data_directory: cfg.recovery_data_directory.clone(),
        },
        provider,
    )
}

async fn observe_operation(
    core: &Core,
    id: &str,
    until: impl Fn(&scheduler::Operation) -> bool,
) -> Result<scheduler::Operation> {
    tokio::time::timeout(Duration::from_secs(1200), async {
        loop {
            let mut row = scheduler::operation(core, id)?;
            if let Some(progress) = scheduler::operations_for_display(core, Some(&row.space_id))?
                .into_iter()
                .find(|op| op.operation_id == id)
            {
                for field in ["completed_bytes", "transfer_started"] {
                    if let Some(value) = progress.payload.get(field) {
                        row.payload[field] = value.clone();
                    }
                }
            }
            if until(&row) {
                return Ok(row);
            }
            if matches!(row.state.as_str(), "blocked" | "completed" | "canceled") {
                bail!(
                    "fixture operation ended early: {} {} {:?}",
                    row.state,
                    row.phase,
                    row.last_error
                );
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .context("fixture operation exceeded 20 minutes")?
}

async fn fixture_media(
    core: &Arc<Core>,
    p: &dyn BackupProvider,
    fixture: &Run,
) -> Result<MediaRef> {
    let upload = fixture
        .operation_id
        .as_deref()
        .context("missing fixture operation")?;
    let row = observe_operation(core, upload, |o| o.state == "completed").await?;
    anyhow::ensure!(
        row.asset_id.as_deref() == Some(&fixture.asset_id) && row.space_id == p.scope().space_id
    );
    let remote = sync_read::remote_id(core, &p.scope().space_id, &fixture.asset_id)?;
    p.read_asset(&remote)
        .await?
        .filter(|s| !s.snapshot.deleted)
        .and_then(|s| s.snapshot.media)
        .context("fixture media was not published")
}

async fn drain_checked(
    reader: &mut (dyn AsyncRead + Unpin + Send),
    expected: &MediaRef,
    first: u8,
) -> Result<()> {
    let mut hash = Sha256::new();
    hash.update([first]);
    let mut bytes = 1u64;
    let mut buffer = vec![0u8; 65536];
    loop {
        let n = reader.read(&mut buffer).await?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
        bytes += n as u64;
    }
    anyhow::ensure!(
        bytes == expected.bytes && format!("{:x}", hash.finalize()) == expected.sha256,
        "independent download did not verify"
    );
    Ok(())
}

async fn measured_delete(
    core: &Arc<Core>,
    provider: &dyn BackupProvider,
    fixture: &Run,
) -> Result<(serde_json::Value, Instant)> {
    let begin = Instant::now();
    let op = sync_delete::delete(core, &provider.scope().space_id, &fixture.asset_id).await?;
    observe_operation(core, &op, |o| o.payload["local_cleanup_done"] == true).await?;
    let finished = Instant::now();
    let row = observe_operation(core, &op, |o| {
        o.payload["foreground_elapsed_ms"].is_number()
            && o.payload["timings"]["foreground"]["total"]["outcome"] == "ok"
    })
    .await?;
    Ok((
        json!({
            "native_command_to_local_ms":finished.duration_since(begin).as_millis(),
            "foreground_elapsed_ms":row.payload["foreground_elapsed_ms"],
            "initial_queue_ms":row.payload["initial_queue_ms"], "attempt":row.attempt,
            "phases":row.payload["timings"]["foreground"],
        }),
        finished,
    ))
}

// Interrupted runs can retire only assets with the matching private manifest, captured upload and original receipt.
async fn cleanup_contention(
    core: &Arc<Core>,
    p: &dyn BackupProvider,
    cfg: &Config,
    dir: &Path,
) -> Result<()> {
    let manifest = dir.join("run.json");
    if !manifest.exists() {
        return Ok(());
    }
    let run: Run = serde_json::from_slice(&std::fs::read(manifest)?)?;
    let id = run
        .operation_id
        .as_deref()
        .context("incomplete fixture manifest")?;
    let op = observe_operation(core, id, |o| o.state == "completed").await?;
    let path = dir.join("input").join("video.mp4");
    let captured_path = op.payload["capture"]["asset"]["path"]
        .as_str()
        .map(Path::new);
    anyhow::ensure!(
        op.kind == "upload"
            && op.asset_id.as_deref() == Some(&run.asset_id)
            && op.space_id == p.scope().space_id
            && captured_path == Some(path.as_path()),
        "fixture manifest did not match the original captured upload"
    );
    let receipt: UploadReceipt = serde_json::from_value(op.payload["video_receipt"].clone())?;
    let remote = op.payload["capture"]["remote"]
        .as_str()
        .context("invalid captured remote")?;
    let latest = p
        .read_asset(remote)
        .await?
        .context("fixture missing from replay")?;
    let local = core
        .backup_assets()?
        .into_iter()
        .find(|a| a.asset_id == run.asset_id)
        .context("missing local fixture row")?;
    if let Some(current) = local.path.filter(|v| v.exists()) {
        let current = current.canonicalize()?;
        let downloads = cfg.work_directory.join("source-downloads").canonicalize()?;
        anyhow::ensure!(
            (current.parent() == Some(downloads.as_path())
                || path.canonicalize().is_ok_and(|input| input == current))
                && repository::fingerprint(&current)?.0 == receipt.reference.sha256,
            "unowned local cleanup target"
        );
    }
    if latest.snapshot.deleted {
        let deletion = scheduler::operation(core, &latest.snapshot.operation_id)?;
        anyhow::ensure!(
            deletion.space_id == p.scope().space_id
                && deletion.kind == "delete"
                && deletion.asset_id.as_deref() == Some(&run.asset_id)
                && deletion.payload["remote_asset_id"] == remote,
            "tombstone must belong to this fixture's deletion"
        );
        observe_operation(core, &deletion.operation_id, |o| {
            o.payload["local_cleanup_done"] == true
        })
        .await?;
        return Ok(());
    }
    anyhow::ensure!(
        latest.snapshot.operation_id == id
            && latest.snapshot.media.as_ref() == Some(&receipt.reference),
        "refusing to retire a different publication"
    );
    measured_delete(core, p, &run).await?;
    Ok(())
}

#[tokio::test]
#[ignore = "explicit authorized QA private channel and new synthetic directory; deletes run-owned fixtures"]
async fn telegram_delete_perf_live_unrelated_transfer() -> Result<()> {
    let cfg: Config = serde_json::from_slice(&std::fs::read(
        std::env::var_os("YINGQU_TELEGRAM_DELETE_PERF_CONFIG")
            .context("explicit QA config required")?,
    )?)?;
    let case = std::env::var("YINGQU_TELEGRAM_DELETE_PERF_CASE")?;
    anyhow::ensure!(matches!(case.as_str(), "read100" | "restore" | "upload"));
    anyhow::ensure!(
        cfg.work_directory.file_name().and_then(|s| s.to_str())
            == Some("telegram-delete-perf-validation")
            && cfg.input_directory.parent() == Some(&cfg.work_directory)
            && cfg.source_data_directory.canonicalize()?.parent()
                == cfg.work_directory.canonicalize()?.parent()
            && cfg
                .source_data_directory
                .file_name()
                .and_then(|s| s.to_str())
                == Some("me.disign.yingqu.telegram-validation")
    );
    let small_hash = "b70dff6557209ff24e8dc7e28fa8ca8f895050b1dc0e532184e8e2de405a2219";
    let large_hash = "9516a32b35fc26c443ba7c02812ab5ba69425cd150e1f4e02f8ea57caba8b486";
    anyhow::ensure!(
        repository::fingerprint(&cfg.input_directory.join("video.mp4"))?.0 == small_hash
    );
    let background_hash = if case == "read100" {
        large_hash
    } else {
        small_hash
    };
    anyhow::ensure!(
        repository::fingerprint(&cfg.input_directory.join("background.mp4"))?.0 == background_hash
    );
    let samples = cfg.work_directory.join("unrelated-transfers");
    let cleanup = std::env::var("YINGQU_TELEGRAM_DELETE_PERF_CLEANUP").as_deref() == Ok("1");
    if cleanup {
        anyhow::ensure!(samples.is_dir());
    } else {
        std::fs::create_dir(&samples)?;
    }
    let core = open(&cfg, false)?;
    let result = async {
        if !cleanup {
            anyhow::ensure!(core.backup_assets()?.is_empty() && scheduler::operations(&core)?.is_empty(),
                "new contention samples require an empty isolated QA database");
        }
        let provider = connect(&core, &cfg, false).await?;
        let p = provider.as_ref();
        anyhow::ensure!(p.capabilities().can_delete_owned_media);
        core.start_backup_service();
        if cleanup {
            for index in 1..=10 {
                let dir = samples.join(format!("{index:02}"));
                for part in ["target", "upload"] { cleanup_contention(&core, p, &cfg, &dir.join(part)).await?; }
            }
            return cleanup_contention(&core, p, &cfg, &samples.join("background")).await;
        }
        let shared = if case != "upload" {
            let bg = contention_fixture(&core, &cfg, p, &samples.join("background"), "background.mp4")?;
            let reference = fixture_media(&core, p, &bg).await?;
            anyhow::ensure!(reference.sha256 == background_hash);
            Some((bg, reference))
        } else { None };
        if case == "read100" {
            let (_, reference) = shared.as_ref().context("missing read reference")?;
            let mut solo = vec![];
            // Finish every control measurement before creating the first delete or deferred recheck.
            for index in 1..=10 {
                let started = Instant::now();
                let mut media = p.download_media(reference.clone(), 0).await?;
                let first = media.reader.read_u8().await?;
                let first_byte = Instant::now();
                drain_checked(media.reader.as_mut(), reference, first).await?;
                let finished = Instant::now();
                solo.push(json!({"sample":index, "bytes":reference.bytes, "sha256_verified":true,
                    "transfer_ms":finished.duration_since(started).as_millis(),
                    "first_byte_to_eof_ms":finished.duration_since(first_byte).as_millis()}));
                std::fs::write(samples.join("solo-read100.json"), serde_json::to_vec_pretty(&solo)?)?;
            }
        }
        for index in 1..=10 {
            let dir = samples.join(format!("{index:02}"));
            std::fs::create_dir(&dir)?;
            let target_dir = dir.join("target");
            let fg = contention_fixture(&core, &cfg, p, &target_dir, "video.mp4")?;
            anyhow::ensure!(fixture_media(&core, p, &fg).await?.sha256 == small_hash);
            let remote = sync_read::remote_id(&core, &p.scope().space_id, &fg.asset_id)?;
            let start = Instant::now();
            let (mut metrics, deleted, transferred, bytes) = match case.as_str() {
                "read100" => {
                    let (_, reference) = shared.as_ref().context("missing read reference")?;
                    anyhow::ensure!(reference.bytes == 100 * 1024 * 1024);
                    let mut stream = p.download_media(reference.clone(), 0).await?;
                    let first = stream.reader.read_u8().await?;
                    let first_byte = Instant::now();
                    let copy = async {
                        drain_checked(stream.reader.as_mut(), reference, first).await?;
                        Ok::<_, anyhow::Error>(Instant::now())
                    };
                    let ((mut metrics, deleted), transferred) = tokio::try_join!(measured_delete(&core, p, &fg), copy)?;
                    metrics["first_byte_to_eof_ms"] = json!(transferred.duration_since(first_byte).as_millis());
                    metrics["sha256_verified"] = true.into();
                    (metrics, deleted, transferred, reference.bytes)
                }
                "restore" => {
                    let (bg, reference) = shared.as_ref().context("missing restore reference")?;
                    // Only detach this QA row. A previous restored file is left intact.
                    core.backup_db(|db| {
                        let changed = db.execute("UPDATE asset_local_files SET path=NULL,local_state='missing' WHERE asset_id=?1 AND sha256=?2",
                            params![bg.asset_id, reference.sha256])?;
                        anyhow::ensure!(changed == 1, "QA fixture did not have the expected local file");
                        Ok(())
                    })?;
                    let restore = sync_read::ensure(&core, p, &bg.asset_id);
                    tokio::pin!(restore);
                    let transferring = async {
                        loop {
                            let active: bool = core.backup_db(|db| Ok(db.query_row("SELECT EXISTS(SELECT 1 FROM backup_operations
                                WHERE asset_id=? AND kind='restore' AND phase='downloading' AND state='running' AND json_extract(payload_json,'$.completed_bytes')>0)",
                                [&bg.asset_id], |r| r.get(0))?))?;
                            if active { return Ok::<_, anyhow::Error>(()); }
                            tokio::time::sleep(Duration::from_millis(50)).await;
                        }
                    };
                    tokio::select! {
                        r = &mut restore => bail!("restore ended before progress witness: {:?}",r.err()),
                        r = transferring => r?,
                    }
                    let restore = async {
                        let path = restore.await?;
                        let finished = Instant::now();
                        let downloads = cfg.work_directory.join("source-downloads").canonicalize()?;
                        anyhow::ensure!(path.canonicalize()?.parent() == Some(downloads.as_path())
                            && repository::fingerprint(&path)?.0 == reference.sha256, "restored file did not verify");
                        Ok::<_, anyhow::Error>(finished)
                    };
                    let ((metrics, deleted), transferred) = tokio::try_join!(measured_delete(&core, p, &fg), restore)?;
                    (metrics, deleted, transferred, reference.bytes)
                }
                _ => {
                    let bg = contention_fixture(&core, &cfg, p, &dir.join("upload"), "background.mp4")?;
                    let op = bg.operation_id.as_deref().context("missing unrelated upload")?;
                    observe_operation(&core, op, |o| o.state == "running" && o.payload["completed_bytes"].as_u64().unwrap_or(0)>0).await?;
                    let uploading = async {
                        observe_operation(&core, op, |o| o.state == "completed").await?;
                        Ok::<_, anyhow::Error>(Instant::now())
                    };
                    let ((metrics, deleted), transferred) = tokio::try_join!(measured_delete(&core, p, &fg), uploading)?;
                    let reference = fixture_media(&core, p, &bg).await?;
                    anyhow::ensure!(reference.sha256 == background_hash);
                    p.verify_media(reference.clone()).await?;
                    (metrics, deleted, transferred, reference.bytes)
                }
            };
            metrics["sample"] = index.into();
            metrics["case"] = case.clone().into();
            metrics["transfer_bytes"] = bytes.into();
            metrics["transfer_ms"] = json!(transferred.duration_since(start).as_millis());
            metrics["delete_finished_before_transfer"] = (deleted < transferred).into();
            anyhow::ensure!(!target_dir.join("input/video.mp4").exists()
                && p.read_asset(&remote).await?.is_some_and(|s| s.snapshot.deleted), "target did not converge");
            std::fs::write(dir.join("timings.json"), serde_json::to_vec_pretty(&metrics)?)?;
            println!("{case} verified {index}/10 synthetic deletes");
        }
        if case == "upload" {
            for index in 1..=10 {
                cleanup_contention(&core, p, &cfg, &samples.join(format!("{index:02}/upload"))).await?;
            }
        } else {
            cleanup_contention(&core, p, &cfg, &samples.join("background")).await?;
        }
        Ok(())
    }.await;
    core.shutdown().await;
    result
}
