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
pub const STAGE_SLICE_CONTROL_GROUP: &str = "/calyx-gate.slice";
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
    #[error("{primary}; cleanup evidence: {cleanup}")]
    Cleanup { primary: String, cleanup: String },
    #[error("I/O failure during {operation}: {source}")]
    Io {
        operation: &'static str,
        #[source]
        source: io::Error,
    },
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

fn io_error(operation: &'static str, source: io::Error) -> SystemdError {
    SystemdError::Io { operation, source }
}

fn valid_unit_component(value: &str, suffix: &str) -> bool {
    value.ends_with(suffix)
        && value.len() <= 240
        && !value.starts_with('.')
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn normalized_absolute(path: &Path) -> bool {
    path.is_absolute()
        && path.parent().is_some()
        && path
            .components()
            .all(|component| matches!(component, Component::RootDir | Component::Normal(_)))
        && path.to_str().is_some()
}

fn normalized_relative(path: &Path) -> bool {
    if path == Path::new(".") {
        return true;
    }
    !path.as_os_str().is_empty()
        && !path.is_absolute()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
        && path.to_str().is_some()
}

fn is_exact_published_object(path: &Path) -> bool {
    let Ok(relative) = path.strip_prefix(OBJECT_ROOT) else {
        return false;
    };
    let components: Vec<_> = relative.components().collect();
    components.len() == 2
        && matches!(components[0], Component::Normal(name) if name == "tmp" || name == "target")
        && matches!(components[1], Component::Normal(_))
}

fn reserved_environment_name(name: &[u8]) -> bool {
    matches!(
        name,
        b"PATH"
            | b"HOME"
            | b"USER"
            | b"LOGNAME"
            | b"SHELL"
            | b"LANG"
            | b"LC_ALL"
            | b"XDG_RUNTIME_DIR"
            | b"NOTIFY_SOCKET"
            | b"INVOCATION_ID"
            | b"JOURNAL_STREAM"
            | b"GLIBC_TUNABLES"
            | b"GCONV_PATH"
            | b"LOCPATH"
            | b"NLSPATH"
            | b"HOSTALIASES"
            | b"RES_OPTIONS"
            | b"LOCALDOMAIN"
    ) || name.starts_with(b"LD_")
        || name.starts_with(b"DYLD_")
        || name.starts_with(b"DBUS_")
        || name.starts_with(b"LISTEN_")
        || name.starts_with(b"SYSTEMD_")
}

fn validate_spec(spec: &StageSpec) -> Result<(), SystemdError> {
    if !valid_unit_component(&spec.unit_name, ".service") {
        return Err(SystemdError::InvalidSpec(
            "invalid service unit name".into(),
        ));
    }
    if !spec
        .worker_user
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        || spec.worker_user.is_empty()
    {
        return Err(SystemdError::InvalidSpec(
            "invalid worker account name".into(),
        ));
    }
    if !normalized_absolute(&spec.execution_root) {
        return Err(SystemdError::InvalidSpec(
            "execution root must be a normalized UTF-8 absolute path".into(),
        ));
    }
    if !normalized_relative(&spec.relative_cwd) {
        return Err(SystemdError::InvalidSpec(
            "relative cwd must be traversal-free UTF-8".into(),
        ));
    }
    if spec.execution_root_mode > 0o7777 {
        return Err(SystemdError::InvalidSpec(
            "invalid execution-root mode".into(),
        ));
    }
    if spec.cwd_fd < 0 {
        return Err(SystemdError::InvalidSpec(
            "cwd descriptor is invalid".into(),
        ));
    }
    if spec.argv.is_empty() || spec.argv[0].as_os_str().as_bytes().first() != Some(&b'/') {
        return Err(SystemdError::InvalidSpec(
            "argv[0] must be an absolute executable path".into(),
        ));
    }
    let mut argv_bytes = 0_usize;
    for value in &spec.argv {
        let bytes = value.as_os_str().as_bytes();
        if bytes.contains(&0) {
            return Err(SystemdError::InvalidSpec("argv contains NUL".into()));
        }
        argv_bytes = argv_bytes.saturating_add(bytes.len());
    }
    if argv_bytes > 65_536 {
        return Err(SystemdError::InvalidSpec("argv exceeds 65536 bytes".into()));
    }
    let mut names = BTreeSet::new();
    for (name, value) in &spec.environment {
        let name = name.as_os_str().as_bytes();
        let value = value.as_os_str().as_bytes();
        let portable = name
            .first()
            .is_some_and(|byte| byte.is_ascii_alphabetic() || *byte == b'_')
            && name[1..]
                .iter()
                .all(|byte| byte.is_ascii_alphanumeric() || *byte == b'_');
        if !portable || value.contains(&0) || reserved_environment_name(name) {
            return Err(SystemdError::InvalidSpec(format!(
                "unsafe environment assignment: {}",
                String::from_utf8_lossy(name)
            )));
        }
        if !names.insert(name.to_vec()) {
            return Err(SystemdError::InvalidSpec(
                "duplicate environment name".into(),
            ));
        }
    }
    for path in &spec.writable_paths {
        if !normalized_absolute(path)
            || !is_exact_published_object(path)
            || path.starts_with(PRIVATE_STATE_ROOT)
        {
            return Err(SystemdError::InvalidSpec(format!(
                "writable path is not an exact published object: {}",
                path.display()
            )));
        }
    }
    Ok(())
}

fn duplicate_fd(fd: RawFd, operation: &'static str) -> Result<OwnedFd, SystemdError> {
    let duplicate = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 3) };
    if duplicate < 0 {
        Err(io_error(operation, io::Error::last_os_error()))
    } else {
        Ok(unsafe { OwnedFd::from_raw_fd(duplicate) })
    }
}

fn fstat(fd: RawFd, operation: &'static str) -> Result<libc::stat, SystemdError> {
    let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
    if unsafe { libc::fstat(fd, stat.as_mut_ptr()) } != 0 {
        Err(io_error(operation, io::Error::last_os_error()))
    } else {
        Ok(unsafe { stat.assume_init() })
    }
}

fn trusted_executable(
    path: &'static str,
    exact_shim: bool,
) -> Result<TrustedIdentity, SystemdError> {
    let path_c = CString::new(path).expect("static executable path");
    let flags = if exact_shim {
        libc::O_PATH | libc::O_CLOEXEC | libc::O_NOFOLLOW
    } else {
        libc::O_PATH | libc::O_CLOEXEC
    };
    let raw = unsafe { libc::open(path_c.as_ptr(), flags) };
    if raw < 0 {
        return Err(SystemdError::ExecutablePolicy {
            path,
            detail: io::Error::last_os_error().to_string(),
        });
    }
    let fd = unsafe { OwnedFd::from_raw_fd(raw) };
    let stat = fstat(fd.as_raw_fd(), "fstat trusted executable")?;
    let mode = stat.st_mode as u32 & 0o7777;
    let valid = stat.st_mode & libc::S_IFMT == libc::S_IFREG
        && stat.st_uid == 0
        && stat.st_gid == 0
        && mode & 0o022 == 0
        && (!exact_shim || (mode == 0o755 && stat.st_nlink == 1));
    if !valid {
        return Err(SystemdError::ExecutablePolicy {
            path,
            detail: format!(
                "expected root:root regular nonwritable{}; actual uid={} gid={} mode={mode:04o} nlink={}",
                if exact_shim { " mode 0755 nlink 1" } else { "" },
                stat.st_uid,
                stat.st_gid,
                stat.st_nlink
            ),
        });
    }
    Ok(TrustedIdentity { stat })
}

fn manager_command(path: &'static str) -> Command {
    let mut command = Command::new(path);
    command.env_clear();
    command.env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin");
    command.env("LC_ALL", "C");
    command.env("SYSTEMD_COLORS", "0");
    command.env("SYSTEMD_URLIFY", "0");
    command
}

fn command_output(mut command: Command, operation: &'static str) -> Result<String, SystemdError> {
    let output = command
        .output()
        .map_err(|source| io_error(operation, source))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !output.status.success() {
        return Err(SystemdError::Manager {
            operation,
            detail: format!(
                "status={} stdout={} stderr={}",
                output.status,
                stdout.trim(),
                stderr.trim()
            ),
        });
    }
    Ok(stdout.into_owned())
}

fn verify_systemd_contract() -> Result<(), SystemdError> {
    trusted_executable(SYSTEMD_RUN, false)?;
    trusted_executable(SYSTEMCTL, false)?;
    let mut command = manager_command(SYSTEMCTL);
    command.arg("--version");
    let output = command_output(command, "query systemd version")?;
    let version = output
        .lines()
        .next()
        .and_then(|line| line.strip_prefix("systemd "))
        .and_then(|tail| tail.split_whitespace().next())
        .and_then(|value| value.parse::<u32>().ok())
        .ok_or_else(|| SystemdError::Manager {
            operation: "query systemd version",
            detail: format!("malformed version output: {output:?}"),
        })?;
    if version < MINIMUM_SYSTEMD_VERSION {
        return Err(SystemdError::Manager {
            operation: "query systemd version",
            detail: format!("systemd {version} is below required {MINIMUM_SYSTEMD_VERSION}"),
        });
    }
    Ok(())
}

