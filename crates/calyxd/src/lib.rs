//! `calyxd` library surface.
//!
//! The daemon binary (`src/main.rs`) compiles its modules privately; this
//! library exposes only what external consumers need: the stable
//! `CALYX_DAEMON_*` error taxonomy and the PH67 `verify-restore` byte-level
//! verification tool reused by `calyx-cli`.

pub mod error;
pub mod verify;
