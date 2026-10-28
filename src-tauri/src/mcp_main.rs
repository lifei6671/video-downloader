use std::{process::ExitCode, time::Duration};
use yingqu_core::mcp_adapter;

fn main() -> ExitCode {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 2 || args[0] != "--endpoint" {
        eprintln!("Usage: yingqu-mcp --endpoint <local-core-endpoint>");
        return ExitCode::from(2);
    }
    let Some(endpoint) = args[1].to_str() else {
        eprintln!("INVALID_ENDPOINT");
        return ExitCode::from(2);
    };
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(_) => {
            eprintln!("MCP_RUNTIME_FAILED");
            return ExitCode::FAILURE;
        }
    };
    let result = runtime.block_on(mcp_adapter::run(endpoint));
    // Tokio stdin uses an uncancellable blocking read. IPC loss must still exit
    // while the MCP client keeps stdin open; application tasks are joined by run.
    // https://docs.rs/tokio/1.53.2/tokio/io/fn.stdin.html
    runtime.shutdown_timeout(Duration::from_millis(100));
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{}", error.code());
            ExitCode::FAILURE
        }
    }
}
