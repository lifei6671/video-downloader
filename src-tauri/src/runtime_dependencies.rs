use serde::{Deserialize, Serialize};
use anyhow::{Context, Result};
use std::{io::Write, path::{Path, PathBuf}};

/// Rust retains the pre-install state because killing a sidecar bypasses its
/// Python exception rollback. Called only for Core's validated dependency IDs.
pub(crate) struct InstallRollback {
    target: PathBuf,
    had_target: bool,
    selection: Option<PathBuf>,
    selection_before: Option<Vec<u8>>,
}

fn read_optional(path: &Path) -> Result<Option<Vec<u8>>> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).context("IO_ERROR"),
    }
}

impl InstallRollback {
    pub(crate) fn capture(root: &Path, tool: &str, variant: Option<&str>) -> Result<Self> {
        let target = root.join(if tool == "fun-asr-nano" && variant == Some("cuda") {
            "fun-asr-nano-cuda"
        } else { tool });
        let selection = (tool == "fun-asr-nano").then(|| root.join("fun-asr-nano-backend.json"));
        let selection_before = selection.as_deref().map(read_optional).transpose()?.flatten();
        Ok(Self { had_target: target.exists(), target, selection, selection_before })
    }

    pub(crate) fn restore(&self, stage: &Path) -> Result<()> {
        let name = self.target.file_name().context("IO_ERROR")?;
        let backup = stage.join("previous").join(name);
        let rejected = stage.join("parent-rejected");
        if backup.exists() || (!self.had_target && !stage.join("prepared").exists() && self.target.exists()) {
            std::fs::create_dir_all(&rejected).context("IO_ERROR")?;
            if self.target.exists() {
                std::fs::rename(&self.target, rejected.join(name)).context("IO_ERROR")?;
            }
            if backup.exists() {
                std::fs::rename(backup, &self.target).context("IO_ERROR")?;
            }
        }
        if let Some(selection) = &self.selection {
            if read_optional(selection)? != self.selection_before {
                if let Some(bytes) = &self.selection_before {
                    let mut restored = tempfile::NamedTempFile::new_in(stage).context("IO_ERROR")?;
                    restored.write_all(bytes).context("IO_ERROR")?;
                    restored.persist(selection).map_err(|_| anyhow::anyhow!("IO_ERROR"))?;
                } else {
                    std::fs::remove_file(selection).context("IO_ERROR")?;
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct RuntimeStatus {
    pub platform: String,
    pub directory: String,
    pub tools: Vec<RuntimeTool>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct RuntimeTool {
    pub id: String,
    pub available: bool,
    pub provider: Option<String>,
    pub version: Option<String>,
    pub path: Option<String>,
    pub source: Option<String>,
    pub can_install: bool,
    #[serde(default)]
    pub download_bytes: Option<u64>,
    #[serde(default)]
    pub variants: Vec<String>,
    #[serde(default)]
    pub variant_download_bytes: std::collections::HashMap<String, u64>,
    #[serde(default)]
    pub variant_model_bytes: std::collections::HashMap<String, u64>,
}