fn lookup_worker(user: &str) -> Result<WorkerAccount, SystemdError> {
    let user_c = CString::new(user).map_err(|_| SystemdError::WorkerLookup {
        user: user.into(),
        detail: "name contains NUL".into(),
    })?;
    let mut passwd = unsafe { std::mem::zeroed::<libc::passwd>() };
    let mut result = std::ptr::null_mut();
    let size = unsafe { libc::sysconf(libc::_SC_GETPW_R_SIZE_MAX) };
    let mut buffer = vec![0_u8; if size > 0 { size as usize } else { 16_384 }];
    let status = unsafe {
        libc::getpwnam_r(
            user_c.as_ptr(),
            &mut passwd,
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            &mut result,
        )
    };
    if status != 0 || result.is_null() {
        return Err(SystemdError::WorkerLookup {
            user: user.into(),
            detail: if status == 0 {
                "account does not exist".into()
            } else {
                io::Error::from_raw_os_error(status).to_string()
            },
        });
    }
    let shell = unsafe { CStr::from_ptr(passwd.pw_shell) }.to_bytes();
    let home = unsafe { CStr::from_ptr(passwd.pw_dir) }.to_bytes();
    if passwd.pw_uid == 0
        || passwd.pw_gid == 0
        || shell != b"/usr/sbin/nologin"
        || home != b"/nonexistent"
    {
        return Err(SystemdError::WorkerPolicy {
            user: user.into(),
            detail: format!(
                "expected nonzero uid/gid home=/nonexistent shell=/usr/sbin/nologin; actual uid={} gid={} home={} shell={}",
                passwd.pw_uid,
                passwd.pw_gid,
                String::from_utf8_lossy(home),
                String::from_utf8_lossy(shell)
            ),
        });
    }
    let mut shadow = unsafe { std::mem::zeroed::<libc::spwd>() };
    let mut shadow_result = std::ptr::null_mut();
    let mut shadow_buffer = vec![0_u8; 16_384];
    let shadow_status = unsafe {
        libc::getspnam_r(
            user_c.as_ptr(),
            &mut shadow,
            shadow_buffer.as_mut_ptr().cast(),
            shadow_buffer.len(),
            &mut shadow_result,
        )
    };
    if shadow_status != 0 || shadow_result.is_null() {
        return Err(SystemdError::WorkerPolicy {
            user: user.into(),
            detail: "locked shadow record is unavailable".into(),
        });
    }
    let password = unsafe { CStr::from_ptr(shadow.sp_pwdp) }.to_bytes();
    if !matches!(password.first(), Some(b'!' | b'*')) {
        return Err(SystemdError::WorkerPolicy {
            user: user.into(),
            detail: "password field is not locked".into(),
        });
    }
    let mut group_count = 0_i32;
    unsafe {
        libc::getgrouplist(
            user_c.as_ptr(),
            passwd.pw_gid,
            std::ptr::null_mut(),
            &mut group_count,
        )
    };
    if group_count <= 0 || group_count > 1024 {
        return Err(SystemdError::WorkerPolicy {
            user: user.into(),
            detail: format!("invalid supplementary group count {group_count}"),
        });
    }
    let mut groups = vec![0 as libc::gid_t; group_count as usize];
    if unsafe {
        libc::getgrouplist(
            user_c.as_ptr(),
            passwd.pw_gid,
            groups.as_mut_ptr(),
            &mut group_count,
        )
    } < 0
    {
        return Err(SystemdError::WorkerPolicy {
            user: user.into(),
            detail: "supplementary group lookup changed during validation".into(),
        });
    }
    groups.truncate(group_count as usize);
    groups.sort_unstable();
    groups.dedup();
    if groups != [passwd.pw_gid] {
        return Err(SystemdError::WorkerPolicy {
            user: user.into(),
            detail: format!("supplementary groups are forbidden: {groups:?}"),
        });
    }
    Ok(WorkerAccount {
        uid: passwd.pw_uid,
        gid: passwd.pw_gid,
    })
}

fn show_properties(
    unit: &str,
    properties: &[&str],
) -> Result<BTreeMap<String, String>, SystemdError> {
    let mut command = manager_command(SYSTEMCTL);
    command.args(["--system", "--no-pager", "show", unit]);
    for property in properties {
        command.arg(format!("--property={property}"));
    }
    let raw = command_output(command, "inspect system unit")?;
    let mut values = BTreeMap::new();
    for line in raw.lines() {
        if let Some((key, value)) = line.split_once('=') {
            values.insert(key.to_owned(), value.to_owned());
        }
    }
    Ok(values)
}

fn verify_broker_unit(unit: &str) -> Result<(), SystemdError> {
    let values = show_properties(unit, &["LoadState", "Id", "ActiveState"])?;
    if values.get("LoadState").map(String::as_str) != Some("loaded")
        || values.get("Id").map(String::as_str) != Some(unit)
        || values.get("ActiveState").map(String::as_str) != Some("active")
    {
        return Err(SystemdError::Manager {
            operation: "verify broker unit",
            detail: format!("unit={unit} properties={values:?}"),
        });
    }
    Ok(())
}

fn property_has_unit(values: &BTreeMap<String, String>, property: &str, unit: &str) -> bool {
    values
        .get(property)
        .is_some_and(|value| value.split_whitespace().any(|candidate| candidate == unit))
}

fn verify_fixed_slice_contract() -> Result<BTreeMap<String, String>, SystemdError> {
    let values = show_properties(
        STAGE_SLICE_NAME,
        &[
            "LoadState",
            "Id",
            "ActiveState",
            "ControlGroup",
            "Transient",
            "StopWhenUnneeded",
            "BindsTo",
            "After",
        ],
    )?;
    let control_group = values
        .get("ControlGroup")
        .map(String::as_str)
        .unwrap_or_default();
    if values.get("LoadState").map(String::as_str) != Some("loaded")
        || values.get("Id").map(String::as_str) != Some(STAGE_SLICE_NAME)
        || values.get("Transient").map(String::as_str) != Some("no")
        || values.get("StopWhenUnneeded").map(String::as_str) != Some("yes")
        || !property_has_unit(&values, "BindsTo", BROKER_UNIT_NAME)
        || !property_has_unit(&values, "After", BROKER_UNIT_NAME)
        || (!control_group.is_empty() && control_group != STAGE_SLICE_CONTROL_GROUP)
    {
        return Err(SystemdError::Manager {
            operation: "verify fixed stage slice",
            detail: format!(
                "expected installed {STAGE_SLICE_NAME} bound/ordered to {BROKER_UNIT_NAME}, StopWhenUnneeded=yes, Transient=no, cgroup={STAGE_SLICE_CONTROL_GROUP}; properties={values:?}"
            ),
        });
    }
    Ok(values)
}

fn verify_worker_manager_absent(
    account: &WorkerAccount,
    worker_user: &str,
) -> Result<(), SystemdError> {
    let uid = account.uid;
    for path in [
        format!("/run/user/{uid}/bus"),
        format!("/run/user/{uid}/systemd/private"),
        format!("/var/lib/systemd/linger/{worker_user}"),
    ] {
        match fs::symlink_metadata(&path) {
            Ok(metadata) => {
                return Err(SystemdError::WorkerManagerPresent {
                    uid,
                    evidence: format!("{path} exists with type {:?}", metadata.file_type()),
                });
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(SystemdError::WorkerManagerPresent {
                    uid,
                    evidence: format!("cannot prove {path} absent: {error}"),
                });
            }
        }
    }
    let user_unit = format!("user@{uid}.service");
    let values = show_properties(&user_unit, &["LoadState", "ActiveState"])?;
    let active = values
        .get("ActiveState")
        .map(String::as_str)
        .unwrap_or_default();
    if matches!(
        active,
        "active" | "activating" | "reloading" | "deactivating"
    ) {
        return Err(SystemdError::WorkerManagerPresent {
            uid,
            evidence: format!("unit={user_unit} properties={values:?}"),
        });
    }
    Ok(())
}

