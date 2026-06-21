//! MCP server: tool registry plus dispatch for the three mandatory methods
//! (`initialize`, `tools/list`, `tools/call`).
//!
//! Dispatch never panics out: a tool that panics is caught and converted to a
//! `-32603` internal error so the stdio loop survives. A tool that returns a
//! [`CalyxError`] is mapped to a `-32000` error preserving its `CALYX_*` code.

use std::collections::BTreeMap;
use std::panic::{AssertUnwindSafe, catch_unwind};

use calyx_core::{AuthN, CalyxError, no_anonymous_write};
use serde_json::{Value, json};

use crate::jsonrpc::JsonRpcRequest;
use crate::protocol::{JsonRpcError, JsonRpcResponse, ToolCallResult, ToolDef};

/// MCP protocol revision this scaffold speaks (echoed in `initialize`).
pub const MCP_PROTOCOL_VERSION: &str = "2024-11-05";
/// Server name reported in `initialize.serverInfo`.
pub const SERVER_NAME: &str = "calyx-mcp";

/// Local code for a duplicate tool registration (a setup-time programming error;
/// kept MCP-local rather than widening the closed `calyx-core` catalog).
pub const CALYX_MCP_TOOL_DUPLICATE: &str = "CALYX_MCP_TOOL_DUPLICATE";

/// Failure class returned by a tool call.
#[derive(Debug)]
pub enum ToolError {
    /// Structurally wrong arguments: maps to JSON-RPC `-32602`.
    InvalidParams(String),
    /// Calyx domain failure: maps to JSON-RPC `-32000` with `CALYX_*` data.
    Calyx(CalyxError),
}

impl ToolError {
    /// Builds an invalid-params error.
    pub fn invalid_params(message: impl Into<String>) -> Self {
        Self::InvalidParams(message.into())
    }
}

impl From<CalyxError> for ToolError {
    fn from(error: CalyxError) -> Self {
        Self::Calyx(error)
    }
}

/// Tool call result type.
pub type ToolResult<T> = std::result::Result<T, ToolError>;

/// A registerable MCP tool. Implementors are `Send + Sync` so a server can be
/// shared across threads; `call` must be side-effect-honest and fail closed.
pub trait Tool: Send + Sync {
    /// The descriptor advertised by `tools/list`.
    fn def(&self) -> ToolDef;
    /// Whether this tool can mutate durable Calyx state and therefore requires
    /// a caller identity before dispatch.
    fn requires_authn(&self) -> bool;
    /// Executes the tool against decoded `arguments`, returning a JSON payload.
    fn call(&self, params: Value) -> ToolResult<Value>;
}

/// The dispatch surface: an ordered registry of tools keyed by name.
#[derive(Default)]
pub struct McpServer {
    tools: BTreeMap<String, Box<dyn Tool>>,
}

impl McpServer {
    /// Creates an empty server (no tools registered).
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers `tool`, failing closed on a duplicate name so two tools can
    /// never silently shadow one another.
    pub fn register(&mut self, tool: Box<dyn Tool>) -> Result<(), CalyxError> {
        let name = tool.def().name;
        if self.tools.contains_key(&name) {
            return Err(CalyxError {
                code: CALYX_MCP_TOOL_DUPLICATE,
                message: format!("tool already registered: {name}"),
                remediation: "register each MCP tool under a unique name",
            });
        }
        self.tools.insert(name, tool);
        Ok(())
    }

    /// Number of registered tools.
    pub fn tool_count(&self) -> usize {
        self.tools.len()
    }

    /// Routes a decoded request to its handler, always returning a response.
    pub fn dispatch(&self, request: JsonRpcRequest) -> JsonRpcResponse {
        self.dispatch_with_authn(request, None)
    }

    /// Routes a decoded request with an optional authenticated caller identity.
    ///
    /// Mutating tools are rejected before their `call` body runs when `authn` is
    /// absent. Read-only tools continue to work anonymously.
    pub fn dispatch_with_authn(
        &self,
        request: JsonRpcRequest,
        authn: Option<&AuthN>,
    ) -> JsonRpcResponse {
        match request.method.as_str() {
            "initialize" => self.handle_initialize(request),
            "tools/list" => self.handle_tools_list(request),
            "tools/call" => self.handle_tools_call(request, authn),
            other => JsonRpcResponse::error(request.id, JsonRpcError::method_not_found(other)),
        }
    }

    fn handle_initialize(&self, request: JsonRpcRequest) -> JsonRpcResponse {
        let result = json!({
            "protocolVersion": MCP_PROTOCOL_VERSION,
            "capabilities": { "tools": {} },
            "serverInfo": {
                "name": SERVER_NAME,
                "version": env!("CARGO_PKG_VERSION"),
            },
        });
        JsonRpcResponse::success(request.id, result)
    }

    fn handle_tools_list(&self, request: JsonRpcRequest) -> JsonRpcResponse {
        let defs: Vec<ToolDef> = self.tools.values().map(|tool| tool.def()).collect();
        JsonRpcResponse::success(request.id, json!({ "tools": defs }))
    }

