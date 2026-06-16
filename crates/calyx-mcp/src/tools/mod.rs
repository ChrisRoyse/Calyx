//! Registered MCP tool groups.

use calyx_core::CalyxError;

use crate::server::McpServer;

pub mod ingest;
pub mod intelligence;
pub mod provenance;
pub mod search;
#[cfg(test)]
pub(crate) mod test_support;
pub mod vault;

/// Registers every built-in Calyx MCP tool.
pub fn register_all(server: &mut McpServer) -> Result<(), CalyxError> {
    vault::register(server)
        .and_then(|_| ingest::register(server))
        .and_then(|_| search::register(server))
        .and_then(|_| intelligence::register(server))
        .and_then(|_| provenance::register(server))
}
