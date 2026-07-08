//! `calyx-leapable` stdio entrypoint.
//!
//! Reads newline-delimited JSON-RPC 2.0 requests from stdin and writes exactly
//! one response line to stdout for every non-notification request. Diagnostics
//! are stderr-only so the Bun sidecar can treat stdout as protocol bytes.

use std::io::{self, BufRead, Write};
use std::process::ExitCode;

use calyx_leapable::{Engine, EngineConfig, LEAPABLE_CAPABILITIES};
use calyx_mcp::{JsonRpcId, decode_jsonrpc_request};

fn main() -> ExitCode {
    install_panic_hook();
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--build-info") {
        return print_build_info(&args);
    }
    let config = match EngineConfig::from_args(&args) {
        Ok(config) => config,
        Err(error) => {
            eprintln!("calyx-leapable: {}: {}", error.code, error.message);
            return ExitCode::from(2);
        }
    };
    let mut engine = Engine::new(config);
    eprintln!("calyx-leapable: stdio engine ready");

    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut out = stdout.lock();

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(line) => line,
            Err(error) => {
                eprintln!("calyx-leapable: stdin read error: {error}");
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
                eprintln!("calyx-leapable: {}: {}", error.code, error.message);
                continue;
            }
        };
        let is_notification = request.id.is_none() || matches!(request.id, Some(JsonRpcId::Null));
        let response = engine.dispatch(request);
        if is_notification {
            continue;
        }
        match serde_json::to_string(&response) {
            Ok(line) => {
                if let Err(error) = writeln!(out, "{line}") {
                    eprintln!("calyx-leapable: stdout write error: {error}");
                    return ExitCode::FAILURE;
                }
                if let Err(error) = out.flush() {
                    eprintln!("calyx-leapable: stdout flush error: {error}");
                    return ExitCode::FAILURE;
                }
            }
            Err(error) => eprintln!("calyx-leapable: response serialize error: {error}"),
        }
    }
    ExitCode::SUCCESS
}

fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        eprintln!("calyx-leapable: request panic isolated: {info}");
    }));
}

fn print_build_info(args: &[String]) -> ExitCode {
    if args != ["--build-info"] {
        eprintln!("calyx-leapable: --build-info takes no other arguments");
        return ExitCode::from(2);
    }
    let mut report = match serde_json::to_value(calyx_buildinfo::build_info!(
        capabilities: LEAPABLE_CAPABILITIES
    )) {
        Ok(value) => value,
        Err(error) => {
            eprintln!("calyx-leapable: CALYX_BUILD_INFO_INVALID: serialize build info: {error}");
            return ExitCode::from(2);
        }
    };
    report["binary"] = serde_json::Value::from("calyx-leapable");
    report["executable"] = serde_json::Value::from(
        std::env::current_exe()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|error| format!("unavailable: {error}")),
    );
    match serde_json::to_string_pretty(&report) {
        Ok(json) => {
            println!("{json}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("calyx-leapable: response serialize error: {error}");
            ExitCode::from(2)
        }
    }
}