fn worker_processes(uid: u32) -> Result<Vec<u32>, SystemdError> {
    let mut pids = Vec::new();
    let entries = fs::read_dir("/proc").map_err(|source| io_error("enumerate /proc", source))?;
    for entry in entries {
        let entry = entry.map_err(|source| io_error("read /proc entry", source))?;
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|value| value.parse::<u32>().ok())
        else {
            continue;
        };
        let status = match fs::read_to_string(entry.path().join("status")) {
            Ok(status) => status,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(io_error("read worker process status", error)),
        };
        let matches_uid = status
            .lines()
            .find_map(|line| line.strip_prefix("Uid:"))
            .is_some_and(|tail| {
                let values: Vec<_> = tail
                    .split_whitespace()
                    .filter_map(|value| value.parse::<u32>().ok())
                    .collect();
                values.len() == 4 && values.iter().any(|value| *value == uid)
            });
        if matches_uid {
            pids.push(pid);
        }
    }
    pids.sort_unstable();
    Ok(pids)
}

fn proc_control_group(pid: u32) -> Result<Option<String>, SystemdError> {
    let text = match fs::read_to_string(format!("/proc/{pid}/cgroup")) {
        Ok(value) => value,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(io_error("read process cgroup", error)),
    };
    let mut unified = text.lines().filter_map(|line| line.strip_prefix("0::"));
    let first = unified
        .next()
        .ok_or_else(|| SystemdError::RecoveryRequired {
            detail: format!("pid {pid} has no unified cgroup entry: {text:?}"),
        })?;
    if unified.next().is_some() || (first != "/" && validate_control_group(first).is_err()) {
        return Err(SystemdError::RecoveryRequired {
            detail: format!("pid {pid} has malformed unified cgroup entry: {text:?}"),
        });
    }
    Ok(Some(first.to_owned()))
}

fn cgroup_contains(boundary: &str, candidate: &str) -> bool {
    candidate == boundary
        || candidate
            .strip_prefix(boundary)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

fn worker_process_locations(uid: u32) -> Result<Vec<(u32, String)>, SystemdError> {
    let mut locations = Vec::new();
    for pid in worker_processes(uid)? {
        if let Some(control_group) = proc_control_group(pid)? {
            locations.push((pid, control_group));
        }
    }
    locations.sort_unstable();
    Ok(locations)
}

fn processes_in_cgroup(boundary: &str) -> Result<Vec<u32>, SystemdError> {
    let mut pids = Vec::new();
    let entries = fs::read_dir("/proc").map_err(|source| io_error("enumerate /proc", source))?;
    for entry in entries {
        let entry = entry.map_err(|source| io_error("read /proc entry", source))?;
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|value| value.parse::<u32>().ok())
        else {
            continue;
        };
        if proc_control_group(pid)?
            .as_deref()
            .is_some_and(|candidate| cgroup_contains(boundary, candidate))
        {
            pids.push(pid);
        }
    }
    pids.sort_unstable();
    Ok(pids)
}

fn require_no_worker_process(account: &WorkerAccount) -> Result<(), SystemdError> {
    let pids = worker_processes(account.uid)?;
    if pids.is_empty() {
        Ok(())
    } else {
        Err(SystemdError::WorkerProcessPresent {
            uid: account.uid,
            pids,
        })
    }
}

impl ProcessFd {
    fn open(pid: u32) -> Result<Self, SystemdError> {
        if pid == 0 || pid > i32::MAX as u32 {
            return Err(SystemdError::ProcessIdentity {
                pid,
                detail: "invalid MainPID".into(),
            });
        }
        let raw = unsafe { libc::syscall(libc::SYS_pidfd_open, pid as libc::pid_t, 0_u32) };
        if raw < 0 {
            return Err(SystemdError::ProcessIdentity {
                pid,
                detail: format!("pidfd_open: {}", io::Error::last_os_error()),
            });
        }
        Ok(Self {
            pid,
            fd: unsafe { OwnedFd::from_raw_fd(raw as RawFd) },
        })
    }

    fn duplicate(&self) -> Result<Self, SystemdError> {
        Ok(Self {
            pid: self.pid,
            fd: duplicate_fd(self.fd.as_raw_fd(), "duplicate stage pidfd")?,
        })
    }

    fn exited(&self) -> Result<bool, SystemdError> {
        poll_pidfd(self.fd.as_raw_fd(), Duration::ZERO)
    }

    fn wait_timeout(&self, timeout: Duration) -> Result<bool, SystemdError> {
        poll_pidfd(self.fd.as_raw_fd(), timeout)
    }

    fn kill(&self) -> Result<(), SystemdError> {
        let result = unsafe {
            libc::syscall(
                libc::SYS_pidfd_send_signal,
                self.fd.as_raw_fd(),
                libc::SIGKILL,
                std::ptr::null::<libc::siginfo_t>(),
                0_u32,
            )
        };
        if result == 0 {
            Ok(())
        } else {
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::ESRCH) {
                Ok(())
            } else {
                Err(SystemdError::ProcessIdentity {
                    pid: self.pid,
                    detail: format!("pidfd_send_signal: {error}"),
                })
            }
        }
    }
}

fn poll_pidfd(fd: RawFd, timeout: Duration) -> Result<bool, SystemdError> {
    let timeout_ms = i32::try_from(timeout.as_millis()).unwrap_or(i32::MAX);
    let mut pollfd = libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };
    loop {
        let result = unsafe { libc::poll(&mut pollfd, 1, timeout_ms) };
        if result > 0 {
            return Ok(pollfd.revents & (libc::POLLIN | libc::POLLHUP) != 0);
        }
        if result == 0 {
            return Ok(false);
        }
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::Interrupted {
            return Err(io_error("poll pidfd", error));
        }
    }
}

impl CgroupRoot {
    fn open() -> Result<Self, SystemdError> {
        let path = CString::new(CGROUP_ROOT).expect("static cgroup path");
        let raw = unsafe {
            libc::open(
                path.as_ptr(),
                libc::O_PATH | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            )
        };
        if raw < 0 {
            return Err(SystemdError::Cgroup {
                control: CGROUP_ROOT.into(),
                detail: format!("open root: {}", io::Error::last_os_error()),
            });
        }
        let fd = unsafe { OwnedFd::from_raw_fd(raw) };
        let mut statfs = std::mem::MaybeUninit::<libc::statfs>::uninit();
        if unsafe { libc::fstatfs(fd.as_raw_fd(), statfs.as_mut_ptr()) } != 0 {
            return Err(SystemdError::Cgroup {
                control: CGROUP_ROOT.into(),
                detail: format!("fstatfs: {}", io::Error::last_os_error()),
            });
        }
        let statfs = unsafe { statfs.assume_init() };
        if statfs.f_type != CGROUP2_SUPER_MAGIC {
            return Err(SystemdError::Cgroup {
                control: CGROUP_ROOT.into(),
                detail: format!(
                    "expected cgroup2 magic {CGROUP2_SUPER_MAGIC:#x}; actual={:#x}",
                    statfs.f_type
                ),
            });
        }
        Ok(Self { fd })
    }

    fn open_group(&self, control_group: &str) -> Result<CgroupAuthority, SystemdError> {
        open_cgroup(self.fd.as_raw_fd(), control_group)
    }

    fn open_group_optional(
        &self,
        control_group: &str,
    ) -> Result<Option<CgroupAuthority>, SystemdError> {
        match open_cgroup_raw(self.fd.as_raw_fd(), control_group) {
            Ok(directory) => CgroupAuthority::from_directory(control_group, directory).map(Some),
            Err(error) if error.raw_os_error() == Some(libc::ENOENT) => Ok(None),
            Err(error) => Err(SystemdError::Cgroup {
                control: control_group.into(),
                detail: format!("openat2: {error}"),
            }),
        }
    }
}

fn validate_control_group(control_group: &str) -> Result<&str, SystemdError> {
    let Some(relative) = control_group.strip_prefix('/') else {
        return Err(SystemdError::Cgroup {
            control: control_group.into(),
            detail: "path is not absolute".into(),
        });
    };
    if relative.is_empty()
        || relative
            .split('/')
            .any(|component| component.is_empty() || matches!(component, "." | ".."))
    {
        return Err(SystemdError::Cgroup {
            control: control_group.into(),
            detail: "path is not normalized".into(),
        });
    }
    Ok(relative)
}

fn openat2(parent: RawFd, path: &CStr, flags: u64, resolve: u64) -> io::Result<OwnedFd> {
    let how = OpenHow {
        flags,
        mode: 0,
        resolve,
    };
    let raw = unsafe {
        libc::syscall(
            libc::SYS_openat2,
            parent,
            path.as_ptr(),
            &how,
            std::mem::size_of::<OpenHow>(),
        ) as RawFd
    };
    if raw < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(unsafe { OwnedFd::from_raw_fd(raw) })
    }
}

fn open_cgroup_raw(root_fd: RawFd, control_group: &str) -> io::Result<OwnedFd> {
    let relative = validate_control_group(control_group)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error.to_string()))?;
    let path = CString::new(relative)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "cgroup path contains NUL"))?;
    openat2(
        root_fd,
        &path,
        (libc::O_PATH | libc::O_DIRECTORY | libc::O_CLOEXEC) as u64,
        RESOLVE_BENEATH | RESOLVE_NO_SYMLINKS | RESOLVE_NO_MAGICLINKS | RESOLVE_NO_XDEV,
    )
}

