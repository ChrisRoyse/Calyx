//! `calyxd` library surface.
//!
//! The daemon binary (`src/main.rs`) compiles its modules privately; this
//! library exposes what external consumers (notably `calyx-cli`) need: the
//! stable `CALYX_DAEMON_*` error taxonomy, the PH67 `verify-restore` byte-level
//! verification tool, the authoritative [`config::CalyxConfig`] runtime
//! configuration, the [`cuda_probe`]/[`vram`] startup probes (T02/T03), the
//! [`health`] daemon-readiness probe (T04), the [`metrics`] Prometheus
//! surface served at `/metrics` (PH66 T03), and the [`mcp_server`] loopback
//! MCP-over-socket dispatch transport (T05). The probe and metrics modules are
//! the daemon's single source of truth, consumed by the binary from the library
//! so the recording API (driven later by the ingest/search dispatch paths) is
//! public API rather than binary dead code.

pub mod config;
pub mod cuda_probe;
pub mod error;
pub mod health;
pub mod learner_origin;
pub mod mcp_server;
pub mod metrics;
pub mod server;
pub mod verify;
pub mod vram;
