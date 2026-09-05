//! Exact controller lifetime binding through Linux pidfds.

#![cfg(target_os = "linux")]

use std::fs;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::time::{Duration, Instant};

use thiserror::Error;

#[derive(Debug)]
pub struct OwnerLease {
    pid: u32,
    uid: u32,
    starttime: u64,
    pidfd: OwnedFd,
}

#[derive(Debug, Error)]
pub enum OwnerError {
    #[error("owner pid {pid} is invalid")]
    InvalidPid { pid: u32 },
    #[error("pidfd_open failed for owner pid {pid}: {source}")]
    Pidfd {
        pid: u32,
        #[source]
        source: io::Error,
    },
    #[error("cannot read owner identity for pid {pid}: {detail}")]
    Identity { pid: u32, detail: String },
    #[error(
        "owner identity mismatch for pid {pid}: expected uid={expected_uid} starttime={expected_starttime}; actual uid={actual_uid} starttime={actual_starttime}"
    )]
    Mismatch {
        pid: u32,
        expected_uid: u32,
        expected_starttime: u64,
        actual_uid: u32,
        actual_starttime: u64,
    },
    #[error("poll failed for owner pid {pid}: {source}")]
    Poll {
        pid: u32,
        #[source]
        source: io::Error,
    },
    #[error("pidfd poll returned unexpected events for owner pid {pid}: revents=0x{revents:x}")]
    UnexpectedPollEvents { pid: u32, revents: i16 },
}

impl OwnerLease {
    pub fn open(pid: u32, expected_uid: u32, expected_starttime: u64) -> Result<Self, OwnerError> {
        if pid == 0 || pid > i32::MAX as u32 || expected_starttime == 0 {
            return Err(OwnerError::InvalidPid { pid });
        }
        let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid as libc::pid_t, 0u32) };
        if fd < 0 {
            return Err(OwnerError::Pidfd {
                pid,
                source: io::Error::last_os_error(),
            });
        }
        let pidfd = unsafe { OwnedFd::from_raw_fd(fd as i32) };
        let (uid, starttime) = read_identity(pid)?;
        if uid != expected_uid || starttime != expected_starttime {
            return Err(OwnerError::Mismatch {
                pid,
                expected_uid,
                expected_starttime,
                actual_uid: uid,
                actual_starttime: starttime,
            });
        }
        let lease = Self {
            pid,
            uid,
            starttime,
            pidfd,
        };
        if lease.has_exited()? {
            return Err(OwnerError::Identity {
                pid,
                detail: "owner exited during identity binding".into(),
            });
        }
        Ok(lease)
    }

    pub fn pid(&self) -> u32 {
        self.pid
    }

    pub fn uid(&self) -> u32 {
        self.uid
    }

    pub fn starttime(&self) -> u64 {
        self.starttime
    }

    pub fn raw_fd(&self) -> std::os::fd::RawFd {
        self.pidfd.as_raw_fd()
    }

    pub fn has_exited(&self) -> Result<bool, OwnerError> {
        self.poll(Some(Duration::ZERO))
    }

    pub fn wait_for_exit(&self) -> Result<(), OwnerError> {
        while !self.poll(None)? {}
        Ok(())
    }

    pub fn wait_timeout(&self, timeout: Duration) -> Result<bool, OwnerError> {
        self.poll(Some(timeout))
    }

    fn poll(&self, timeout: Option<Duration>) -> Result<bool, OwnerError> {
        let started = timeout.map(|_| Instant::now());
        let mut attempted = false;
        let mut descriptor = libc::pollfd {
            fd: self.pidfd.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        loop {
            let (timeout_ms, final_chunk) = match (timeout, started) {
                (None, _) => (-1, false),
                (Some(limit), Some(started)) => {
                    let elapsed = started.elapsed();
                    // A zero timeout still performs one poll so has_exited()
                    // observes an owner that was already ready.
                    if attempted && elapsed >= limit {
                        return Ok(false);
                    }
                    poll_timeout(limit.saturating_sub(elapsed))
                }
                (Some(_), None) => unreachable!("finite timeout always records a start"),
            };
            attempted = true;
            descriptor.revents = 0;
            let result = unsafe { libc::poll(&mut descriptor, 1, timeout_ms) };
            if result > 0 {
                let revents = descriptor.revents;
                let unknown = revents & !(libc::POLLIN | libc::POLLHUP);
                if revents & libc::POLLIN == 0 || unknown != 0 {
                    return Err(OwnerError::UnexpectedPollEvents {
                        pid: self.pid,
                        revents,
                    });
                }
                return Ok(true);
            }
            if result == 0 {
                if final_chunk {
                    return Ok(false);
                }
                // poll(2) is capped at i32::MAX milliseconds. For a larger
                // requested Duration, wait another bounded chunk while
                // retaining the original monotonic deadline.
                continue;
            }
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted {
                // Recompute against the original monotonic start. Reusing the
                // initial timeout here would let repeated signals extend an
                // owner-death deadline indefinitely.
                continue;
            }
            return Err(OwnerError::Poll {
                pid: self.pid,
                source: error,
            });
        }
    }
}

fn poll_timeout(remaining: Duration) -> (i32, bool) {
    let maximum = Duration::from_millis(i32::MAX as u64);
    if remaining > maximum {
        return (i32::MAX, false);
    }
    let mut milliseconds = remaining.as_millis();
    if !remaining.subsec_nanos().is_multiple_of(1_000_000) {
        milliseconds += 1;
    }
    (milliseconds as i32, true)
}

pub fn process_starttime(pid: u32) -> Result<u64, OwnerError> {
    read_identity(pid).map(|(_, starttime)| starttime)
}

