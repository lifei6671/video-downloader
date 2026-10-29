use super::AdapterError;
use crate::core_ipc::{MAX_REQUEST_BYTES, MAX_RESPONSE_BYTES, VERSION};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{io, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader},
    sync::{mpsc, oneshot},
    task::JoinHandle,
};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
#[cfg(windows)]
type Stream = tokio::net::windows::named_pipe::NamedPipeClient;
#[cfg(unix)]
type Stream = tokio::net::UnixStream;

fn validate_endpoint(endpoint: &str) -> Result<(), AdapterError> {
    #[cfg(windows)]
    let key = endpoint.strip_prefix(r"\\.\pipe\yingqu-mcp-v1-");
    #[cfg(unix)]
    let key = {
        // SAFETY: geteuid has no preconditions and does not retain memory.
        let uid = unsafe { libc::geteuid() };
        endpoint
            .strip_prefix(&format!("/tmp/yingqu-mcp-{uid}/"))
            .and_then(|key| key.strip_suffix(".sock"))
    };
    if key.is_some_and(|key| {
        key.len() == 24
            && key
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    }) {
        Ok(())
    } else {
        Err(AdapterError::InvalidEndpoint)
    }
}

#[cfg(windows)]
async fn connect_stream(endpoint: &str) -> io::Result<Stream> {
    loop {
        match tokio::net::windows::named_pipe::ClientOptions::new().open(endpoint) {
            Ok(stream) => return Ok(stream),
            // A listener can be between pipe instances, or at its connection cap.
            Err(error) if error.raw_os_error() == Some(231) => {
                tokio::time::sleep(Duration::from_millis(25)).await
            }
            Err(error) => return Err(error),
        }
    }
}
#[cfg(unix)]
async fn connect_stream(endpoint: &str) -> io::Result<Stream> {
    tokio::net::UnixStream::connect(endpoint).await
}

struct Call {
    method: &'static str,
    params: Value,
    reply: oneshot::Sender<Result<Value, AdapterError>>,
}
#[derive(Clone)]
pub(super) struct Client {
    requests: mpsc::Sender<Call>,
}
impl Client {
    #[cfg(test)]
    pub(super) async fn call(&self, method: &'static str) -> Result<Value, AdapterError> {
        self.request(method, json!({})).await
    }
    pub(super) async fn request(
        &self,
        method: &'static str,
        params: Value,
    ) -> Result<Value, AdapterError> {
        let (reply, response) = oneshot::channel();
        self.requests
            .send(Call {
                method,
                params,
                reply,
            })
            .await
            .map_err(|_| AdapterError::IpcDisconnected)?;
        response.await.unwrap_or(Err(AdapterError::IpcDisconnected))
    }
}

