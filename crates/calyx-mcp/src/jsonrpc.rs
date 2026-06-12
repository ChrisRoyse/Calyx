//! Fail-closed JSON-RPC wire decoding for MCP requests.

use calyx_core::{CalyxError, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const CALYX_MCP_JSONRPC_INVALID: &str = "CALYX_MCP_JSONRPC_INVALID";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum JsonRpcId {
    String(String),
    Number(i64),
    Null,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<JsonRpcId>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum JsonRpcWire {
    Single(JsonRpcRequest),
    Batch(Vec<JsonRpcRequest>),
}

pub fn decode_jsonrpc_wire(bytes: &[u8]) -> Result<JsonRpcWire> {
    let value: Value = serde_json::from_slice(bytes)
        .map_err(|error| jsonrpc_error(format!("decode JSON-RPC request: {error}")))?;
    match value {
        Value::Object(_) => Ok(JsonRpcWire::Single(decode_request_value(value)?)),
        Value::Array(values) => {
            if values.is_empty() {
                return Err(jsonrpc_error("JSON-RPC batch must not be empty"));
            }
            values
                .into_iter()
                .map(decode_request_value)
                .collect::<Result<Vec<_>>>()
                .map(JsonRpcWire::Batch)
        }
        _ => Err(jsonrpc_error("JSON-RPC wire value must be object or batch")),
    }
}

pub fn decode_jsonrpc_request(bytes: &[u8]) -> Result<JsonRpcRequest> {
    match decode_jsonrpc_wire(bytes)? {
        JsonRpcWire::Single(request) => Ok(request),
        JsonRpcWire::Batch(_) => Err(jsonrpc_error(
            "expected a single JSON-RPC request, got a batch",
        )),
    }
}

impl JsonRpcRequest {
    pub fn validate(&self) -> Result<()> {
        if self.jsonrpc != "2.0" {
            return Err(jsonrpc_error("JSON-RPC version must be exactly 2.0"));
        }
        if self.method.trim().is_empty() {
            return Err(jsonrpc_error("JSON-RPC method must not be empty"));
        }
        if self.method.starts_with("rpc.") {
            return Err(jsonrpc_error("JSON-RPC method prefix rpc. is reserved"));
        }
        if let Some(params) = &self.params
            && !matches!(params, Value::Object(_) | Value::Array(_))
        {
            return Err(jsonrpc_error("JSON-RPC params must be object or array"));
        }
        Ok(())
    }
}

fn decode_request_value(value: Value) -> Result<JsonRpcRequest> {
    let request: JsonRpcRequest = serde_json::from_value(value)
        .map_err(|error| jsonrpc_error(format!("decode JSON-RPC request shape: {error}")))?;
    request.validate()?;
    Ok(request)
}

fn jsonrpc_error(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_MCP_JSONRPC_INVALID,
        message: message.into(),
        remediation: "send a valid JSON-RPC 2.0 MCP request object or non-empty batch",
    }
}
