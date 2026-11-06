//! Opt-in read-only Telegram QA. Configuration and media stay in the ignored work directory.
use anyhow::{bail, Context, Result};
use serde::Deserialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{fs, io::Read, path::Path, path::PathBuf, sync::Arc, time::Duration};
use tokio::time::{sleep, timeout, Instant};
use yingqu_core::{
    backup::telegram_session::Phase,
    desktop_core::{Core, Status, Task},
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    session_device_id: String,
    work_directory: PathBuf,
    links: Vec<String>,
    engine: PathBuf,
}

#[tokio::test]
#[ignore = "requires explicit authorization for configured message links and an existing SDK QA session"]
async fn telegram_live_download_queue_cancel_and_restart() -> Result<()> {
    let config: Config = serde_json::from_slice(&fs::read(
        std::env::var_os("YINGQU_TELEGRAM_DOWNLOAD_TEST_CONFIG")
            .context("set explicit ignored QA config")?,
    )?)?;
    uuid::Uuid::parse_str(&config.session_device_id).context("invalid session reference")?;
    if config.links.len() != 2 || !config.engine.is_file() || !config.work_directory.is_absolute() {
        bail!("two authorized links, an existing engine and an absolute QA work directory are required");
    }
    for link in &config.links {
        if !yingqu_core::telegram_download::is_telegram(link) {
            bail!("only Telegram message links are allowed");
        }
    }
    fs::create_dir_all(&config.work_directory)?;
    let run = config.work_directory.join(uuid::Uuid::new_v4().to_string());
    fs::create_dir(&run)?;
    println!("QA run: {}", run.display());
    let core = open(&config, &run)?;
    let result = exercise(&core, &config, &run).await;
    core.shutdown().await;
    drop(core);
    let report = result?;
    // Reopen only the QA database. Do not copy a user database or rerun a download.
    let reopened = open(&config, &run)?;
    let checked = (|| -> Result<()> {
        let tasks = reopened.list();
        for entry in report["files"].as_array().context("missing QA files")? {
            let task = tasks
                .iter()
                .find(|task| Some(task.id.as_str()) == entry["task_id"].as_str())
                .context("missing restored task")?;
            if task.status != Status::Completed || task.file_missing {
                bail!("committed QA task did not recover");
            }
            let path = task.path.as_ref().context("missing restored path")?;
            if fingerprint(path)? != entry["sha256"].as_str().context("missing digest")? {
                bail!("committed QA file changed on reopen");
            }
        }
        Ok(())
    })();
    reopened.shutdown().await;
    drop(reopened);
    checked?;
    let db = rusqlite::Connection::open_with_flags(
        run.join("qa.sqlite"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    for table in ["backup_spaces", "backup_operations"] {
        let count: i64 = db.query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
            row.get(0)
        })?;
        if count != 0 {
            bail!("QA unexpectedly joined a backup space or queued a backup");
        }
    }
    let mut report = report;
    report["restart_hashes_match"] = true.into();
    report["backup_spaces"] = 0.into();
    report["backup_operations"] = 0.into();
    fs::write(run.join("report.json"), serde_json::to_vec_pretty(&report)?)?;
    println!("QA report saved; original media and session authorization retained");
    Ok(())
}

fn open(config: &Config, run: &Path) -> Result<Arc<Core>> {
    Core::open(
        &run.join("qa.sqlite"),
        run.join("downloads"),
        config.engine.to_string_lossy().into_owned(),
        vec![],
        |_| {},
    )
}

async fn ready(core: &Core, id: &str) -> Result<Task> {
    timeout(Duration::from_secs(120), async {
        loop {
            let task = core
                .list()
                .into_iter()
                .find(|task| task.id == id)
                .context("missing QA task")?;
            match task.status {
                Status::Ready => return Ok(task),
                Status::Failed | Status::Canceled => bail!(
                    "QA parse failed: {}",
                    task.error.as_deref().unwrap_or("CANCELED")
                ),
                _ => sleep(Duration::from_millis(50)).await,
            }
        }
    })
    .await
    .context("QA parse deadline exceeded")?
}

