//! PID 1-owned, descriptor-bound lifecycle for gate stages.
//!
//! Capture and release are deliberately separate. `capture()` starts only the
//! root-owned release shim, binds its process/cgroup/cwd identity, and returns
//! durable evidence. The caller must commit that evidence to the journal before
//! calling `release()`. No payload byte executes before the one-use token is
//! written.

#![cfg(target_os = "linux")]

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{CStr, CString, OsString};
use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::process::ExitStatusExt;
use std::path::{Component, Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use thiserror::Error;

use crate::protocol::{AbsolutePath, InvocationId, UnitName};

const SYSTEMD_RUN: &str = "/usr/bin/systemd-run";
const SYSTEMCTL: &str = "/usr/bin/systemctl";
const STAGE_SHIM: &str = "/usr/libexec/calyx-gate-stage-shim";
const CGROUP_ROOT: &str = "/sys/fs/cgroup";
pub const BROKER_UNIT_NAME: &str = "calyx-gatebrokerd.service";
pub const STAGE_SLICE_NAME: &str = "calyx-gate.slice";
// `calyx-gate.slice` is the `/calyx/gate` slice in systemd's hierarchical
// dash encoding, so PID1 publishes both hierarchy components in cgroup v2.
pub const STAGE_SLICE_CONTROL_GROUP: &str = "/calyx.slice/calyx-gate.slice";
const PRIVATE_STATE_ROOT: &str = "/var/lib/calyx-gatebrokerd/private";
const OBJECT_ROOT: &str = "/var/lib/calyx-gatebrokerd/objects";
const TOKEN_BYTES: usize = 32;
const MINIMUM_SYSTEMD_VERSION: u32 = 259;
const START_TIMEOUT: Duration = Duration::from_secs(10);
const DRAIN_TIMEOUT: Duration = Duration::from_secs(10);
const POLL_INTERVAL: Duration = Duration::from_millis(10);
const CGROUP2_SUPER_MAGIC: libc::c_long = 0x6367_7270;
const RESOLVE_NO_XDEV: u64 = 0x01;
const RESOLVE_NO_MAGICLINKS: u64 = 0x02;
const RESOLVE_NO_SYMLINKS: u64 = 0x04;
const RESOLVE_BENEATH: u64 = 0x08;

#[repr(C)]
struct OpenHow {
    flags: u64,
    mode: u64,
    resolve: u64,
}

#[derive(Debug, Clone)]
pub struct StageSpec {
    pub unit_name: String,
    pub worker_user: String,
    pub worker_uid: u32,
    pub execution_root: PathBuf,
    pub relative_cwd: PathBuf,
    pub execution_root_uid: u32,
    pub execution_root_mode: u32,
    /// A broker-held descriptor for the already resolved execution directory.
    /// `capture()` duplicates it with `CLOEXEC` before any asynchronous work.
    pub cwd_fd: RawFd,
    pub argv: Vec<OsString>,
    pub environment: Vec<(OsString, OsString)>,
    /// Exact published object directories, never a shared/private container.
    pub writable_paths: Vec<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnitEvidence {
    pub unit_name: String,
    pub invocation_id: String,
    pub control_group: String,
    pub control_group_device: u64,
    pub control_group_inode: u64,
    pub slice_control_group: String,
    pub slice_control_group_device: u64,
    pub slice_control_group_inode: u64,
    pub worker_user: String,
    pub worker_uid: u32,
    pub main_pid: u32,
    pub active_state: String,
    pub result: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageResult {
    pub evidence: UnitEvidence,
    pub exit_status: i32,
    pub main_code: String,
    pub main_status: i32,
    pub systemd_run_status: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryOutcome {
    AbsentOrEmpty,
    Killed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CgroupIdentity {
    pub control_group: AbsolutePath,
    pub device: u64,
    pub inode: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerIdentity {
    pub user: String,
    pub uid: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CleanupProof {
    pub pidfd_exited: bool,
    pub service_cgroup_empty: bool,
    pub slice_cgroup_empty: bool,
    pub systemd_run_reaped: bool,
    pub detail: String,
}

impl CleanupProof {
    pub fn proves_stage_drained(&self) -> bool {
        self.pidfd_exited && self.service_cgroup_empty && self.slice_cgroup_empty
    }

    fn unbound(detail: String, systemd_run_reaped: bool) -> Self {
        Self {
            pidfd_exited: false,
            service_cgroup_empty: false,
            slice_cgroup_empty: false,
            systemd_run_reaped,
            detail,
        }
    }
}

impl fmt::Display for CleanupProof {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "pidfd_exited={} service_empty={} slice_empty={} systemd_run_reaped={} detail={}",
            self.pidfd_exited,
            self.service_cgroup_empty,
            self.slice_cgroup_empty,
            self.systemd_run_reaped,
            self.detail
        )
    }
}

#[derive(Debug)]
pub struct AbortAuthority {
    evidence: UnitEvidence,
    stage_pidfd: ProcessFd,
    service: CgroupAuthority,
    slice: CgroupAuthority,
}

#[derive(Debug)]
pub struct CapturedStage {
    child: Option<Child>,
    release_pipe: Option<ChildStdin>,
    release_token: [u8; TOKEN_BYTES],
    evidence: UnitEvidence,
    stage_pidfd: Option<ProcessFd>,
    service: Option<CgroupAuthority>,
    slice: Option<CgroupAuthority>,
    cwd_guard: Option<OwnedFd>,
}

#[derive(Debug)]
pub struct RunningStage {
    child: Option<Child>,
    evidence: UnitEvidence,
    stage_pidfd: Option<ProcessFd>,
    service: Option<CgroupAuthority>,
    slice: Option<CgroupAuthority>,
    cwd_guard: Option<OwnedFd>,
    owner_guard: Option<OwnedFd>,
}

#[derive(Debug, Error)]
pub enum SystemdError {
    #[error("invalid stage specification: {0}")]
    InvalidSpec(String),
    #[error(
        "broker must run as real/effective root; actual real={real_uid} effective={effective_uid}"
    )]
    BrokerIdentity { real_uid: u32, effective_uid: u32 },
    #[error("trusted executable policy failed for {path}: {detail}")]
    ExecutablePolicy { path: &'static str, detail: String },
    #[error("worker account lookup failed for {user}: {detail}")]
    WorkerLookup { user: String, detail: String },
    #[error("worker account {user} violates isolation policy: {detail}")]
    WorkerPolicy { user: String, detail: String },
    #[error("worker user manager is present: uid={uid} evidence={evidence}")]
    WorkerManagerPresent { uid: u32, evidence: String },
    #[error("unexpected process already uses worker uid {uid}: pids={pids:?}")]
    WorkerProcessPresent { uid: u32, pids: Vec<u32> },
    #[error("system manager command {operation} failed: {detail}")]
    Manager {
        operation: &'static str,
        detail: String,
    },
    #[error("unit publication was not observed: unit={unit} detail={detail}")]
    Publication { unit: String, detail: String },
    #[error("unit identity changed: unit={unit} expected={expected} actual={actual}")]
    UnitIdentity {
        unit: String,
        expected: String,
        actual: String,
    },
    #[error("stage process identity failed for pid {pid}: {detail}")]
    ProcessIdentity { pid: u32, detail: String },
    #[error("cgroup authority failed: control={control} detail={detail}")]
    Cgroup { control: String, detail: String },
    #[error("owner exited before stage release")]
    OwnerExited,
    #[error("stage release failed: {0}")]
    Release(String),
    #[error("recorded stage requires operator recovery: {detail}")]
    RecoveryRequired { detail: String },
    #[error("terminal stage invariant violated after safety drain: {detail}")]
    TerminalStageInvariant { detail: String },
    #[error("{primary}; cleanup evidence: {cleanup}")]
    Cleanup {
        primary: String,
        cleanup: CleanupProof,
    },
    #[error("I/O failure during {operation}: {source}")]
    Io {
        operation: &'static str,
        #[source]
        source: io::Error,
    },
}

impl SystemdError {
    pub fn cleanup_proved_drained(&self) -> bool {
        matches!(
            self,
            Self::Cleanup { cleanup, .. } if cleanup.proves_stage_drained()
        )
    }
}

#[derive(Debug)]
struct WorkerAccount {
    uid: u32,
    gid: u32,
}

#[derive(Debug)]
struct TrustedIdentity {
    stat: libc::stat,
}

#[derive(Debug)]
struct ProcessFd {
    pid: u32,
    fd: OwnedFd,
}

#[derive(Debug)]
struct CgroupRoot {
    fd: OwnedFd,
}

#[derive(Debug)]
struct CgroupAuthority {
    control_group: String,
    directory: OwnedFd,
    events: OwnedFd,
    kill: OwnedFd,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CgroupPopulation {
    Empty,
    Populated,
    Removed,
}
mod account;
mod builder;
mod capture;
mod cgroup;
mod evidence;
mod manager;
mod recovery;
mod running;
mod validation;

pub use account::verify_worker_idle;
pub use recovery::{
    audit_terminal_recorded_stage, recover_recorded_stage, recover_worker_boundary,
};
