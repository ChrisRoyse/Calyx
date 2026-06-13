//! JSON-RPC 2.0 response framing and MCP tool descriptors.
//!
//! Requests are decoded by [`crate::jsonrpc`]; this module owns the *reply*
//! side of the wire (responses, errors) plus the MCP `tools/list` descriptor
//! shape. A Calyx engine error never leaks as a bare string: it is mapped to a
//! structured JSON-RPC error so agents can extract the stable `CALYX_*` code and
//! remediation from `error.data`.

use calyx_core::CalyxError;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::jsonrpc::JsonRpcId;

/// JSON-RPC reserved code: requested method is not registered (JSON-RPC 2.0 §5.1).
pub const JSONRPC_METHOD_NOT_FOUND: i32 = -32601;
/// JSON-RPC reserved code: invalid method parameters (JSON-RPC 2.0 §5.1).
pub const JSONRPC_INVALID_PARAMS: i32 = -32602;
/// JSON-RPC reserved code: internal server error (e.g. a caught tool panic).
pub const JSONRPC_INTERNAL_ERROR: i32 = -32603;
/// JSON-RPC implementation-defined server error carrying a Calyx engine failure.
pub const JSONRPC_CALYX_ERROR: i32 = -32000;

/// A JSON-RPC 2.0 error object.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JsonRpcError {
    /// Numeric error code (reserved range or `-32000` for Calyx errors).
    pub code: i32,
    /// Human-readable, single-sentence failure description.
    pub message: String,
    /// Structured payload; for Calyx errors `{calyx_code, remediation}`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl JsonRpcError {
    /// Method-not-found (`-32601`) for an unregistered method or tool name.
    pub fn method_not_found(name: &str) -> Self {
        Self {
            code: JSONRPC_METHOD_NOT_FOUND,
            message: format!("method not found: {name}"),
            data: None,
        }
    }

    /// Invalid-params (`-32602`) for a structurally wrong `tools/call` payload.
    pub fn invalid_params(message: impl Into<String>) -> Self {
        Self {
            code: JSONRPC_INVALID_PARAMS,
            message: message.into(),
            data: None,
        }
    }

    /// Internal-error (`-32603`); used when a tool panics or a reply fails to
    /// serialize. The message is deliberately generic to avoid leaking internals.
    pub fn internal(message: impl Into<String>) -> Self {
        Self {
            code: JSONRPC_INTERNAL_ERROR,
            message: message.into(),
            data: None,
        }
    }

    /// Maps a structured [`CalyxError`] onto a `-32000` JSON-RPC error, preserving
    /// the stable `calyx_code` and `remediation` in `data` for agent recovery.
    pub fn from_calyx(error: &CalyxError) -> Self {
        Self {
            code: JSONRPC_CALYX_ERROR,
            message: error.message.clone(),
            data: Some(json!({
                "calyx_code": error.code,
                "remediation": error.remediation,
            })),
        }
    }
}

/// A JSON-RPC 2.0 response. Exactly one of `result`/`error` is `Some`; `id`
/// mirrors the request id (or `null` when it could not be determined).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    /// Protocol tag, always `"2.0"`.
    pub jsonrpc: String,
    /// Success payload (mutually exclusive with `error`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    /// Failure payload (mutually exclusive with `result`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
    /// Correlates the response with its request; serialized as `null` when absent.
    pub id: Option<JsonRpcId>,
}

impl JsonRpcResponse {
    /// Builds a success response for `id` carrying `result`.
    pub fn success(id: Option<JsonRpcId>, result: Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            result: Some(result),
            error: None,
            id,
        }
    }

    /// Builds an error response for `id` carrying `error`.
    pub fn error(id: Option<JsonRpcId>, error: JsonRpcError) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            result: None,
            error: Some(error),
            id,
        }
    }
}

/// A single MCP tool descriptor returned by `tools/list`.
///
/// `use_when` is a one-line agent hint (Calyx extension); `input_schema` is the
/// JSON Schema for the tool's arguments and is serialized as MCP's `inputSchema`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolDef {
    /// Unique tool name (the `tools/call` selector).
    pub name: String,
    /// What the tool does.
    pub description: String,
    /// One-line "use this when …" hint for agents.
    pub use_when: String,
    /// JSON Schema for the tool's arguments object.
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
}

/// A unit of `tools/call` output. Text blocks carry a JSON payload as a string.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ContentBlock {
    /// A text block; `text` is typically a serialized JSON document.
    #[serde(rename = "text")]
    Text {
        /// The text payload.
        text: String,
    },
}

/// The MCP `tools/call` result envelope.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCallResult {
    /// Ordered content blocks produced by the tool.
    pub content: Vec<ContentBlock>,
}

impl ToolCallResult {
    /// Wraps a single text block (a serialized JSON payload) as a call result.
    pub fn text(payload: impl Into<String>) -> Self {
        Self {
            content: vec![ContentBlock::Text {
                text: payload.into(),
            }],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jsonrpc::JsonRpcId;

    #[test]
    fn success_response_serializes_without_error_field() {
        let resp = JsonRpcResponse::success(Some(JsonRpcId::Number(7)), json!({"ok": true}));
        let wire = serde_json::to_string(&resp).unwrap();
        assert!(wire.contains("\"jsonrpc\":\"2.0\""));
        assert!(wire.contains("\"result\":{\"ok\":true}"));
        assert!(!wire.contains("\"error\""));
        assert!(wire.contains("\"id\":7"));
    }

    #[test]
    fn error_response_serializes_without_result_field() {
        let resp = JsonRpcResponse::error(
            Some(JsonRpcId::String("a".into())),
            JsonRpcError::method_not_found("foo"),
        );
        let wire = serde_json::to_string(&resp).unwrap();
        assert!(wire.contains("\"error\""));
        assert!(!wire.contains("\"result\""));
        assert!(wire.contains("-32601"));
        assert!(wire.contains("\"id\":\"a\""));
    }

    #[test]
    fn absent_id_serializes_as_null() {
        let resp = JsonRpcResponse::error(None, JsonRpcError::internal("boom"));
        let wire = serde_json::to_string(&resp).unwrap();
        assert!(wire.contains("\"id\":null"));
    }

    #[test]
    fn calyx_error_maps_to_minus_32000_with_structured_data() {
        let calyx = CalyxError::assay_insufficient_samples("n=30");
        let rpc = JsonRpcError::from_calyx(&calyx);
        assert_eq!(rpc.code, JSONRPC_CALYX_ERROR);
        let data = rpc.data.expect("data present");
        assert_eq!(data["calyx_code"], "CALYX_ASSAY_INSUFFICIENT_SAMPLES");
        // The remediation is the catalog's source-of-truth value, not invented here.
        assert_eq!(data["remediation"], "anchor more outcomes");
    }

    #[test]
    fn tool_def_serializes_input_schema_as_camel_case() {
        let def = ToolDef {
            name: "search".into(),
            description: "search the vault".into(),
            use_when: "you need recall".into(),
            input_schema: json!({"type": "object"}),
        };
        let wire = serde_json::to_string(&def).unwrap();
        assert!(wire.contains("\"inputSchema\""));
        assert!(!wire.contains("input_schema"));
        assert!(wire.contains("\"use_when\""));
    }

    #[test]
    fn content_block_is_tagged_text() {
        let result = ToolCallResult::text("{\"x\":1}");
        let wire = serde_json::to_string(&result).unwrap();
        assert_eq!(wire, r#"{"content":[{"type":"text","text":"{\"x\":1}"}]}"#);
    }
}
