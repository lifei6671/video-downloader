//! Loopback-only Streamable HTTP transport. Core and its existing IPC own all data.
use crate::{core_ipc::MAX_REQUEST_BYTES, mcp_adapter::Adapter, mcp_connection::ServiceStatus};
use anyhow::{Context, Result};
use axum::{extract::{Request, State}, http::StatusCode, middleware::{self, Next}, response::{IntoResponse, Response}, Router};
use rmcp::transport::streamable_http_server::{session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService};
use std::{net::{Ipv4Addr, SocketAddrV4}, time::Duration};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

// A stable local URL lets clients retain their configuration across app restarts.
pub const PORT: u16 = 17653;

pub(crate) struct Server {
    url: String,
    cancel: CancellationToken,
    task: JoinHandle<std::io::Result<()>>,
}
async fn stop_requests(State(cancel): State<CancellationToken>, request: Request, next: Next) -> Response {
    tokio::select! {
        biased;
        _ = cancel.cancelled() => StatusCode::SERVICE_UNAVAILABLE.into_response(),
        response = next.run(request) => response,
    }
}
impl Server {
    pub(crate) async fn start(endpoint: String, port: u16) -> Result<Self> {
        let listener = tokio::net::TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, port))
            .await.context("MCP_HTTP_BIND_FAILED")?;
        let address = listener.local_addr().context("MCP_HTTP_BIND_FAILED")?;
        let cancel = CancellationToken::new();
        let config = StreamableHttpServerConfig::default()
            .with_legacy_session_mode(false)
            .with_json_response(true)
            .with_sse_keep_alive(None)
            .with_max_request_body_bytes(MAX_REQUEST_BYTES)
            .with_allowed_hosts([address.to_string()])
            .with_allowed_origins([format!("http://{address}")])
            .enforce_origin_validation()
            .with_cancellation_token(cancel.clone());
        let service: StreamableHttpService<Adapter, LocalSessionManager> = StreamableHttpService::new(
            move || Ok(Adapter::new(endpoint.clone())), Default::default(), config,
        );
        let app = Router::new().route_service("/mcp", service)
            .layer(middleware::from_fn_with_state(cancel.clone(), stop_requests));
        let stopping = cancel.clone();
        let finished = cancel.clone();
        let task = tokio::spawn(async move {
            let result = axum::serve(listener, app)
                .with_graceful_shutdown(stopping.cancelled_owned()).await;
            finished.cancel();
            result
        });
        Ok(Self { url: format!("http://{address}/mcp"), cancel, task })
    }
    pub(crate) fn status(&self) -> ServiceStatus {
        let enabled = !self.cancel.is_cancelled() && !self.task.is_finished();
        ServiceStatus { enabled, endpoint: enabled.then(|| self.url.clone()), error: (!enabled).then_some("MCP_HTTP_SERVICE_FAILED") }
    }
    pub(crate) fn cancel(&self) { self.cancel.cancel(); }
    pub(crate) async fn stop(mut self) -> Result<()> {
        self.cancel.cancel();
        match tokio::time::timeout(Duration::from_secs(5), &mut self.task).await {
            Ok(result) => result.context("MCP_HTTP_SERVICE_FAILED")?.context("MCP_HTTP_SERVICE_FAILED"),
            Err(_) => { self.task.abort(); let _ = (&mut self.task).await; anyhow::bail!("MCP_HTTP_SERVICE_FAILED"); }
        }
    }
}
impl Drop for Server {
    fn drop(&mut self) { self.cancel.cancel(); self.task.abort(); }
}

#[cfg(test)]
#[path = "mcp_http_tests.rs"]
mod tests;