fn open_cgroup(root_fd: RawFd, control_group: &str) -> Result<CgroupAuthority, SystemdError> {
    let directory =
        open_cgroup_raw(root_fd, control_group).map_err(|error| SystemdError::Cgroup {
            control: control_group.into(),
            detail: format!("openat2: {error}"),
        })?;
    CgroupAuthority::from_directory(control_group, directory)
}

impl CgroupAuthority {
    fn from_directory(control_group: &str, directory: OwnedFd) -> Result<Self, SystemdError> {
        let mut statfs = std::mem::MaybeUninit::<libc::statfs>::uninit();
        if unsafe { libc::fstatfs(directory.as_raw_fd(), statfs.as_mut_ptr()) } != 0 {
            return Err(SystemdError::Cgroup {
                control: control_group.into(),
                detail: format!("fstatfs: {}", io::Error::last_os_error()),
            });
        }
        if unsafe { statfs.assume_init() }.f_type != CGROUP2_SUPER_MAGIC {
            return Err(SystemdError::Cgroup {
                control: control_group.into(),
                detail: "descriptor is not on cgroup2".into(),
            });
        }
        let events = openat2(
            directory.as_raw_fd(),
            c"cgroup.events",
            (libc::O_RDONLY | libc::O_CLOEXEC) as u64,
            RESOLVE_BENEATH | RESOLVE_NO_SYMLINKS | RESOLVE_NO_MAGICLINKS | RESOLVE_NO_XDEV,
        )
        .map_err(|error| SystemdError::Cgroup {
            control: control_group.into(),
            detail: format!("open cgroup.events: {error}"),
        })?;
        let kill = openat2(
            directory.as_raw_fd(),
            c"cgroup.kill",
            (libc::O_WRONLY | libc::O_CLOEXEC) as u64,
            RESOLVE_BENEATH | RESOLVE_NO_SYMLINKS | RESOLVE_NO_MAGICLINKS | RESOLVE_NO_XDEV,
        )
        .map_err(|error| SystemdError::Cgroup {
            control: control_group.into(),
            detail: format!("open cgroup.kill: {error}"),
        })?;
        Ok(Self {
            control_group: control_group.into(),
            directory,
            events,
            kill,
        })
    }

    fn duplicate(&self) -> Result<Self, SystemdError> {
        Ok(Self {
            control_group: self.control_group.clone(),
            directory: duplicate_fd(self.directory.as_raw_fd(), "duplicate cgroup directory")?,
            events: duplicate_fd(self.events.as_raw_fd(), "duplicate cgroup.events")?,
            kill: duplicate_fd(self.kill.as_raw_fd(), "duplicate cgroup.kill")?,
        })
    }

    fn identity(&self) -> Result<(u64, u64), SystemdError> {
        let stat = fstat(self.directory.as_raw_fd(), "fstat cgroup directory")?;
        Ok((stat.st_dev as u64, stat.st_ino as u64))
    }

    fn verify_identity(&self, expected: &CgroupIdentity) -> Result<(), SystemdError> {
        if self.control_group != expected.control_group.as_str() {
            return Err(SystemdError::RecoveryRequired {
                detail: format!(
                    "cgroup path mismatch expected={} actual={}",
                    expected.control_group.as_str(),
                    self.control_group
                ),
            });
        }
        let (device, inode) = self.identity()?;
        if device != expected.device || inode != expected.inode {
            return Err(SystemdError::RecoveryRequired {
                detail: format!(
                    "cgroup descriptor identity mismatch path={} expected_dev={} expected_ino={} actual_dev={device} actual_ino={inode}",
                    self.control_group, expected.device, expected.inode
                ),
            });
        }
        Ok(())
    }

    fn population(&self) -> Result<u8, SystemdError> {
        if unsafe { libc::lseek(self.events.as_raw_fd(), 0, libc::SEEK_SET) } < 0 {
            return Err(SystemdError::Cgroup {
                control: self.control_group.clone(),
                detail: format!("seek cgroup.events: {}", io::Error::last_os_error()),
            });
        }
        let mut bytes = [0_u8; 4096];
        let length = unsafe {
            libc::read(
                self.events.as_raw_fd(),
                bytes.as_mut_ptr().cast(),
                bytes.len(),
            )
        };
        if length < 0 {
            return Err(SystemdError::Cgroup {
                control: self.control_group.clone(),
                detail: format!("read cgroup.events: {}", io::Error::last_os_error()),
            });
        }
        let text =
            std::str::from_utf8(&bytes[..length as usize]).map_err(|_| SystemdError::Cgroup {
                control: self.control_group.clone(),
                detail: "cgroup.events is not UTF-8".into(),
            })?;
        for line in text.lines() {
            if let Some(value) = line.strip_prefix("populated ") {
                return match value {
                    "0" => Ok(0),
                    "1" => Ok(1),
                    _ => Err(SystemdError::Cgroup {
                        control: self.control_group.clone(),
                        detail: format!("invalid populated value {value:?}"),
                    }),
                };
            }
        }
        Err(SystemdError::Cgroup {
            control: self.control_group.clone(),
            detail: "cgroup.events lacks populated".into(),
        })
    }

    fn kill_if_populated(&self) -> Result<(), SystemdError> {
        if self.population()? == 0 {
            return Ok(());
        }
        let payload = b"1";
        loop {
            let written = unsafe {
                libc::write(
                    self.kill.as_raw_fd(),
                    payload.as_ptr().cast(),
                    payload.len(),
                )
            };
            if written == payload.len() as isize {
                return Ok(());
            }
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::Interrupted {
                return Err(SystemdError::Cgroup {
                    control: self.control_group.clone(),
                    detail: format!("write cgroup.kill: {error}"),
                });
            }
        }
    }

    fn prove_empty(&self, timeout: Duration) -> Result<(), SystemdError> {
        let deadline = Instant::now() + timeout;
        loop {
            let populated = self.population()?;
            if populated == 0 {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(SystemdError::Cgroup {
                    control: self.control_group.clone(),
                    detail: format!("remains populated={populated}"),
                });
            }
            thread::sleep(POLL_INTERVAL);
        }
    }
}

fn parse_unit_evidence(
    unit: &str,
    expected_worker: &str,
    expected_uid: u32,
) -> Result<UnitEvidence, SystemdError> {
    let values = show_properties(
        unit,
        &[
            "LoadState",
            "Id",
            "InvocationID",
            "ControlGroup",
            "ActiveState",
            "Result",
            "MainPID",
            "User",
            "Group",
            "Slice",
            "CollectMode",
            "Conflicts",
            "BindsTo",
            "After",
        ],
    )?;
    if values.get("LoadState").map(String::as_str) != Some("loaded")
        || values.get("Id").map(String::as_str) != Some(unit)
    {
        return Err(SystemdError::Publication {
            unit: unit.into(),
            detail: format!("properties={values:?}"),
        });
    }
    let invocation_id = values.get("InvocationID").cloned().unwrap_or_default();
    if invocation_id.len() != 32
        || !invocation_id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        return Err(SystemdError::Publication {
            unit: unit.into(),
            detail: format!("invalid InvocationID {invocation_id:?}"),
        });
    }
    let control_group = values.get("ControlGroup").cloned().unwrap_or_default();
    validate_control_group(&control_group)?;
    let main_pid = values
        .get("MainPID")
        .and_then(|value| value.parse::<u32>().ok())
        .filter(|pid| *pid > 1)
        .ok_or_else(|| SystemdError::Publication {
            unit: unit.into(),
            detail: format!("invalid MainPID in {values:?}"),
        })?;
    let expected_conflict = format!("user@{expected_uid}.service");
    let conflicts = values
        .get("Conflicts")
        .map(|value| value.split_whitespace().collect::<BTreeSet<_>>())
        .unwrap_or_default();
    let active_state = values.get("ActiveState").cloned().unwrap_or_default();
    if !matches!(active_state.as_str(), "active" | "activating")
        || values.get("User").map(String::as_str) != Some(expected_worker)
        || values.get("Group").map(String::as_str) != Some(expected_worker)
        || values.get("Slice").map(String::as_str) != Some(STAGE_SLICE_NAME)
        || values.get("CollectMode").map(String::as_str) != Some("inactive-or-failed")
        || !conflicts.contains(expected_conflict.as_str())
        || !property_has_unit(&values, "BindsTo", BROKER_UNIT_NAME)
        || !property_has_unit(&values, "After", BROKER_UNIT_NAME)
    {
        return Err(SystemdError::Publication {
            unit: unit.into(),
            detail: format!("transient policy mismatch: {values:?}"),
        });
    }
    let slice_values = verify_fixed_slice_contract()?;
    if slice_values.get("LoadState").map(String::as_str) != Some("loaded")
        || slice_values.get("Id").map(String::as_str) != Some(STAGE_SLICE_NAME)
    {
        return Err(SystemdError::Publication {
            unit: STAGE_SLICE_NAME.into(),
            detail: format!("properties={slice_values:?}"),
        });
    }
    let slice_control_group = slice_values
        .get("ControlGroup")
        .cloned()
        .unwrap_or_default();
    validate_control_group(&slice_control_group)?;
    if slice_control_group != STAGE_SLICE_CONTROL_GROUP {
        return Err(SystemdError::Publication {
            unit: STAGE_SLICE_NAME.into(),
            detail: format!(
                "expected cgroup {STAGE_SLICE_CONTROL_GROUP}; actual={slice_control_group}"
            ),
        });
    }
    let expected_prefix = format!("{slice_control_group}/");
    if !control_group.starts_with(&expected_prefix)
        || control_group[expected_prefix.len()..].contains('/')
    {
        return Err(SystemdError::Publication {
            unit: unit.into(),
            detail: format!(
                "service cgroup {control_group} is not an immediate child of slice {slice_control_group}"
            ),
        });
    }
    Ok(UnitEvidence {
        unit_name: unit.into(),
        invocation_id,
        control_group,
        control_group_device: 0,
        control_group_inode: 0,
        slice_control_group,
        slice_control_group_device: 0,
        slice_control_group_inode: 0,
        worker_user: expected_worker.into(),
        worker_uid: expected_uid,
        main_pid,
        active_state,
        result: values.get("Result").cloned().unwrap_or_default(),
    })
}

