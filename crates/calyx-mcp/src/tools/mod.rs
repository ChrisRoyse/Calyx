//! Registered MCP tool groups.

use calyx_core::CalyxError;

use crate::server::McpServer;

pub mod vault;

/// Registers every built-in Calyx MCP tool.
pub fn register_all(server: &mut McpServer) -> Result<(), CalyxError> {
    vault::register(server)
}
