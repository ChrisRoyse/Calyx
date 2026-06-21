//! Manual FSV harness for PH65 · T05: binds the real [`CalyxMcpServer`] on a
//! loopback port with a real `adder` tool registered, explicit mTLS material,
//! prints the bound address, and serves until SIGINT/kill. Used to prove — via
//! the OS TCP listening table (`Get-NetTCPConnection` / `ss -tlnp`), an
//! independent Source of Truth — that the listener is loopback-only and mTLS-only.
//!
//! Run: `cargo run -p calyxd --example mcp_fsv_server -- 127.0.0.1:7755 server-cert.pem server-key.pem client-ca.pem`

use std::path::PathBuf;
use std::sync::Arc;

use calyx_core::{MtlsConfig, TlsConfig};
use calyx_mcp::{McpServer, Tool, ToolDef, ToolResult};
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

    fn call(&self, params: Value) -> ToolResult<Value> {
        let a = params.get("a").and_then(Value::as_i64).unwrap_or(0);
        let b = params.get("b").and_then(Value::as_i64).unwrap_or(0);
        Ok(json!({ "sum": a + b }))
    }

    fn requires_authn(&self) -> bool {
        false
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 5 {
        eprintln!(
            "usage: {} 127.0.0.1:7755 server-cert.pem server-key.pem client-ca.pem",
            args[0]
        );
        std::process::exit(2);
    }
    let mtls = MtlsConfig {
        tls: TlsConfig {
            cert_pem_path: PathBuf::from(&args[2]),
            key_pem_path: PathBuf::from(&args[3]),
            ca_pem_path: Some(PathBuf::from(&args[4])),
        },
        require_client_cert: true,
    };
    let mut dispatcher = McpServer::new();
    dispatcher
        .register(Box::new(AdderTool))
        .expect("register adder");
    let server = match CalyxMcpServer::bind(
        args[1].parse().expect("addr parses"),
        Arc::new(dispatcher),
        mtls,
    ) {
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
