//! Opt-in anonymous Bilibili QA through the desktop core and packaged engine.
use anyhow::{ensure, Context, Result};
use std::{path::Path, sync::Arc, time::Duration};
use yingqu_core::desktop_core::{Core, Status, Task};

fn open(root: &Path) -> Result<Arc<Core>> {
    let project = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let engine = project.join(if cfg!(windows) {
        "python-engine/dist/video-engine/video-engine.exe"
    } else {
        "python-engine/dist/video-engine/video-engine"
    });
    Core::open(
        &root.join("qa.sqlite"),
        root.join("downloads"),
        engine.to_string_lossy().into_owned(),
        vec![],
        |_| {},
    )
}

async fn wait(core: &Core, id: &str, status: Status) -> Result<Task> {
    tokio::time::timeout(Duration::from_secs(180), async {
        loop {
            let task = core
                .list()
                .into_iter()
                .find(|t| t.id == id)
                .context("missing task")?;
            ensure!(
                task.status != Status::Failed,
                "waiting for {status:?}; task failed: {:?}",
                task.error
            );
            if task.status == status {
                return Ok(task);
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .context("Bilibili QA timed out")?
}

#[tokio::test]
#[ignore = "requires an explicitly configured Bilibili URL, packaged engine and FFmpeg"]
async fn bilibili_author_task_download_and_restart() -> Result<()> {
    let input = std::env::var("YINGQU_BILIBILI_TEST_URL").context("set the authorized test URL")?;
    let project = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let parent = project.join(".local/bilibili-author-fix/live");
    std::fs::create_dir_all(&parent)?;
    let run = tempfile::Builder::new().prefix("qa-").tempdir_in(parent)?;
    let core = open(run.path())?;
    let result = async {
        let task = core.resolve(input)?;
        let ready = wait(&core, &task.id, Status::Ready).await?;
        let video = ready.video.as_ref().context("missing preview")?;
        ensure!(video["platform"] == "bilibili");
        ensure!(
            video["author_profile"]["bio"]
                .as_str()
                .is_some_and(|bio| !bio.is_empty()),
            "sample author biography missing"
        );
        ensure!(
            !core.author_avatar(&task.id)?.is_empty(),
            "preview avatar missing"
        );
        ensure!(video["author_profile"]["id"]
            .as_str()
            .is_some_and(|id| !id.is_empty()));
        core.download(&task.id, false)?;
        wait(&core, &task.id, Status::Completed).await
    }
    .await;
    core.shutdown().await;
    drop(core);
    let completed = result?;
    let avatar_before = {
        let db = rusqlite::Connection::open(run.path().join("qa.sqlite"))?;
        db.query_row(
            "SELECT avatar FROM authors WHERE platform='bilibili'",
            [],
            |row| row.get::<_, Vec<u8>>(0),
        )?
    };
    let original = std::fs::read(completed.path.as_ref().context("missing media path")?)?;
    ensure!(!original.is_empty());
    let restarted = open(run.path())?;
    let recovered = restarted.list().into_iter().find(|t| t.id == completed.id);
    let avatar_after = restarted.author_avatar(&completed.id)?;
    restarted.shutdown().await;
    drop(restarted);
    let recovered = recovered.context("task missing after restart")?;
    ensure!(recovered.status == Status::Completed && !recovered.file_missing);
    ensure!(avatar_before == avatar_after && !avatar_after.is_empty());
    ensure!(
        recovered.video.as_ref().context("missing restored video")?["author_profile"]
            == completed
                .video
                .as_ref()
                .context("missing completed video")?["author_profile"]
    );
    ensure!(std::fs::read(recovered.path.context("missing restored path")?)? == original);
    let db = rusqlite::Connection::open(run.path().join("qa.sqlite"))?;
    ensure!(
        db.query_row(
            "SELECT count(*) FROM task_authors WHERE task_id=?",
            [&completed.id],
            |row| row.get::<_, i64>(0)
        )? == 1
    );
    ensure!(db.query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0))? == "ok");
    println!("Bilibili packaged engine: parsed, downloaded {} bytes, saved author and recovered task/media after restart", original.len());
    Ok(())
}
