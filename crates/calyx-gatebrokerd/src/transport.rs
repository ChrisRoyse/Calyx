//! Bounded AF_UNIX `SOCK_SEQPACKET` transport with kernel peer credentials.

#![cfg(target_os = "linux")]

use std::env;
use std::io;
use std::mem::{size_of, zeroed};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

use thiserror::Error;

use crate::protocol::MAX_FRAME_BYTES;

const MAX_RIGHTS: usize = 4;
const CONTROL_BYTES: usize = 256;

#[repr(C, align(16))]
struct AlignedControl([u8; CONTROL_BYTES]);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeerCredentials {
    pub pid: u32,
    pub uid: u32,
    pub gid: u32,
}

#[derive(Debug)]
pub struct ReceivedFrame {
    pub bytes: Vec<u8>,
    pub rights: Vec<OwnedFd>,
}

#[derive(Debug)]
pub struct SeqpacketListener(OwnedFd);

#[derive(Debug)]
pub struct SeqpacketConnection(OwnedFd);

#[derive(Debug, Error)]
pub enum TransportError {
    #[error("transport I/O failed during {operation}: {source}")]
    Io {
        operation: &'static str,
        #[source]
        source: io::Error,
    },
    #[error("systemd socket activation contract failed: {0}")]
    Activation(String),
    #[error("Unix socket path is invalid: {0}")]
    InvalidPath(String),
    #[error("received frame is {actual} bytes; maximum is {maximum}")]
    OversizedFrame { actual: usize, maximum: usize },
    #[error("received too many file descriptors: {0}")]
    TooManyRights(usize),
    #[error("ancillary data was truncated (receive capacity={capacity} bytes)")]
    TruncatedAncillary { capacity: usize },
    #[error("malformed ancillary data: {0}")]
    MalformedAncillary(String),
    #[error("unexpected ancillary message level={level} type={kind}")]
    UnexpectedAncillary { level: i32, kind: i32 },
    #[error("short seqpacket send: expected={expected} actual={actual}")]
    ShortSend { expected: usize, actual: usize },
}

fn io_error(operation: &'static str) -> TransportError {
    TransportError::Io {
        operation,
        source: io::Error::last_os_error(),
    }
}

fn socket_option_i32(
    fd: RawFd,
    option: i32,
    operation: &'static str,
) -> Result<i32, TransportError> {
    let mut value = 0i32;
    let mut length = size_of::<i32>() as libc::socklen_t;
    let result = unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            option,
            (&mut value as *mut i32).cast(),
            &mut length,
        )
    };
    if result != 0 {
        return Err(io_error(operation));
    }
    if length as usize != size_of::<i32>() {
        return Err(TransportError::Activation(format!(
            "{operation} returned an invalid length"
        )));
    }
    Ok(value)
}

fn verify_listener_contract(fd: RawFd) -> Result<(), TransportError> {
    let domain = socket_option_i32(fd, libc::SO_DOMAIN, "read socket domain")?;
    if domain != libc::AF_UNIX {
        return Err(TransportError::Activation(format!(
            "fd 3 must have SO_DOMAIN=AF_UNIX({}); actual={domain}",
            libc::AF_UNIX
        )));
    }

    let socket_type = socket_option_i32(fd, libc::SO_TYPE, "read socket type")?;
    if socket_type != libc::SOCK_SEQPACKET {
        return Err(TransportError::Activation(format!(
            "fd 3 must have SO_TYPE=SOCK_SEQPACKET({}); actual={socket_type}",
            libc::SOCK_SEQPACKET
        )));
    }

    let listening = socket_option_i32(fd, libc::SO_ACCEPTCONN, "read listening state")?;
    if listening != 1 {
        return Err(TransportError::Activation(format!(
            "fd 3 must be a listening socket (SO_ACCEPTCONN=1); actual={listening}"
        )));
    }

    let pass_credentials =
        socket_option_i32(fd, libc::SO_PASSCRED, "read credential-passing state")?;
    if pass_credentials != 0 {
        return Err(TransportError::Activation(format!(
            "fd 3 must have SO_PASSCRED=0; actual={pass_credentials}"
        )));
    }
    Ok(())
}