fn read_identity(pid: u32) -> Result<(u32, u64), OwnerError> {
    let stat = fs::read(format!("/proc/{pid}/stat")).map_err(|error| OwnerError::Identity {
        pid,
        detail: format!("read stat: {error}"),
    })?;
    let end = stat
        .windows(2)
        .rposition(|window| window == b") ")
        .ok_or_else(|| OwnerError::Identity {
            pid,
            detail: "malformed stat comm field".into(),
        })?;
    let fields: Vec<&[u8]> = stat[end + 2..].split(|byte| *byte == b' ').collect();
    // Field 22 is starttime. The split tail starts at field 3, so index 19.
    let starttime = fields
        .get(19)
        .and_then(|raw| std::str::from_utf8(raw).ok())
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .filter(|value| *value != 0)
        .ok_or_else(|| OwnerError::Identity {
            pid,
            detail: "missing or invalid stat starttime".into(),
        })?;
    let status = fs::read_to_string(format!("/proc/{pid}/status")).map_err(|error| {
        OwnerError::Identity {
            pid,
            detail: format!("read status: {error}"),
        }
    })?;
    let uid = status
        .lines()
        .find_map(|line| line.strip_prefix("Uid:"))
        .and_then(|tail| tail.split_whitespace().next())
        .and_then(|raw| raw.parse::<u32>().ok())
        .ok_or_else(|| OwnerError::Identity {
            pid,
            detail: "missing real uid".into(),
        })?;
    Ok((uid, starttime))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::fd::RawFd;

    fn pipe() -> (OwnedFd, OwnedFd) {
        let mut descriptors: [RawFd; 2] = [-1; 2];
        assert_eq!(
            unsafe { libc::pipe2(descriptors.as_mut_ptr(), libc::O_CLOEXEC) },
            0,
            "pipe2 failed: {}",
            io::Error::last_os_error()
        );
        unsafe {
            (
                OwnedFd::from_raw_fd(descriptors[0]),
                OwnedFd::from_raw_fd(descriptors[1]),
            )
        }
    }

    fn test_lease(pidfd: OwnedFd) -> OwnerLease {
        OwnerLease {
            pid: std::process::id(),
            uid: unsafe { libc::getuid() },
            starttime: 1,
            pidfd,
        }
    }

    #[test]
    fn binds_current_process_and_detects_mismatch() {
        let pid = std::process::id();
        let uid = unsafe { libc::getuid() };
        let start = process_starttime(pid).unwrap();
        let lease = OwnerLease::open(pid, uid, start).unwrap();
        assert!(!lease.has_exited().unwrap());
        assert!(matches!(
            OwnerLease::open(pid, uid, start + 1),
            Err(OwnerError::Mismatch { .. })
        ));
    }

    #[test]
    fn pidfd_observes_real_child_exit() {
        let mut child = std::process::Command::new("/bin/sh")
            .args(["-c", "exit 0"])
            .spawn()
            .unwrap();
        let pid = child.id();
        let start = process_starttime(pid).unwrap();
        let lease = OwnerLease::open(pid, unsafe { libc::getuid() }, start).unwrap();
        child.wait().unwrap();
        assert!(lease.wait_timeout(Duration::from_secs(1)).unwrap());
    }

    #[test]
    fn unexpected_poll_hangup_is_an_error_not_a_false_timeout() {
        let (read_end, write_end) = pipe();
        drop(write_end);
        let lease = test_lease(read_end);
        assert!(matches!(
            lease.wait_timeout(Duration::from_millis(100)),
            Err(OwnerError::UnexpectedPollEvents { revents, .. })
                if revents & libc::POLLHUP != 0 && revents & libc::POLLIN == 0
        ));
    }

    #[test]
    fn interrupted_poll_retains_its_original_deadline() {
        extern "C" fn no_op_signal_handler(_: libc::c_int) {}

        struct RestoreSignalAction {
            previous: libc::sigaction,
        }
        impl Drop for RestoreSignalAction {
            fn drop(&mut self) {
                unsafe {
                    libc::sigaction(libc::SIGUSR1, &self.previous, std::ptr::null_mut());
                }
            }
        }

        let mut action = unsafe { std::mem::zeroed::<libc::sigaction>() };
        action.sa_sigaction = no_op_signal_handler as *const () as usize;
        assert_eq!(unsafe { libc::sigemptyset(&mut action.sa_mask) }, 0);
        action.sa_flags = 0;
        let mut previous = unsafe { std::mem::zeroed::<libc::sigaction>() };
        assert_eq!(
            unsafe { libc::sigaction(libc::SIGUSR1, &action, &mut previous) },
            0,
            "sigaction failed: {}",
            io::Error::last_os_error()
        );
        let _restore = RestoreSignalAction { previous };

        let (read_end, _write_end) = pipe();
        let lease = test_lease(read_end);
        let target = unsafe { libc::pthread_self() };
        let interrupter = std::thread::spawn(move || {
            for _ in 0..12 {
                std::thread::sleep(Duration::from_millis(10));
                assert_eq!(unsafe { libc::pthread_kill(target, libc::SIGUSR1) }, 0);
            }
        });

        let started = Instant::now();
        let outcome = lease.wait_timeout(Duration::from_millis(60));
        let elapsed = started.elapsed();
        let interrupt_result = interrupter.join();
        assert!(matches!(outcome, Ok(false)), "poll result was {outcome:?}");
        interrupt_result.unwrap();
        assert!(
            elapsed < Duration::from_millis(140),
            "EINTR extended a 60 ms deadline to {elapsed:?}"
        );
    }
}
