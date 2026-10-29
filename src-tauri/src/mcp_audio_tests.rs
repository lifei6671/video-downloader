use super::*;
use std::{path::Path, sync::Arc, time::Duration};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

fn core(root: &Path, real: bool) -> Arc<Core> {
    let project = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    Core::open(&root.join("audio.sqlite"), root.join("downloads"),
        project.join(if cfg!(windows) { ".venv/Scripts/python.exe" } else { ".venv/bin/python" }).to_string_lossy().into(),
        vec![project.join(if real { "python-engine/bridge.py" } else { "tests/fixtures/desktop_engine.py" }).to_string_lossy().into()], |_| {}).unwrap()
}
fn seed(core: &Core, root: &Path, name: &str, real: bool) -> library::ImportedVideo {
    let path = root.join(format!("{name}.mp4"));
    if real {
        let result = std::process::Command::new(&core.program)
            .env("PYTHONPATH", Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join("python-engine"))
            .args(["-c", "import subprocess,sys;from local_tools import installed_tool;subprocess.run([installed_tool('ffmpeg'),'-nostdin','-v','error','-f','lavfi','-i','color=s=64x48:r=10:d=18','-f','lavfi','-i','sine=frequency=440:duration=18','-c:v','libx264','-c:a','aac','-shortest',sys.argv[1]],check=True)"])
            .arg(&path).output().unwrap();
        assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    } else { std::fs::write(&path, b"synthetic registered source").unwrap(); }
    library::register_local(&core.inner.lock().unwrap().db, &Source::read(&path).unwrap(), &library::LocalMetadata { width: 64, height: 48, duration_ms: Some(18000) }).unwrap()
}
fn input(item: &library::ImportedVideo) -> Value { json!({"asset_id":item.asset_id,"file_id":item.file_id,"start_ms":0,"end_ms":15000}) }

#[tokio::test]
async fn audio_real_wav_is_bounded_read_only_and_source_bound() {
    let root = tempfile::tempdir().unwrap(); let core = core(root.path(), true);
    let item = seed(&core, root.path(), "real", true);
    let changes = core.inner.lock().unwrap().db.total_changes();
    let result = core.mcp_audio_excerpt(input(&item), std::future::pending()).await.unwrap();
    assert_eq!(result["frames"], 240000);
    assert_eq!(result["actual_start_ms"], 0); assert_eq!(result["actual_end_ms"], 15000);
    let bytes = STANDARD.decode(result["audio_base64"].as_str().unwrap()).unwrap();
    validate_wav(&bytes, 240000).unwrap();
    assert!(serde_json::to_vec(&result).unwrap().len() < crate::core_ipc::MAX_RESPONSE_BYTES - 100);
    assert_eq!(core.inner.lock().unwrap().db.total_changes(), changes);
    assert!(!result.to_string().contains(&root.path().to_string_lossy().to_string()));
    for extra in [json!({"path":"private"}), json!({"end_ms":15001}), json!({"asset_id":"other"}), json!({"file_id":"other"})] {
        let mut invalid = input(&item); invalid.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        assert!(core.mcp_audio_excerpt(invalid, std::future::pending()).await.is_err());
    }
    core.archive_library_asset(&item.asset_id, true).unwrap();
    assert_eq!(core.mcp_audio_excerpt(input(&item), std::future::pending()).await.unwrap_err(), ReadError::AssetNotFound);
    core.archive_library_asset(&item.asset_id, false).unwrap();
    let path = core.library_asset(&item.asset_id).unwrap().files[0].path.clone();
    std::fs::write(&path, b"changed").unwrap();
    assert_eq!(core.mcp_audio_excerpt(input(&item), std::future::pending()).await.unwrap_err(), ReadError::AudioSourceChanged);
    std::fs::remove_file(path).unwrap();
    assert_eq!(core.mcp_audio_excerpt(input(&item), std::future::pending()).await.unwrap_err(), ReadError::FileMissing);
    assert!(std::fs::read_dir(root.path()).unwrap().all(|e| !e.unwrap().file_name().to_string_lossy().starts_with("yingqu-audio-")));
    core.shutdown().await;
}

