//! Real PID 1/cgroup-v2 verification for the two-phase stage lifecycle.
//!
//! Run explicitly as root on systemd 259+:
//! `CALYX_GATEBROKER_REAL_SYSTEMD=1 cargo test -p calyx-gatebrokerd \
//!   --test systemd_real -- --ignored --nocapture`

#![cfg(target_os = "linux")]

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{CString, OsString};
use std::fs::{self, File, OpenOptions};
use std::io::Read;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use calyx_gatebrokerd::protocol::{AbsolutePath, InvocationId, UnitName};
use calyx_gatebrokerd::systemd::{
    CapturedStage, RecoveryOutcome, StageSpec, recover_recorded_stage,
};

const SHIM_INSTALL: &str = "/usr/libexec/calyx-gate-stage-shim";

struct Cleanup {
    worker: String,
    worker_created: bool,
    shim_identity: Option<(u64, u64)>,
    directories: Vec<PathBuf>,
}

impl Drop for Cleanup {
    fn drop(&mut self) {
        if self.worker_created {
            let _ = Command::new("/usr/sbin/userdel")
                .arg(&self.worker)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
            let _ = Command::new("/usr/sbin/groupdel")
                .arg(&self.worker)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
        if let Some((device, inode)) = self.shim_identity {
            if fs::metadata(SHIM_INSTALL)
                .is_ok_and(|metadata| metadata.dev() == device && metadata.ino() == inode)
            {
                let _ = fs::remove_file(SHIM_INSTALL);
            }
        }
        for directory in self.directories.iter().rev() {
            let _ = fs::remove_dir(directory);
        }
    }
}

fn command_success(path: &str, args: &[&str]) {
    let output = Command::new(path).args(args).output().unwrap();
    assert!(
        output.status.success(),
        "{path} {args:?}: status={} stdout={} stderr={}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn random_suffix() -> String {
    fs::read_to_string("/proc/sys/kernel/random/uuid")
        .unwrap()
        .bytes()
        .filter(|byte| byte.is_ascii_hexdigit())
        .take(12)
        .map(char::from)
        .collect()
}

fn ensure_root_directory(path: &Path, mode: u32, cleanup: &mut Cleanup) {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            assert!(metadata.is_dir() && !metadata.file_type().is_symlink());
            assert_eq!((metadata.uid(), metadata.gid()), (0, 0));
            assert_eq!(metadata.mode() & 0o7777, mode);
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir(path).unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
            let c_path = CString::new(path.as_os_str().as_encoded_bytes()).unwrap();
            assert_eq!(unsafe { libc::chown(c_path.as_ptr(), 0, 0) }, 0);
            cleanup.directories.push(path.to_owned());
        }
        Err(error) => panic!("inspect {}: {error}", path.display()),
    }
}

fn install_worker(worker: &str, cleanup: &mut Cleanup) -> (u32, u32) {
    assert!(
        !Command::new("/usr/bin/getent")
            .args(["passwd", worker])
            .status()
            .unwrap()
            .success()
    );
    command_success(
        "/usr/sbin/useradd",
        &[
            "--system",
            "--user-group",
            "--no-create-home",
            "--home-dir",
            "/nonexistent",
            "--shell",
            "/usr/sbin/nologin",
            worker,
        ],
    );
    cleanup.worker_created = true;
    let output = Command::new("/usr/bin/id")
        .args(["-u", worker])
        .output()
        .unwrap();
    let uid = String::from_utf8(output.stdout)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    let output = Command::new("/usr/bin/id")
        .args(["-g", worker])
        .output()
        .unwrap();
    let gid = String::from_utf8(output.stdout)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    (uid, gid)
}

fn install_shim(cleanup: &mut Cleanup) {
    assert!(
        !Path::new(SHIM_INSTALL).exists(),
        "refusing to replace an existing production shim"
    );
    let built = Path::new(env!("CARGO_BIN_EXE_calyx-gate-stage-shim"));
    fs::copy(built, SHIM_INSTALL).unwrap();
    fs::set_permissions(SHIM_INSTALL, fs::Permissions::from_mode(0o755)).unwrap();
    let path = CString::new(SHIM_INSTALL).unwrap();
    assert_eq!(unsafe { libc::chown(path.as_ptr(), 0, 0) }, 0);
    File::open(SHIM_INSTALL).unwrap().sync_all().unwrap();
    let metadata = fs::metadata(SHIM_INSTALL).unwrap();
    assert_eq!(
        (metadata.uid(), metadata.gid(), metadata.mode() & 0o7777),
        (0, 0, 0o755)
    );
    assert_eq!(metadata.nlink(), 1);
    cleanup.shim_identity = Some((metadata.dev(), metadata.ino()));
}