async fn exercise(core: &Arc<Core>, config: &Config, _run: &Path) -> Result<serde_json::Value> {
    let view = timeout(
        Duration::from_secs(120),
        core.telegram_accounts.binding(yingqu_core::telegram_accounts::Purpose::Download).session
            .resume(config.session_device_id.clone()),
    )
    .await
    .context("QA authorization deadline exceeded")??;
    if !matches!(view.phase, Phase::Ready) {
        bail!("SDK QA authorization must be restored by the user");
    }
    core.set_max_downloads(2)?;
    let mut tasks = vec![];
    for (index, link) in config.links.iter().enumerate() {
        let task = core.resolve(link.clone())?;
        let task = ready(core, &task.id).await?;
        println!(
            "QA message {} resolved; bytes={}",
            index + 1,
            task.video
                .as_ref()
                .and_then(|video| video["filesize"].as_u64())
                .unwrap_or(0)
        );
        tasks.push(task);
    }
    for task in &tasks {
        core.download(&task.id, false)?;
    }
    let started = Instant::now();
    let mut last_notice = started;
    let mut parallel_observed = false;
    let mut canceled = false;
    let mut retried = false;
    let mut cancel_bytes = None;
    let mut other_bytes_at_cancel = None;
    loop {
        let all = core.list();
        let first = all
            .iter()
            .find(|task| task.id == tasks[0].id)
            .context("missing first QA task")?;
        let second = all
            .iter()
            .find(|task| task.id == tasks[1].id)
            .context("missing second QA task")?;
        parallel_observed |= first.status == Status::Downloading
            && second.status == Status::Downloading
            && first.bytes > 0
            && second.bytes > 0;
        if [first, second]
            .iter()
            .any(|task| task.status == Status::Failed)
        {
            let error = [first, second]
                .into_iter()
                .find(|task| task.status == Status::Failed)
                .and_then(|task| task.error.as_deref())
                .unwrap_or("UNKNOWN");
            bail!("QA transfer failed: {error}");
        }
        if !canceled
            && first.status == Status::Downloading
            && first.bytes > 0
            && first.total_bytes.is_some_and(|total| first.bytes < total)
            && second.status == Status::Downloading
            && second.bytes > 0
        {
            cancel_bytes = Some(first.bytes);
            other_bytes_at_cancel = Some(second.bytes);
            core.cancel(&first.id).await?;
            canceled = true;
            println!("QA canceled one partial transfer; the other remains independently active");
        } else if canceled
            && !retried
            && first.status == Status::Canceled
            && other_bytes_at_cancel.is_some_and(|bytes| second.bytes > bytes)
        {
            core.retry(&first.id)?;
            retried = true;
            println!("QA retry retained the original task identity and public link");
        } else if first.status == Status::Completed && second.status == Status::Completed {
            break;
        }
        if started.elapsed() > Duration::from_secs(3600) {
            bail!("QA transfer deadline exceeded");
        }
        if last_notice.elapsed() > Duration::from_secs(15) {
            println!("QA progress: first={} second={}", first.bytes, second.bytes);
            last_notice = Instant::now();
        }
        sleep(Duration::from_millis(50)).await;
    }
    if !parallel_observed || !canceled || !retried {
        bail!("QA did not exercise concurrent progress, partial cancellation and same-task retry");
    }
    let mut files = vec![];
    for task in &tasks {
        let done = core
            .list()
            .into_iter()
            .find(|value| value.id == task.id)
            .context("missing completed QA task")?;
        let path = done.path.as_ref().context("missing committed QA file")?;
        if fs::metadata(path)?.len() != done.bytes || done.total_bytes != Some(done.bytes) {
            bail!("QA committed size mismatch");
        }
        files.push(json!({"task_id":done.id,"bytes":done.bytes,"sha256":fingerprint(path)?,"path":path,"cover_cached":done.cover_cached,"resolver":done.video.as_ref().map(|video| &video["resolver"])}));
    }
    Ok(
        json!({"parallel_progress_observed":parallel_observed,"partial_cancel_exercised":canceled,"retried_same_task":retried,"cancel_bytes":cancel_bytes,"other_bytes_at_cancel":other_bytes_at_cancel,"elapsed_seconds":started.elapsed().as_secs_f64(),"files":files}),
    )
}

fn fingerprint(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hash = Sha256::new();
    let mut buffer = [0; 65536];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hash.finalize()))
}
