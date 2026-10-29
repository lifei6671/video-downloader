use super::*;
use std::{sync::Arc, time::Duration};
use tokio::io::AsyncReadExt;

fn fixture(root: &Path) -> Arc<Core> {
    Core::open(
        &root.join("test.sqlite"),
        root.join("downloads"),
        "unused-fixture".into(),
        vec![],
        |_| {},
    )
    .unwrap()
}
#[cfg(windows)]
async fn connect(endpoint: &str) -> io::Result<tokio::net::windows::named_pipe::NamedPipeClient> {
    tokio::net::windows::named_pipe::ClientOptions::new().open(endpoint)
}
#[cfg(unix)]
async fn connect(endpoint: &str) -> io::Result<tokio::net::UnixStream> {
    tokio::net::UnixStream::connect(endpoint).await
}
async fn call(endpoint: &str, request: Value) -> Value {
    let mut stream = BufReader::new(connect(endpoint).await.unwrap());
    let mut bytes = serde_json::to_vec(&request).unwrap();
    bytes.push(b'\n');
    stream.write_all(&bytes).await.unwrap();
    let mut line = String::new();
    tokio::time::timeout(Duration::from_secs(3), stream.read_line(&mut line))
        .await
        .unwrap()
        .unwrap();
    serde_json::from_str(&line).unwrap()
}
fn request(method: &str) -> Value {
    json!({"version":1,"id":"中文😀","method":method})
}

#[tokio::test]
async fn service_stop_interrupts_backpressured_response_writes() {
    let root = tempfile::tempdir().unwrap(); let core = fixture(root.path());
    let db = rusqlite::Connection::open(root.path().join("test.sqlite")).unwrap();
    db.execute("INSERT INTO library_assets(id,title,created_at,updated_at) VALUES('large',?1,1,1)", ["x".repeat(800_000)]).unwrap();
    let (mut client, server) = tokio::io::duplex(1024);
    let (stop, stopping) = watch::channel(false);
    let worker = tokio::spawn(serve(server, Arc::downgrade(&core), stopping));
    client.write_all(b"{\"version\":1,\"id\":\"test\",\"method\":\"mcp_get_asset\",\"params\":{\"asset_id\":\"large\"}}\n").await.unwrap();
    let mut first = [0]; client.read_exact(&mut first).await.unwrap();
    assert!(!worker.is_finished(), "response should exceed the unread duplex buffer");
    stop.send(true).unwrap();
    tokio::time::timeout(Duration::from_secs(1), worker).await.unwrap().unwrap().unwrap();
    core.shutdown().await;
}

#[tokio::test]
async fn additive_writes_require_desktop_permission_and_reset_on_service_restart() {
    let root = tempfile::tempdir().unwrap();
    let core = fixture(root.path());
    assert!(!core.mcp_write_enabled());
    assert!(core.set_mcp_write_enabled(true).await.is_err());
    let endpoint = core.set_mcp_ipc_enabled(true).await.unwrap().endpoint.unwrap();
    let mut create = request("mcp_create_category");
    create["params"] = json!({"name":"AI 灵感😀","parent_id":null});
    assert_eq!(call(&endpoint, create.clone()).await["error"]["code"], "WRITE_DISABLED");
    core.set_mcp_write_enabled(true).await.unwrap();
    let result = call(&endpoint, create.clone()).await;
    assert!(result["result"]["id"].is_string());
    assert!(core.set_mcp_ipc_enabled(true).await.unwrap().enabled);
    assert!(core.mcp_write_enabled()); // idempotent service enable preserves an explicit grant
    for method in ["set_mcp_write_enabled", "delete_library_asset", "edit_library_note", "save_library_document", "update_settings"] {
        assert_eq!(call(&endpoint, request(method)).await["error"]["code"], "METHOD_NOT_ALLOWED");
    }
    core.set_mcp_write_enabled(false).await.unwrap();
    create["params"]["name"] = json!("未授权新增");
    assert_eq!(call(&endpoint, create.clone()).await["error"]["code"], "WRITE_DISABLED");
    assert_eq!(core.library_categories().unwrap().len(), 1);
    core.set_mcp_write_enabled(true).await.unwrap();
    core.set_mcp_ipc_enabled(false).await.unwrap();
    assert!(!core.mcp_write_enabled());
    let endpoint = core.set_mcp_ipc_enabled(true).await.unwrap().endpoint.unwrap();
    assert_eq!(call(&endpoint, create).await["error"]["code"], "WRITE_DISABLED");
    core.shutdown().await;
}

