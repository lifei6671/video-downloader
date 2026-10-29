//! WB-203 synthetic Core host. Never points at a daily application profile.
use std::path::PathBuf;
use tokio::io::{AsyncBufReadExt, BufReader};
use yingqu_core::{desktop_core::Core, library};
#[path = "../tests/fixtures/mcp_library.rs"]
mod fixture;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    anyhow::ensure!(
        std::env::var("YINGQU_WB203_FIXTURE").as_deref() == Ok("1"),
        "fixture only"
    );
    let root = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .ok_or_else(|| anyhow::anyhow!("fixture profile required"))?,
    );
    let marker: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("fixture.json"))?)?;
    anyhow::ensure!(
        marker == serde_json::json!({"purpose":"wb203-synthetic"}),
        "fixture marker required"
    );
    anyhow::ensure!(
        !root.join("yingqu.sqlite").exists(),
        "fresh fixture required"
    );
    if std::env::var("YINGQU_WB204_FIXTURE").as_deref() == Ok("1") {
        let path = root.join("yingqu.sqlite");
        let mut db = rusqlite::Connection::open(&path)?;
        library::initialize(&mut db, &path)?;
        fixture::seed(&db, &root)?;
        if std::env::var("YINGQU_WB312_FIXTURE").as_deref() == Ok("1") {
            fixture::seed_media(&db, &root)?;
        }
        if std::env::var("YINGQU_WB205_FIXTURE").as_deref() == Ok("1") {
            yingqu_core::settings::load(&db, root.join("downloads"))?;
            fixture::seed_resource_boundary(&db)?;
        }
    }
    let operation_fixture = std::env::var("YINGQU_WB311_FIXTURE").as_deref() == Ok("1");
    let (program, args) = if operation_fixture {
        let project = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let db = rusqlite::Connection::open(root.join("yingqu.sqlite"))?;
        for (file, name) in [("file-a", "hold-mcp"), ("file-c", "speech")] {
            let media = root.join(format!("{name}.mp4")); std::fs::write(&media, b"1234")?;
            db.execute("UPDATE asset_files SET path=?1,size=4 WHERE id=?2",rusqlite::params![media.canonicalize()?.to_string_lossy(),file])?;
        }
        (project.join(if cfg!(windows) {".venv/Scripts/python.exe"} else {".venv/bin/python"}).to_string_lossy().into_owned(), vec![project.join("tests/fixtures/desktop_engine.py").to_string_lossy().into_owned()])
    } else { ("unused-fixture".into(), vec![]) };
    let core = Core::open(
        &root.join("yingqu.sqlite"),
        root.join("downloads"),
        program,
        args,
        |_| {},
    )?;
    core.create_library_category(None, "中文灵感😀")?;
    let tag = core.create_library_tag("制作手法")?;
    core.set_mcp_ipc_enabled(true).await?;
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    while let Some(line) = lines.next_line().await? {
        let request: serde_json::Value = serde_json::from_str(&line)?;
        let result = match request["method"].as_str() {
            Some("status") => serde_json::to_value(core.mcp_ipc_status().await)?,
            Some("write_permission") => { core.set_mcp_write_enabled(request["params"]["enabled"].as_bool().unwrap_or(false)).await?; serde_json::json!({"write_enabled":core.mcp_write_enabled()}) }
            Some("transcription_processes") if operation_fixture => { let marker=root.join("mcp-transcription-started.json"); if marker.exists() { serde_json::from_slice(&std::fs::read(marker)?)? } else { serde_json::Value::Null } },
            Some("private_probe") => {
                serde_json::json!({"configured":core.settings().profiles.iter().any(|profile|profile.platform == "youtube" && profile.cookie_configured && profile.proxy.as_ref().is_some_and(|proxy|proxy.contains("WB205_SYNTHETIC_PROXY_PASSWORD")))})
            }
            Some("stop") => serde_json::to_value(core.set_mcp_ipc_enabled(false).await?)?,
            Some("start") => serde_json::to_value(core.set_mcp_ipc_enabled(true).await?)?,
            Some("rename") => {
                core.rename_library_tag(&tag, "镜头节奏😀")?;
                serde_json::json!({"renamed":true})
            }
            Some("edit_note") => {
                core.edit_library_note("asset-a", "note-a", "第二版笔记😀", 1)?;
                serde_json::json!({"revision":2})
            }
            Some("edit_document") => {
                let id = core.save_library_document("asset-a", "新的第三版😀", 2, None)?;
                serde_json::json!({"document_id":id})
            }
            _ => anyhow::bail!("unknown fixture command"),
        };
        println!(
            "{}",
            serde_json::json!({"id":request["id"],"result":result})
        );
    }
    core.shutdown().await;
    Ok(())
}
