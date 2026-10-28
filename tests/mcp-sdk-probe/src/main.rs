//! WB-201 synthetic stdio probe. No Core, database, IPC or media access.
use rmcp::{
    Json, ServerHandler, ServiceExt,
    handler::server::wrapper::Parameters,
    model::{Implementation, ServerCapabilities, ServerConfig},
    schemars, tool, tool_handler, tool_router,
};

#[derive(Clone)]
struct Probe;

#[derive(serde::Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct EchoInput {
    /// Synthetic text only; never pass user material to this probe.
    text: String,
}

#[derive(serde::Serialize, schemars::JsonSchema)]
struct EchoOutput {
    text: String,
}

#[tool_router]
impl Probe {
    #[tool(
        description = "WB-201 SDK compatibility probe. Echo synthetic text without accessing application data.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    fn probe_echo(&self, Parameters(input): Parameters<EchoInput>) -> Json<EchoOutput> {
        Json(EchoOutput { text: input.text })
    }
}

#[tool_handler]
impl ServerHandler for Probe {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build()).with_server_info(
            Implementation::new("yingqu-wb201-probe", env!("CARGO_PKG_VERSION")),
        )
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    eprintln!("WB-201 synthetic SDK probe; no application connection");
    Probe
        .serve(rmcp::transport::stdio())
        .await?
        .waiting()
        .await?;
    Ok(())
}
