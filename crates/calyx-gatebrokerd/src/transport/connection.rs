use std::mem::{size_of, size_of_val, zeroed};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::path::Path;
use std::time::Duration;

use crate::protocol::MAX_FRAME_BYTES;

use super::address::sockaddr;
use super::ancillary::received_rights;
use super::{
    CONTROL_BYTES, MAX_RIGHTS, PeerCredentials, ReceivedFrame, TransportError, io_error,
    socket_io_error,
};

#[repr(C, align(16))]
struct AlignedControl([u8; CONTROL_BYTES]);

#[cfg(test)]
pub(super) static RECEIVE_INTERRUPTS: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

#[cfg(test)]
pub(super) static SEND_INTERRUPTS: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

#[derive(Debug)]
pub struct SeqpacketConnection(OwnedFd);

impl SeqpacketConnection {
    pub(super) fn from_accepted(fd: OwnedFd) -> Self {
        Self(fd)
    }

    pub fn connect(path: &Path) -> Result<Self, TransportError> {
        let (address, length) = sockaddr(path)?;
        let fd =
            unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_SEQPACKET | libc::SOCK_CLOEXEC, 0) };
        if fd < 0 {
            return Err(io_error("create seqpacket client"));
        }
        let owned = unsafe { OwnedFd::from_raw_fd(fd) };
        if unsafe {
            libc::connect(
                owned.as_raw_fd(),
                (&address as *const libc::sockaddr_un).cast(),
                length,
            )
        } != 0
        {
            return Err(io_error("connect seqpacket socket"));
        }
        Ok(Self(owned))
    }

    pub fn peer_credentials(&self) -> Result<PeerCredentials, TransportError> {
        let mut credentials = unsafe { zeroed::<libc::ucred>() };
        let mut length = size_of::<libc::ucred>() as libc::socklen_t;
        if unsafe {
            libc::getsockopt(
                self.0.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_PEERCRED,
                (&mut credentials as *mut libc::ucred).cast(),
                &mut length,
            )
        } != 0
        {
            return Err(io_error("read peer credentials"));
        }
        if length as usize != size_of::<libc::ucred>()
            || credentials.pid <= 0
            || credentials.uid == u32::MAX
            || credentials.gid == u32::MAX
        {
            return Err(TransportError::Activation(
                "SO_PEERCRED returned invalid credentials".into(),
            ));
        }
        Ok(PeerCredentials {
            pid: credentials.pid as u32,
            uid: credentials.uid,
            gid: credentials.gid,
        })
    }

    pub fn recv(&self) -> Result<ReceivedFrame, TransportError> {
        let mut bytes = vec![0u8; MAX_FRAME_BYTES + 1];
        let mut iov = libc::iovec {
            iov_base: bytes.as_mut_ptr().cast(),
            iov_len: bytes.len(),
        };
        let mut control = AlignedControl([0u8; CONTROL_BYTES]);
        let mut message = unsafe { zeroed::<libc::msghdr>() };
        message.msg_iov = &mut iov;
        message.msg_iovlen = 1;
        message.msg_control = control.0.as_mut_ptr().cast();
        message.msg_controllen = control.0.len();
        let length = loop {
            message.msg_flags = 0;
            message.msg_controllen = control.0.len();
            let result = unsafe {
                libc::recvmsg(
                    self.0.as_raw_fd(),
                    &mut message,
                    libc::MSG_CMSG_CLOEXEC | libc::MSG_TRUNC,
                )
            };
            if result >= 0 {
                break result;
            }
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::Interrupted {
                #[cfg(test)]
                RECEIVE_INTERRUPTS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                continue;
            }
            return Err(socket_io_error("receive seqpacket frame", error));
        };
        // recvmsg installs SCM_RIGHTS descriptors. Adopt them before any
        // validation that can return so every error path closes them.
        let rights_result = unsafe { received_rights(&message) };
        if message.msg_flags & libc::MSG_CTRUNC != 0 {
            drop(rights_result);
            return Err(TransportError::TruncatedAncillary {
                capacity: CONTROL_BYTES,
            });
        }
        let rights = rights_result?;
        if length as usize > MAX_FRAME_BYTES || message.msg_flags & libc::MSG_TRUNC != 0 {
            return Err(TransportError::OversizedFrame {
                actual: length as usize,
                maximum: MAX_FRAME_BYTES,
            });
        }
        bytes.truncate(length as usize);
        Ok(ReceivedFrame { bytes, rights })
    }

    pub fn send(&self, bytes: &[u8], rights: &[RawFd]) -> Result<(), TransportError> {
        if bytes.len() > MAX_FRAME_BYTES {
            return Err(TransportError::OversizedFrame {
                actual: bytes.len(),
                maximum: MAX_FRAME_BYTES,
            });
        }
        if rights.len() > MAX_RIGHTS {
            return Err(TransportError::TooManyRights(rights.len()));
        }
        let mut iov = libc::iovec {
            iov_base: bytes.as_ptr() as *mut libc::c_void,
            iov_len: bytes.len(),
        };
        let mut control = AlignedControl([0u8; CONTROL_BYTES]);
        let mut message = unsafe { zeroed::<libc::msghdr>() };
        message.msg_iov = &mut iov;
        message.msg_iovlen = 1;
        if !rights.is_empty() {
            let required = unsafe { libc::CMSG_SPACE(size_of_val(rights) as u32) } as usize;
            if required > control.0.len() {
                return Err(TransportError::TooManyRights(rights.len()));
            }
            message.msg_control = control.0.as_mut_ptr().cast();
            message.msg_controllen = required;
            unsafe {
                let header = libc::CMSG_FIRSTHDR(&message);
                if header.is_null() {
                    return Err(TransportError::Activation(
                        "could not construct SCM_RIGHTS header".into(),
                    ));
                }
                (*header).cmsg_level = libc::SOL_SOCKET;
                (*header).cmsg_type = libc::SCM_RIGHTS;
                (*header).cmsg_len = libc::CMSG_LEN(size_of_val(rights) as u32) as usize;
                std::ptr::copy_nonoverlapping(
                    rights.as_ptr().cast::<u8>(),
                    libc::CMSG_DATA(header),
                    size_of_val(rights),
                );
            }
        }
        let written = loop {
            let result = unsafe { libc::sendmsg(self.0.as_raw_fd(), &message, libc::MSG_NOSIGNAL) };
            if result >= 0 {
                break result;
            }
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::Interrupted {
                #[cfg(test)]
                SEND_INTERRUPTS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                continue;
            }
            return Err(socket_io_error("send seqpacket frame", error));
        };
        if written as usize != bytes.len() {
            return Err(TransportError::ShortSend {
                expected: bytes.len(),
                actual: written as usize,
            });
        }
        Ok(())
    }

    pub fn raw_fd(&self) -> RawFd {
        self.0.as_raw_fd()
    }

    #[cfg(test)]
    pub(super) fn set_io_timeout(&self, timeout: Duration) -> Result<(), TransportError> {
        configure_io_timeout(self.raw_fd(), timeout)
    }
}

