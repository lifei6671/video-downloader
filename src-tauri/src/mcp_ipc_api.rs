use super::Core;
use crate::core_ipc::{IpcStatus, Server};
use anyhow::{bail, Result};
use std::sync::Arc;

impl Core {
    /// Authorize and commit under the same Core lock used by desktop writes.
    pub fn write_mcp_tool(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, crate::mcp_write::WriteError> {
        use crate::mcp_write::WriteError;
        let written = {
            let inner = self.inner.lock().unwrap();
            if inner.closing {
                return Err(WriteError::ServiceUnavailable);
            }
            if !inner.mcp_write_enabled {
                return Err(WriteError::WriteDisabled);
            }
            crate::mcp_write::dispatch(&inner.db, method, params)
                .map_err(WriteError::from_domain)?
        };
        (self.emit)(super::TaskEvent::LibraryChanged(
            super::library_api::LibraryChange {
                scope: written.scope,
                asset_ids: written.asset_ids,
            },
        ));
        Ok(written.value)
    }

    pub fn mcp_write_enabled(&self) -> bool {
        self.inner.lock().unwrap().mcp_write_enabled
    }

    /// A desktop-only permission, reset on service stop and application restart.
    pub async fn set_mcp_write_enabled(&self, enabled: bool) -> Result<()> {
        let server = self.mcp_ipc.lock().await;
        let mut inner = self.inner.lock().unwrap();
        if enabled && (inner.closing || !server.as_ref().is_some_and(|s| s.status().enabled)) {
            bail!("MCP_SERVICE_DISABLED");
        }
        inner.mcp_write_enabled = enabled;
        Ok(())
    }
    /// Run one whitelisted public read against this Core's existing connection.
    pub fn read_mcp_tool(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, crate::mcp_read::ReadError> {
        let inner = self.inner.lock().unwrap();
        if inner.closing {
            return Err(crate::mcp_read::ReadError::ServiceUnavailable);
        }
        crate::mcp_read::dispatch(&inner.db, method, params)
    }
    /// Return the actual state of this Core's optional local IPC listener.
    pub async fn mcp_ipc_status(&self) -> IpcStatus {
        self.mcp_ipc
            .lock()
            .await
            .as_ref()
            .map_or_else(IpcStatus::disabled, Server::status)
    }

    /// Enable or stop local IPC. New Core instances always begin disabled.
    ///
    /// Returns an error if the Core is closing or the OS endpoint cannot bind.
    pub async fn set_mcp_ipc_enabled(self: &Arc<Self>, enabled: bool) -> Result<IpcStatus> {
        // Serialize enable/disable through completion, including active connection teardown.
        let mut server = self.mcp_ipc.lock().await;
        if enabled && self.inner.lock().unwrap().closing {
            bail!("INVALID_STATE");
        }
        if let Some(current) = server
            .as_ref()
            .filter(|current| enabled && current.status().enabled)
        {
            return Ok(current.status());
        }
        if let Some(previous) = server.take() {
            self.inner.lock().unwrap().mcp_write_enabled = false;
            previous.stop().await?;
        }
        if enabled {
            *server = Some(Server::start(Arc::downgrade(self), &self.ipc_profile).await?);
        }
        Ok(server
            .as_ref()
            .map_or_else(IpcStatus::disabled, Server::status))
    }
}

#[cfg(feature = "mcp")]
impl Core {
    pub async fn mcp_service_status(&self) -> crate::mcp_connection::ServiceStatus {
        self.mcp_http.lock().await.as_ref().map_or_else(crate::mcp_connection::ServiceStatus::disabled, crate::mcp_http::Server::status)
    }
    pub async fn set_mcp_service_enabled(self: &Arc<Self>, enabled: bool) -> Result<crate::mcp_connection::ServiceStatus> {
        self.set_mcp_service_enabled_on_port(enabled, crate::mcp_http::PORT).await
    }
    /// Rust integration fixtures use an OS-assigned port; the interface is always loopback.
    pub async fn set_mcp_service_enabled_on_port(self: &Arc<Self>, enabled: bool, port: u16) -> Result<crate::mcp_connection::ServiceStatus> {
        let mut server = self.mcp_http.lock().await;
        if enabled && self.inner.lock().unwrap().closing { bail!("INVALID_STATE"); }
        if let Some(current) = server.as_ref().filter(|current| enabled && current.status().enabled) { return Ok(current.status()); }
        if let Some(previous) = server.take() {
            self.inner.lock().unwrap().mcp_write_enabled = false;
            previous.cancel();
            let ipc = self.set_mcp_ipc_enabled(false).await;
            let http = previous.stop().await;
            ipc?; http?;
        }
        if enabled {
            self.inner.lock().unwrap().mcp_write_enabled = false;
            let ipc = self.set_mcp_ipc_enabled(true).await?;
            match crate::mcp_http::Server::start(ipc.endpoint.ok_or_else(|| anyhow::anyhow!("IPC_SERVICE_FAILED"))?, port).await {
                Ok(started) => *server = Some(started),
                Err(error) => { self.set_mcp_ipc_enabled(false).await?; return Err(error); }
            }
        } else {
            self.set_mcp_ipc_enabled(false).await?;
        }
        Ok(server.as_ref().map_or_else(crate::mcp_connection::ServiceStatus::disabled, crate::mcp_http::Server::status))
    }
}
