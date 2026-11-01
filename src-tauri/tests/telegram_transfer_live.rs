//! Explicitly authorized live test only. Reads the app's existing dedicated vault namespace.
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{path::PathBuf, sync::Arc};
use yingqu_core::{
    backup::{
        model::AssetTranscript,
        provider::{MediaSource, ProviderType, Scope, VideoAttributes},
        telegram_session::Phase,
        telegram_transfer::{RestoreRequest, Target, UploadRequest},
    },
    desktop_core::Core,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    channel_name: String,
    data_directory: PathBuf,
    work_directory: PathBuf,
    #[serde(default)]
    video_metadata: Option<serde_json::Value>,
}
#[derive(Serialize, Deserialize)]
struct Run {
    target: Target,
    asset_id: String,
    operation_id: String,
}

#[tokio::test]
#[ignore = "requires explicit channel upload authorization and an existing dedicated app session"]
async fn telegram_transfer_live_original_bytes_and_restart_recovery() -> Result<()> {
    let config: Config = serde_json::from_slice(&std::fs::read(
        std::env::var_os("YINGQU_TELEGRAM_TRANSFER_TEST_CONFIG")
            .context("set explicit ignored test config path")?,
    )?)?;
    if config.data_directory.file_name().and_then(|v| v.to_str())
        != Some("me.disign.yingqu.telegram-validation")
        || config.channel_name.trim().is_empty()
        || !config.data_directory.join("yingqu.sqlite").is_file()
    {
        bail!("dedicated existing validation directory and channel are required");
    }
    let core = open(&config)?;
    let result = transfer(&core, &config).await;
    core.shutdown().await;
    drop(core);
    result?;

    // A new Core and vault load must recover the same receipts, without publishing more messages.
    let core = open(&config)?;
    let result = transfer(&core, &config).await;
    core.shutdown().await;
    result
}
fn open(config: &Config) -> Result<Arc<Core>> {
    Core::open(
        &config.data_directory.join("yingqu.sqlite"),
        config.work_directory.clone(),
        "unused-engine".into(),
        vec![],
        |_| {},
    )
}
async fn transfer(core: &Arc<Core>, config: &Config) -> Result<()> {
    let device_id = core.telegram_storage_device_id()?;
    let view = core.telegram_storage.resume(device_id.clone()).await?;
    if !matches!(view.phase, Phase::Ready) {
        bail!("existing session did not restore Ready");
    }
    let mut login_id = view.operation_id.context("missing login operation")?;
    let account = view.account.context("missing verified account")?;
    let channels = core.telegram_storage.channels(&login_id).await?;
    let matching: Vec<_> = channels
        .iter()
        .filter(|c| c.title == config.channel_name)
        .collect();
    if matching.len() != 1 || !matching[0].can_post {
        bail!("expected exactly one writable private broadcast channel with requested title; exact_matches={}, writable={}", matching.len(), matching.first().is_some_and(|c| c.can_post));
    }
    std::fs::create_dir_all(&config.work_directory)?;
    let manifest = config.work_directory.join("run.json");
    let run = if manifest.exists() {
        serde_json::from_slice::<Run>(&std::fs::read(&manifest)?)?
    } else {
        let run = Run {
            target: Target {
                scope: Scope {
                    space_id: uuid::Uuid::new_v4().to_string(),
                    library_id: uuid::Uuid::new_v4().to_string(),
                    provider_type: ProviderType::Telegram,
                    protocol: 2,
                },
                account_id: account.id.clone(),
                channel_id: matching[0].id.clone(),
            },
            asset_id: uuid::Uuid::new_v4().to_string(),
            operation_id: uuid::Uuid::new_v4().to_string(),
        };
        std::fs::write(&manifest, serde_json::to_vec_pretty(&run)?)?;
        run
    };
    if run.target.account_id != account.id || run.target.channel_id != matching[0].id {
        bail!("saved run is bound to a different account or channel");
    }
    let input = config.work_directory.join("input");
    let video_hash = format!(
        "{:x}",
        Sha256::digest(std::fs::read(input.join("video.mp4"))?)
    );
    let transcript = AssetTranscript {
        asset_id: run.asset_id.clone(),
        content_sha256: Some(video_hash),
        text: "映取 Telegram T2 合成传输验证。\n中文、English、123 与换行应原样恢复。".into(),
        model_revision: "synthetic-test".into(),
        model_name: Some("synthetic-test".into()),
        pipeline_revision: "telegram-t2".into(),
        completed_at: 1,
    };
    let transcript_path = input.join("transcript.json");
    if !transcript_path.exists() {
        std::fs::write(&transcript_path, serde_json::to_vec_pretty(&transcript)?)?;
    }
    let output = config
        .work_directory
        .join(format!("restore-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&output)?;
    let (_cancel, rx) = tokio::sync::watch::channel(false);
    let mut reports = vec![];
    for (kind, name, container) in [
        ("video", "video.mp4", "mp4"),
        ("cover", "cover.png", "png"),
        ("transcript", "transcript.json", "json"),
    ] {
        let path = input.join(name);
        let original = std::fs::read(&path)?;
        let sha256 = format!("{:x}", Sha256::digest(&original));
        let video = if kind == "video" {
            config.video_metadata.as_ref().map(|v| VideoAttributes::from_metadata(v).context("invalid video presentation fixture")).transpose()?
        } else { None };
        let video_media_requested = video.is_some();
        let receipt = core
            .telegram_storage
            .upload_media(
                core,
                UploadRequest {
                    login_id: login_id.clone(),
                    target: run.target.clone(),
                    source: MediaSource {
                        video,
                        base_revision: None,
                        recreate_revision: None,
                        asset_id: run.asset_id.clone(),
                        kind: kind.into(),
                        path: path.clone(),
                        bytes: original.len() as u64,
                        sha256: sha256.clone(),
                        container: container.into(),
                        operation_id: run.operation_id.clone(),
                        device_id: device_id.clone(),
                        control: None,
                    },
                },
                rx.clone(),
            )
            .await?;
        let destination = output.join(name);
        core.telegram_storage
            .restore_media(
                core,
                RestoreRequest {
                    login_id: login_id.clone(),
                    target: run.target.clone(),
                    reference: receipt.reference.clone(),
                    destination: destination.clone(),
                },
                rx.clone(),
            )
            .await?;
        if std::fs::read(&destination)? != original || std::fs::read(path)? != original {
            bail!("original/restore byte mismatch for {kind}");
        }
        // The existing destination must stay intact.
        let error = core
            .telegram_storage
            .restore_media(
                core,
                RestoreRequest {
                    login_id: login_id.clone(),
                    target: run.target.clone(),
                    reference: receipt.reference.clone(),
                    destination: destination.clone(),
                },
                rx.clone(),
            )
            .await
            .unwrap_err();
        if error.to_string() != "TELEGRAM_DESTINATION_EXISTS"
            || std::fs::read(&destination)? != original
        {
            bail!("no-clobber validation failed");
        }
        // A failed transfer closes its sender; resume before the next transport request.
        login_id = core
            .telegram_storage
            .resume(device_id.clone())
            .await?
            .operation_id
            .context("missing resumed operation")?;
        reports.push(serde_json::json!({"kind":kind,"bytes":original.len(),"sha256":sha256,"reference":receipt.reference,"restored":true,"video_media_requested":video_media_requested}));
        println!(
            "{kind}: {} bytes, SHA-256 matched, original retained, no-clobber passed",
            original.len()
        );
    }
    std::fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&reports)?,
    )?;
    Ok(())
}
