//! Native cross-process smoke test; only synthetic data in a temporary profile.
#![cfg(any(windows, unix))]
use serde_json::{json, Value};
use std::{io::Write, process::Stdio, time::Duration};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use yingqu_core::desktop_core::Core;

#[cfg(windows)]
async fn connect(
    endpoint: &str,
) -> std::io::Result<tokio::net::windows::named_pipe::NamedPipeClient> {
    tokio::net::windows::named_pipe::ClientOptions::new().open(endpoint)
}
#[cfg(unix)]
async fn connect(endpoint: &str) -> std::io::Result<tokio::net::UnixStream> {
    tokio::net::UnixStream::connect(endpoint).await
}

#[tokio::test]
async fn separate_process_reads_running_core_and_exit_disconnects() {
    let root = tempfile::Builder::new()
        .prefix("WB202 中文 IPC ")
        .tempdir()
        .unwrap();
    let mut child = tokio::process::Command::new(std::env::current_exe().unwrap())
        .args(["--ignored", "--exact", "ipc_child_fixture", "--nocapture"])
        .env("YINGQU_WB202_TEST_PROFILE", root.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap()).lines();
    let endpoint = tokio::time::timeout(Duration::from_secs(10), async {
        while let Some(line) = output.next_line().await.unwrap() {
            if let Some(endpoint) = line.strip_prefix("WB202_ENDPOINT=") {
                return endpoint.to_owned();
            }
        }
        panic!("child exited without starting the IPC service");
    })
    .await
    .unwrap();
    let mut connection = BufReader::new(connect(&endpoint).await.unwrap());
    connection
        .write_all(b"{\"version\":1,\"id\":\"native\",\"method\":\"list_categories\"}\n")
        .await
        .unwrap();
    let mut line = String::new();
    tokio::time::timeout(Duration::from_secs(3), connection.read_line(&mut line))
        .await
        .unwrap()
        .unwrap();
    let result: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(result["id"], "native");
    assert_eq!(result["result"][0]["name"], "原生进程中的知识库😀");
    assert_eq!(result["result"].as_array().unwrap().len(), 1);
    // The child exits through the same Core::shutdown used by desktop request_exit.
    child.stdin.take().unwrap().write_all(b"q").await.unwrap();
    let status = tokio::time::timeout(Duration::from_secs(10), child.wait())
        .await
        .unwrap()
        .unwrap();
    let mut stderr = String::new();
    child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut stderr)
        .await
        .unwrap();
    assert!(status.success(), "child failed: {status}; {stderr}");
    let mut byte = [0];
    let closed = tokio::time::timeout(Duration::from_secs(3), connection.read(&mut byte))
        .await
        .unwrap();
    assert!(closed.is_err() || closed.unwrap() == 0);
    assert!(connect(&endpoint).await.is_err());
}

#[tokio::test]
#[ignore = "subprocess fixture, invoked by the cross-process test"]
async fn ipc_child_fixture() {
    let root = std::path::PathBuf::from(
        std::env::var_os("YINGQU_WB202_TEST_PROFILE")
            .expect("fixture requires its isolated profile"),
    );
    let core = Core::open(
        &root.join("test.sqlite"),
        root.join("downloads"),
        "unused-fixture".into(),
        vec![],
        |_| {},
    )
    .unwrap();
    assert!(!core.mcp_ipc_status().await.enabled);
    core.create_library_category(None, "原生进程中的知识库😀")
        .unwrap();
    let status = core.set_mcp_ipc_enabled(true).await.unwrap();
    println!("WB202_ENDPOINT={}", status.endpoint.unwrap());
    std::io::stdout().flush().unwrap();
    tokio::task::spawn_blocking(|| {
        use std::io::Read;
        let mut byte = [0];
        std::io::stdin().read_exact(&mut byte).unwrap();
        assert_eq!(byte, [b'q']);
    })
    .await
    .unwrap();
    core.shutdown().await;
    assert_eq!(
        serde_json::to_value(core.mcp_ipc_status().await).unwrap(),
        json!({"enabled":false,"endpoint":null,"error":null})
    );
}
