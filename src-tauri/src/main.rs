use anyhow::{bail, Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::PathBuf;
use tokio::io::{stdin, AsyncReadExt};
use yingqu_core::{
    files::{finalize, Video},
    python_bridge::PythonBridge,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Probe {
    inputs: Vec<String>,
    profiles: Value,
    output_dir: PathBuf,
}

async fn run(program: &str, args: &[String], config: Probe) -> Result<()> {
    if config.inputs.is_empty() || !config.output_dir.is_dir() {
        bail!("INVALID_PROBE_CONFIG");
    }
    let directory = std::fs::canonicalize(&config.output_dir).context("IO_ERROR")?;
    let mut failed = false;
    for (index, input) in config.inputs.iter().enumerate() {
        let id = format!("probe-{index}");
        let saved = async {
            let resolved = PythonBridge::operation(
                program,
                args,
                &id,
                "resolve_video",
                json!({"input":input,"profiles":config.profiles}),
                |_| {},
            )
            .await?;
            let video: Video = serde_json::from_value(resolved["result"].clone())
                .context("INVALID_ENGINE_RESULT")?;
            video.validate()?;
            println!(
                "{}",
                json!({"id":id,"event":"resolved","platform":video.platform,
                              "video_id":video.video_id,"canonical_url":video.canonical_url})
            );
            let staging = tempfile::Builder::new()
                .prefix("yingqu-")
                .tempdir_in(&directory)
                .context("IO_ERROR")?;
            let result = PythonBridge::operation(
                program,
                args,
                &id,
                "download_video",
                json!({"input":video.canonical_url,"profiles":config.profiles,
                       "expected":{"platform":video.platform,"video_id":video.video_id},
                       "staging_dir":staging.path()}),
                |event| println!("{event}"),
            )
            .await;
            let result =
                result.and_then(|value| finalize(&video, &value, staging.path(), &directory));
            // The child has exited before any cleanup. Surface cleanup failure.
            staging.close().context("IO_ERROR: staging cleanup")?;
            result
        }
        .await;
        match saved {
            Ok(file) => println!(
                "{}",
                json!({"id":id,"ok":true,"event":"completed","file":file})
            ),
            Err(error) => {
                failed = true;
                // Only our bounded outer context, never chained OS paths/upstream data.
                let message = error.to_string();
                println!(
                    "{}",
                    json!({"id":id,"ok":false,"event":"failed","error":message})
                );
                if message == "CANCELED" {
                    bail!("CANCELED");
                }
            }
        }
    }
    if failed {
        bail!("PROBE_FAILED");
    }
    Ok(())
}

#[tokio::main]
async fn main() {
    if let Err(error) = entry().await {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

async fn entry() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let program = args
        .first()
        .context("Usage: m1-probe <engine executable> [script]; JSON config on stdin")?;
    let mut buffer = Vec::new();
    stdin()
        .take(1024 * 1024 + 1)
        .read_to_end(&mut buffer)
        .await?;
    if buffer.len() > 1024 * 1024 {
        bail!("CONFIG_TOO_LARGE");
    }
    let config: Probe = serde_json::from_slice(&buffer).context("INVALID_PROBE_CONFIG")?;
    run(program, &args[1..], config).await
}
