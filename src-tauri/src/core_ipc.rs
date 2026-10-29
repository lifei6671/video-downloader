//! Local, opt-in access to the running Core. This is not the MCP wire protocol.
use crate::desktop_core::Core;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{io, path::Path, sync::Weak};
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader},
    sync::watch,
    task::{JoinHandle, JoinSet},
};

#[cfg(windows)]
#[path = "core_ipc/windows.rs"]
mod transport;
#[cfg(unix)]
#[path = "core_ipc/unix.rs"]
mod transport;

pub(crate) const VERSION: u32 = 1;
pub(crate) const MAX_REQUEST_BYTES: usize = 64 * 1024;
pub(crate) const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
const MAX_CONNECTIONS: usize = 32;

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct IpcStatus {
    pub enabled: bool,
    pub endpoint: Option<String>,
    pub error: Option<&'static str>,
}
impl IpcStatus {
    pub(crate) fn disabled() -> Self {
        Self {
            enabled: false,
            endpoint: None,
            error: None,
        }
    }
}

pub(crate) struct Server {
    endpoint: String,
    stop: watch::Sender<bool>,
    task: JoinHandle<io::Result<()>>,
}
impl Server {
    pub(crate) async fn start(core: Weak<Core>, profile: &Path) -> Result<Self> {
        let listener = transport::Listener::bind(profile)
            .await
            .context("IPC_BIND_FAILED")?;
        let endpoint = listener.endpoint().to_owned();
        let (stop, mut stopping) = watch::channel(false);
        let stop_on_exit = stop.clone();
        let task = tokio::spawn(async move {
            let mut listener = listener;
            let mut connections = JoinSet::new();
            let outcome = loop {
                tokio::select! {
                    biased;
                    _ = stopping.changed() => break Ok(()),
                    Some(result) = connections.join_next(), if !connections.is_empty() => {
                        // Per-client read/write failures do not take down the listener.
                        if result.is_err() {
                            eprintln!("MCP IPC connection task failed");
                        }
                    }
                    stream = listener.accept(), if connections.len() < MAX_CONNECTIONS => {
                        match stream {
                            Ok(stream) => {
                                connections.spawn(serve(stream, core.clone(), stopping.clone()));
                            }
                            Err(error) => break Err(error),
                        }
                    }
                }
            };
            let _ = stop_on_exit.send(true);
            while connections.join_next().await.is_some() {}
            outcome
        });
        Ok(Self {
            endpoint,
            stop,
            task,
        })
    }

    pub(crate) fn status(&self) -> IpcStatus {
        let enabled = !self.task.is_finished();
        IpcStatus {
            enabled,
            endpoint: enabled.then(|| self.endpoint.clone()),
            error: (!enabled).then_some("IPC_SERVICE_FAILED"),
        }
    }

    pub(crate) async fn stop(mut self) -> Result<()> {
        let _ = self.stop.send(true);
        (&mut self.task)
            .await
            .context("IPC_SERVICE_FAILED")?
            .context("IPC_SERVICE_FAILED")
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        // Also cover runtime teardown or a caller dropping an in-progress stop.
        self.task.abort();
    }
}

