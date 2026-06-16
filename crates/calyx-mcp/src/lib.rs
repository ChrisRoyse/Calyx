//! MCP interface for agent-facing Calyx operations.
//!
//! The wire stack is split across modules: [`jsonrpc`] decodes inbound requests,
//! [`protocol`] frames responses and MCP descriptors, [`schema`] builds tool
//! input schemas, and [`server`] holds the tool registry and dispatch.

pub mod jsonrpc;
pub mod protocol;
pub mod schema;
pub mod server;
pub mod tools;

pub use jsonrpc::{
    CALYX_MCP_JSONRPC_INVALID, JsonRpcId, JsonRpcRequest, JsonRpcWire, decode_jsonrpc_request,
    decode_jsonrpc_wire,
};
pub use protocol::{
    ContentBlock, JSONRPC_CALYX_ERROR, JSONRPC_INTERNAL_ERROR, JSONRPC_INVALID_PARAMS,
    JSONRPC_METHOD_NOT_FOUND, JsonRpcError, JsonRpcResponse, ToolCallResult, ToolDef,
};
pub use server::{
    CALYX_MCP_TOOL_DUPLICATE, MCP_PROTOCOL_VERSION, McpServer, SERVER_NAME, Tool, ToolError,
    ToolResult,
};

#[cfg(test)]
mod tests {
    #[test]
    fn crate_metadata_is_present() {
        assert_eq!(env!("CARGO_PKG_NAME"), "calyx-mcp");
    }
}
