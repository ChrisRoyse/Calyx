use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, RawFd};
use std::time::{Duration, Instant};

use calyx_gatebrokerd::protocol::{MAX_FRAME_BYTES, RunToken};

use crate::CliError;

const MIN_INHERITED_FD: RawFd = 3;
const MAX_INHERITED_FD: RawFd = 1024;
const PIPE_IO_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_RESPONSE_BYTES: usize = MAX_FRAME_BYTES * 2;

pub(super) struct TokenReader(PipeFd);

pub(super) struct TokenWriter(PipeFd);

pub(super) struct PipeWriter(PipeFd);

struct PipeFd {
    file: File,
    label: &'static str,
    capacity: usize,
}

impl TokenReader {
    pub(super) fn new(fd: RawFd, label: &'static str) -> Result<Self, String> {
        PipeFd::take(fd, label, libc::O_RDONLY).map(Self)
    }

    pub(super) fn read(mut self) -> Result<String, String> {
        let mut bytes = Vec::with_capacity(66);
        let mut buffer = [0_u8; 66];
        loop {
            match self.0.file.read(&mut buffer[..66 - bytes.len()]) {
                Ok(0) => break,
                Ok(count) => {
                    bytes.extend_from_slice(&buffer[..count]);
                    if bytes.len() == 66 {
                        return Err(format!(
                            "{} exceeds the 65-byte token frame limit",
                            self.0.label
                        ));
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    wait(
                        self.0.file.as_raw_fd(),
                        libc::POLLIN | libc::POLLHUP,
                        self.0.label,
                    )
                    .map_err(|error| error.to_string())?;
                }
                Err(error) => return Err(format!("read {}: {error}", self.0.label)),
            }
        }
        if bytes.last() == Some(&b'\n') {
            bytes.pop();
        }
        if bytes.len() != 64
            || !bytes
                .iter()
                .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
        {
            return Err(format!(
                "{} must contain exactly 64 lowercase hexadecimal bytes and an optional newline",
                self.0.label
            ));
        }
        String::from_utf8(bytes).map_err(|_| format!("{} is not UTF-8", self.0.label))
    }
}

impl TokenWriter {
    pub(super) fn new(fd: RawFd, label: &'static str) -> Result<Self, String> {
        PipeFd::take(fd, label, libc::O_WRONLY).map(Self)
    }

    pub(super) fn write(mut self, token: &RunToken) -> Result<(), CliError> {
        let mut frame = [b'\n'; 65];
        frame[..64].copy_from_slice(token.as_str().as_bytes());
        self.0.write_bounded(&frame, 65)
    }
}

impl PipeWriter {
    pub(super) fn new(fd: RawFd, label: &'static str) -> Result<Self, String> {
        PipeFd::take(fd, label, libc::O_WRONLY).map(Self)
    }

    pub(super) fn raw_fd(&self) -> RawFd {
        self.0.file.as_raw_fd()
    }

