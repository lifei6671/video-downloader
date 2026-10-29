//! Desktop-only metadata for the running loopback HTTP MCP listener.
use anyhow::{Context, Result};
use serde::Serialize;

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct ServiceStatus {
    pub enabled: bool,
    pub endpoint: Option<String>,
    pub error: Option<&'static str>,
}
impl ServiceStatus {
    pub fn disabled() -> Self { Self { enabled: false, endpoint: None, error: None } }
}
#[derive(Debug, Serialize)]
pub struct ConnectionInfo {
    pub status: ServiceStatus,
    pub write_enabled: bool,
}
pub fn connection_info(status: ServiceStatus, write_enabled: bool) -> ConnectionInfo {
    ConnectionInfo { write_enabled: status.enabled && write_enabled, status }
}
impl ConnectionInfo {
    pub fn url(&self) -> Result<&str> {
        self.status.endpoint.as_deref().filter(|_| self.status.enabled).context("MCP_CONNECTION_UNAVAILABLE")
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn disabled_service_never_offers_an_address_or_write_permission() {
        let info = connection_info(ServiceStatus::disabled(), true);
        assert!(info.url().is_err()); assert!(!info.write_enabled);
    }
    #[test]
    fn connection_address_is_the_actual_http_endpoint_without_a_command() {
        let endpoint = "http://127.0.0.1:17653/mcp";
        let info = connection_info(ServiceStatus { enabled: true, endpoint: Some(endpoint.into()), error: None }, false);
        assert_eq!(info.url().unwrap(), endpoint);
        let json = serde_json::to_string(&info).unwrap();
        assert!(!json.contains("command")); assert!(!json.contains("adapter_path"));
    }
}
