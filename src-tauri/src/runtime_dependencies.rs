use serde::{Deserialize, Serialize};

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
}