    pub(super) fn write_frame(&mut self, bytes: &[u8]) -> Result<(), CliError> {
        if bytes.len() >= MAX_RESPONSE_BYTES {
            return Err(CliError::protocol(format!(
                "rendered response is {} bytes; limit is {} bytes",
                bytes.len(),
                MAX_RESPONSE_BYTES - 1
            )));
        }
        let mut frame = Vec::with_capacity(bytes.len() + 1);
        frame.extend_from_slice(bytes);
        frame.push(b'\n');
        self.0.write_bounded(&frame, MAX_RESPONSE_BYTES)
    }
}

impl PipeFd {
    fn take(fd: RawFd, label: &'static str, access: libc::c_int) -> Result<Self, String> {
        if !(MIN_INHERITED_FD..=MAX_INHERITED_FD).contains(&fd) {
            return Err(format!(
                "{label} must be between {MIN_INHERITED_FD} and {MAX_INHERITED_FD}"
            ));
        }
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        if flags < 0 {
            return Err(format!("inspect {label}: {}", io::Error::last_os_error()));
        }
        // SAFETY: F_GETFL just proved this is a live descriptor, and the CLI
        // contract transfers its sole ownership to this process.
        let file = unsafe { File::from_raw_fd(fd) };
        let stat = fstat(file.as_raw_fd()).map_err(|error| format!("fstat {label}: {error}"))?;
        if stat.st_mode & libc::S_IFMT != libc::S_IFIFO {
            return Err(format!(
                "{label} must be a Linux pipe/FIFO, not file type {:#o}",
                stat.st_mode & libc::S_IFMT
            ));
        }
        if flags & libc::O_ACCMODE != access {
            let direction = if access == libc::O_RDONLY {
                "read-only"
            } else {
                "write-only"
            };
            return Err(format!("{label} must be a {direction} pipe endpoint"));
        }
        let capacity = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETPIPE_SZ) };
        if capacity <= 0 {
            return Err(format!(
                "read finite capacity for {label}: {}",
                io::Error::last_os_error()
            ));
        }
        if unsafe { libc::fcntl(file.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
            return Err(format!(
                "set bounded nonblocking mode for {label}: {}",
                io::Error::last_os_error()
            ));
        }
        Ok(Self {
            file,
            label,
            capacity: capacity as usize,
        })
    }

    fn write_bounded(&mut self, bytes: &[u8], limit: usize) -> Result<(), CliError> {
        if bytes.len() > limit {
            return Err(CliError::protocol(format!(
                "{} frame length {} exceeds bounded pipe limit {} (kernel capacity {})",
                self.label,
                bytes.len(),
                limit,
                self.capacity
            )));
        }
        let deadline = Instant::now() + PIPE_IO_TIMEOUT;
        let mut remaining = bytes;
        while !remaining.is_empty() {
            match self.file.write(remaining) {
                Ok(0) => {
                    return Err(CliError::io(
                        "write pipe frame",
                        format!("{} returned a zero-byte write", self.label),
                    ));
                }
                Ok(count) => remaining = &remaining[count..],
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    wait_until(self.file.as_raw_fd(), libc::POLLOUT, self.label, deadline)
                        .map_err(|error| CliError::io("wait for writable pipe", error))?;
                }
                Err(error) => return Err(CliError::io("write pipe frame", error)),
            }
        }
        Ok(())
    }
}

fn fstat(fd: RawFd) -> io::Result<libc::stat> {
    let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
    if unsafe { libc::fstat(fd, stat.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { stat.assume_init() })
}

fn wait(fd: RawFd, events: i16, label: &str) -> io::Result<()> {
    wait_until(fd, events, label, Instant::now() + PIPE_IO_TIMEOUT)
}

fn wait_until(fd: RawFd, events: i16, label: &str, deadline: Instant) -> io::Result<()> {
    loop {
        let now = Instant::now();
        if now >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!("{label} did not become ready within {PIPE_IO_TIMEOUT:?}"),
            ));
        }
        let remaining = deadline.saturating_duration_since(now);
        let millis = i32::try_from(remaining.as_millis().max(1)).unwrap_or(i32::MAX);
        let mut pollfd = libc::pollfd {
            fd,
            events,
            revents: 0,
        };
        let result = unsafe { libc::poll(&mut pollfd, 1, millis) };
        if result > 0 {
            if pollfd.revents & libc::POLLNVAL != 0 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("{label} became invalid"),
                ));
            }
            if pollfd.revents & libc::POLLERR != 0 {
                return Err(io::Error::other(format!("{label} reported POLLERR")));
            }
            if pollfd.revents & (events | libc::POLLHUP) != 0 {
                return Ok(());
            }
            continue;
        }
        if result == 0 {
            continue;
        }
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::Interrupted {
            return Err(error);
        }
    }
}

#[cfg(test)]
#[path = "fd_pipe_tests.rs"]
mod tests;
