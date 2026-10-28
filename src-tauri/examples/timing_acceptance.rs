//! WB-302 opt-in Windows/Core acceptance using an isolated marked profile and real engine.
use anyhow::{ensure, Context, Result};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{path::PathBuf, sync::Arc, time::Duration};
use yingqu_core::desktop_core::{transcription::Snapshot, Core};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    root: PathBuf,
    program: String,
    args: Vec<String>,
    video: PathBuf,
    backend: String,
}

async fn done(core: &Arc<Core>, asset: &str, file: &str) -> Result<Snapshot> {
    tokio::time::timeout(Duration::from_secs(600), async {
        loop {
            let value = core
                .asset_transcript(asset, file)?
                .context("missing operation")?;
            if value.state != "running" {
                return Ok(value);
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .context("acceptance timeout")?
}

#[tokio::main]
async fn main() -> Result<()> {
    let config: Config = serde_json::from_slice(&std::fs::read(
        std::env::args_os().nth(1).context("config required")?,
    )?)?;
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .canonicalize()?;
    let root = config.root.canonicalize()?;
    ensure!(
        root.starts_with(repo.join(".local").canonicalize()?),
        "isolated profile only"
    );
    let marker: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("fixture.json"))?)?;
    ensure!(
        marker == serde_json::json!({"purpose":"wb302-synthetic"}),
        "fixture marker required"
    );
    ensure!(
        matches!(config.backend.as_str(), "cpu" | "cuda"),
        "invalid backend"
    );
    let db = root.join("yingqu.sqlite");
    ensure!(!db.exists(), "fresh profile required");
    std::fs::write(
        root.join("runtime/fun-asr-nano-backend.json"),
        serde_json::to_vec(&serde_json::json!({"variant":config.backend}))?,
    )?;
    let before = Sha256::digest(std::fs::read(&config.video)?);
    let core = Core::open(
        &db,
        root.join("downloads"),
        config.program.clone(),
        config.args.clone(),
        |_| {},
    )?;
    let imported = core.import_local_video(config.video.clone()).await?;
    core.start_asset_transcription(&imported.asset_id, &imported.file_id, false)?;
    let first = done(&core, &imported.asset_id, &imported.file_id).await?;
    ensure!(
        first.state == "completed",
        "first operation: {:?}",
        first.error_code
    );
    let result = first.result.context("missing result")?;
    let timing = result.timing.context("missing timing")?;
    ensure!(
        !timing.segments.is_empty() && timing.time_base == "media_zero",
        "invalid timing"
    );
    ensure!(
        result.pipeline_revision.ends_with("media-zero-v2"),
        "wrong pipeline"
    );
    let old = core.library_documents(&imported.asset_id)?;
    ensure!(
        old.len() == 1 && old[0].version == 1,
        "unexpected initial versions"
    );
    let id = old[0].id.clone();
    ensure!(
        core.library_document(&imported.asset_id, &id)?
            .timing
            .as_ref()
            == Some(&timing),
        "database mismatch"
    );
    core.start_asset_transcription(&imported.asset_id, &imported.file_id, true)?;
    let second = done(&core, &imported.asset_id, &imported.file_id).await?;
    ensure!(
        second.state == "completed",
        "second operation: {:?}",
        second.error_code
    );
    ensure!(
        core.library_documents(&imported.asset_id)?.len() == 2,
        "retranscription must create a version"
    );
    ensure!(
        core.library_document(&imported.asset_id, &id)?
            .timing
            .as_ref()
            == Some(&timing),
        "old version changed"
    );
    core.shutdown().await;
    drop(core);
    let reopened = Core::open(
        &db,
        root.join("downloads"),
        config.program,
        config.args,
        |_| {},
    )?;
    let stored = reopened.library_document(&imported.asset_id, &id)?;
    ensure!(
        stored.timing.as_ref() == Some(&timing) && stored.source_changed == Some(false),
        "reopen binding mismatch"
    );
    ensure!(
        before == Sha256::digest(std::fs::read(&config.video)?),
        "source modified"
    );
    reopened.shutdown().await;
    let report = serde_json::json!({"status":"PASSED","backend":config.backend,"asset_id":imported.asset_id,"file_id":imported.file_id,
        "old_document_id":id,"timing":timing,"source_unchanged":true,"versions":2,"reopen_binding":true});
    std::fs::write(
        root.join("report.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    println!(
        "PASSED: {} real Core/engine, two immutable timed versions and restart",
        config.backend
    );
    Ok(())
}
