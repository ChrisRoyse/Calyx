use super::cgroup::validate_control_group;
use super::manager::{property_has_unit, show_properties, verify_fixed_slice_contract};
use super::validation::fstat;
use super::*;

pub(super) fn parse_unit_evidence(
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

pub(super) fn read_proc_bytes(pid: u32, name: &str) -> Result<Vec<u8>, SystemdError> {
    fs::read(format!("/proc/{pid}/{name}")).map_err(|error| SystemdError::ProcessIdentity {
        pid,
        detail: format!("read {name}: {error}"),
    })
}

pub(super) fn verify_process_identity(
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
