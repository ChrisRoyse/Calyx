use std::mem::{size_of, zeroed};
use std::os::fd::RawFd;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

use super::{TransportError, io_error};

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

pub(super) fn verify_listener_contract(fd: RawFd) -> Result<(), TransportError> {
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

pub(super) fn set_close_on_exec(fd: RawFd) -> Result<(), TransportError> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC) } < 0 {
        return Err(io_error("set close-on-exec"));
    }
    Ok(())
}

pub(super) fn local_socket_path(fd: RawFd) -> Result<Vec<u8>, TransportError> {
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

pub(super) fn sockaddr(
    path: &Path,
) -> Result<(libc::sockaddr_un, libc::socklen_t), TransportError> {
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