    fn handle_tools_call(&self, request: JsonRpcRequest, authn: Option<&AuthN>) -> JsonRpcResponse {
        let id = request.id.clone();
        let params = request.params.unwrap_or(Value::Null);

        let name = match params.get("name").and_then(Value::as_str) {
            Some(name) if !name.is_empty() => name.to_string(),
            _ => {
                return JsonRpcResponse::error(
                    id,
                    JsonRpcError::invalid_params("tools/call requires a non-empty string `name`"),
                );
            }
        };
        let arguments = params
            .get("arguments")
            .cloned()
            .unwrap_or_else(|| json!({}));

        let Some(tool) = self.tools.get(&name) else {
            return JsonRpcResponse::error(id, JsonRpcError::method_not_found(&name));
        };
        if tool.requires_authn()
            && let Err(error) = no_anonymous_write(authn)
        {
            return JsonRpcResponse::error(id, JsonRpcError::from_calyx(&error));
        }

        // A tool is third-party logic: isolate panics so one bad call cannot take
        // down the stdio loop. AssertUnwindSafe is sound here — on panic we only
        // construct a fresh error and touch no tool-owned state afterwards.
        let outcome = catch_unwind(AssertUnwindSafe(|| tool.call(arguments)));
        match outcome {
            Ok(Ok(value)) => match serde_json::to_string(&value) {
                Ok(payload) => JsonRpcResponse::success(id, json!(ToolCallResult::text(payload))),
                Err(error) => JsonRpcResponse::error(
                    id,
                    JsonRpcError::internal(format!("serialize tool result: {error}")),
                ),
            },
            Ok(Err(ToolError::InvalidParams(message))) => {
                JsonRpcResponse::error(id, JsonRpcError::invalid_params(message))
            }
            Ok(Err(ToolError::Calyx(calyx))) => {
                JsonRpcResponse::error(id, JsonRpcError::from_calyx(&calyx))
            }
            Err(_panic) => {
                JsonRpcResponse::error(id, JsonRpcError::internal("internal server error"))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jsonrpc::{JsonRpcId, decode_jsonrpc_request};
    use crate::schema::{object_schema, string_schema};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct EchoTool;
    impl Tool for EchoTool {
        fn def(&self) -> ToolDef {
            ToolDef {
                name: "echo".into(),
                description: "echo the input back".into(),
                use_when: "you need a round-trip probe".into(),
                input_schema: object_schema(&[("msg", string_schema(), true)]),
            }
        }
        fn requires_authn(&self) -> bool {
            false
        }
        fn call(&self, params: Value) -> ToolResult<Value> {
            Ok(json!({ "echoed": params["msg"] }))
        }
    }

    struct FailingTool;
    impl Tool for FailingTool {
        fn def(&self) -> ToolDef {
            ToolDef {
                name: "fail".into(),
                description: "always fails closed".into(),
                use_when: "never".into(),
                input_schema: object_schema(&[]),
            }
        }
        fn requires_authn(&self) -> bool {
            false
        }
        fn call(&self, _params: Value) -> ToolResult<Value> {
            Err(CalyxError::assay_insufficient_samples("n=30").into())
        }
    }

    struct PanicTool;
    impl Tool for PanicTool {
        fn def(&self) -> ToolDef {
            ToolDef {
                name: "panic".into(),
                description: "panics".into(),
                use_when: "never".into(),
                input_schema: object_schema(&[]),
            }
        }
        fn requires_authn(&self) -> bool {
            false
        }
        fn call(&self, _params: Value) -> ToolResult<Value> {
            panic!("boom");
        }
    }

    struct MutatingTool {
        writes: Arc<AtomicUsize>,
    }

    impl Tool for MutatingTool {
        fn def(&self) -> ToolDef {
            ToolDef {
                name: "mutate".into(),
                description: "increments a durable-state stand-in".into(),
                use_when: "security regression probe".into(),
                input_schema: object_schema(&[]),
            }
        }
        fn requires_authn(&self) -> bool {
            true
        }
        fn call(&self, _params: Value) -> ToolResult<Value> {
            let count = self.writes.fetch_add(1, Ordering::SeqCst) + 1;
            Ok(json!({ "writes": count }))
        }
    }

    fn req(line: &str) -> JsonRpcRequest {
        decode_jsonrpc_request(line.as_bytes()).unwrap()
    }

    #[test]
    fn tools_list_empty_returns_empty_array_with_id() {
        let server = McpServer::new();
        let resp = server.dispatch(req(r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#));
        assert_eq!(resp.id, Some(JsonRpcId::Number(1)));
        assert_eq!(resp.result.unwrap(), json!({ "tools": [] }));
        assert!(resp.error.is_none());
    }

    #[test]
    fn unknown_method_returns_minus_32601() {
        let server = McpServer::new();
        let resp = server.dispatch(req(r#"{"jsonrpc":"2.0","id":2,"method":"foo"}"#));
        let error = resp.error.unwrap();
        assert_eq!(error.code, -32601);
        assert_eq!(resp.id, Some(JsonRpcId::Number(2)));
    }

    #[test]
    fn initialize_reports_server_info() {
        let server = McpServer::new();
        let resp = server.dispatch(req(r#"{"jsonrpc":"2.0","id":9,"method":"initialize"}"#));
        let result = resp.result.unwrap();
        assert_eq!(result["serverInfo"]["name"], "calyx-mcp");
        assert_eq!(result["protocolVersion"], MCP_PROTOCOL_VERSION);
    }

    #[test]
    fn tools_list_includes_registered_tool() {
        let mut server = McpServer::new();
        server.register(Box::new(EchoTool)).unwrap();
        let resp = server.dispatch(req(r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#));
        let tools = resp.result.unwrap()["tools"].clone();
        assert_eq!(tools[0]["name"], "echo");
        assert_eq!(
            tools[0]["inputSchema"]["properties"]["msg"]["type"],
            "string"
        );
    }

    #[test]
    fn registered_tool_call_round_trips() {
        let mut server = McpServer::new();
        server.register(Box::new(EchoTool)).unwrap();
        let resp = server.dispatch(req(
            r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"echo","arguments":{"msg":"hi"}}}"#,
        ));
        let content = resp.result.unwrap()["content"][0]["text"].clone();
        let text = content.as_str().unwrap();
        let payload: Value = serde_json::from_str(text).unwrap();
        assert_eq!(payload["echoed"], "hi");
    }

    #[test]
    fn unauthenticated_mutating_tool_rejected_before_side_effect() {
        let writes = Arc::new(AtomicUsize::new(0));
        let mut server = McpServer::new();
        server
            .register(Box::new(MutatingTool {
                writes: Arc::clone(&writes),
            }))
            .unwrap();
        let resp = server.dispatch(req(
            r#"{"jsonrpc":"2.0","id":10,"method":"tools/call","params":{"name":"mutate","arguments":{}}}"#,
        ));
        let error = resp.error.unwrap();
        assert_eq!(error.code, -32000);
        assert_eq!(
            error.data.unwrap()["calyx_code"],
            calyx_core::CALYX_AUTHN_REQUIRED
        );
        assert_eq!(writes.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn authenticated_mutating_tool_executes() {
        let writes = Arc::new(AtomicUsize::new(0));
        let mut server = McpServer::new();
        server
            .register(Box::new(MutatingTool {
                writes: Arc::clone(&writes),
            }))
            .unwrap();
        let authn = AuthN::InProcess {
            host_app_id: "calyx-mcp-test".into(),
        };
        let resp = server.dispatch_with_authn(
            req(
                r#"{"jsonrpc":"2.0","id":11,"method":"tools/call","params":{"name":"mutate","arguments":{}}}"#,
            ),
            Some(&authn),
        );
        let text = resp.result.unwrap()["content"][0]["text"]
            .as_str()
            .unwrap()
            .to_string();
        let payload: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(payload["writes"], 1);
        assert_eq!(writes.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn calyx_error_tool_maps_to_structured_data() {
        let mut server = McpServer::new();
        server.register(Box::new(FailingTool)).unwrap();
        let resp = server.dispatch(req(
            r#"{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"fail"}}"#,
        ));
        let error = resp.error.unwrap();
        assert_eq!(error.code, -32000);
        let data = error.data.unwrap();
        assert_eq!(data["calyx_code"], "CALYX_ASSAY_INSUFFICIENT_SAMPLES");
        assert_eq!(data["remediation"], "anchor more outcomes");
    }

    #[test]
    fn panicking_tool_is_caught_as_minus_32603() {
        let mut server = McpServer::new();
        server.register(Box::new(PanicTool)).unwrap();
        let resp = server.dispatch(req(
            r#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"panic"}}"#,
        ));
        let error = resp.error.unwrap();
        assert_eq!(error.code, -32603);
        assert_eq!(error.message, "internal server error");
    }

    #[test]
    fn unknown_tool_call_returns_minus_32601() {
        let server = McpServer::new();
        let resp = server.dispatch(req(
            r#"{"jsonrpc":"2.0","id":8,"method":"tools/call","params":{"name":"nope"}}"#,
        ));
        assert_eq!(resp.error.unwrap().code, -32601);
    }

    #[test]
    fn tools_call_without_name_is_invalid_params() {
        let server = McpServer::new();
        let resp = server.dispatch(req(
            r#"{"jsonrpc":"2.0","id":8,"method":"tools/call","params":{}}"#,
        ));
        assert_eq!(resp.error.unwrap().code, -32602);
    }

    #[test]
    fn duplicate_registration_fails_closed() {
        let mut server = McpServer::new();
        server.register(Box::new(EchoTool)).unwrap();
        let err = server.register(Box::new(EchoTool)).unwrap_err();
        assert_eq!(err.code, CALYX_MCP_TOOL_DUPLICATE);
        assert_eq!(server.tool_count(), 1);
    }
}