#[tokio::test]
async fn starts_disabled_reads_live_core_and_stops_all_connections() {
    let root = tempfile::tempdir().unwrap();
    let core = fixture(root.path());
    assert_eq!(core.mcp_ipc_status().await, IpcStatus::disabled());
    let status = core.set_mcp_ipc_enabled(true).await.unwrap();
    let endpoint = status.endpoint.unwrap();
    assert_eq!(
        core.set_mcp_ipc_enabled(true)
            .await
            .unwrap()
            .endpoint
            .as_deref(),
        Some(endpoint.as_str())
    );
    let category = core.create_library_category(None, "中文灵感😀").unwrap();
    let tag = core.create_library_tag("制作手法").unwrap();
    let categories = call(&endpoint, request("list_categories")).await;
    assert_eq!(categories["id"], "中文😀");
    assert_eq!(categories["result"][0]["id"], category);
    assert_eq!(categories["result"][0]["name"], "中文灵感😀");
    assert_eq!(
        call(&endpoint, request("list_tags")).await["result"][0]["id"],
        tag
    );
    // Mutate through the existing desktop/Core API while the IPC listener is running.
    core.rename_library_tag(&tag, "镜头节奏").unwrap();
    assert_eq!(
        call(&endpoint, request("list_tags")).await["result"][0]["name"],
        "镜头节奏"
    );
    let mut idle = connect(&endpoint).await.unwrap();
    idle.write_all(b"{\"partial\":").await.unwrap();
    assert!(!core.set_mcp_ipc_enabled(false).await.unwrap().enabled);
    let mut byte = [0];
    let closed = tokio::time::timeout(Duration::from_secs(3), idle.read(&mut byte))
        .await
        .unwrap();
    assert!(closed.is_err() || closed.unwrap() == 0);
    assert!(connect(&endpoint).await.is_err());
    let restarted = core
        .set_mcp_ipc_enabled(true)
        .await
        .unwrap()
        .endpoint
        .unwrap();
    assert_eq!(endpoint, restarted);
    assert_eq!(
        call(&endpoint, request("ping")).await["result"]["service"],
        "yingqu-core"
    );
    core.shutdown().await;
    assert_eq!(core.mcp_ipc_status().await, IpcStatus::disabled());
    assert!(connect(&endpoint).await.is_err());
    assert_eq!(
        core.set_mcp_ipc_enabled(true)
            .await
            .unwrap_err()
            .to_string(),
        "INVALID_STATE"
    );
    drop(core);
    let reopened = fixture(root.path());
    assert_eq!(reopened.mcp_ipc_status().await, IpcStatus::disabled());
    reopened.shutdown().await;
}

#[tokio::test]
async fn rejects_invalid_requests_without_writing_or_losing_the_service() {
    let root = tempfile::tempdir().unwrap();
    let core = fixture(root.path());
    let endpoint = core
        .set_mcp_ipc_enabled(true)
        .await
        .unwrap()
        .endpoint
        .unwrap();
    for method in [
        "create_category",
        "search_assets",
        "get_settings",
        "read_file",
        "execute_sql",
        "shutdown",
    ] {
        assert_eq!(
            call(&endpoint, request(method)).await["error"]["code"],
            "METHOD_NOT_ALLOWED"
        );
    }
    let mut version = request("ping");
    version["version"] = 99.into();
    assert_eq!(
        call(&endpoint, version).await["error"]["code"],
        "VERSION_UNSUPPORTED"
    );
    let mut params = request("list_categories");
    params["params"] = json!({"path":"C:/private"});
    assert_eq!(
        call(&endpoint, params).await["error"]["code"],
        "INVALID_PARAMS"
    );
    let mut unknown = request("ping");
    unknown["extra"] = true.into();
    assert_eq!(
        call(&endpoint, unknown).await["error"]["code"],
        "INVALID_REQUEST"
    );
    let mut stream = BufReader::new(connect(&endpoint).await.unwrap());
    stream
        .write_all(b"not-json\n{\"version\":1,\"id\":\"2\",\"method\":\"ping\"}\n")
        .await
        .unwrap();
    let mut line = String::new();
    stream.read_line(&mut line).await.unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&line).unwrap()["error"]["code"],
        "INVALID_REQUEST"
    );
    line.clear();
    stream.read_line(&mut line).await.unwrap();
    assert_eq!(serde_json::from_str::<Value>(&line).unwrap()["id"], "2");
    stream
        .write_all(&vec![b'x'; MAX_REQUEST_BYTES + 1])
        .await
        .unwrap();
    line.clear();
    stream.read_line(&mut line).await.unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&line).unwrap()["error"]["code"],
        "REQUEST_TOO_LARGE"
    );
    assert!(core.library_categories().unwrap().is_empty());
    assert_eq!(
        call(&endpoint, request("ping")).await["result"]["protocol_version"],
        1
    );
    core.shutdown().await;
}

