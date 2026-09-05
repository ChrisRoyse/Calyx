//! Bounded AF_UNIX `SOCK_SEQPACKET` transport with kernel peer credentials.

#![cfg(target_os = "linux")]

mod address;
mod ancillary;
mod connection;
mod listener;

use std::io;

use thiserror::Error;

pub use connection::SeqpacketConnection;
pub use listener::SeqpacketListener;

pub(crate) const MAX_RIGHTS: usize = 4;
pub(crate) const CONTROL_BYTES: usize = 256;

/// An accepted broker connection may wait this long in an individual kernel
/// receive or send operation. Stage execution itself does not consume this
/// budget; the timeout applies only while bytes are moving over the socket.
pub const ACCEPTED_IO_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeerCredentials {
    pub pid: u32,
    pub uid: u32,
    pub gid: u32,
}

#[derive(Debug)]
pub struct ReceivedFrame {
    pub bytes: Vec<u8>,
    pub rights: Vec<std::os::fd::OwnedFd>,
}

#[derive(Debug, Error)]
pub enum TransportError {
    #[error("transport I/O failed during {operation}: {source}")]
    Io {
        operation: &'static str,
        #[source]
        source: io::Error,
    },
    #[error("transport deadline expired during {operation}")]
    TimedOut { operation: &'static str },
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

pub(crate) fn io_error(operation: &'static str) -> TransportError {
    TransportError::Io {
        operation,
        source: io::Error::last_os_error(),
    }
}

pub(crate) fn socket_io_error(operation: &'static str, source: io::Error) -> TransportError {
    if matches!(source.raw_os_error(), Some(code) if code == libc::EAGAIN) {
        TransportError::TimedOut { operation }
    } else {
        TransportError::Io { operation, source }
    }
}

#[cfg(test)]
mod signal_tests;
#[cfg(test)]
mod tests;