pub(super) fn configure_io_timeout(fd: RawFd, timeout: Duration) -> Result<(), TransportError> {
    if timeout.is_zero() {
        return Err(TransportError::Activation(
            "socket I/O timeout must be nonzero".into(),
        ));
    }
    // timeval cannot represent nanoseconds. Round the deadline upward to one
    // microsecond so a positive requested bound can never silently become the
    // kernel's special "timeouts disabled" value of {0, 0}.
    let mut microseconds = timeout.as_micros();
    if !timeout.subsec_nanos().is_multiple_of(1_000) {
        microseconds += 1;
    }
    let seconds = (microseconds / 1_000_000).try_into().map_err(|_| {
        TransportError::Activation("socket I/O timeout seconds overflow time_t".into())
    })?;
    let remainder = (microseconds % 1_000_000).try_into().map_err(|_| {
        TransportError::Activation("socket I/O timeout microseconds overflow suseconds_t".into())
    })?;
    let timeval = libc::timeval {
        tv_sec: seconds,
        tv_usec: remainder,
    };
    for (option, operation) in [
        (libc::SO_RCVTIMEO, "set receive deadline"),
        (libc::SO_SNDTIMEO, "set send deadline"),
    ] {
        if unsafe {
            libc::setsockopt(
                fd,
                libc::SOL_SOCKET,
                option,
                (&timeval as *const libc::timeval).cast(),
                size_of::<libc::timeval>() as libc::socklen_t,
            )
        } != 0
        {
            return Err(io_error(operation));
        }
    }
    Ok(())
}
