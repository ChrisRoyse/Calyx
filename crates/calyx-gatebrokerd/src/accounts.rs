//! Linux account and exact process-ancestry checks used at the RPC boundary.

use std::ffi::{CStr, CString};
use std::fs;
use std::io;

use thiserror::Error;

use crate::transport::PeerCredentials;

const MAX_ANCESTRY_DEPTH: usize = 128;

#[derive(Debug, Clone)]
pub struct Account {
    pub name: String,
    pub uid: u32,
    pub gid: u32,
    pub home: String,
    pub shell: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcessIdentity {
    pub pid: u32,
    pub parent_pid: u32,
    pub uid: u32,
    pub gid: u32,
    pub starttime: u64,
}

#[derive(Debug, Error)]
pub enum AccountError {
    #[error("account {0:?} does not exist")]
    UnknownAccount(String),
    #[error("group {0:?} does not exist")]
    UnknownGroup(String),
    #[error("account lookup failed: {0}")]
    Lookup(String),
    #[error("cannot read process identity for pid {pid}: {detail}")]
    Process { pid: u32, detail: String },
    #[error(
        "peer credential mismatch for pid {pid}: socket uid/gid={peer_uid}/{peer_gid}, proc uid/gid={proc_uid}/{proc_gid}"
    )]
    PeerMismatch {
        pid: u32,
        peer_uid: u32,
        peer_gid: u32,
        proc_uid: u32,
        proc_gid: u32,
    },
    #[error("peer pid {pid} uid {uid} is not in required client group gid {required_gid}")]
    ClientGroup {
        pid: u32,
        uid: u32,
        required_gid: u32,
    },
    #[error(
        "peer pid {peer_pid} is not a live descendant of owner pid {owner_pid} starttime {owner_starttime}"
    )]
    NotDescendant {
        peer_pid: u32,
        owner_pid: u32,
        owner_starttime: u64,
    },
}

pub fn lookup_account(name: &str) -> Result<Account, AccountError> {
    let name_c =
        CString::new(name).map_err(|_| AccountError::Lookup("account name contains NUL".into()))?;
    let mut record = unsafe { std::mem::zeroed::<libc::passwd>() };
    let mut result = std::ptr::null_mut();
    let configured = unsafe { libc::sysconf(libc::_SC_GETPW_R_SIZE_MAX) };
    let mut buffer = vec![
        0_u8;
        if configured > 0 {
            configured as usize
        } else {
            16_384
        }
    ];
    let status = unsafe {
        libc::getpwnam_r(
            name_c.as_ptr(),
            &mut record,
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            &mut result,
        )
    };
    if status != 0 {
        return Err(AccountError::Lookup(
            io::Error::from_raw_os_error(status).to_string(),
        ));
    }
    if result.is_null() {
        return Err(AccountError::UnknownAccount(name.into()));
    }
    Ok(Account {
        name: c_string(record.pw_name, "pw_name")?,
        uid: record.pw_uid,
        gid: record.pw_gid,
        home: c_string(record.pw_dir, "pw_dir")?,
        shell: c_string(record.pw_shell, "pw_shell")?,
    })
}

pub fn lookup_group_gid(name: &str) -> Result<u32, AccountError> {
    let name_c =
        CString::new(name).map_err(|_| AccountError::Lookup("group name contains NUL".into()))?;
    let mut record = unsafe { std::mem::zeroed::<libc::group>() };
    let mut result = std::ptr::null_mut();
    let configured = unsafe { libc::sysconf(libc::_SC_GETGR_R_SIZE_MAX) };
    let mut buffer = vec![
        0_u8;
        if configured > 0 {
            configured as usize
        } else {
            16_384
        }
    ];
    let status = unsafe {
        libc::getgrnam_r(
            name_c.as_ptr(),
            &mut record,
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            &mut result,
        )
    };
    if status != 0 {
        return Err(AccountError::Lookup(
            io::Error::from_raw_os_error(status).to_string(),
        ));
    }
    if result.is_null() {
        return Err(AccountError::UnknownGroup(name.into()));
    }
    Ok(record.gr_gid)
}

pub fn authorize_client(
    peer: PeerCredentials,
    required_gid: u32,
    forbidden_uid: u32,
) -> Result<ProcessIdentity, AccountError> {
    let identity = process_identity(peer.pid)?;
    if identity.uid != peer.uid || identity.gid != peer.gid {
        return Err(AccountError::PeerMismatch {
            pid: peer.pid,
            peer_uid: peer.uid,
            peer_gid: peer.gid,
            proc_uid: identity.uid,
            proc_gid: identity.gid,
        });
    }
    if peer.uid == forbidden_uid {
        return Err(AccountError::ClientGroup {
            pid: peer.pid,
            uid: peer.uid,
            required_gid,
        });
    }
    if peer.uid != 0 {
        let groups = process_groups(peer.pid)?;
        if identity.gid != required_gid && !groups.contains(&required_gid) {
            return Err(AccountError::ClientGroup {
                pid: peer.pid,
                uid: peer.uid,
                required_gid,
            });
        }
    }
    Ok(identity)
}