fn profile_key(profile: &Path, user: &str) -> String {
    let mut hash = Sha256::new();
    hash.update(profile.as_os_str().as_encoded_bytes());
    hash.update([0]);
    hash.update(user);
    format!("{:x}", hash.finalize())[..24].to_owned()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    version: u32,
    id: String,
    method: String,
    #[serde(default = "empty_params")]
    params: Value,
}
fn empty_params() -> Value {
    json!({})
}
fn failure(id: Option<&str>, code: &str) -> Value {
    json!({"version":VERSION,"id":id,"error":{"code":code}})
}
fn dispatch(core: &Weak<Core>, frame: &[u8]) -> Value {
    let request: Request = match serde_json::from_slice(frame) {
        Ok(request) => request,
        Err(_) => return failure(None, "INVALID_REQUEST"),
    };
    let id = Some(request.id.as_str());
    if request.id.is_empty() || request.id.len() > 128 {
        return failure(None, "INVALID_REQUEST");
    }
    if request.version != VERSION {
        return failure(id, "VERSION_UNSUPPORTED");
    }
    if crate::mcp_operations::METHODS.contains(&request.method.as_str()) {
        let Some(core) = core.upgrade() else { return failure(id, "SERVICE_UNAVAILABLE"); };
        return match core.mcp_operation_tool(&request.method, request.params) {
            Ok(value) => json!({"version":VERSION,"id":request.id,"result":value}),
            Err(error) => failure(id, error.code()),
        };
    }
    if crate::mcp_write::METHODS.contains(&request.method.as_str()) {
        let Some(core) = core.upgrade() else { return failure(id, "SERVICE_UNAVAILABLE"); };
        return match core.write_mcp_tool(&request.method, request.params) {
            Ok(value) => json!({"version":VERSION,"id":request.id,"result":value}),
            Err(error) => failure(id, error.code()),
        };
    }
    if matches!(request.method.as_str(), "mcp_get_clip" | "mcp_get_keyframes" | "mcp_search_assets" | "mcp_get_asset" | "mcp_read_transcript" | "mcp_list_categories" | "mcp_list_tags" | "mcp_list_notes" | "mcp_read_note") {
        let Some(core) = core.upgrade() else { return failure(id, "SERVICE_UNAVAILABLE"); };
        return match core.read_mcp_tool(&request.method, request.params) {
            Ok(value) => json!({"version":VERSION,"id":request.id,"result":value}),
            Err(error) => failure(id, error.code()),
        };
    }
    if !matches!(
        request.method.as_str(),
        "ping" | "list_categories" | "list_tags"
    ) {
        return failure(id, "METHOD_NOT_ALLOWED");
    }
    if request.params != empty_params() {
        return failure(id, "INVALID_PARAMS");
    }
    let Some(core) = core.upgrade() else {
        return failure(id, "SERVICE_UNAVAILABLE");
    };
    let result = match request.method.as_str() {
        "ping" => Ok(json!({"service":"yingqu-core","protocol_version":VERSION})),
        "list_categories" => core
            .library_categories()
            .and_then(|items| Ok(serde_json::to_value(items)?)),
        "list_tags" => core
            .library_tags()
            .and_then(|items| Ok(serde_json::to_value(items)?)),
        _ => unreachable!("method was checked above"),
    };
    match result {
        Ok(value) => json!({"version":VERSION,"id":request.id,"result":value}),
        // Never serialize SQLite errors, paths, params or settings to the client.
        Err(_) => failure(id, "CORE_READ_FAILED"),
    }
}

async fn serve(stream: impl AsyncRead + AsyncWrite + Unpin, core: Weak<Core>, mut stopping: watch::Receiver<bool>) -> io::Result<()> {
    let mut stream = BufReader::new(stream);
    let mut frame = Vec::new();
    loop {
        let buffer = tokio::select! {
            biased;
            _ = stopping.wait_for(|stop| *stop) => return Ok(()),
            result = stream.fill_buf() => result?,
        };
        if buffer.is_empty() {
            return Ok(());
        }
        let newline = buffer.iter().position(|byte| *byte == b'\n');
        let count = newline.map_or(buffer.len(), |index| index + 1);
        if frame.len() + count > MAX_REQUEST_BYTES {
            tokio::select! {
                biased;
                _ = stopping.wait_for(|stop| *stop) => return Ok(()),
                result = write_response(&mut stream, failure(None, "REQUEST_TOO_LARGE")) => result?,
            }
            return Ok(());
        }
        frame.extend_from_slice(&buffer[..count]);
        stream.consume(count);
        if newline.is_some() {
            let audio = serde_json::from_slice::<Request>(&frame).ok().filter(|r| r.method == "mcp_get_audio_excerpt");
            let response = if let Some(request) = audio {
                if request.id.is_empty() || request.id.len() > 128 { failure(None, "INVALID_REQUEST") }
                else if request.version != VERSION { failure(Some(&request.id), "VERSION_UNSUPPORTED") }
                else if let Some(core) = core.upgrade() {
                    let cancel = async {
                        tokio::select! {
                            _ = stopping.wait_for(|stop| *stop) => {},
                            _ = stream.fill_buf() => {},
                        }
                    };
                    match core.mcp_audio_excerpt(request.params, cancel).await {
                        Ok(value) => json!({"version":VERSION,"id":request.id,"result":value}),
                        Err(error) => failure(Some(&request.id), error.code()),
                    }
                } else { failure(Some(&request.id), "SERVICE_UNAVAILABLE") }
            } else { dispatch(&core, &frame) };
            if *stopping.borrow() { return Ok(()); }
            tokio::select! {
                biased;
                _ = stopping.wait_for(|stop| *stop) => return Ok(()),
                result = write_response(&mut stream, response) => result?,
            }
            frame.clear();
        }
    }
}

async fn write_response(stream: &mut (impl AsyncWrite + Unpin), response: Value) -> io::Result<()> {
    let mut bytes = serde_json::to_vec(&response)?;
    if bytes.len() + 1 > MAX_RESPONSE_BYTES {
        bytes = serde_json::to_vec(&failure(response["id"].as_str(), "RESPONSE_TOO_LARGE"))?;
    }
    bytes.push(b'\n');
    stream.write_all(&bytes).await?;
    stream.flush().await
}

#[cfg(test)]
#[path = "core_ipc/tests.rs"]
mod tests;
