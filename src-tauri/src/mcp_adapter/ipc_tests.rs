use super::*;

async fn response(bytes: Vec<u8>) -> Result<Value, AdapterError> {
    let (client, server) = tokio::io::duplex(MAX_RESPONSE_BYTES * 2);
    let task = tokio::spawn(async move {
        let mut server = BufReader::new(server);
        let mut request = String::new();
        server.read_line(&mut request).await.unwrap();
        let request: Value = serde_json::from_str(&request).unwrap();
        assert_eq!(
            request,
            json!({"version":1,"id":"1","method":"list_tags","params":{}})
        );
        server.write_all(&bytes).await.unwrap();
    });
    let result = exchange(&mut BufReader::new(client), "1", "list_tags", json!({})).await;
    task.await.unwrap();
    result
}

#[tokio::test]
async fn validates_reply_envelope_and_does_not_reflect_remote_errors() {
    for message in [
        json!({"version":99,"id":"1","result":[]}),
        json!({"version":1,"id":"wrong","result":[]}),
        json!({"version":1,"id":"1"}),
        json!({"version":1,"id":"1","result":[],"error":{"code":"CORE_READ_FAILED"}}),
        json!({"version":1,"id":"1","result":[],"private_path":"secret"}),
        json!({"version":1,"id":"1","error":{"code":"secret path / sql / credential"}}),
    ] {
        let mut bytes = serde_json::to_vec(&message).unwrap();
        bytes.push(b'\n');
        assert_eq!(response(bytes).await, Err(AdapterError::IpcProtocolError));
    }
    assert_eq!(
        response(b"not-json\n".to_vec()).await,
        Err(AdapterError::IpcProtocolError)
    );
    assert_eq!(
        response(b"{\"version\":1,\"id\":\"1\",\"result\":[]}".to_vec()).await,
        Err(AdapterError::IpcDisconnected)
    );
    assert_eq!(
        response(vec![b'x'; MAX_RESPONSE_BYTES + 1]).await,
        Err(AdapterError::IpcProtocolError)
    );
    for (code, expected) in [
        ("CORE_READ_FAILED", AdapterError::CoreReadFailed),
        ("RESPONSE_TOO_LARGE", AdapterError::ResponseTooLarge),
        (
            "ASSET_NOT_FOUND",
            AdapterError::Domain(crate::mcp_read::ReadError::AssetNotFound),
        ),
    ] {
        let bytes = format!("{{\"version\":1,\"id\":\"1\",\"error\":{{\"code\":\"{code}\"}}}}\n")
            .into_bytes();
        assert_eq!(response(bytes).await, Err(expected));
    }
}

#[tokio::test]
async fn oversized_tool_params_are_rejected_before_writing_to_ipc() {
    let (client, _peer) = tokio::io::duplex(1);
    let result = exchange(
        &mut BufReader::new(client),
        "1",
        "mcp_search_assets",
        json!({"category_ids":vec!["\0".repeat(128);100]}),
    )
    .await;
    assert_eq!(
        result,
        Err(AdapterError::Domain(
            crate::mcp_read::ReadError::InvalidInput
        ))
    );
}

#[tokio::test]
async fn exchange_serializes_concurrent_calls_and_detects_idle_disconnect() {
    let (client, server) = tokio::io::duplex(4096);
    let (requests, receiver) = mpsc::channel(32);
    let actor = tokio::spawn(exchange_loop(client, receiver));
    let client = Client { requests };
    let peer = tokio::spawn(async move {
        let mut server = BufReader::new(server);
        for expected in ["1", "2"] {
            let mut line = String::new();
            server.read_line(&mut line).await.unwrap();
            let value: Value = serde_json::from_str(&line).unwrap();
            assert_eq!(value["id"], expected);
            let reply = json!({"version":1,"id":expected,"result":[value["method"]]});
            let mut bytes = serde_json::to_vec(&reply).unwrap();
            bytes.push(b'\n');
            server.write_all(&bytes).await.unwrap();
        }
        // Dropping the peer with no outstanding call must wake the supervisor.
    });
    let (categories, tags) = tokio::join!(client.call("list_categories"), client.call("list_tags"));
    assert_eq!(categories.unwrap(), json!(["list_categories"]));
    assert_eq!(tags.unwrap(), json!(["list_tags"]));
    peer.await.unwrap();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), actor)
            .await
            .unwrap()
            .unwrap(),
        AdapterError::IpcDisconnected
    );
    assert_eq!(
        client.call("list_tags").await,
        Err(AdapterError::IpcDisconnected)
    );
}

#[tokio::test]
async fn stalled_peer_times_out_and_releases_pending_calls() {
    let (stream, _peer) = tokio::io::duplex(4096);
    let (requests, receiver) = mpsc::channel(32);
    let actor = tokio::spawn(exchange_loop(stream, receiver));
    let client = Client { requests };
    let (first, second) = tokio::join!(client.call("list_categories"), client.call("list_tags"));
    assert_eq!(first, Err(AdapterError::IpcTimeout));
    assert_eq!(second, Err(AdapterError::IpcDisconnected));
    assert_eq!(actor.await.unwrap(), AdapterError::IpcTimeout);
}

#[test]
fn rejects_network_and_arbitrary_local_endpoints() {
    for endpoint in [
        "",
        "http://localhost:1420",
        r"\\server\pipe\yingqu-mcp-v1-000000000000000000000000",
        "/tmp/private.sock",
        r"\\.\pipe\other-service",
    ] {
        assert_eq!(
            validate_endpoint(endpoint),
            Err(AdapterError::InvalidEndpoint)
        );
    }
}
