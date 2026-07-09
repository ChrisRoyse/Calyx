//! Leapable-facing Calyx engine scaffold.
//!
//! The binary speaks newline-delimited JSON-RPC 2.0 over stdio. It is deliberately
//! direct-method JSON-RPC, not MCP `tools/call`: the Bun sidecar owns the MCP
//! surface, while this crate owns the storage-engine process boundary.

pub mod config;
pub mod engine;
pub mod lifecycle;
pub mod paths;

pub use config::EngineConfig;
pub use engine::{Engine, LEAPABLE_CAPABILITIES, mutating_method_requires_id};
