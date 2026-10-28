//! Opt-in WB-304 real-engine verification in a marked, repository-local profile.
use anyhow::{ensure, Context, Result};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{path::PathBuf, sync::Arc, time::Duration};
use yingqu_core::{
    desktop_core::{
        keyframes::{KeyframeRequest, KeyframeSnapshot},
        Core,
    },
    library::ClipInput,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    root: PathBuf,
    video: PathBuf,
    program: String,
    args: Vec<String>,
    position_ms: u64,
    start_ms: u64,
    end_ms: u64,
}
async fn done(core: &Arc<Core>) -> Result<KeyframeSnapshot> {
    tokio::time::timeout(Duration::from_secs(240), async {
        loop {
            let value = core.keyframe_operation().context("missing operation")?;
            if value.state != "running" {
                return Ok(value);
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
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
        "repository-local profile only"
    );
    let marker: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("fixture.json"))?)?;
    ensure!(
        marker == serde_json::json!({"purpose":"wb304-synthetic"}),
        "fixture marker required"
    );
    let db = root.join("yingqu.sqlite");
    ensure!(!db.exists(), "fresh profile required");
    let before = Sha256::digest(std::fs::read(&config.video)?);
    let open = || {
        Core::open(
            &db,
            root.join("downloads"),
            config.program.clone(),
            config.args.clone(),
            |_| {},
        )
    };
    let core = open()?;
    let item = core.import_local_video(config.video.clone()).await?;
    core.start_keyframes(
        &item.asset_id,
        KeyframeRequest::Position {
            file_id: item.file_id.clone(),
            position_ms: config.position_ms,
        },
    )?;
    let first = done(&core).await?;
    ensure!(
        first.state == "completed",
        "position error: {:?}",
        first.error
    );
    let clip = core.create_library_clip(
        &item.asset_id,
        &ClipInput {
            file_id: item.file_id.clone(),
            title: "真实抽帧区间".into(),
            reason: "合成验证".into(),
            start_ms: config.start_ms,
            end_ms: config.end_ms,
            playable: yingqu_core::library::ClipRange {
                start_ms: config.start_ms,
                end_ms: config.end_ms,
            },
            document_id: None,
            quote: None,
        },
    )?;
    core.start_keyframes(
        &item.asset_id,
        KeyframeRequest::Clip {
            clip_id: clip.clone(),
            count: 3,
        },
    )?;
    let second = done(&core).await?;
    ensure!(
        second.state == "completed",
        "range error: {:?}",
        second.error
    );
    let frames = core.library_keyframes(&item.asset_id)?;
    ensure!(frames.len() == 4, "frame count");
    for frame in &frames {
        ensure!(
            core.library_keyframe_image(&item.asset_id, &frame.data.id)?
                .starts_with(b"\x89PNG"),
            "not PNG"
        );
    }
    let backup = root.join("images-without-video");
    core.backup_library(&backup, false, &mut |_| Ok(()))?;
    core.shutdown().await;
    drop(core);
    let reopened = open()?;
    ensure!(
        reopened.library_keyframes(&item.asset_id)?.len() == 4,
        "restart lost resources"
    );
    reopened.shutdown().await;
    drop(reopened);
    let restore = root.join("restore-profile");
    std::fs::create_dir(&restore)?;
    let restored = Core::open(
        &restore.join("yingqu.sqlite"),
        restore.join("downloads"),
        config.program,
        config.args,
        |_| {},
    )?;
    restored.restore_library(&backup, &restore.join("media"), &mut |_| Ok(()))?;
    let recovered = restored.library_keyframes(&item.asset_id)?;
    ensure!(
        recovered.len() == 4 && recovered.iter().all(|f| f.missing && f.source_changed),
        "missing media flags"
    );
    for frame in &recovered {
        ensure!(
            !restored
                .library_keyframe_image(&item.asset_id, &frame.data.id)?
                .is_empty(),
            "lost PNG"
        );
    }
    restored.shutdown().await;
    drop(restored);
    ensure!(
        Sha256::digest(std::fs::read(&config.video)?) == before,
        "source changed"
    );
    let report = serde_json::json!({"status":"PASSED","native_gui":false,"first":first,"range":second,"frames":frames,"reopened":true,"restored_without_media":recovered,"source_sha256":format!("{before:x}")});
    std::fs::write(
        root.join("report.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    println!("{}", root.join("report.json").display());
    Ok(())
}
