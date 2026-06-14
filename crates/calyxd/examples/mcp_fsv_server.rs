//! Manual FSV harness for PH65 · T05: binds the real [`CalyxMcpServer`] on a
//! loopback port with a real `adder` tool registered, prints the bound address,
//! and serves until SIGINT/kill. Used to prove — via the OS TCP listening table
//! (`Get-NetTCPConnection` / `ss -tlnp`), an independent Source of Truth — that
//! the listener is loopback-only, and to drive a real client round-trip.
//!
//! Run: `cargo run -p calyxd --example mcp_fsv_server -- 127.0.0.1:7755`

use std::sync::Arc;

use calyx_core::CalyxError;
use calyx_mcp::{McpServer, Tool, ToolDef};
use calyxd::mcp_server::CalyxMcpServer;
use serde_json::{Value, json};

struct AdderTool;

impl Tool for AdderTool {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "adder".into(),
            description: "add two integers".into(),
            use_when: "FSV transport probe".into(),
            input_schema: json!({"type":"object","properties":{"a":{"type":"integer"},"b":{"type":"integer"}},"required":["a","b"]}),
        }
    }

    fn call(&self, params: Value) -> Result<Value, CalyxError> {
        let a = params.get("a").and_then(Value::as_i64).unwrap_or(0);
        let b = params.get("b").and_then(Value::as_i64).unwrap_or(0);
        Ok(json!({ "sum": a + b }))
    }
}

fn main() {
    let addr = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "127.0.0.1:7755".to_string());
    let mut dispatcher = McpServer::new();
    dispatcher
        .register(Box::new(AdderTool))
        .expect("register adder");
    let server =
        match CalyxMcpServer::bind(addr.parse().expect("addr parses"), Arc::new(dispatcher)) {
            Ok(server) => server,
            Err(error) => {
                eprintln!("FSV-BIND-FAILED {error}");
                std::process::exit(1);
            }
        };
    let bound = server.local_addr().expect("local_addr");
    println!("FSV-LISTENING {bound}");
    server.run().expect("run");
}