fn open_directory(path: &Path) -> OwnedFd {
    let path = CString::new(path.as_os_str().as_encoded_bytes()).unwrap();
    let fd = unsafe {
        libc::open(
            path.as_ptr(),
            libc::O_PATH | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    };
    assert!(
        fd >= 0,
        "open directory: {}",
        std::io::Error::last_os_error()
    );
    unsafe { OwnedFd::from_raw_fd(fd) }
}

fn current_pidfd() -> OwnedFd {
    let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, std::process::id(), 0_u32) } as i32;
    assert!(fd >= 0, "pidfd_open: {}", std::io::Error::last_os_error());
    unsafe { OwnedFd::from_raw_fd(fd) }
}

fn wait_for_file(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !path.is_file() {
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {}",
            path.display()
        );
        thread::sleep(Duration::from_millis(10));
    }
}

fn cgroup_population(control_group: &str) -> Option<String> {
    let path = Path::new("/sys/fs/cgroup")
        .join(control_group.trim_start_matches('/'))
        .join("cgroup.events");
    fs::read_to_string(path).ok().and_then(|text| {
        text.lines()
            .find_map(|line| line.strip_prefix("populated ").map(str::to_owned))
    })
}

fn make_spec(
    suffix: &str,
    worker: &str,
    source_root: &Path,
    cwd: &OwnedFd,
    writable: &Path,
    state_path: &Path,
    go_path: &Path,
) -> StageSpec {
    let script = r#"
import json, os, subprocess, time
def rc(argv):
    try:
        return subprocess.run(argv, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                              stderr=subprocess.DEVNULL, timeout=2, check=False).returncode
    except subprocess.TimeoutExpired:
        return 124
state = {
    "cwd": os.getcwd(),
    "cwd_dev": os.stat(".").st_dev,
    "cwd_ino": os.stat(".").st_ino,
    "stdin_eof": os.read(0, 1) == b"",
    "environment": dict(os.environ),
    "system_manager_rc": rc(["/usr/bin/systemctl", "--system", "is-system-running"]),
    "user_manager_rc": rc(["/usr/bin/systemd-run", "--user", "/usr/bin/true"]),
}
with open(os.environ["CALYX_TEST_STATE"], "w", encoding="utf-8") as stream:
    json.dump(state, stream, sort_keys=True)
while not os.path.exists(os.environ["CALYX_TEST_GO"]):
    time.sleep(0.01)
"#;
    StageSpec {
        unit_name: format!("calyx-gate-real-{suffix}.service"),
        slice_name: format!("calyx-gate-real-{suffix}.slice"),
        broker_unit: "systemd-journald.service".into(),
        worker_user: worker.into(),
        execution_root: source_root.to_owned(),
        relative_cwd: PathBuf::from("work"),
        execution_root_uid: 0,
        execution_root_mode: 0o755,
        cwd_fd: cwd.as_raw_fd(),
        argv: vec![
            OsString::from("/usr/bin/python3"),
            OsString::from("-c"),
            OsString::from(script),
        ],
        environment: vec![
            (
                OsString::from("CALYX_TEST_MARKER"),
                OsString::from("known-value"),
            ),
            (
                OsString::from("CALYX_TEST_STATE"),
                state_path.as_os_str().to_owned(),
            ),
            (
                OsString::from("CALYX_TEST_GO"),
                go_path.as_os_str().to_owned(),
            ),
        ],
        writable_paths: vec![writable.to_owned()],
    }
}

