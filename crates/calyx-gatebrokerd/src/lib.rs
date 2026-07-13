//! Privileged, fail-closed authority primitives for the Calyx gate broker.

#[cfg(target_os = "linux")]
pub mod accounts;
pub mod broker_error;
pub mod config;
#[cfg(target_os = "linux")]
pub mod daemon;
#[cfg(target_os = "linux")]
pub mod exec_root;
pub mod fs_tx;
#[cfg(target_os = "linux")]
pub mod ids;
pub mod journal;
pub mod logging;
pub mod protocol;

#[cfg(target_os = "linux")]
pub mod pidfd;
#[cfg(target_os = "linux")]
pub mod systemd;
#[cfg(target_os = "linux")]
pub mod transport;
