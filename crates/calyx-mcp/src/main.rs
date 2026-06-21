//! `calyx-mcp` stdio entrypoint.
//!
//! Reads newline-delimited JSON-RPC requests from stdin, dispatches each through
//! [`McpServer`], and writes newline-delimited JSON-RPC responses to stdout.
//! Protocol output is stdout-only; every diagnostic goes to stderr so a stray
//! log line can never corrupt the response stream. Notifications (requests with
//! no `id`) receive no reply, per JSON-RPC 2.0.

use std::io::{self, BufRead, Write};
use std::process::ExitCode;

use calyx_core::AuthN;
use calyx_mcp::jsonrpc::{JsonRpcId, decode_jsonrpc_request};
use calyx_mcp::server::McpServer;

fn main() -> ExitCode {
    let mut server = McpServer::new();
    if let Err(error) = calyx_mcp::tools::register_all(&mut server) {
        eprintln!("calyx-mcp: {}: {}", error.code, error.message);
        return ExitCode::FAILURE;
    }
    eprintln!("calyx-mcp: registered {} tools", server.tool_count());
    let authn = AuthN::InProcess {
        host_app_id: "calyx-mcp-stdio".to_string(),
    };

    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut out = stdout.lock();

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(line) => line,
            Err(error) => {
                eprintln!("calyx-mcp: stdin read error: {error}");
                return ExitCode::FAILURE;
            }
        };

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let request = match decode_jsonrpc_request(trimmed.as_bytes()) {
            Ok(request) => request,
            Err(error) => {
                // Malformed line: log to stderr and keep serving the next line.
                eprintln!("calyx-mcp: {}: {}", error.code, error.message);
                continue;
            }
        };

        // Notifications (no id) get no response.
        let is_notification = request.id.is_none() || matches!(request.id, Some(JsonRpcId::Null));
        let response = server.dispatch_with_authn(request, Some(&authn));
        if is_notification {
            continue;
        }

        match serde_json::to_string(&response) {
            Ok(line) => {
                if let Err(error) = writeln!(out, "{line}") {
                    eprintln!("calyx-mcp: stdout write error: {error}");
                    return ExitCode::FAILURE;
                }
                if let Err(error) = out.flush() {
                    eprintln!("calyx-mcp: stdout flush error: {error}");
                    return ExitCode::FAILURE;
                }
            }
            Err(error) => {
                eprintln!("calyx-mcp: response serialize error: {error}");
            }
        }
    }

    // EOF on stdin → clean shutdown.
    ExitCode::SUCCESS
}