fn read_proc_bytes(pid: u32, name: &str) -> Result<Vec<u8>, SystemdError> {
    fs::read(format!("/proc/{pid}/{name}")).map_err(|error| SystemdError::ProcessIdentity {
        pid,
        detail: format!("read {name}: {error}"),
    })
}

fn verify_process_identity(
    evidence: &UnitEvidence,
    account: &WorkerAccount,
    shim: &TrustedIdentity,
    expected_argv: &[OsString],
    expected_cwd: &libc::stat,
) -> Result<(), SystemdError> {
    let pid = evidence.main_pid;
    let status = String::from_utf8(read_proc_bytes(pid, "status")?).map_err(|_| {
        SystemdError::ProcessIdentity {
            pid,
            detail: "status is not UTF-8".into(),
        }
    })?;
    let uids: Vec<u32> = status
        .lines()
        .find_map(|line| line.strip_prefix("Uid:"))
        .into_iter()
        .flat_map(|tail| tail.split_whitespace())
        .filter_map(|value| value.parse().ok())
        .collect();
    let gids: Vec<u32> = status
        .lines()
        .find_map(|line| line.strip_prefix("Gid:"))
        .into_iter()
        .flat_map(|tail| tail.split_whitespace())
        .filter_map(|value| value.parse().ok())
        .collect();
    if uids != [account.uid; 4] || gids != [account.gid; 4] {
        return Err(SystemdError::ProcessIdentity {
            pid,
            detail: format!(
                "expected uid={} gid={}; actual uids={uids:?} gids={gids:?}",
                account.uid, account.gid
            ),
        });
    }
    let exe_path = CString::new(format!("/proc/{pid}/exe")).expect("numeric proc path");
    let exe_raw = unsafe { libc::open(exe_path.as_ptr(), libc::O_PATH | libc::O_CLOEXEC) };
    if exe_raw < 0 {
        return Err(SystemdError::ProcessIdentity {
            pid,
            detail: format!("open exe: {}", io::Error::last_os_error()),
        });
    }
    let exe = unsafe { OwnedFd::from_raw_fd(exe_raw) };
    let exe_stat = fstat(exe.as_raw_fd(), "fstat stage exe")?;
    if exe_stat.st_dev != shim.stat.st_dev
        || exe_stat.st_ino != shim.stat.st_ino
        || exe_stat.st_uid != 0
        || exe_stat.st_gid != 0
        || exe_stat.st_mode & libc::S_IFMT != libc::S_IFREG
        || exe_stat.st_mode as u32 & 0o7777 != 0o755
        || exe_stat.st_nlink != 1
    {
        return Err(SystemdError::ProcessIdentity {
            pid,
            detail: format!(
                "shim identity mismatch expected dev={} ino={}; actual dev={} ino={} uid={} gid={} mode={:04o} nlink={}",
                shim.stat.st_dev,
                shim.stat.st_ino,
                exe_stat.st_dev,
                exe_stat.st_ino,
                exe_stat.st_uid,
                exe_stat.st_gid,
                exe_stat.st_mode as u32 & 0o7777,
                exe_stat.st_nlink
            ),
        });
    }
    let command_line = read_proc_bytes(pid, "cmdline")?;
    let observed: Vec<&[u8]> = command_line
        .split(|byte| *byte == 0)
        .filter(|value| !value.is_empty())
        .collect();
    let expected: Vec<&[u8]> = expected_argv
        .iter()
        .map(|value| value.as_os_str().as_bytes())
        .collect();
    if observed != expected {
        return Err(SystemdError::ProcessIdentity {
            pid,
            detail: format!(
                "cmdline mismatch: expected {} args, observed {}",
                expected.len(),
                observed.len()
            ),
        });
    }
    let cgroup = String::from_utf8(read_proc_bytes(pid, "cgroup")?).map_err(|_| {
        SystemdError::ProcessIdentity {
            pid,
            detail: "cgroup file is not UTF-8".into(),
        }
    })?;
    let expected_cgroup = format!("0::{}", evidence.control_group);
    if cgroup.trim() != expected_cgroup {
        return Err(SystemdError::ProcessIdentity {
            pid,
            detail: format!(
                "expected cgroup {expected_cgroup:?}; actual={:?}",
                cgroup.trim()
            ),
        });
    }
    let environment = read_proc_bytes(pid, "environ")?;
    let invocation_values: Vec<_> = environment
        .split(|byte| *byte == 0)
        .filter_map(|entry| entry.strip_prefix(b"INVOCATION_ID="))
        .collect();
    if invocation_values != [evidence.invocation_id.as_bytes()] {
        return Err(SystemdError::ProcessIdentity {
            pid,
            detail: "process INVOCATION_ID does not match unit evidence".into(),
        });
    }
    let cwd_path = CString::new(format!("/proc/{pid}/cwd")).expect("numeric proc path");
    let cwd_raw = unsafe {
        libc::open(
            cwd_path.as_ptr(),
            libc::O_PATH | libc::O_DIRECTORY | libc::O_CLOEXEC,
        )
    };
    if cwd_raw < 0 {
        return Err(SystemdError::ProcessIdentity {
            pid,
            detail: format!("open cwd: {}", io::Error::last_os_error()),
        });
    }
    let cwd = unsafe { OwnedFd::from_raw_fd(cwd_raw) };
    let cwd_stat = fstat(cwd.as_raw_fd(), "fstat stage cwd")?;
    if cwd_stat.st_dev != expected_cwd.st_dev || cwd_stat.st_ino != expected_cwd.st_ino {
        return Err(SystemdError::ProcessIdentity {
            pid,
            detail: format!(
                "cwd mismatch expected dev={} ino={}; actual dev={} ino={}",
                expected_cwd.st_dev, expected_cwd.st_ino, cwd_stat.st_dev, cwd_stat.st_ino
            ),
        });
    }
    Ok(())
}

fn kernel_random_token() -> Result<[u8; TOKEN_BYTES], SystemdError> {
    let mut token = [0_u8; TOKEN_BYTES];
    let mut offset = 0;
    while offset < token.len() {
        let result = unsafe {
            libc::getrandom(token[offset..].as_mut_ptr().cast(), token.len() - offset, 0)
        };
        if result > 0 {
            offset += result as usize;
            continue;
        }
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::Interrupted {
            token.fill(0);
            return Err(io_error("read kernel release nonce", error));
        }
    }
    Ok(token)
}

