//! `calyxd` library surface.
//!
//! The daemon binary (`src/main.rs`) compiles its modules privately; this
//! library exposes what external consumers (notably `calyx-cli`) need: the
//! stable `CALYX_DAEMON_*` error taxonomy, the PH67 `verify-restore` byte-level
//! verification tool, the authoritative [`config::CalyxConfig`] runtime
//! configuration, the [`cuda_probe`]/[`vram`] startup probes (T02/T03), and the
//! [`health`] daemon-readiness probe (T04). The probe modules are shared source
//! with the binary — one source of truth, compiled into both crate roots.

pub mod config;
pub mod cuda_probe;
pub mod error;
pub mod health;
pub mod verify;
pub mod vram;