fn set_close_on_exec(fd: RawFd) -> Result<(), TransportError> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC) } < 0 {
        return Err(io_error("set close-on-exec"));
    }
    Ok(())
}

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

    /// Verify that the activated listener is bound to the configured
    /// filesystem pathname. This is deliberately separate from activation
    /// environment validation because the configured path is broker state,
    /// not part of systemd's descriptor-passing contract.
    pub fn verify_bound_path(&self, expected: &Path) -> Result<(), TransportError> {
        // Apply the same pathname validation used by connect/bind before
        // comparing it with the kernel's source of truth.
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
        let fd = unsafe {
            libc::accept4(
                self.0.as_raw_fd(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                libc::SOCK_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err(io_error("accept seqpacket connection"));
        }
        Ok(SeqpacketConnection(unsafe { OwnedFd::from_raw_fd(fd) }))
    }
}

impl SeqpacketConnection {
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
        let length = unsafe {
            libc::recvmsg(
                self.0.as_raw_fd(),
                &mut message,
                libc::MSG_CMSG_CLOEXEC | libc::MSG_TRUNC,
            )
        };
        if length < 0 {
            return Err(io_error("receive seqpacket frame"));
        }
        // SCM_RIGHTS descriptors are installed by recvmsg itself. Adopt them
        // before inspecting any condition that can make this call fail so the
        // local Result/Vec owns and closes every delivered descriptor on all
        // error paths.
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
        let written = unsafe { libc::sendmsg(self.0.as_raw_fd(), &message, libc::MSG_NOSIGNAL) };
        if written < 0 {
            return Err(io_error("send seqpacket frame"));
        }
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
}

unsafe fn received_rights(message: &libc::msghdr) -> Result<Vec<OwnedFd>, TransportError> {
    let mut rights = Vec::new();
    let mut first_error = None;
    let base = unsafe { libc::CMSG_LEN(0) } as usize;
    let total = message.msg_controllen;
    if total == 0 {
        return Ok(rights);
    }
    if message.msg_control.is_null() || base < size_of::<libc::cmsghdr>() {
        return Err(TransportError::MalformedAncillary(
            "nonempty control data has no valid storage".into(),
        ));
    }

    let mut offset = 0usize;
    while total.saturating_sub(offset) >= base {
        let header_pointer = unsafe {
            message
                .msg_control
                .cast::<u8>()
                .add(offset)
                .cast::<libc::cmsghdr>()
        };
        let header = unsafe { std::ptr::read_unaligned(header_pointer) };
        let length = header.cmsg_len;
        let remaining = total - offset;
        if length < base || length > remaining {
            first_error.get_or_insert_with(|| {
                TransportError::MalformedAncillary(format!(
                    "invalid cmsg_len={length} at offset={offset} remaining={remaining}"
                ))
            });
            // The next header cannot be located safely. Any ancillary data
            // produced by the kernel is well-formed; this branch protects
            // against corrupt in-process msghdr values in tests/callers.
            break;
        }

        let data_bytes = length - base;
        if header.cmsg_level == libc::SOL_SOCKET && header.cmsg_type == libc::SCM_RIGHTS {
            if data_bytes % size_of::<RawFd>() != 0 {
                first_error.get_or_insert_with(|| {
                    TransportError::MalformedAncillary(format!(
                        "SCM_RIGHTS payload has {data_bytes} bytes"
                    ))
                });
            }
            let count = data_bytes / size_of::<RawFd>();
            let data = unsafe {
                message
                    .msg_control
                    .cast::<u8>()
                    .add(offset + base)
                    .cast::<RawFd>()
            };
            for index in 0..count {
                let fd = unsafe { std::ptr::read_unaligned(data.add(index)) };
                if fd < 0 {
                    first_error.get_or_insert_with(|| {
                        TransportError::MalformedAncillary(format!(
                            "SCM_RIGHTS delivered invalid fd={fd}"
                        ))
                    });
                } else {
                    // Ownership is taken immediately. If any later header,
                    // limit, or truncation check fails, Vec drop closes it.
                    rights.push(unsafe { OwnedFd::from_raw_fd(fd) });
                }
            }
        } else {
            first_error.get_or_insert(TransportError::UnexpectedAncillary {
                level: header.cmsg_level,
                kind: header.cmsg_type,
            });
        }

        let aligned = cmsg_align(length).ok_or_else(|| {
            TransportError::MalformedAncillary("cmsg length alignment overflow".into())
        })?;
        if aligned > remaining {
            // A final cmsg does not need to include all tail padding in
            // msg_controllen. Its payload was already adopted above.
            break;
        }
        offset += aligned;
    }

    if rights.len() > MAX_RIGHTS {
        first_error.get_or_insert(TransportError::TooManyRights(rights.len()));
    }
    match first_error {
        Some(error) => Err(error),
        None => Ok(rights),
    }
}

fn cmsg_align(length: usize) -> Option<usize> {
    let alignment = size_of::<usize>();
    length
        .checked_add(alignment - 1)
        .map(|value| value & !(alignment - 1))
}

fn local_socket_path(fd: RawFd) -> Result<Vec<u8>, TransportError> {
    let mut address = unsafe { zeroed::<libc::sockaddr_un>() };
    let mut length = size_of::<libc::sockaddr_un>() as libc::socklen_t;
    if unsafe {
        libc::getsockname(
            fd,
            (&mut address as *mut libc::sockaddr_un).cast(),
            &mut length,
        )
    } != 0
    {
        return Err(io_error("read activated socket pathname"));
    }
    if address.sun_family != libc::AF_UNIX as libc::sa_family_t {
        return Err(TransportError::Activation(format!(
            "getsockname returned family={}",
            address.sun_family
        )));
    }
    let offset = std::mem::offset_of!(libc::sockaddr_un, sun_path);
    let supplied = length as usize;
    if supplied <= offset {
        return Err(TransportError::Activation(
            "activated Unix socket is unnamed".into(),
        ));
    }
    let path_length = (supplied - offset).min(address.sun_path.len());
    let raw =
        unsafe { std::slice::from_raw_parts(address.sun_path.as_ptr().cast::<u8>(), path_length) };
    if raw.first() == Some(&0) {
        return Err(TransportError::Activation(
            "activated Unix socket uses an abstract name".into(),
        ));
    }
    let end = raw.iter().position(|byte| *byte == 0).unwrap_or(raw.len());
    if end == 0 {
        return Err(TransportError::Activation(
            "activated Unix socket has an empty pathname".into(),
        ));
    }
    Ok(raw[..end].to_vec())
}

fn sockaddr(path: &Path) -> Result<(libc::sockaddr_un, libc::socklen_t), TransportError> {
    let bytes = path.as_os_str().as_bytes();
    if !path.is_absolute() || bytes.is_empty() || bytes.contains(&0) {
        return Err(TransportError::InvalidPath(path.display().to_string()));
    }
    let mut address = unsafe { zeroed::<libc::sockaddr_un>() };
    if bytes.len() >= address.sun_path.len() {
        return Err(TransportError::InvalidPath(format!(
            "path is too long: {}",
            path.display()
        )));
    }
    address.sun_family = libc::AF_UNIX as libc::sa_family_t;
    for (destination, source) in address.sun_path.iter_mut().zip(bytes.iter().copied()) {
        *destination = source as libc::c_char;
    }
    let offset = std::mem::offset_of!(libc::sockaddr_un, sun_path);
    let length = (offset + bytes.len() + 1) as libc::socklen_t;
    Ok((address, length))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::fd::AsRawFd;

    fn pipe() -> (OwnedFd, OwnedFd) {
        let mut descriptors = [-1; 2];
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

    fn socket_pair(socket_type: i32) -> (OwnedFd, OwnedFd) {
        let mut descriptors = [-1; 2];
        assert_eq!(
            unsafe {
                libc::socketpair(
                    libc::AF_UNIX,
                    socket_type | libc::SOCK_CLOEXEC,
                    0,
                    descriptors.as_mut_ptr(),
                )
            },
            0,
            "socketpair failed: {}",
            io::Error::last_os_error()
        );
        unsafe {
            (
                OwnedFd::from_raw_fd(descriptors[0]),
                OwnedFd::from_raw_fd(descriptors[1]),
            )
        }
    }

    fn raw_send(connection: &SeqpacketConnection, bytes: &[u8], rights: &[RawFd]) {
        let mut iov = libc::iovec {
            iov_base: bytes.as_ptr() as *mut libc::c_void,
            iov_len: bytes.len(),
        };
        let required = unsafe { libc::CMSG_SPACE(size_of_val(rights) as u32) } as usize;
        // usize storage gives cmsghdr its required native alignment.
        let mut control = vec![0usize; required.div_ceil(size_of::<usize>())];
        let mut message = unsafe { zeroed::<libc::msghdr>() };
        message.msg_iov = &mut iov;
        message.msg_iovlen = 1;
        message.msg_control = control.as_mut_ptr().cast();
        message.msg_controllen = required;
        unsafe {
            let header = libc::CMSG_FIRSTHDR(&message);
            assert!(!header.is_null());
            (*header).cmsg_level = libc::SOL_SOCKET;
            (*header).cmsg_type = libc::SCM_RIGHTS;
            (*header).cmsg_len = libc::CMSG_LEN(size_of_val(rights) as u32) as usize;
            std::ptr::copy_nonoverlapping(
                rights.as_ptr().cast::<u8>(),
                libc::CMSG_DATA(header),
                size_of_val(rights),
            );
            let sent = libc::sendmsg(connection.raw_fd(), &message, libc::MSG_NOSIGNAL);
            assert_eq!(
                sent,
                bytes.len() as isize,
                "raw sendmsg failed: {}",
                io::Error::last_os_error()
            );
        }
    }

    fn assert_pipe_has_no_leaked_writer(read_end: &OwnedFd) {
        let mut descriptor = libc::pollfd {
            fd: read_end.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let result = unsafe { libc::poll(&mut descriptor, 1, 1_000) };
        assert_eq!(
            result, 1,
            "receiver retained an SCM_RIGHTS writer (revents=0x{:x})",
            descriptor.revents
        );
        let mut byte = 0u8;
        assert_eq!(
            unsafe { libc::read(read_end.as_raw_fd(), (&mut byte as *mut u8).cast(), 1) },
            0,
            "pipe did not reach EOF after failed receive"
        );
    }

    #[test]
    fn explicit_seqpacket_round_trip_and_credentials() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("control.sock");
        let listener = SeqpacketListener::bind(&path, 4).unwrap();
        let thread = std::thread::spawn({
            let path = path.clone();
            move || {
                let connection = SeqpacketConnection::connect(&path).unwrap();
                connection.send(b"request", &[]).unwrap();
                let response = connection.recv().unwrap();
                assert_eq!(response.bytes, b"response");
            }
        });
        let connection = listener.accept().unwrap();
        let credentials = connection.peer_credentials().unwrap();
        assert_eq!(credentials.uid, unsafe { libc::getuid() });
        let request = connection.recv().unwrap();
        assert_eq!(request.bytes, b"request");
        assert!(request.rights.is_empty());
        connection.send(b"response", &[]).unwrap();
        thread.join().unwrap();
    }

    #[test]
    fn scm_rights_arrive_close_on_exec() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("rights.sock");
        let listener = SeqpacketListener::bind(&path, 4).unwrap();
        let thread = std::thread::spawn({
            let path = path.clone();
            move || {
                let connection = SeqpacketConnection::connect(&path).unwrap();
                connection
                    .send(b"with-right", &[std::io::stdout().as_raw_fd()])
                    .unwrap();
            }
        });
        let connection = listener.accept().unwrap();
        let frame = connection.recv().unwrap();
        assert_eq!(frame.rights.len(), 1);
        let flags = unsafe { libc::fcntl(frame.rights[0].as_raw_fd(), libc::F_GETFD) };
        assert_ne!(flags & libc::FD_CLOEXEC, 0);
        thread.join().unwrap();
    }

    #[test]
    fn listener_contract_and_bound_path_are_kernel_verified() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("contract.sock");
        let listener = SeqpacketListener::bind(&path, 4).unwrap();
        verify_listener_contract(listener.0.as_raw_fd()).unwrap();
        listener.verify_bound_path(&path).unwrap();
        assert!(matches!(
            listener.verify_bound_path(&temp.path().join("different.sock")),
            Err(TransportError::Activation(_))
        ));

        let enabled = 1i32;
        assert_eq!(
            unsafe {
                libc::setsockopt(
                    listener.0.as_raw_fd(),
                    libc::SOL_SOCKET,
                    libc::SO_PASSCRED,
                    (&enabled as *const i32).cast(),
                    size_of::<i32>() as libc::socklen_t,
                )
            },
            0
        );
        assert!(matches!(
            verify_listener_contract(listener.0.as_raw_fd()),
            Err(TransportError::Activation(message)) if message.contains("SO_PASSCRED=0")
        ));
    }

    #[test]
    fn listener_contract_rejects_wrong_domain_type_and_listening_state() {
        let inet_fd =
            unsafe { libc::socket(libc::AF_INET, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0) };
        assert!(
            inet_fd >= 0,
            "socket failed: {}",
            io::Error::last_os_error()
        );
        let inet = unsafe { OwnedFd::from_raw_fd(inet_fd) };
        assert!(matches!(
            verify_listener_contract(inet.as_raw_fd()),
            Err(TransportError::Activation(message)) if message.contains("SO_DOMAIN=AF_UNIX")
        ));

        let (stream, _stream_peer) = socket_pair(libc::SOCK_STREAM);
        assert!(matches!(
            verify_listener_contract(stream.as_raw_fd()),
            Err(TransportError::Activation(message)) if message.contains("SO_TYPE=SOCK_SEQPACKET")
        ));

        let (seqpacket, _seqpacket_peer) = socket_pair(libc::SOCK_SEQPACKET);
        assert!(matches!(
            verify_listener_contract(seqpacket.as_raw_fd()),
            Err(TransportError::Activation(message)) if message.contains("SO_ACCEPTCONN=1")
        ));
    }

    #[test]
    fn too_many_rights_error_closes_every_delivered_fd() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("too-many-rights.sock");
        let listener = SeqpacketListener::bind(&path, 4).unwrap();
        let (read_end, write_end) = pipe();
        let thread = std::thread::spawn({
            let path = path.clone();
            move || {
                let connection = SeqpacketConnection::connect(&path).unwrap();
                raw_send(
                    &connection,
                    b"rights",
                    &[write_end.as_raw_fd(); MAX_RIGHTS + 1],
                );
            }
        });
        let connection = listener.accept().unwrap();
        assert!(matches!(
            connection.recv(),
            Err(TransportError::TooManyRights(count)) if count == MAX_RIGHTS + 1
        ));
        thread.join().unwrap();
        assert_pipe_has_no_leaked_writer(&read_end);
    }

    #[test]
    fn oversized_frame_error_closes_delivered_fd() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("oversized-rights.sock");
        let listener = SeqpacketListener::bind(&path, 4).unwrap();
        let (read_end, write_end) = pipe();
        let thread = std::thread::spawn({
            let path = path.clone();
            move || {
                let connection = SeqpacketConnection::connect(&path).unwrap();
                let packet = vec![b'x'; MAX_FRAME_BYTES + 4_096];
                raw_send(&connection, &packet, &[write_end.as_raw_fd()]);
            }
        });
        let connection = listener.accept().unwrap();
        assert!(matches!(
            connection.recv(),
            Err(TransportError::OversizedFrame { actual, maximum })
                if actual > maximum && maximum == MAX_FRAME_BYTES
        ));
        thread.join().unwrap();
        assert_pipe_has_no_leaked_writer(&read_end);
    }

    #[test]
    fn truncated_ancillary_error_closes_every_delivered_fd() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("truncated-rights.sock");
        let listener = SeqpacketListener::bind(&path, 4).unwrap();
        let (read_end, write_end) = pipe();
        let thread = std::thread::spawn({
            let path = path.clone();
            move || {
                let connection = SeqpacketConnection::connect(&path).unwrap();
                let descriptors = vec![write_end.as_raw_fd(); 80];
                raw_send(&connection, b"truncated-rights", &descriptors);
            }
        });
        let connection = listener.accept().unwrap();
        assert!(matches!(
            connection.recv(),
            Err(TransportError::TruncatedAncillary { capacity })
                if capacity == CONTROL_BYTES
        ));
        thread.join().unwrap();
        assert_pipe_has_no_leaked_writer(&read_end);
    }
}