fn hex_token(token: &[u8; TOKEN_BYTES]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(TOKEN_BYTES * 2);
    for byte in token {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

fn shim_argv(spec: &StageSpec, token_hex: &str, cwd_stat: &libc::stat) -> Vec<OsString> {
    let mut argv = vec![
        OsString::from(STAGE_SHIM),
        OsString::from(token_hex),
        spec.execution_root.as_os_str().to_owned(),
        spec.relative_cwd.as_os_str().to_owned(),
        OsString::from(spec.execution_root_uid.to_string()),
        OsString::from(format!("{:04o}", spec.execution_root_mode)),
        OsString::from((cwd_stat.st_dev as u64).to_string()),
        OsString::from((cwd_stat.st_ino as u64).to_string()),
        OsString::from(&spec.worker_user),
        OsString::from(spec.environment.len().to_string()),
    ];
    argv.extend(spec.environment.iter().map(|(name, _)| name.clone()));
    argv.push(OsString::from("--"));
    argv.extend(spec.argv.iter().cloned());
    argv
}

fn append_property(command: &mut Command, name: &str, value: &str) {
    command.arg(format!("--property={name}={value}"));
}

fn build_systemd_run(
    spec: &StageSpec,
    account: &WorkerAccount,
    shim_arguments: &[OsString],
    stdout_fd: RawFd,
    stderr_fd: RawFd,
) -> Result<Command, SystemdError> {
    let stdout = duplicate_fd(stdout_fd, "duplicate stage stdout")?;
    let stderr = duplicate_fd(stderr_fd, "duplicate stage stderr")?;
    let mut command = manager_command(SYSTEMD_RUN);
    command.args([
        "--system",
        "--quiet",
        "--collect",
        "--expand-environment=no",
        "--wait",
        "--pipe",
        "--service-type=exec",
        "--working-directory=/",
    ]);
    command.args(["--unit", &spec.unit_name, "--slice", STAGE_SLICE_NAME]);
    for (name, value) in [
        ("ExitType", "cgroup"),
        ("KillMode", "control-group"),
        ("SendSIGKILL", "yes"),
        ("Delegate", "no"),
        ("NoNewPrivileges", "yes"),
        ("CapabilityBoundingSet", ""),
        ("AmbientCapabilities", ""),
        ("ProtectControlGroups", "strict"),
        ("ProtectKernelTunables", "yes"),
        ("ProtectKernelModules", "yes"),
        ("ProtectKernelLogs", "yes"),
        ("ProtectClock", "yes"),
        ("ProtectHostname", "yes"),
        ("ProtectProc", "invisible"),
        ("ProcSubset", "pid"),
        ("PrivateDevices", "yes"),
        ("PrivateNetwork", "yes"),
        ("ProtectSystem", "strict"),
        ("ProtectHome", "read-only"),
        ("PrivateTmp", "yes"),
        ("RestrictNamespaces", "yes"),
        ("RestrictRealtime", "yes"),
        ("LockPersonality", "yes"),
        ("SystemCallArchitectures", "native"),
        ("UMask", "0077"),
        ("SupplementaryGroups", ""),
        ("PAMName", ""),
    ] {
        append_property(&mut command, name, value);
    }
    // Do not add RestrictSUIDSGID=yes here. systemd's seccomp implementation
    // deliberately returns ENOSYS for every openat2(2) call because classic
    // seccomp cannot inspect open_how.mode. The release shim requires openat2
    // for its race-free cwd binding. Independent NoNewPrivileges, an empty
    // capability set, ProtectSystem=strict, and exact writable paths remain
    // mandatory above.
    append_property(&mut command, "BindsTo", BROKER_UNIT_NAME);
    append_property(&mut command, "After", BROKER_UNIT_NAME);
    append_property(
        &mut command,
        "Conflicts",
        &format!("user@{}.service", account.uid),
    );
    append_property(&mut command, "User", &spec.worker_user);
    append_property(&mut command, "Group", &spec.worker_user);
    append_property(
        &mut command,
        "ReadOnlyPaths",
        spec.execution_root.to_str().expect("validated UTF-8 root"),
    );
    for inaccessible in [
        PRIVATE_STATE_ROOT,
        "/run/user",
        "/run/systemd/private",
        "/run/systemd/notify",
        "/run/dbus/system_bus_socket",
        "/var/run/dbus/system_bus_socket",
    ] {
        append_property(&mut command, "InaccessiblePaths", inaccessible);
    }
    for path in &spec.writable_paths {
        append_property(
            &mut command,
            "ReadWritePaths",
            path.to_str().expect("validated UTF-8 writable path"),
        );
    }
    for (name, value) in &spec.environment {
        let mut assignment = OsString::from("--setenv=");
        assignment.push(name);
        assignment.push("=");
        assignment.push(value);
        command.arg(assignment);
    }
    command.arg("--");
    command.args(shim_arguments);
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));
    Ok(command)
}

fn cleanup_child(child: &mut Child) -> String {
    let deadline = Instant::now() + DRAIN_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return format!("systemd-run reaped status={status}"),
            Ok(None) if Instant::now() < deadline => thread::sleep(POLL_INTERVAL),
            Ok(None) => {
                let kill = child.kill();
                let wait = child.wait();
                return format!("systemd-run timeout kill={kill:?} wait={wait:?}");
            }
            Err(error) => return format!("systemd-run try_wait failed: {error}"),
        }
    }
}

fn cleanup_bound(
    child: &mut Child,
    stage_pidfd: &ProcessFd,
    service: &CgroupAuthority,
    slice: &CgroupAuthority,
) -> String {
    let service_kill = service.kill_if_populated();
    let slice_kill = slice.kill_if_populated();
    let pid = if stage_pidfd.wait_timeout(DRAIN_TIMEOUT).unwrap_or(false) {
        "exited".into()
    } else {
        let kill = stage_pidfd.kill();
        let wait = stage_pidfd.wait_timeout(DRAIN_TIMEOUT);
        format!("forced kill={kill:?} wait={wait:?}")
    };
    let service_empty = service.prove_empty(DRAIN_TIMEOUT);
    let slice_empty = slice.prove_empty(DRAIN_TIMEOUT);
    let child = cleanup_child(child);
    format!(
        "service_kill={service_kill:?} slice_kill={slice_kill:?} pid={pid} service_empty={service_empty:?} slice_empty={slice_empty:?} {child}"
    )
}

