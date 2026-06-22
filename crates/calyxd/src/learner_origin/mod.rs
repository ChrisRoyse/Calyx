//! Worker-only learner-origin API for website learner memory (issue #813).
//!
//! The API is deliberately owned by `calyxd` because the daemon already owns
//! the loopback HTTP boundary, startup config, metrics, and durable Aster vault
//! access. Request success is never the source of truth: accepted writes go
//! through Aster Base/slot CF rows plus the Ledger hash chain, then are flushed
//! for immediate readback.

mod config;
mod metrics;
mod model;
mod privacy;
mod service;

pub use config::{DEFAULT_ORIGIN_SECRET_ENV, LearnerOriginConfig};
pub use metrics::OriginMetrics;
pub use service::{LearnerOriginService, OriginResponse};
