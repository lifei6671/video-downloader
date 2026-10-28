//! Public long-operation DTOs; no transcript bodies, engine arguments or paths.
#[cfg(feature = "mcp")]
use rmcp::schemars;
use serde::{Deserialize, Serialize};

pub const METHODS: &[&str] = &[
    "mcp_start_transcription",
    "mcp_get_operation",
    "mcp_cancel_operation",
];

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(rmcp::schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct StartInput {
    pub asset_id: String,
    pub file_id: String,
    #[serde(default)]
    pub force: bool,
}
#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "mcp", derive(rmcp::schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct OperationInput {
    pub operation_id: String,
}
#[derive(Serialize, Deserialize, PartialEq, Eq, Debug)]
#[cfg_attr(feature = "mcp", derive(rmcp::schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum State {
    Running,
    Completed,
    NoSpeech,
    Failed,
    Canceled,
    Interrupted,
}

#[derive(Serialize, Deserialize, Debug)]
#[cfg_attr(feature = "mcp", derive(rmcp::schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Operation {
    pub operation_id: String,
    pub asset_id: String,
    pub file_id: String,
    pub state: State,
    pub cancel_requested: bool,
    pub phase: Option<String>,
    pub backend: Option<String>,
    #[cfg_attr(feature = "mcp", schemars(transform = json_integer))]
    pub completed_segments: Option<u64>,
    #[cfg_attr(feature = "mcp", schemars(transform = json_integer))]
    pub total_segments: Option<u64>,
    pub error_code: Option<String>,
    pub has_result: bool,
    #[cfg_attr(feature = "mcp", schemars(transform = json_integer))]
    pub sequence: u64,
    #[cfg_attr(feature = "mcp", schemars(transform = json_integer))]
    pub updated_at: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum OperationError {
    InvalidInput,
    WriteDisabled,
    AssetNotFound,
    FileNotFound,
    FileMissing,
    DependencyBusy,
    TranscriptionBusy,
    OperationNotFound,
    ServiceUnavailable,
    CoreOperationFailed,
}
impl OperationError {
    pub fn code(self) -> &'static str {
        match self {
            Self::InvalidInput => "INVALID_INPUT",
            Self::WriteDisabled => "WRITE_DISABLED",
            Self::AssetNotFound => "ASSET_NOT_FOUND",
            Self::FileNotFound => "FILE_NOT_FOUND",
            Self::FileMissing => "FILE_MISSING",
            Self::DependencyBusy => "DEPENDENCY_BUSY",
            Self::TranscriptionBusy => "TRANSCRIPTION_BUSY",
            Self::OperationNotFound => "OPERATION_NOT_FOUND",
            Self::ServiceUnavailable => "SERVICE_UNAVAILABLE",
            Self::CoreOperationFailed => "CORE_OPERATION_FAILED",
        }
    }
    pub(crate) fn from_domain(error: anyhow::Error) -> Self {
        if error.to_string() == "INVALID_STATE" {
            return Self::ServiceUnavailable;
        }
        serde_json::from_value(serde_json::Value::String(error.to_string()))
            .unwrap_or(Self::CoreOperationFailed)
    }
}
#[cfg(feature = "mcp")]
fn json_integer(schema: &mut schemars::Schema) {
    if let Some(object) = schema.as_object_mut() {
        object.remove("format");
    }
}