impl CapturedStage {
    pub fn capture(
        spec: &StageSpec,
        stdout_fd: RawFd,
        stderr_fd: RawFd,
    ) -> Result<Self, SystemdError> {
        validate_spec(spec)?;
        let real_uid = unsafe { libc::getuid() };
        let effective_uid = unsafe { libc::geteuid() };
        if real_uid != 0 || effective_uid != 0 {
            return Err(SystemdError::BrokerIdentity {
                real_uid,
                effective_uid,
            });
        }
        verify_systemd_contract()?;
        verify_broker_unit(BROKER_UNIT_NAME)?;
        verify_fixed_slice_contract()?;
        let account = lookup_worker(&spec.worker_user)?;
        verify_worker_manager_absent(&account, &spec.worker_user)?;
        require_no_worker_process(&account)?;
        let shim = trusted_executable(STAGE_SHIM, true)?;
        let cgroup_root = CgroupRoot::open()?;
        let cwd_guard = duplicate_fd(spec.cwd_fd, "duplicate resolved cwd")?;
        let cwd_stat = fstat(cwd_guard.as_raw_fd(), "fstat resolved cwd")?;
        if cwd_stat.st_mode & libc::S_IFMT != libc::S_IFDIR {
            return Err(SystemdError::InvalidSpec(
                "cwd fd is not a directory".into(),
            ));
        }
        let mut release_token = kernel_random_token()?;
        let token_hex = hex_token(&release_token);
        let shim_arguments = shim_argv(spec, &token_hex, &cwd_stat);
        let mut command = build_systemd_run(spec, &account, &shim_arguments, stdout_fd, stderr_fd)?;
        let mut child = command
            .spawn()
            .map_err(|source| io_error("launch /usr/bin/systemd-run", source))?;
        let release_pipe = match child.stdin.take() {
            Some(pipe) => pipe,
            None => {
                release_token.fill(0);
                let cleanup = cleanup_child(&mut child);
                return Err(SystemdError::Cleanup {
                    primary: SystemdError::Release(
                        "systemd-run did not expose the release pipe".into(),
                    )
                    .to_string(),
                    cleanup,
                });
            }
        };
        let deadline = Instant::now() + START_TIMEOUT;
        let (evidence, stage_pidfd, service, slice) = loop {
            let mut evidence =
                match parse_unit_evidence(&spec.unit_name, &spec.worker_user, account.uid) {
                    Ok(evidence) => evidence,
                    Err(error) if Instant::now() < deadline => {
                        let _ = error;
                        thread::sleep(POLL_INTERVAL);
                        continue;
                    }
                    Err(error) => {
                        drop(release_pipe);
                        release_token.fill(0);
                        let cleanup = cleanup_child(&mut child);
                        return Err(SystemdError::Cleanup {
                            primary: error.to_string(),
                            cleanup,
                        });
                    }
                };
            let stage_pidfd = match ProcessFd::open(evidence.main_pid) {
                Ok(pidfd) => pidfd,
                Err(error) if Instant::now() < deadline => {
                    let _ = error;
                    thread::sleep(POLL_INTERVAL);
                    continue;
                }
                Err(error) => {
                    drop(release_pipe);
                    release_token.fill(0);
                    let cleanup = cleanup_child(&mut child);
                    return Err(SystemdError::Cleanup {
                        primary: error.to_string(),
                        cleanup,
                    });
                }
            };
            let service = match cgroup_root.open_group(&evidence.control_group) {
                Ok(value) => value,
                Err(error) if Instant::now() < deadline => {
                    let _ = error;
                    thread::sleep(POLL_INTERVAL);
                    continue;
                }
                Err(error) => {
                    drop(release_pipe);
                    release_token.fill(0);
                    let cleanup = cleanup_child(&mut child);
                    return Err(SystemdError::Cleanup {
                        primary: error.to_string(),
                        cleanup,
                    });
                }
            };
            let slice = match cgroup_root.open_group(&evidence.slice_control_group) {
                Ok(value) => value,
                Err(error) => {
                    drop(release_pipe);
                    release_token.fill(0);
                    let service_kill = service.kill_if_populated();
                    let cleanup = cleanup_child(&mut child);
                    return Err(SystemdError::Cleanup {
                        primary: error.to_string(),
                        cleanup: format!("service_kill={service_kill:?} {cleanup}"),
                    });
                }
            };
            let (service_device, service_inode) = match service.identity() {
                Ok(identity) => identity,
                Err(error) => {
                    drop(release_pipe);
                    release_token.fill(0);
                    let cleanup = cleanup_bound(&mut child, &stage_pidfd, &service, &slice);
                    return Err(SystemdError::Cleanup {
                        primary: error.to_string(),
                        cleanup,
                    });
                }
            };
            let (slice_device, slice_inode) = match slice.identity() {
                Ok(identity) => identity,
                Err(error) => {
                    drop(release_pipe);
                    release_token.fill(0);
                    let cleanup = cleanup_bound(&mut child, &stage_pidfd, &service, &slice);
                    return Err(SystemdError::Cleanup {
                        primary: error.to_string(),
                        cleanup,
                    });
                }
            };
            evidence.control_group_device = service_device;
            evidence.control_group_inode = service_inode;
            evidence.slice_control_group_device = slice_device;
            evidence.slice_control_group_inode = slice_inode;
            let shim_exited = match stage_pidfd.exited() {
                Ok(value) => value,
                Err(error) => {
                    drop(release_pipe);
                    release_token.fill(0);
                    let cleanup = cleanup_bound(&mut child, &stage_pidfd, &service, &slice);
                    return Err(SystemdError::Cleanup {
                        primary: error.to_string(),
                        cleanup,
                    });
                }
            };
            if shim_exited {
                drop(release_pipe);
                release_token.fill(0);
                let cleanup = cleanup_bound(&mut child, &stage_pidfd, &service, &slice);
                return Err(SystemdError::Cleanup {
                    primary: "release shim exited before identity binding".into(),
                    cleanup,
                });
            }
            match verify_process_identity(&evidence, &account, &shim, &shim_arguments, &cwd_stat) {
                Ok(()) => break (evidence, stage_pidfd, service, slice),
                Err(error) if Instant::now() < deadline => {
                    let _ = error;
                    thread::sleep(POLL_INTERVAL);
                }
                Err(error) => {
                    drop(release_pipe);
                    release_token.fill(0);
                    let cleanup = cleanup_bound(&mut child, &stage_pidfd, &service, &slice);
                    return Err(SystemdError::Cleanup {
                        primary: error.to_string(),
                        cleanup,
                    });
                }
            }
        };
        if let Err(error) = verify_worker_manager_absent(&account, &spec.worker_user) {
            drop(release_pipe);
            release_token.fill(0);
            let cleanup = cleanup_bound(&mut child, &stage_pidfd, &service, &slice);
            return Err(SystemdError::Cleanup {
                primary: error.to_string(),
                cleanup,
            });
        }
        let pids = match worker_processes(account.uid) {
            Ok(pids) => pids,
            Err(error) => {
                drop(release_pipe);
                release_token.fill(0);
                let cleanup = cleanup_bound(&mut child, &stage_pidfd, &service, &slice);
                return Err(SystemdError::Cleanup {
                    primary: error.to_string(),
                    cleanup,
                });
            }
        };
        if pids != [evidence.main_pid] {
            drop(release_pipe);
            release_token.fill(0);
            let cleanup = cleanup_bound(&mut child, &stage_pidfd, &service, &slice);
            return Err(SystemdError::Cleanup {
                primary: SystemdError::WorkerProcessPresent {
                    uid: account.uid,
                    pids,
                }
                .to_string(),
                cleanup,
            });
        }
        let slice_pids = match processes_in_cgroup(&evidence.slice_control_group) {
            Ok(pids) => pids,
            Err(error) => {
                drop(release_pipe);
                release_token.fill(0);
                let cleanup = cleanup_bound(&mut child, &stage_pidfd, &service, &slice);
                return Err(SystemdError::Cleanup {
                    primary: error.to_string(),
                    cleanup,
                });
            }
        };
        if slice_pids != [evidence.main_pid] {
            drop(release_pipe);
            release_token.fill(0);
            let cleanup = cleanup_bound(&mut child, &stage_pidfd, &service, &slice);
            return Err(SystemdError::Cleanup {
                primary: SystemdError::RecoveryRequired {
                    detail: format!(
                        "fixed slice contains processes other than the captured shim: expected=[{}] actual={slice_pids:?}",
                        evidence.main_pid
                    ),
                }
                .to_string(),
                cleanup,
            });
        }
        Ok(Self {
            child: Some(child),
            release_pipe: Some(release_pipe),
            release_token,
            evidence,
            stage_pidfd: Some(stage_pidfd),
            service: Some(service),
            slice: Some(slice),
            cwd_guard: Some(cwd_guard),
        })
    }

    pub fn evidence(&self) -> &UnitEvidence {
        &self.evidence
    }

    pub fn abort_authority(&self) -> Result<AbortAuthority, SystemdError> {
        duplicate_abort(
            &self.evidence,
            self.stage_pidfd.as_ref(),
            self.service.as_ref(),
            self.slice.as_ref(),
        )
    }

    pub fn release(mut self, owner_pidfd: RawFd) -> Result<RunningStage, SystemdError> {
        let owner_guard = duplicate_fd(owner_pidfd, "duplicate owner pidfd")?;
        let stage_pidfd = self.stage_pidfd.as_ref().expect("captured pidfd");
        if stage_pidfd.exited()? {
            return Err(self.fail_release(SystemdError::Release(
                "release shim exited before release".into(),
            )));
        }
        // This poll is intentionally the final observation before write_all.
        if poll_pidfd(owner_guard.as_raw_fd(), Duration::ZERO)? {
            return Err(self.fail_release(SystemdError::OwnerExited));
        }
        let mut release_pipe = self.release_pipe.take().expect("captured release pipe");
        if let Err(error) = release_pipe.write_all(&self.release_token) {
            drop(release_pipe);
            return Err(self.fail_release(SystemdError::Release(format!(
                "write release token: {error}"
            ))));
        }
        drop(release_pipe);
        self.release_token.fill(0);
        Ok(RunningStage {
            child: self.child.take(),
            evidence: self.evidence.clone(),
            stage_pidfd: self.stage_pidfd.take(),
            service: self.service.take(),
            slice: self.slice.take(),
            cwd_guard: self.cwd_guard.take(),
            owner_guard: Some(owner_guard),
        })
    }

    fn fail_release(&mut self, primary: SystemdError) -> SystemdError {
        self.release_pipe.take();
        self.release_token.fill(0);
        let cleanup = self.cleanup();
        SystemdError::Cleanup {
            primary: primary.to_string(),
            cleanup,
        }
    }

    fn cleanup(&mut self) -> String {
        self.release_pipe.take();
        self.release_token.fill(0);
        let result = match (
            self.child.as_mut(),
            self.stage_pidfd.as_ref(),
            self.service.as_ref(),
            self.slice.as_ref(),
        ) {
            (Some(child), Some(pidfd), Some(service), Some(slice)) => {
                cleanup_bound(child, pidfd, service, slice)
            }
            (Some(child), _, _, _) => cleanup_child(child),
            _ => "nothing remained to clean".into(),
        };
        self.child.take();
        self.stage_pidfd.take();
        self.service.take();
        self.slice.take();
        self.cwd_guard.take();
        result
    }
}

impl Drop for CapturedStage {
    fn drop(&mut self) {
        if self.child.is_some() {
            let cleanup = self.cleanup();
            eprintln!("CALYX_GATEBROKER_CAPTURE_DROP_CLEANUP: {cleanup}");
        }
    }
}

fn duplicate_abort(
    evidence: &UnitEvidence,
    pidfd: Option<&ProcessFd>,
    service: Option<&CgroupAuthority>,
    slice: Option<&CgroupAuthority>,
) -> Result<AbortAuthority, SystemdError> {
    Ok(AbortAuthority {
        evidence: evidence.clone(),
        stage_pidfd: pidfd
            .ok_or_else(|| SystemdError::Release("stage pidfd was consumed".into()))?
            .duplicate()?,
        service: service
            .ok_or_else(|| SystemdError::Release("service cgroup was consumed".into()))?
            .duplicate()?,
        slice: slice
            .ok_or_else(|| SystemdError::Release("slice cgroup was consumed".into()))?
            .duplicate()?,
    })
}