#[test]
#[ignore = "requires root, systemd 259+, cgroup v2, and temporary host user/files"]
fn real_two_phase_stage_has_exact_process_and_cgroup_state() {
    assert_eq!(
        std::env::var("CALYX_GATEBROKER_REAL_SYSTEMD").as_deref(),
        Ok("1")
    );
    assert_eq!(
        (unsafe { libc::getuid() }, unsafe { libc::geteuid() }),
        (0, 0)
    );
    command_success(
        "/usr/bin/systemctl",
        &["is-active", "systemd-journald.service"],
    );

    let suffix = random_suffix();
    let worker = format!("calyxgt{}", &suffix[..8]);
    let mut cleanup = Cleanup {
        worker: worker.clone(),
        worker_created: false,
        shim_identity: None,
        directories: Vec::new(),
    };
    let (worker_uid, worker_gid) = install_worker(&worker, &mut cleanup);
    install_shim(&mut cleanup);

    for (path, mode) in [
        (Path::new("/var/lib/calyx-gatebrokerd"), 0o711),
        (Path::new("/var/lib/calyx-gatebrokerd/objects"), 0o711),
        (Path::new("/var/lib/calyx-gatebrokerd/objects/tmp"), 0o711),
        (Path::new("/var/lib/calyx-gatebrokerd/private"), 0o700),
    ] {
        ensure_root_directory(path, mode, &mut cleanup);
    }
    let source_base = PathBuf::from("/var/lib/calyx-gatebrokerd-real-source");
    ensure_root_directory(&source_base, 0o755, &mut cleanup);
    let source_root = source_base.join(&suffix);
    ensure_root_directory(&source_root, 0o755, &mut cleanup);
    let work = source_root.join("work");
    ensure_root_directory(&work, 0o755, &mut cleanup);
    let cwd = open_directory(&work);
    let cwd_metadata = fs::metadata(&work).unwrap();

    let writable = PathBuf::from(format!("/var/lib/calyx-gatebrokerd/objects/tmp/{suffix}"));
    fs::create_dir(&writable).unwrap();
    fs::set_permissions(&writable, fs::Permissions::from_mode(0o700)).unwrap();
    let writable_c = CString::new(writable.as_os_str().as_encoded_bytes()).unwrap();
    assert_eq!(
        unsafe { libc::chown(writable_c.as_ptr(), worker_uid, worker_gid) },
        0
    );
    cleanup.directories.push(writable.clone());

    let state_path = writable.join("state.json");
    let go_path = writable.join("go");
    let stdout_path = source_root.join("stdout.log");
    let stderr_path = source_root.join("stderr.log");
    let stdout = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&stdout_path)
        .unwrap();
    let stderr = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&stderr_path)
        .unwrap();
    let spec = make_spec(
        &suffix,
        &worker,
        &source_root,
        &cwd,
        &writable,
        &state_path,
        &go_path,
    );
    let captured = CapturedStage::capture(&spec, stdout.as_raw_fd(), stderr.as_raw_fd()).unwrap();
    let evidence = captured.evidence().clone();
    println!(
        "CAPTURED unit={} invocation={} main_pid={} service_cgroup={} slice_cgroup={}",
        evidence.unit_name,
        evidence.invocation_id,
        evidence.main_pid,
        evidence.control_group,
        evidence.slice_control_group
    );
    assert!(
        !state_path.exists(),
        "payload ran before journal/release boundary"
    );
    assert!(Path::new(&format!("/proc/{}", evidence.main_pid)).is_dir());
    assert_eq!(
        cgroup_population(&evidence.control_group).as_deref(),
        Some("1")
    );
    assert_eq!(
        cgroup_population(&evidence.slice_control_group).as_deref(),
        Some("1")
    );

    let owner = current_pidfd();
    let running = captured.release(owner.as_raw_fd()).unwrap();
    wait_for_file(&state_path);
    let mut bytes = Vec::new();
    File::open(&state_path)
        .unwrap()
        .read_to_end(&mut bytes)
        .unwrap();
    let state: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    println!("RUNNING_STATE {}", String::from_utf8_lossy(&bytes));
    assert_eq!(state["cwd"], work.to_str().unwrap());
    assert_eq!(state["cwd_dev"].as_u64(), Some(cwd_metadata.dev()));
    assert_eq!(state["cwd_ino"].as_u64(), Some(cwd_metadata.ino()));
    assert_eq!(state["stdin_eof"], true);
    assert_ne!(state["system_manager_rc"], 0);
    assert_ne!(state["user_manager_rc"], 0);
    let environment: BTreeMap<String, String> =
        serde_json::from_value(state["environment"].clone()).unwrap();
    let expected_names: BTreeSet<String> = [
        "PATH",
        "HOME",
        "USER",
        "LOGNAME",
        "SHELL",
        "LANG",
        "LC_ALL",
        "INVOCATION_ID",
        "CALYX_TEST_MARKER",
        "CALYX_TEST_STATE",
        "CALYX_TEST_GO",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    assert_eq!(
        environment.keys().cloned().collect::<BTreeSet<_>>(),
        expected_names
    );
    assert_eq!(environment["CALYX_TEST_MARKER"], "known-value");
    assert_eq!(environment["INVOCATION_ID"], evidence.invocation_id);

    File::create(&go_path).unwrap().sync_all().unwrap();
    let result = running.wait().unwrap();
    println!(
        "FINISHED status={} main_pid={}",
        result.exit_status, result.evidence.main_pid
    );
    assert_eq!(result.exit_status, 0);
    assert!(!Path::new(&format!("/proc/{}", evidence.main_pid)).exists());
    assert!(matches!(
        cgroup_population(&evidence.control_group).as_deref(),
        None | Some("0")
    ));
    assert!(matches!(
        cgroup_population(&evidence.slice_control_group).as_deref(),
        None | Some("0")
    ));

    let unit = UnitName::new(evidence.unit_name).unwrap();
    let invocation = InvocationId::new(evidence.invocation_id).unwrap();
    let control_group = AbsolutePath::new(evidence.control_group).unwrap();
    assert_eq!(
        recover_recorded_stage(&unit, &invocation, &control_group).unwrap(),
        RecoveryOutcome::AbsentOrEmpty
    );
    println!("RECOVERY_READBACK outcome=absent_or_empty");

    fs::remove_file(&state_path).unwrap();
    fs::remove_file(&go_path).unwrap();
    fs::remove_file(&stdout_path).unwrap();
    fs::remove_file(&stderr_path).unwrap();
}
