//! Real PID 1/cgroup-v2 verification for the two-phase stage lifecycle.
//!
//! Run explicitly as root on systemd 259+:
//! `CALYX_GATEBROKER_REAL_SYSTEMD=1 cargo test -p calyx-gatebrokerd \
//!   --test __calyx_integration_platform_0 systemd_real -- --ignored --nocapture`

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
    BROKER_UNIT_NAME, CapturedStage, CgroupIdentity, RecoveryOutcome, STAGE_SLICE_CONTROL_GROUP,
    STAGE_SLICE_NAME, StageSpec, SystemdError, WorkerIdentity, recover_recorded_stage,
};

#[path = "systemd_real/support.rs"]
mod support;
use support::*;

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
    let suffix = random_suffix();
    let worker = format!("calyxgt{}", &suffix[..8]);
    let mut cleanup = Cleanup {
        worker: worker.clone(),
        worker_created: false,
        shim_identity: None,
        directories: Vec::new(),
        broker_started: false,
        slice_installed: false,
        broker_unit_installed: false,
        finished: false,
    };
    install_systemd_fixture(&mut cleanup);
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
    let spec = make_spec(SpecFixture {
        suffix: &suffix,
        worker: &worker,
        worker_uid,
        source_root: &source_root,
        cwd: &cwd,
        writable: &writable,
        state_path: &state_path,
        go_path: &go_path,
    });
    let captured = CapturedStage::capture(&spec, stdout.as_raw_fd(), stderr.as_raw_fd()).unwrap();
    let evidence = captured.evidence().clone();
    println!(
        "CAPTURED unit={} invocation={} main_pid={} worker={}:{} service_cgroup={} dev={} ino={} slice_cgroup={} dev={} ino={}",
        evidence.unit_name,
        evidence.invocation_id,
        evidence.main_pid,
        evidence.worker_user,
        evidence.worker_uid,
        evidence.control_group,
        evidence.control_group_device,
        evidence.control_group_inode,
        evidence.slice_control_group,
        evidence.slice_control_group_device,
        evidence.slice_control_group_inode,
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
    println!(
        "SLICE_READBACK id={} active={} control_group={} stop_when_unneeded={} transient={}",
        systemd_value(STAGE_SLICE_NAME, "Id"),
        systemd_value(STAGE_SLICE_NAME, "ActiveState"),
        systemd_value(STAGE_SLICE_NAME, "ControlGroup"),
        systemd_value(STAGE_SLICE_NAME, "StopWhenUnneeded"),
        systemd_value(STAGE_SLICE_NAME, "Transient")
    );
    assert_eq!(systemd_value(STAGE_SLICE_NAME, "ActiveState"), "active");
    assert_eq!(
        systemd_value(STAGE_SLICE_NAME, "ControlGroup"),
        STAGE_SLICE_CONTROL_GROUP
    );
    assert_eq!(
        cgroup_population(STAGE_SLICE_CONTROL_GROUP).as_deref(),
        Some("0")
    );

    let unit = UnitName::new(evidence.unit_name).unwrap();
    let invocation = InvocationId::new(evidence.invocation_id).unwrap();
    let service_identity = CgroupIdentity {
        control_group: AbsolutePath::new(evidence.control_group).unwrap(),
        device: evidence.control_group_device,
        inode: evidence.control_group_inode,
    };
    let slice_identity = CgroupIdentity {
        control_group: AbsolutePath::new(evidence.slice_control_group).unwrap(),
        device: evidence.slice_control_group_device,
        inode: evidence.slice_control_group_inode,
    };
    let worker_identity = WorkerIdentity {
        user: evidence.worker_user,
        uid: evidence.worker_uid,
    };
    assert_eq!(
        recover_recorded_stage(
            &unit,
            &invocation,
            &service_identity,
            &slice_identity,
            &worker_identity,
        )
        .unwrap(),
        RecoveryOutcome::AbsentOrEmpty
    );
    println!("RECOVERY_READBACK outcome=absent_or_empty");

    let owner_suffix = format!("{suffix}dead");
    let owner_writable = PathBuf::from(format!(
        "/var/lib/calyx-gatebrokerd/objects/tmp/{owner_suffix}"
    ));
    fs::create_dir(&owner_writable).unwrap();
    fs::set_permissions(&owner_writable, fs::Permissions::from_mode(0o700)).unwrap();
    let owner_writable_c = CString::new(owner_writable.as_os_str().as_encoded_bytes()).unwrap();
    assert_eq!(
        unsafe { libc::chown(owner_writable_c.as_ptr(), worker_uid, worker_gid) },
        0
    );
    cleanup.directories.push(owner_writable.clone());
    let owner_state = owner_writable.join("state.json");
    let owner_go = owner_writable.join("go");
    let owner_stdout_path = source_root.join("owner-stdout.log");
    let owner_stderr_path = source_root.join("owner-stderr.log");
    let owner_stdout = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&owner_stdout_path)
        .unwrap();
    let owner_stderr = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&owner_stderr_path)
        .unwrap();
    let owner_spec = make_spec(SpecFixture {
        suffix: &owner_suffix,
        worker: &worker,
        worker_uid,
        source_root: &source_root,
        cwd: &cwd,
        writable: &owner_writable,
        state_path: &owner_state,
        go_path: &owner_go,
    });
    let owner_captured = CapturedStage::capture(
        &owner_spec,
        owner_stdout.as_raw_fd(),
        owner_stderr.as_raw_fd(),
    )
    .unwrap();
    let owner_evidence = owner_captured.evidence().clone();
    println!(
        "OWNER_EXIT_BEFORE unit={} main_pid={} state_exists={} service_population={:?} slice_population={:?}",
        owner_evidence.unit_name,
        owner_evidence.main_pid,
        owner_state.exists(),
        cgroup_population(&owner_evidence.control_group),
        cgroup_population(&owner_evidence.slice_control_group)
    );
    assert!(!owner_state.exists());
    assert!(Path::new(&format!("/proc/{}", owner_evidence.main_pid)).is_dir());
    let exited_owner = exited_pidfd();
    let owner_error = owner_captured
        .release(exited_owner.as_raw_fd())
        .unwrap_err();
    println!("OWNER_EXIT_ACTION error={owner_error}");
    assert!(owner_error.cleanup_proved_drained());
    assert!(matches!(
        &owner_error,
        SystemdError::Cleanup { primary, .. } if primary == "owner exited before stage release"
    ));
    let owner_deadline = Instant::now() + Duration::from_secs(10);
    while systemd_value(&owner_evidence.unit_name, "LoadState") != "not-found" {
        assert!(
            Instant::now() < owner_deadline,
            "owner-exit unit remained published"
        );
        thread::sleep(Duration::from_millis(10));
    }
    drop(owner_stdout);
    drop(owner_stderr);
    assert!(!owner_state.exists());
    assert!(!owner_go.exists());
    assert!(!Path::new(&format!("/proc/{}", owner_evidence.main_pid)).exists());
    assert!(matches!(
        cgroup_population(&owner_evidence.control_group).as_deref(),
        None | Some("0")
    ));
    assert_eq!(
        cgroup_population(STAGE_SLICE_CONTROL_GROUP).as_deref(),
        Some("0")
    );
    assert_eq!(fs::metadata(&owner_stdout_path).unwrap().len(), 0);
    assert_eq!(fs::metadata(&owner_stderr_path).unwrap().len(), 0);
    println!(
        "OWNER_EXIT_AFTER unit=not-found pid=absent state=absent stdout_bytes=0 stderr_bytes=0 service_population={:?} slice_population={:?}",
        cgroup_population(&owner_evidence.control_group),
        cgroup_population(STAGE_SLICE_CONTROL_GROUP)
    );

    fs::remove_file(&state_path).unwrap();
    fs::remove_file(&go_path).unwrap();
    fs::remove_file(&stdout_path).unwrap();
    fs::remove_file(&stderr_path).unwrap();
    fs::remove_file(&owner_stdout_path).unwrap();
    fs::remove_file(&owner_stderr_path).unwrap();
    cleanup.verify_and_finish();
}