#[tokio::test]
async fn profile_isolation_and_drop_release_the_core_lock() {
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    let core = fixture(first.path());
    let other = fixture(second.path());
    let first_endpoint = core
        .set_mcp_ipc_enabled(true)
        .await
        .unwrap()
        .endpoint
        .unwrap();
    let second_endpoint = other
        .set_mcp_ipc_enabled(true)
        .await
        .unwrap()
        .endpoint
        .unwrap();
    assert_ne!(first_endpoint, second_endpoint);
    core.create_library_tag("仅在第一库").unwrap();
    assert!(call(&second_endpoint, request("list_tags")).await["result"]
        .as_array()
        .unwrap()
        .is_empty());
    drop(core);
    // The listener holds Weak<Core>, not a second owner of the database.
    let reopened = fixture(first.path());
    assert_eq!(reopened.mcp_ipc_status().await, IpcStatus::disabled());
    other.shutdown().await;
    reopened.shutdown().await;
}

#[tokio::test]
async fn duplicate_listener_fails_without_disrupting_the_running_core() {
    let root = tempfile::tempdir().unwrap();
    let core = fixture(root.path());
    let profile = root.path().join("test.sqlite").canonicalize().unwrap();
    let blocker = transport::Listener::bind(&profile).await.unwrap();
    assert_eq!(
        core.set_mcp_ipc_enabled(true)
            .await
            .unwrap_err()
            .to_string(),
        "IPC_BIND_FAILED"
    );
    assert_eq!(core.mcp_ipc_status().await, IpcStatus::disabled());
    drop(blocker);
    let endpoint = core
        .set_mcp_ipc_enabled(true)
        .await
        .unwrap()
        .endpoint
        .unwrap();
    assert!(transport::Listener::bind(&profile).await.is_err());
    assert_eq!(
        call(&endpoint, request("ping")).await["result"]["service"],
        "yingqu-core"
    );
    core.shutdown().await;
}

#[tokio::test]
async fn response_limit_and_fragmented_unicode_frames_are_enforced() {
    let root = tempfile::tempdir().unwrap();
    let core = fixture(root.path());
    let endpoint = core
        .set_mcp_ipc_enabled(true)
        .await
        .unwrap()
        .endpoint
        .unwrap();
    let mut stream = BufReader::new(connect(&endpoint).await.unwrap());
    let mut bytes = serde_json::to_vec(&request("ping")).unwrap();
    bytes.push(b'\n');
    for byte in bytes {
        stream.write_all(&[byte]).await.unwrap();
    }
    let mut line = String::new();
    stream.read_line(&mut line).await.unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&line).unwrap()["id"],
        "中文😀"
    );
    let (mut reader, mut writer) = tokio::io::duplex(MAX_RESPONSE_BYTES * 2);
    write_response(
        &mut writer,
        json!({"version":1,"id":"large","result":"x".repeat(MAX_RESPONSE_BYTES)}),
    )
    .await
    .unwrap();
    let mut result = String::new();
    BufReader::new(&mut reader)
        .read_line(&mut result)
        .await
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&result).unwrap()["error"]["code"],
        "RESPONSE_TOO_LARGE"
    );
    core.shutdown().await;
}
