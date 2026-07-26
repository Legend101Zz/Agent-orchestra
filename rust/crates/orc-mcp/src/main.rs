//! `pio-mcp`: the pi-orchestra MCP server binary.
//!
//! With no arguments it serves the seven `orch_*` tools over stdio (the shape an
//! MCP client launches). `--version`/`--help` are handled locally so a client or
//! operator can introspect the binary without opening a session. Config snippets
//! for Claude Code and Codex are emitted by `pio mcp print-config`.

use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        None => match orc_mcp::run_stdio() {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("pio-mcp: {error:#}");
                ExitCode::FAILURE
            }
        },
        Some("--version" | "-V") => {
            println!("pio-mcp {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Some("--help" | "-h") => {
            println!(
                "pio-mcp — pi-orchestra MCP (stdio) server\n\n\
                 usage: pio-mcp            serve the seven orch_* tools over stdio\n\
                 \x20      pio-mcp --version print the version\n\n\
                 register it with a client using: pio mcp print-config"
            );
            ExitCode::SUCCESS
        }
        Some(other) => {
            eprintln!("pio-mcp: unexpected argument {other:?}; run `pio-mcp --help`");
            ExitCode::FAILURE
        }
    }
}
