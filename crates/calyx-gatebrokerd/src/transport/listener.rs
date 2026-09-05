use std::env;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

use super::address::{local_socket_path, set_close_on_exec, sockaddr, verify_listener_contract};
use super::connection::configure_io_timeout;
use super::{ACCEPTED_IO_TIMEOUT, SeqpacketConnection, TransportError, io_error};

#[cfg(test)]
pub(super) static ACCEPT_INTERRUPTS: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

#[derive(Debug)]
pub struct SeqpacketListener(pub(super) OwnedFd);

impl SeqpacketListener {
    pub fn from_systemd() -> Result<Self, TransportError> {
        let expected_pid = unsafe { libc::getpid() }.to_string();
        let listen_pid = env::var("LISTEN_PID")
            .map_err(|_| TransportError::Activation("LISTEN_PID is missing".into()))?;
        let listen_fds = env::var("LISTEN_FDS")
            .map_err(|_| TransportError::Activation("LISTEN_FDS is missing".into()))?;
        let listen_names = env::var("LISTEN_FDNAMES")
            .map_err(|_| TransportError::Activation("LISTEN_FDNAMES is missing".into()))?;
        if listen_pid != expected_pid || listen_fds != "1" {
            return Err(TransportError::Activation(format!(
                "expected LISTEN_PID={expected_pid} LISTEN_FDS=1; actual pid={listen_pid:?} fds={listen_fds:?}"
            )));
        }
        if listen_names != "control" {
            return Err(TransportError::Activation(format!(
                "expected descriptor name control; actual={listen_names:?}"
            )));
        }
        let fd = 3;
        verify_listener_contract(fd)?;
        set_close_on_exec(fd)?;
        // The variables must not be inherited by any helper or stage.
        unsafe {
            env::remove_var("LISTEN_PID");
            env::remove_var("LISTEN_FDS");
            env::remove_var("LISTEN_FDNAMES");
        }
        Ok(Self(unsafe { OwnedFd::from_raw_fd(fd) }))
    }

    pub fn verify_bound_path(&self, expected: &Path) -> Result<(), TransportError> {
        let _ = sockaddr(expected)?;
        let actual = local_socket_path(self.0.as_raw_fd())?;
        if actual.as_slice() != expected.as_os_str().as_bytes() {
            return Err(TransportError::Activation(format!(
                "activated socket pathname mismatch: expected={} actual={}",
                expected.display(),
                String::from_utf8_lossy(&actual)
            )));
        }
        Ok(())
    }

    /// Bind an explicit test/foreground socket. Existing filesystem entries
    /// are never unlinked or replaced.
    pub fn bind(path: &Path, backlog: i32) -> Result<Self, TransportError> {
        if path.as_os_str().as_bytes().contains(&0) || path.exists() {
            return Err(TransportError::InvalidPath(format!(
                "path contains NUL or already exists: {}",
                path.display()
            )));
        }
        let (address, length) = sockaddr(path)?;
        let fd =
            unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_SEQPACKET | libc::SOCK_CLOEXEC, 0) };
        if fd < 0 {
            return Err(io_error("create seqpacket listener"));
        }
        let owned = unsafe { OwnedFd::from_raw_fd(fd) };
        if unsafe {
            libc::bind(
                owned.as_raw_fd(),
                (&address as *const libc::sockaddr_un).cast(),
                length,
            )
        } != 0
        {
            return Err(io_error("bind seqpacket listener"));
        }
        if unsafe { libc::listen(owned.as_raw_fd(), backlog) } != 0 {
            return Err(io_error("listen on seqpacket socket"));
        }
        Ok(Self(owned))
    }

    pub fn accept(&self) -> Result<SeqpacketConnection, TransportError> {
        let fd = loop {
            let result = unsafe {
                libc::accept4(
                    self.0.as_raw_fd(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    libc::SOCK_CLOEXEC,
                )
            };
            if result >= 0 {
                break result;
            }
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::Interrupted {
                #[cfg(test)]
                ACCEPT_INTERRUPTS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                continue;
            }
            return Err(TransportError::Io {
                operation: "accept seqpacket connection",
                source: error,
            });
        };
        let owned = unsafe { OwnedFd::from_raw_fd(fd) };
        configure_io_timeout(owned.as_raw_fd(), ACCEPTED_IO_TIMEOUT)?;
        Ok(SeqpacketConnection::from_accepted(owned))
    }
}