pub(super) struct Connection {
    pub(super) client: Client,
    pub(super) task: JoinHandle<AdapterError>,
}
impl Connection {
    pub(super) async fn connect(endpoint: &str) -> Result<Self, AdapterError> {
        validate_endpoint(endpoint)?;
        let stream = tokio::time::timeout(REQUEST_TIMEOUT, connect_stream(endpoint))
            .await
            .map_err(|_| AdapterError::IpcTimeout)?
            .map_err(|_| AdapterError::IpcUnavailable)?;
        let (requests, receiver) = mpsc::channel(32);
        let task = tokio::spawn(exchange_loop(stream, receiver));
        Ok(Self {
            client: Client { requests },
            task,
        })
    }
    pub(super) async fn close(&mut self) {
        // The supervisor may already have joined this task after IPC loss.
        if !self.task.is_finished() {
            self.task.abort();
            let _ = (&mut self.task).await;
        }
    }
}
impl Drop for Connection {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn exchange_loop(
    stream: impl AsyncRead + AsyncWrite + Unpin,
    mut requests: mpsc::Receiver<Call>,
) -> AdapterError {
    let mut stream = BufReader::new(stream);
    let mut sequence = 0u64;
    loop {
        tokio::select! {
            ready = stream.fill_buf() => return match ready {
                Ok([]) | Err(_) => AdapterError::IpcDisconnected,
                _ => AdapterError::IpcProtocolError,
            },
            request = requests.recv() => {
                let Some(mut request) = request else { return AdapterError::IpcDisconnected; };
                sequence += 1;
                let id = sequence.to_string();
                let deadline = if request.method == "mcp_get_audio_excerpt" { Duration::from_secs(35) } else { REQUEST_TIMEOUT };
                let outcome = tokio::select! {
                    result = tokio::time::timeout(deadline, exchange(&mut stream, &id, request.method, request.params)) => result.unwrap_or(Err(AdapterError::IpcTimeout)),
                    _ = request.reply.closed() => return AdapterError::IpcDisconnected,
                };
                let fatal = match outcome {
                    Err(AdapterError::CoreReadFailed | AdapterError::ResponseTooLarge | AdapterError::Domain(_) | AdapterError::WriteDomain(_) | AdapterError::OperationDomain(_)) | Ok(_) => None,
                    Err(error) => Some(error),
                };
                let _ = request.reply.send(outcome);
                if let Some(error) = fatal { return error; }
            }
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Reply {
    version: u32,
    id: String,
    result: Option<Value>,
    error: Option<RemoteError>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RemoteError {
    code: String,
}

async fn exchange(
    stream: &mut (impl AsyncWrite + Unpin + tokio::io::AsyncBufRead),
    id: &str,
    method: &str,
    params: Value,
) -> Result<Value, AdapterError> {
    let mut bytes =
        serde_json::to_vec(&json!({"version":VERSION,"id":id,"method":method,"params":params}))
            .map_err(|_| AdapterError::IpcProtocolError)?;
    bytes.push(b'\n');
    if bytes.len() > MAX_REQUEST_BYTES {
        return Err(AdapterError::Domain(
            crate::mcp_read::ReadError::InvalidInput,
        ));
    }
    stream
        .write_all(&bytes)
        .await
        .map_err(|_| AdapterError::IpcDisconnected)?;
    stream
        .flush()
        .await
        .map_err(|_| AdapterError::IpcDisconnected)?;
    let mut frame = Vec::new();
    loop {
        let buffer = stream
            .fill_buf()
            .await
            .map_err(|_| AdapterError::IpcDisconnected)?;
        if buffer.is_empty() {
            return Err(AdapterError::IpcDisconnected);
        }
        let newline = buffer.iter().position(|byte| *byte == b'\n');
        let count = newline.map_or(buffer.len(), |index| index + 1);
        if frame.len() + count > MAX_RESPONSE_BYTES {
            return Err(AdapterError::IpcProtocolError);
        }
        frame.extend_from_slice(&buffer[..count]);
        stream.consume(count);
        if newline.is_some() {
            break;
        }
    }
    let reply: Reply =
        serde_json::from_slice(&frame).map_err(|_| AdapterError::IpcProtocolError)?;
    if reply.version != VERSION || reply.id != id {
        return Err(AdapterError::IpcProtocolError);
    }
    match (reply.result, reply.error) {
        (Some(value), None) => Ok(value),
        (None, Some(error)) => Err(match error.code.as_str() {
            "CORE_READ_FAILED" => AdapterError::CoreReadFailed,
            "RESPONSE_TOO_LARGE" => AdapterError::ResponseTooLarge,
            "SERVICE_UNAVAILABLE" => AdapterError::IpcDisconnected,
            _ => serde_json::from_value(Value::String(error.code.clone()))
                .map(AdapterError::Domain)
                .or_else(|_| serde_json::from_value(Value::String(error.code.clone())).map(AdapterError::WriteDomain))
                .or_else(|_| serde_json::from_value(Value::String(error.code)).map(AdapterError::OperationDomain))
                .unwrap_or(AdapterError::IpcProtocolError),
        }),
        _ => Err(AdapterError::IpcProtocolError),
    }
}

#[cfg(test)]
#[path = "ipc_tests.rs"]
mod tests;