pub fn require_owner_ancestry(
    peer_pid: u32,
    expected_uid: u32,
    owner_pid: u32,
    owner_starttime: u64,
) -> Result<(), AccountError> {
    let mut current = peer_pid;
    for _ in 0..MAX_ANCESTRY_DEPTH {
        let identity = process_identity(current)?;
        if identity.uid != expected_uid {
            break;
        }
        if identity.pid == owner_pid && identity.starttime == owner_starttime {
            return Ok(());
        }
        if identity.parent_pid == 0 || identity.parent_pid == identity.pid {
            break;
        }
        current = identity.parent_pid;
    }
    Err(AccountError::NotDescendant {
        peer_pid,
        owner_pid,
        owner_starttime,
    })
}

pub fn process_identity(pid: u32) -> Result<ProcessIdentity, AccountError> {
    if pid == 0 || pid > i32::MAX as u32 {
        return Err(AccountError::Process {
            pid,
            detail: "pid is outside the Linux pid range".into(),
        });
    }
    let raw = fs::read(format!("/proc/{pid}/stat")).map_err(|error| AccountError::Process {
        pid,
        detail: format!("read stat: {error}"),
    })?;
    let end = raw
        .windows(2)
        .rposition(|window| window == b") ")
        .ok_or_else(|| AccountError::Process {
            pid,
            detail: "malformed stat comm field".into(),
        })?;
    let fields: Vec<&[u8]> = raw[end + 2..].split(|byte| *byte == b' ').collect();
    let parent_pid = parse_field(&fields, 1, pid, "ppid")?;
    let starttime = parse_field(&fields, 19, pid, "starttime")?;
    let status = fs::read_to_string(format!("/proc/{pid}/status")).map_err(|error| {
        AccountError::Process {
            pid,
            detail: format!("read status: {error}"),
        }
    })?;
    let uid = status_id(&status, "Uid:", pid)?;
    let gid = status_id(&status, "Gid:", pid)?;
    Ok(ProcessIdentity {
        pid,
        parent_pid: u32::try_from(parent_pid).map_err(|_| AccountError::Process {
            pid,
            detail: "ppid exceeds u32".into(),
        })?,
        uid,
        gid,
        starttime,
    })
}

fn process_groups(pid: u32) -> Result<Vec<u32>, AccountError> {
    let status = fs::read_to_string(format!("/proc/{pid}/status")).map_err(|error| {
        AccountError::Process {
            pid,
            detail: format!("read groups: {error}"),
        }
    })?;
    let line = status
        .lines()
        .find_map(|line| line.strip_prefix("Groups:"))
        .ok_or_else(|| AccountError::Process {
            pid,
            detail: "status has no Groups field".into(),
        })?;
    line.split_whitespace()
        .map(|value| {
            value.parse::<u32>().map_err(|error| AccountError::Process {
                pid,
                detail: format!("invalid supplementary group {value:?}: {error}"),
            })
        })
        .collect()
}

fn parse_field(fields: &[&[u8]], index: usize, pid: u32, name: &str) -> Result<u64, AccountError> {
    fields
        .get(index)
        .and_then(|value| std::str::from_utf8(value).ok())
        .and_then(|value| value.parse::<u64>().ok())
        .ok_or_else(|| AccountError::Process {
            pid,
            detail: format!("missing or invalid stat {name}"),
        })
}

fn status_id(status: &str, prefix: &str, pid: u32) -> Result<u32, AccountError> {
    let values = status
        .lines()
        .find_map(|line| line.strip_prefix(prefix))
        .ok_or_else(|| AccountError::Process {
            pid,
            detail: format!("status has no valid {prefix} field"),
        })?
        .split_whitespace()
        .map(str::parse::<u32>)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| AccountError::Process {
            pid,
            detail: format!("status has invalid {prefix} field: {error}"),
        })?;
    let first = *values.first().ok_or_else(|| AccountError::Process {
        pid,
        detail: format!("status has empty {prefix} field"),
    })?;
    if values.len() != 4 || values.iter().any(|value| *value != first) {
        return Err(AccountError::Process {
            pid,
            detail: format!("real/effective/saved/fs {prefix} values are not identical"),
        });
    }
    Ok(first)
}

fn c_string(pointer: *const libc::c_char, field: &str) -> Result<String, AccountError> {
    if pointer.is_null() {
        return Err(AccountError::Lookup(format!("{field} is null")));
    }
    unsafe { CStr::from_ptr(pointer) }
        .to_str()
        .map(str::to_owned)
        .map_err(|error| AccountError::Lookup(format!("{field} is not UTF-8: {error}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_process_is_its_own_exact_owner() {
        let pid = std::process::id();
        let identity = process_identity(pid).unwrap();
        require_owner_ancestry(pid, identity.uid, pid, identity.starttime).unwrap();
        assert!(require_owner_ancestry(pid, identity.uid, pid, identity.starttime + 1).is_err());
    }

    #[test]
    fn real_account_and_primary_group_are_resolved() {
        let uid = unsafe { libc::geteuid() };
        let pointer = unsafe { libc::getpwuid(uid) };
        assert!(!pointer.is_null());
        let name = unsafe { CStr::from_ptr((*pointer).pw_name) }
            .to_str()
            .unwrap();
        let account = lookup_account(name).unwrap();
        assert_eq!(account.uid, uid);
    }
}
