//! Release barrier for PID 1-created gate stages.
//!
//! The shim is installed root-owned at `/usr/libexec/calyx-gate-stage-shim`.
//! PID 1 starts it directly in the final service cgroup and worker identity.
//! It will not execute the requested payload until the broker proves the
//! process and cgroup identity and writes the one-use kernel-random token.

#[cfg(target_os = "linux")]
fn main() {
    linux::run();
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("CALYX_GATE_STAGE_SHIM_UNSUPPORTED: Linux is required");
    std::process::exit(125);
}

#[cfg(target_os = "linux")]
#[path = "calyx-gate-stage-shim/linux.rs"]
mod linux;
