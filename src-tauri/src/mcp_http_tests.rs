use super::*;
use crate::{desktop_core::{Core, transcription::Source}, library};
use serde_json::{json, Value};
use std::{path::Path, sync::Arc};
use tokio::io::AsyncWriteExt;

fn core(root: &Path) -> (Arc<Core>, library::ImportedVideo) {
    let media = root.join("hold-audio.mp4");
    std::fs::write(&media, b"registered synthetic media").unwrap();
    let database = root.join("test.sqlite");
    let mut db = rusqlite::Connection::open(&database).unwrap();
    library::initialize(&mut db, &database).unwrap();
    let item = library::register_local(&db, &Source::read(&media).unwrap(), &library::LocalMetadata {width:64,height:48,duration_ms:Some(18000)}).unwrap();
    drop(db);
    let project = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let core = Core::open(&database, root.join("downloads"), project.join(if cfg!(windows) {".venv/Scripts/python.exe"} else {".venv/bin/python"}).to_string_lossy().into(), vec![project.join("tests/fixtures/desktop_engine.py").to_string_lossy().into()], |_| {}).unwrap();
    (core, item)
}
fn request(method: &str, params: Value) -> Value { json!({"jsonrpc":"2.0","id":1,"method":method,"params":params}) }
async fn tool(client: &reqwest::Client, url: &str, name: &str, arguments: Value) -> Value {
    let response = client.post(url).header("accept","application/json, text/event-stream").header("mcp-protocol-version","2025-11-25")
        .json(&request("tools/call", json!({"name":name,"arguments":arguments}))).send().await.unwrap();
    assert_eq!(response.status(), 200);
    response.json().await.unwrap()
}
#[tokio::test]
async fn loopback_permissions_http_validation_and_stop_revoke_access() {
    let root = tempfile::tempdir().unwrap(); let (core, _) = core(root.path());
    assert!(!core.mcp_service_status().await.enabled);
    let status = core.set_mcp_service_enabled_on_port(true, 0).await.unwrap();
    let url = status.endpoint.unwrap();
    assert_eq!(url::Url::parse(&url).unwrap().host_str(), Some("127.0.0.1"));
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    assert!(tool(&client,&url,"create_category",json!({"name":"readonly"})).await.to_string().contains("WRITE_DISABLED"));
    core.set_mcp_write_enabled(true).await.unwrap();
    assert!(tool(&client,&url,"create_category",json!({"name":"one"})).await["result"]["structuredContent"]["id"].is_string());
    core.set_mcp_write_enabled(false).await.unwrap();
    assert!(tool(&client,&url,"list_categories",json!({})).await["result"]["structuredContent"]["categories"].is_array());
    assert!(tool(&client,&url,"create_category",json!({"name":"again"})).await.to_string().contains("WRITE_DISABLED"));
    for (header, value) in [("origin","https://evil.invalid"),("host","evil.invalid"),("origin","null")] {
        let response = client.post(&url).header("accept","application/json, text/event-stream").header(header,value).json(&request("tools/list",json!({}))).send().await.unwrap();
        assert_eq!(response.status(),403);
        assert!(!response.text().await.unwrap().contains(&root.path().to_string_lossy().to_string()));
    }
    let large = client.post(&url).header("accept","application/json, text/event-stream").header("content-type","application/json").body(" ".repeat(MAX_REQUEST_BYTES+1)).send().await.unwrap();
    assert_eq!(large.status(),413);
    core.set_mcp_write_enabled(true).await.unwrap();
    core.set_mcp_service_enabled(false).await.unwrap();
    assert!(!core.mcp_write_enabled()); assert!(!core.mcp_ipc_status().await.enabled);
    assert!(client.post(&url).json(&json!({})).send().await.is_err());
    core.set_mcp_service_enabled_on_port(true,0).await.unwrap();
    assert!(!core.mcp_write_enabled());
    core.shutdown().await;
}
#[tokio::test]
async fn occupied_port_rolls_back_ipc_and_slow_bodies_do_not_block_shutdown() {
    let root = tempfile::tempdir().unwrap(); let (core, _) = core(root.path());
    let occupied = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    assert_eq!(core.set_mcp_service_enabled_on_port(true,occupied.local_addr().unwrap().port()).await.unwrap_err().to_string(),"MCP_HTTP_BIND_FAILED");
    assert!(!core.mcp_ipc_status().await.enabled); assert!(!core.mcp_write_enabled());
    let url = core.set_mcp_service_enabled_on_port(true,0).await.unwrap().endpoint.unwrap();
    let uri = url::Url::parse(&url).unwrap(); let authority = format!("127.0.0.1:{}",uri.port().unwrap());
    let mut stream = tokio::net::TcpStream::connect(&authority).await.unwrap();
    stream.write_all(format!("POST /mcp HTTP/1.1\r\nHost: {authority}\r\nAccept: application/json, text/event-stream\r\nContent-Type: application/json\r\nContent-Length: 10000\r\n\r\n{{").as_bytes()).await.unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    tokio::time::timeout(Duration::from_secs(2),core.set_mcp_service_enabled(false)).await.unwrap().unwrap();
    drop(stream); core.shutdown().await;
}
async fn started(root: &Path) -> Value {
    tokio::time::timeout(Duration::from_secs(10), async { loop {
        if let Ok(bytes) = std::fs::read(root.join("audio-started.json")) { if let Ok(value) = serde_json::from_slice(&bytes) { return value; } }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }}).await.unwrap()
}
fn reaped(started: &Value) {
    assert!(!Path::new(started["stage"].as_str().unwrap()).exists());
    #[cfg(windows)]
    for key in ["engine","child"] {
        use windows_sys::Win32::{Foundation::{CloseHandle,STILL_ACTIVE},System::Threading::{OpenProcess,GetExitCodeProcess,PROCESS_QUERY_LIMITED_INFORMATION}};
        // SAFETY: query only the fixture's own PIDs and release each acquired handle.
        unsafe { let handle=OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION,0,started[key].as_u64().unwrap() as u32); if !handle.is_null() {let mut status=0; assert_ne!(GetExitCodeProcess(handle,&mut status),0); CloseHandle(handle); assert_ne!(status,STILL_ACTIVE as u32);} }
    }
}
#[tokio::test]
async fn http_disconnect_and_switch_off_reap_audio_processes_and_temporary_files() {
    let project = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let python = project.join(if cfg!(windows) { ".venv/Scripts/python.exe" } else { ".venv/bin/python" });
    let check = std::process::Command::new(python)
        .arg(project.join("scripts/prepare_test_media.py")).arg("--check").output().unwrap();
    assert!(check.status.success(), "Media test dependency check failed: {}", String::from_utf8_lossy(&check.stderr));
    for stop in [false,true] {
        let root = tempfile::tempdir().unwrap(); let (core,item) = core(root.path());
        let url = core.set_mcp_service_enabled_on_port(true,0).await.unwrap().endpoint.unwrap();
        let uri = url::Url::parse(&url).unwrap(); let authority=format!("127.0.0.1:{}",uri.port().unwrap());
        let mut stream=tokio::net::TcpStream::connect(&authority).await.unwrap();
        let body=serde_json::to_string(&request("tools/call",json!({"name":"get_audio_excerpt","arguments":{"asset_id":item.asset_id,"file_id":item.file_id,"start_ms":0,"end_ms":15000}}))).unwrap();
        stream.write_all(format!("POST /mcp HTTP/1.1\r\nHost: {authority}\r\nAccept: application/json, text/event-stream\r\nMCP-Protocol-Version: 2025-11-25\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
        let marker=started(root.path()).await;
        let client=reqwest::Client::builder().no_proxy().build().unwrap();
        assert!(tool(&client,&url,"list_tags",json!({})).await["result"].is_object());
        if stop { core.set_mcp_service_enabled(false).await.unwrap(); drop(stream); } else { drop(stream); }
        tokio::time::timeout(Duration::from_secs(8),async { while Path::new(marker["stage"].as_str().unwrap()).exists() {tokio::time::sleep(Duration::from_millis(10)).await;} }).await.unwrap();
        reaped(&marker); core.shutdown().await;
    }
}