impl RunningStage {
    pub fn evidence(&self) -> &UnitEvidence {
        &self.evidence
    }

    pub fn abort_authority(&self) -> Result<AbortAuthority, SystemdError> {
        duplicate_abort(
            &self.evidence,
            self.stage_pidfd.as_ref(),
            self.service.as_ref(),
            self.slice.as_ref(),
        )
    }

    pub fn wait(mut self) -> Result<StageResult, SystemdError> {
        let status = self
            .child
            .as_mut()
            .expect("running systemd-run child")
            .wait()
            .map_err(|source| io_error("wait for /usr/bin/systemd-run", source))?;
        let pidfd = self.stage_pidfd.as_ref().expect("running stage pidfd");
        let service = self.service.as_ref().expect("running service cgroup");
        let slice = self.slice.as_ref().expect("running slice cgroup");
        let proof = (|| {
            if !pidfd.wait_timeout(DRAIN_TIMEOUT)? {
                return Err(SystemdError::ProcessIdentity {
                    pid: pidfd.pid,
                    detail: "pidfd did not report exit after systemd-run completed".into(),
                });
            }
            service.prove_empty(DRAIN_TIMEOUT)?;
            slice.prove_empty(DRAIN_TIMEOUT)?;
            Ok(())
        })();
        if let Err(error) = proof {
            let cleanup = cleanup_bound(
                self.child.as_mut().expect("running child"),
                pidfd,
                service,
                slice,
            );
            self.child.take();
            return Err(SystemdError::Cleanup {
                primary: error.to_string(),
                cleanup,
            });
        }
        let systemd_run_status = status
            .code()
            .unwrap_or_else(|| status.signal().map(|signal| 128 + signal).unwrap_or(125));
        let (main_code, main_status) = match (status.code(), status.signal()) {
            (Some(code), _) => ("exited".into(), code),
            (_, Some(signal)) => ("killed".into(), signal),
            _ => ("unknown".into(), 125),
        };
        self.child.take();
        self.stage_pidfd.take();
        self.service.take();
        self.slice.take();
        self.cwd_guard.take();
        self.owner_guard.take();
        Ok(StageResult {
            evidence: self.evidence.clone(),
            exit_status: systemd_run_status,
            main_code,
            main_status,
            systemd_run_status,
        })
    }

    fn cleanup(&mut self) -> String {
        match (
            self.child.as_mut(),
            self.stage_pidfd.as_ref(),
            self.service.as_ref(),
            self.slice.as_ref(),
        ) {
            (Some(child), Some(pidfd), Some(service), Some(slice)) => {
                cleanup_bound(child, pidfd, service, slice)
            }
            (Some(child), _, _, _) => cleanup_child(child),
            _ => "nothing remained to clean".into(),
        }
    }
}

impl Drop for RunningStage {
    fn drop(&mut self) {
        if self.child.is_some() {
            let cleanup = self.cleanup();
            eprintln!("CALYX_GATEBROKER_RUNNING_DROP_CLEANUP: {cleanup}");
        }
    }
}

impl AbortAuthority {
    pub fn evidence(&self) -> &UnitEvidence {
        &self.evidence
    }

    pub fn kill_and_prove_empty(&self) -> Result<(), SystemdError> {
        self.service.kill_if_populated()?;
        self.slice.kill_if_populated()?;
        if !self.stage_pidfd.wait_timeout(DRAIN_TIMEOUT)? {
            self.stage_pidfd.kill()?;
            if !self.stage_pidfd.wait_timeout(DRAIN_TIMEOUT)? {
                return Err(SystemdError::ProcessIdentity {
                    pid: self.stage_pidfd.pid,
                    detail: "pidfd remained live after cgroup.kill".into(),
                });
            }
        }
        self.service.prove_empty(DRAIN_TIMEOUT)?;
        self.slice.prove_empty(DRAIN_TIMEOUT)
    }
}

pub fn recover_recorded_stage(
    unit_name: &UnitName,
    expected_invocation_id: &InvocationId,
    expected_control_group: &AbsolutePath,
) -> Result<RecoveryOutcome, SystemdError> {
    let unit_name = unit_name.as_str();
    let expected_invocation_id = expected_invocation_id.as_str();
    let expected_control_group = expected_control_group.as_str();
    if !valid_unit_component(unit_name, ".service")
        || expected_invocation_id.len() != 32
        || !expected_invocation_id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        return Err(SystemdError::RecoveryRequired {
            detail: "recorded unit/invocation identity is malformed".into(),
        });
    }
    validate_control_group(expected_control_group).map_err(|error| {
        SystemdError::RecoveryRequired {
            detail: error.to_string(),
        }
    })?;
    verify_systemd_contract()?;
    let root = CgroupRoot::open()?;
    let values = show_properties(
        unit_name,
        &[
            "LoadState",
            "Id",
            "InvocationID",
            "ControlGroup",
            "ActiveState",
        ],
    )?;
    let load_state = values
        .get("LoadState")
        .map(String::as_str)
        .unwrap_or_default();
    if load_state == "not-found" {
        return match root.open_group_optional(expected_control_group)? {
            None => Ok(RecoveryOutcome::AbsentOrEmpty),
            Some(group) if group.population()? == 0 => Ok(RecoveryOutcome::AbsentOrEmpty),
            Some(group) => Err(SystemdError::RecoveryRequired {
                detail: format!(
                    "unit is absent but recorded cgroup {} is populated; refusing an identity-free kill",
                    group.control_group
                ),
            }),
        };
    }
    if values.get("Id").map(String::as_str) != Some(unit_name) {
        return Err(SystemdError::RecoveryRequired {
            detail: format!("unit Id mismatch: {values:?}"),
        });
    }
    let observed_invocation = values.get("InvocationID").cloned().unwrap_or_default();
    if observed_invocation != expected_invocation_id {
        return Err(SystemdError::RecoveryRequired {
            detail: format!(
                "unit invocation mismatch expected={expected_invocation_id} actual={observed_invocation}"
            ),
        });
    }
    let observed_control = values.get("ControlGroup").cloned().unwrap_or_default();
    if observed_control != expected_control_group {
        return Err(SystemdError::RecoveryRequired {
            detail: format!(
                "unit cgroup mismatch expected={expected_control_group} actual={observed_control}"
            ),
        });
    }
    let Some(group) = root.open_group_optional(expected_control_group)? else {
        return Ok(RecoveryOutcome::AbsentOrEmpty);
    };
    let active = values
        .get("ActiveState")
        .map(String::as_str)
        .unwrap_or_default();
    if matches!(
        active,
        "active" | "activating" | "reloading" | "deactivating"
    ) {
        group.kill_if_populated()?;
        group.prove_empty(DRAIN_TIMEOUT)?;
        return Ok(RecoveryOutcome::Killed);
    }
    if group.population()? == 0 {
        Ok(RecoveryOutcome::AbsentOrEmpty)
    } else {
        Err(SystemdError::RecoveryRequired {
            detail: format!(
                "same invocation is inactive but cgroup remains populated: state={active}"
            ),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_names_are_bounded_and_not_templates() {
        assert!(valid_unit_component("calyx-gate-a.service", ".service"));
        assert!(!valid_unit_component("../escape.service", ".service"));
        assert!(!valid_unit_component("calyx@evil.service", ".service"));
        assert!(!valid_unit_component("calyx gate.service", ".service"));
    }

    #[test]
    fn published_writable_path_is_exactly_alias_and_leaf() {
        assert!(is_exact_published_object(Path::new(
            "/var/lib/calyx-gatebrokerd/objects/tmp/abc"
        )));
        assert!(!is_exact_published_object(Path::new(
            "/var/lib/calyx-gatebrokerd/objects/tmp"
        )));
        assert!(!is_exact_published_object(Path::new(
            "/var/lib/calyx-gatebrokerd/private/quarantine/tmp/abc"
        )));
    }

    #[test]
    fn loader_and_manager_environment_is_reserved() {
        assert!(reserved_environment_name(b"LD_PRELOAD"));
        assert!(reserved_environment_name(b"DBUS_SYSTEM_BUS_ADDRESS"));
        assert!(reserved_environment_name(b"XDG_RUNTIME_DIR"));
        assert!(!reserved_environment_name(b"CALYX_TEST_MARKER"));
    }

    #[test]
    fn relative_cwd_rejects_traversal() {
        assert!(normalized_relative(Path::new(".")));
        assert!(normalized_relative(Path::new("src/bin")));
        assert!(!normalized_relative(Path::new("../src")));
        assert!(!normalized_relative(Path::new("/src")));
    }
}
