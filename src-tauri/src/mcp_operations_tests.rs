use super::*;
use crate::{
    desktop_core::transcription::Source,
    library::{self, LocalMetadata},
};
use serde_json::json;
use std::{path::Path, time::Duration};

fn fixture(root: &Path) -> Arc<Core> {
    let project = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    Core::open(
        &root.join("tasks.sqlite"),
        root.join("downloads"),
        project
            .join(if cfg!(windows) {
                ".venv/Scripts/python.exe"
            } else {
                ".venv/bin/python"
            })
            .to_string_lossy()
            .into(),
        vec![project
            .join("tests/fixtures/desktop_engine.py")
            .to_string_lossy()
            .into()],
        |_| {},
    )
    .unwrap()
}
fn seed(core: &Core, root: &Path, name: &str) -> (String, String) {
    let path = root.join(format!("{name}.mp4"));
    std::fs::write(&path, b"1234").unwrap();
    let item = library::register_local(
        &core.inner.lock().unwrap().db,
        &Source::read(&path).unwrap(),
        &LocalMetadata {
            width: 128,
            height: 72,
            duration_ms: Some(3000),
        },
    )
    .unwrap();
    (item.asset_id, item.file_id)
}
fn start(core: &Arc<Core>, asset: &str, file: &str, force: bool) -> Value {
    core.mcp_operation_tool(
        "mcp_start_transcription",
        json!({"asset_id":asset,"file_id":file,"force":force}),
    )
    .unwrap()
}
fn call(core: &Arc<Core>, method: &str, operation: &str) -> Value {
    core.mcp_operation_tool(method, json!({"operation_id":operation}))
        .unwrap()
}
async fn done(core: &Arc<Core>, operation: &str) -> Value {
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let value = call(core, "mcp_get_operation", operation);
            if value["state"] != "running" {
                return value;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap()
}
async fn grant(core: &Arc<Core>) {
    core.set_mcp_ipc_enabled(true).await.unwrap();
    core.set_mcp_write_enabled(true).await.unwrap();
}

#[tokio::test]
async fn permissions_duplicates_single_worker_revoke_and_cancel_request_are_truthful() {
    let root = tempfile::tempdir().unwrap();
    let core = fixture(root.path());
    let (a, f) = seed(&core, root.path(), "hold-mcp");
    let (b, g) = seed(&core, root.path(), "speech");
    assert_eq!(
        core.mcp_operation_tool("mcp_start_transcription", json!({"asset_id":a,"file_id":f}))
            .unwrap_err(),
        OperationError::WriteDisabled
    );
    grant(&core).await;
    assert_eq!(
        core.mcp_operation_tool("mcp_start_transcription", json!({"asset_id":a,"file_id":g}))
            .unwrap_err(),
        OperationError::FileNotFound
    );
    let first = start(&core, &a, &f, false);
    let operation = first["operation_id"].as_str().unwrap();
    assert_eq!(first["state"], "running");
    assert_eq!(start(&core, &a, &f, true)["operation_id"], operation);
    assert_eq!(
        core.mcp_operation_tool("mcp_start_transcription", json!({"asset_id":b,"file_id":g}))
            .unwrap_err(),
        OperationError::TranscriptionBusy
    );
    core.set_mcp_write_enabled(false).await.unwrap();
    assert_eq!(
        call(&core, "mcp_get_operation", operation)["state"],
        "running"
    );
    assert_eq!(
        core.mcp_operation_tool("mcp_cancel_operation", json!({"operation_id":operation}))
            .unwrap_err(),
        OperationError::WriteDisabled
    );
    core.set_mcp_write_enabled(true).await.unwrap();
    let pending = call(&core, "mcp_cancel_operation", operation);
    assert_eq!(pending["state"], "running");
    assert_eq!(pending["cancel_requested"], true);
    assert_eq!(start(&core, &a, &f, false)["operation_id"], operation);
    let finished = done(&core, operation).await;
    assert_eq!(finished["state"], "canceled");
    assert_eq!(finished["cancel_requested"], false);
    assert_eq!(
        call(&core, "mcp_cancel_operation", operation)["state"],
        "canceled"
    );
    core.shutdown().await;
}

#[tokio::test]
async fn saved_results_cache_force_old_ids_failures_and_restart_keep_exact_identity() {
    let root = tempfile::tempdir().unwrap();
    let core = fixture(root.path());
    let (a, f) = seed(&core, root.path(), "speech");
    grant(&core).await;
    let first = start(&core, &a, &f, false);
    let operation = first["operation_id"].as_str().unwrap();
    let result = done(&core, operation).await;
    assert_eq!(result["state"], "completed");
    assert_eq!(result["has_result"], true);
    assert_eq!(start(&core, &a, &f, false)["operation_id"], operation);
    let next = start(&core, &a, &f, true);
    let next_id = next["operation_id"].as_str().unwrap();
    assert_ne!(next_id, operation);
    assert_eq!(
        core.mcp_operation_tool("mcp_cancel_operation", json!({"operation_id":operation}))
            .unwrap_err(),
        OperationError::OperationNotFound
    );
    assert_eq!(done(&core, next_id).await["state"], "completed");
    let (b, g) = seed(&core, root.path(), "fail");
    let failed = start(&core, &b, &g, false);
    assert_eq!(
        done(&core, failed["operation_id"].as_str().unwrap()).await["state"],
        "failed"
    );
    let (c, h) = seed(&core, root.path(), "silent");
    let silent = start(&core, &c, &h, false);
    assert_eq!(
        done(&core, silent["operation_id"].as_str().unwrap()).await["state"],
        "no_speech"
    );
    let expected = call(&core, "mcp_get_operation", next_id);
    core.shutdown().await;
    drop(core);
    let core = fixture(root.path());
    assert!(!core.mcp_write_enabled());
    assert_eq!(call(&core, "mcp_get_operation", next_id), expected);
    core.shutdown().await;
}

#[tokio::test]
async fn actual_engine_and_descendant_are_gone_before_canceled_and_stage_is_removed() {
    let root = tempfile::tempdir().unwrap();
    let core = fixture(root.path());
    let (a, f) = seed(&core, root.path(), "hold-mcp");
    grant(&core).await;
    let started = start(&core, &a, &f, false);
    let operation = started["operation_id"].as_str().unwrap();
    let marker = root.path().join("mcp-transcription-started.json");
    tokio::time::timeout(Duration::from_secs(10), async {
        while !marker.exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let processes: Value = serde_json::from_slice(&std::fs::read(&marker).unwrap()).unwrap();
    assert_eq!(
        call(&core, "mcp_cancel_operation", operation)["state"],
        "running"
    );
    assert_eq!(done(&core, operation).await["state"], "canceled");
    assert!(!Path::new(processes["stage"].as_str().unwrap()).exists());
    assert_eq!(
        core.inner
            .lock()
            .unwrap()
            .db
            .query_row("SELECT count(*) FROM task_staging", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    #[cfg(windows)]
    for key in ["engine", "child"] {
        use windows_sys::Win32::{
            Foundation::{CloseHandle, STILL_ACTIVE},
            System::Threading::{
                GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
            },
        };
        // SAFETY: Only test-owned PIDs; each non-null handle is queried and closed once.
        unsafe {
            let handle = OpenProcess(
                PROCESS_QUERY_LIMITED_INFORMATION,
                0,
                processes[key].as_u64().unwrap() as u32,
            );
            if !handle.is_null() {
                let mut code = 0;
                assert_ne!(GetExitCodeProcess(handle, &mut code), 0);
                CloseHandle(handle);
                assert_ne!(
                    code, STILL_ACTIVE as u32,
                    "{key} survived terminal canceled"
                );
            }
        }
    }
    core.shutdown().await;
}

#[tokio::test]
async fn operation_views_reject_unknown_fields_and_sanitize_stored_diagnostics() {
    let root = tempfile::tempdir().unwrap();
    let core = fixture(root.path());
    let (a, f) = seed(&core, root.path(), "speech");
    grant(&core).await;
    assert_eq!(
        core.mcp_operation_tool("mcp_get_operation", json!({"operation_id":"missing"}))
            .unwrap_err(),
        OperationError::OperationNotFound
    );
    assert_eq!(
        core.mcp_operation_tool(
            "mcp_start_transcription",
            json!({"asset_id":a,"file_id":f,"path":"PRIVATE"})
        )
        .unwrap_err(),
        OperationError::InvalidInput
    );
    let started = start(&core, &a, &f, false);
    let operation = started["operation_id"].as_str().unwrap();
    done(&core, operation).await;
    core.inner.lock().unwrap().db.execute("UPDATE asset_transcriptions SET snapshot=json_set(snapshot,'$.error_code','PRIVATE_PATH_AND_CREDENTIAL','$.phase','PRIVATE_PHASE','$.backend','PRIVATE_BACKEND') WHERE file_id=?1",[f]).unwrap();
    let text = call(&core, "mcp_get_operation", operation).to_string();
    assert!(!text.contains("PRIVATE"));
    assert!(!text.contains("source") && !text.contains("text") && !text.contains("path"));
    core.shutdown().await;
}