async fn marker(root: &Path) -> Value {
    tokio::time::timeout(Duration::from_secs(10), async { loop {
        if let Ok(body) = std::fs::read(root.join("audio-started.json")) { if let Ok(value) = serde_json::from_slice(&body) { return value; } }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }}).await.unwrap()
}
fn reaped(started: &Value) {
    assert!(!Path::new(started["stage"].as_str().unwrap()).exists());
    #[cfg(windows)]
    for key in ["engine", "child"] {
        use windows_sys::Win32::{Foundation::{CloseHandle, STILL_ACTIVE}, System::Threading::{OpenProcess, GetExitCodeProcess, PROCESS_QUERY_LIMITED_INFORMATION}};
        // SAFETY: Only query the test-owned PID and close its checked handle once.
        unsafe {
            let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, started[key].as_u64().unwrap() as u32);
            if !handle.is_null() { let mut status = 0; assert_ne!(GetExitCodeProcess(handle, &mut status), 0); CloseHandle(handle); assert_ne!(status, STILL_ACTIVE as u32); }
        }
    }
}

#[tokio::test]
async fn audio_cancel_and_timeout_reap_actual_ffmpeg_and_remove_temp() {
    for timeout in [false, true] {
        let root = tempfile::tempdir().unwrap(); let core = core(root.path(), false);
        let item = seed(&core, root.path(), "hold-audio", false);
        let (cancel, receive) = tokio::sync::oneshot::channel();
        let worker = core.clone(); let params = input(&item);
        let task = tokio::spawn(async move { worker.mcp_audio_excerpt(params, async { let _ = receive.await; }).await });
        let started = marker(root.path()).await;
        if !timeout { cancel.send(()).unwrap(); } else { let _keep_cancel = cancel; assert_eq!(task.await.unwrap().unwrap_err(), ReadError::AudioTimeout); reaped(&started); core.shutdown().await; continue; }
        assert_eq!(task.await.unwrap().unwrap_err(), ReadError::Canceled);
        reaped(&started); core.shutdown().await;
    }
}

#[cfg(windows)]
async fn connect(endpoint: &str) -> tokio::net::windows::named_pipe::NamedPipeClient { tokio::net::windows::named_pipe::ClientOptions::new().open(endpoint).unwrap() }
#[cfg(unix)]
async fn connect(endpoint: &str) -> tokio::net::UnixStream { tokio::net::UnixStream::connect(endpoint).await.unwrap() }

#[tokio::test]
async fn audio_disconnect_and_service_stop_cleanup_without_blocking_text_calls() {
    for stop in [false, true] {
        let root = tempfile::tempdir().unwrap(); let core = core(root.path(), false);
        let item = seed(&core, root.path(), "hold-audio", false);
        let endpoint = core.set_mcp_ipc_enabled(true).await.unwrap().endpoint.unwrap();
        let mut stream = connect(&endpoint).await;
        let mut request = serde_json::to_vec(&json!({"version":1,"id":"audio","method":"mcp_get_audio_excerpt","params":input(&item)})).unwrap(); request.push(b'\n');
        stream.write_all(&request).await.unwrap();
        let started = marker(root.path()).await;
        let mut other = BufReader::new(connect(&endpoint).await);
        other.write_all(b"{\"version\":1,\"id\":\"ping\",\"method\":\"ping\"}\n").await.unwrap();
        let mut response = String::new(); tokio::time::timeout(Duration::from_secs(2), other.read_line(&mut response)).await.unwrap().unwrap();
        assert!(response.contains("yingqu-core"));
        if stop { core.set_mcp_ipc_enabled(false).await.unwrap(); } else { drop(stream); }
        tokio::time::timeout(Duration::from_secs(8), async { while Path::new(started["stage"].as_str().unwrap()).exists() { tokio::time::sleep(Duration::from_millis(10)).await; } }).await.unwrap();
        reaped(&started); core.shutdown().await;
    }
}
