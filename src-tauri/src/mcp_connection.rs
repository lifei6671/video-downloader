//! Desktop-only connection metadata. Public MCP read views never include these paths.
use crate::core_ipc::IpcStatus;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize)]
pub struct ClientConfigs {
    pub codex: String,
    pub json: String,
}

#[derive(Debug, Serialize)]
pub struct ConnectionInfo {
    pub status: IpcStatus,
    pub write_enabled: bool,
    pub adapter_path: Option<String>,
    pub adapter_error: Option<&'static str>,
    pub configs: Option<ClientConfigs>,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigFormat {
    Codex,
    Json,
}

pub fn adapter_path(executable: &Path) -> Result<PathBuf> {
    let path = executable
        .parent()
        .context("MCP_ADAPTER_MISSING")?
        .join(if cfg!(windows) {
            "yingqu-mcp.exe"
        } else {
            "yingqu-mcp"
        });
    if !path.is_file() {
        bail!("MCP_ADAPTER_MISSING");
    }
    if path.to_str().is_none() {
        bail!("MCP_ADAPTER_PATH_INVALID");
    }
    Ok(path)
}

pub fn connection_info(status: IpcStatus, executable: &Path) -> Result<ConnectionInfo> {
    let (adapter_path, adapter_error) = match adapter_path(executable) {
        Ok(path) => (path.to_str().map(str::to_owned), None),
        Err(error) => (
            None,
            Some(if error.to_string() == "MCP_ADAPTER_PATH_INVALID" {
                "MCP_ADAPTER_PATH_INVALID"
            } else {
                "MCP_ADAPTER_MISSING"
            }),
        ),
    };
    let configs = if status.enabled {
        match (adapter_path.as_deref(), status.endpoint.as_deref()) {
            (Some(command), Some(endpoint)) => {
                let args = ["--endpoint", endpoint];
                Some(ClientConfigs {
                    // JSON basic strings also provide TOML basic-string escaping.
                    codex: format!(
                        "[mcp_servers.yingqu]\ncommand = {}\nargs = {}\n",
                        serde_json::to_string(command)?,
                        serde_json::to_string(&args)?
                    ),
                    json: serde_json::to_string_pretty(&serde_json::json!({
                        "mcpServers": {"yingqu": {"command": command, "args": args}}
                    }))?,
                })
            }
            _ => None,
        }
    } else {
        None
    };
    Ok(ConnectionInfo {
        write_enabled: false,
        status,
        adapter_path,
        adapter_error,
        configs,
    })
}

impl ConnectionInfo {
    pub fn config(&self, format: ConfigFormat) -> Result<&str> {
        let configs = self.configs.as_ref().context("MCP_CONFIG_UNAVAILABLE")?;
        Ok(match format {
            ConfigFormat::Codex => &configs.codex,
            ConfigFormat::Json => &configs.json,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_adapter_and_disabled_service_do_not_offer_configs() {
        let directory = tempfile::tempdir().unwrap();
        let exe = directory.path().join("app.exe");
        let info = connection_info(IpcStatus::disabled(), &exe).unwrap();
        assert_eq!(info.adapter_error, Some("MCP_ADAPTER_MISSING"));
        assert!(info.config(ConfigFormat::Codex).is_err());
        let adapter = directory.path().join(if cfg!(windows) {
            "yingqu-mcp.exe"
        } else {
            "yingqu-mcp"
        });
        std::fs::write(adapter, b"fixture").unwrap();
        let info = connection_info(IpcStatus::disabled(), &exe).unwrap();
        assert!(info.adapter_path.is_some());
        assert!(info.configs.is_none());
    }

    #[test]
    fn config_preserves_unicode_spaces_and_separate_arguments() {
        let directory = tempfile::tempdir().unwrap();
        let installed = directory.path().join("中文 MCP app");
        std::fs::create_dir(&installed).unwrap();
        let adapter = installed.join(if cfg!(windows) {
            "yingqu-mcp.exe"
        } else {
            "yingqu-mcp"
        });
        std::fs::write(&adapter, b"fixture").unwrap();
        let endpoint = r"\\.\pipe\yingqu-mcp-v1-0123456789abcdef01234567";
        let info = connection_info(
            IpcStatus {
                enabled: true,
                endpoint: Some(endpoint.into()),
                error: None,
            },
            &installed.join("app.exe"),
        )
        .unwrap();
        let json: serde_json::Value =
            serde_json::from_str(info.config(ConfigFormat::Json).unwrap()).unwrap();
        assert_eq!(
            json["mcpServers"]["yingqu"]["command"],
            adapter.to_str().unwrap()
        );
        assert_eq!(
            json["mcpServers"]["yingqu"]["args"],
            serde_json::json!(["--endpoint", endpoint])
        );
        assert!(!json.to_string().contains("settings"));
        assert!(info
            .config(ConfigFormat::Codex)
            .unwrap()
            .starts_with("[mcp_servers.yingqu]\ncommand = \""));
        // A failed listener must not publish a usable configuration.
        assert!(connection_info(
            IpcStatus {
                enabled: false,
                endpoint: None,
                error: Some("IPC_SERVICE_FAILED")
            },
            &installed.join("app.exe")
        )
        .unwrap()
        .configs
        .is_none());
    }
}
