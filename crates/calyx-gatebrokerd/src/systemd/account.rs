use super::cgroup::validate_control_group;
use super::manager::{show_properties, verify_systemd_contract};
use super::validation::io_error;
use super::*;

pub(super) fn lookup_worker(user: &str) -> Result<WorkerAccount, SystemdError> {
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

pub(super) fn verify_worker_manager_absent(
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

pub(super) fn worker_processes(uid: u32) -> Result<Vec<u32>, SystemdError> {
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
                values.len() == 4 && values.contains(&uid)
            });
        if matches_uid {
            pids.push(pid);
        }
    }
    pids.sort_unstable();
    Ok(pids)
}

pub(super) fn proc_control_group(pid: u32) -> Result<Option<String>, SystemdError> {
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

pub(super) fn cgroup_contains(boundary: &str, candidate: &str) -> bool {
    candidate == boundary
        || candidate
            .strip_prefix(boundary)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

pub(super) fn worker_process_locations(uid: u32) -> Result<Vec<(u32, String)>, SystemdError> {
    let mut locations = Vec::new();
    for pid in worker_processes(uid)? {
        if let Some(control_group) = proc_control_group(pid)? {
            locations.push((pid, control_group));
        }
    }
    locations.sort_unstable();
    Ok(locations)
}

pub(super) fn processes_in_cgroup(boundary: &str) -> Result<Vec<u32>, SystemdError> {
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

pub(super) fn proc_uids(pid: u32) -> Result<Option<Vec<u32>>, SystemdError> {
    let status = match fs::read_to_string(format!("/proc/{pid}/status")) {
        Ok(value) => value,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(io_error("read process status", error)),
    };
    let values: Vec<u32> = status
        .lines()
        .find_map(|line| line.strip_prefix("Uid:"))
        .into_iter()
        .flat_map(str::split_whitespace)
        .filter_map(|value| value.parse().ok())
        .collect();
    if values.len() != 4 {
        return Err(SystemdError::ProcessIdentity {
            pid,
            detail: format!("malformed Uid status row: {values:?}"),
        });
    }
    Ok(Some(values))
}

pub(super) fn require_no_worker_process(account: &WorkerAccount) -> Result<(), SystemdError> {
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

pub(super) fn verify_worker_account_identity(
    worker_user: &str,
    expected_uid: u32,
) -> Result<WorkerAccount, SystemdError> {
    let account = lookup_worker(worker_user)?;
    if account.uid != expected_uid {
        return Err(SystemdError::WorkerPolicy {
            user: worker_user.into(),
            detail: format!(
                "numeric identity changed: expected uid={expected_uid}, resolved uid={}",
                account.uid
            ),
        });
    }
    Ok(account)
}

pub(super) fn verify_worker_idle_account(
    worker_user: &str,
    expected_uid: u32,
) -> Result<WorkerAccount, SystemdError> {
    let account = verify_worker_account_identity(worker_user, expected_uid)?;
    verify_worker_manager_absent(&account, worker_user)?;
    require_no_worker_process(&account)?;
    Ok(account)
}

pub fn verify_worker_idle(worker_user: &str, worker_uid: u32) -> Result<(), SystemdError> {
    let real_uid = unsafe { libc::getuid() };
    let effective_uid = unsafe { libc::geteuid() };
    if real_uid != 0 || effective_uid != 0 {
        return Err(SystemdError::BrokerIdentity {
            real_uid,
            effective_uid,
        });
    }
    verify_systemd_contract()?;
    verify_worker_idle_account(worker_user, worker_uid).map(|_| ())
}
