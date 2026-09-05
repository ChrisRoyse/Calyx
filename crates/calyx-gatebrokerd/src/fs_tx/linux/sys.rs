use std::ffi::{CStr, CString, OsStr};
use std::fs;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

use super::*;

const RESOLVE_NO_XDEV: u64 = 0x01;
const RESOLVE_NO_MAGICLINKS: u64 = 0x02;
const RESOLVE_NO_SYMLINKS: u64 = 0x04;
const RESOLVE_BENEATH: u64 = 0x08;
const RENAME_NOREPLACE: u32 = 1;

#[repr(C)]
struct OpenHow {
    flags: u64,
    mode: u64,
    resolve: u64,
}

pub(super) fn validate_spec(spec: &FsRootSpec) -> Result<(), FsTxError> {
    if spec.private_mode != 0o700 || spec.published_mode & 0o700 != 0o700 {
        return Err(FsTxError::InvalidSpec(
            "private mode must be 0700 and published owner must have rwx".into(),
        ));
    }
    for path in [&spec.common_ancestor, &spec.shared_path, &spec.private_path] {
        if !path.is_absolute() {
            return Err(FsTxError::InvalidSpec(format!(
                "{} is not absolute",
                path.display()
            )));
        }
    }
    Ok(())
}

pub(super) fn relative_to<'a>(base: &Path, path: &'a Path) -> Result<&'a OsStr, FsTxError> {
    let relative = path.strip_prefix(base).map_err(|_| {
        FsTxError::InvalidSpec(format!(
            "{} is not below {}",
            path.display(),
            base.display()
        ))
    })?;
    if relative.as_os_str().is_empty() {
        return Err(FsTxError::InvalidSpec(
            "root must be below common ancestor".into(),
        ));
    }
    Ok(relative.as_os_str())
}

pub(super) fn open_absolute_directory(path: &Path) -> Result<OwnedFd, FsTxError> {
    let slash = CString::new("/").expect("literal");
    let raw = unsafe {
        libc::open(
            slash.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
        )
    };
    let root = owned(raw, "open", "/")?;
    let relative = path
        .strip_prefix("/")
        .map_err(|_| FsTxError::InvalidSpec("path is not absolute".into()))?;
    open_directory_at(root.as_raw_fd(), relative.as_os_str())
}

pub(super) fn open_directory_at(parent: RawFd, name: &OsStr) -> Result<OwnedFd, FsTxError> {
    let name = cstring_os(name)?;
    openat2(
        parent,
        &name,
        (libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC) as u64,
        RESOLVE_BENEATH | RESOLVE_NO_SYMLINKS | RESOLVE_NO_MAGICLINKS,
    )
}

pub(super) fn openat2(
    parent: RawFd,
    name: &CStr,
    flags: u64,
    resolve: u64,
) -> Result<OwnedFd, FsTxError> {
    let how = OpenHow {
        flags,
        mode: 0,
        resolve,
    };
    let raw = unsafe {
        libc::syscall(
            libc::SYS_openat2,
            parent,
            name.as_ptr(),
            &how,
            std::mem::size_of::<OpenHow>(),
        ) as i32
    };
    owned(raw, "openat2", &name.to_string_lossy()).map_err(|error| match &error {
        FsTxError::Io { source, .. } if source.raw_os_error() == Some(libc::ENOSYS) => {
            FsTxError::CapabilityUnavailable {
                capability: "openat2",
                detail: source.to_string(),
            }
        }
        _ => error,
    })
}

pub(super) fn probe_openat2(fd: RawFd) -> Result<(), FsTxError> {
    let dot = cstr_dot();
    let _ = openat2(
        fd,
        dot,
        (libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC) as u64,
        RESOLVE_BENEATH | RESOLVE_NO_SYMLINKS | RESOLVE_NO_MAGICLINKS | RESOLVE_NO_XDEV,
    )?;
    Ok(())
}

pub(super) fn probe_rename_noreplace(fd: RawFd) -> Result<(), FsTxError> {
    let source = CString::new(".calyx-probe-source-does-not-exist").expect("literal");
    let target = CString::new(".calyx-probe-target-does-not-exist").expect("literal");
    let result =
        unsafe { libc::renameat2(fd, source.as_ptr(), fd, target.as_ptr(), RENAME_NOREPLACE) };
    if result == -1 {
        let error = io::Error::last_os_error();
        return match error.raw_os_error() {
            Some(libc::ENOENT) => Ok(()),
            Some(libc::ENOSYS | libc::EINVAL | libc::EOPNOTSUPP) => {
                Err(FsTxError::CapabilityUnavailable {
                    capability: "renameat2(RENAME_NOREPLACE)",
                    detail: error.to_string(),
                })
            }
            _ => Err(io_error("probe renameat2", "shared", error)),
        };
    }
    Err(FsTxError::CapabilityUnavailable {
        capability: "renameat2 probe invariant",
        detail: "rename of a guaranteed-missing source unexpectedly succeeded".into(),
    })
}

pub(super) fn rename_noreplace(
    old_fd: RawFd,
    old: &CStr,
    new_fd: RawFd,
    new: &CStr,
    display: &str,
) -> Result<(), FsTxError> {
    let result =
        unsafe { libc::renameat2(old_fd, old.as_ptr(), new_fd, new.as_ptr(), RENAME_NOREPLACE) };
    if result == 0 {
        return Ok(());
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::EEXIST) {
        Err(FsTxError::Collision(display.into()))
    } else {
        Err(io_error("renameat2(RENAME_NOREPLACE)", display, error))
    }
}

pub(super) fn identity_at(parent: RawFd, name: &CStr) -> Result<ObjectIdentity, FsTxError> {
    let fd = openat2(
        parent,
        name,
        (libc::O_PATH | libc::O_CLOEXEC | libc::O_NOFOLLOW) as u64,
        RESOLVE_BENEATH | RESOLVE_NO_MAGICLINKS | RESOLVE_NO_XDEV,
    )?;
    let stat = fstat(fd.as_raw_fd(), &name.to_string_lossy())?;
    let opaque = opaque_handle_at(parent, name)?;
    Ok(ObjectIdentity {
        device: stat.st_dev,
        inode: stat.st_ino,
        owner_uid: stat.st_uid,
        owner_gid: stat.st_gid,
        mode: stat.st_mode & 0o7777,
        opaque,
    })
}

pub(super) fn identity_optional_at(
    parent: RawFd,
    name: &CStr,
) -> Result<Option<ObjectIdentity>, FsTxError> {
    match identity_at(parent, name) {
        Ok(identity) => Ok(Some(identity)),
        Err(FsTxError::Io { source, .. }) if source.raw_os_error() == Some(libc::ENOENT) => {
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

pub(super) fn opaque_handle_at(parent: RawFd, name: &CStr) -> Result<OpaqueHandle, FsTxError> {
    let header = std::mem::size_of::<u32>() + std::mem::size_of::<i32>();
    let mut storage = vec![0_u8; header + MAX_OPAQUE_HANDLE_BYTES];
    storage[..4].copy_from_slice(
        &u32::try_from(MAX_OPAQUE_HANDLE_BYTES)
            .expect("opaque-handle bound fits u32")
            .to_ne_bytes(),
    );
    let mut mount_id = 0_i32;
    let result = unsafe {
        libc::name_to_handle_at(
            parent,
            name.as_ptr(),
            storage.as_mut_ptr().cast::<libc::file_handle>(),
            &mut mount_id,
            0,
        )
    };
    if result != 0 {
        let error = io::Error::last_os_error();
        return Err(FsTxError::CapabilityUnavailable {
            capability: "name_to_handle_at opaque handles",
            detail: format!("{}: {error}", name.to_string_lossy()),
        });
    }
    let length = u32::from_ne_bytes(storage[..4].try_into().expect("four bytes")) as usize;
    let handle_type = i32::from_ne_bytes(storage[4..8].try_into().expect("four bytes"));
    OpaqueHandle::new(
        mount_id,
        handle_type,
        storage[header..header + length].to_vec(),
    )
}

pub(super) fn probe_open_by_handle(
    mount_fd: RawFd,
    identity: &ObjectIdentity,
) -> Result<(), FsTxError> {
    let opaque = &identity.opaque;
    let header = 8;
    let mut storage = vec![0_u8; header + opaque.bytes.len()];
    storage[..4].copy_from_slice(
        &u32::try_from(opaque.bytes.len())
            .expect("validated opaque-handle length fits u32")
            .to_ne_bytes(),
    );
    storage[4..8].copy_from_slice(&opaque.handle_type.to_ne_bytes());
    storage[8..].copy_from_slice(&opaque.bytes);
    let raw = unsafe {
        libc::open_by_handle_at(
            mount_fd,
            storage.as_mut_ptr().cast::<libc::file_handle>(),
            libc::O_PATH | libc::O_CLOEXEC,
        )
    };
    let fd = owned(raw, "open_by_handle_at", "opaque handle").map_err(|error| {
        FsTxError::CapabilityUnavailable {
            capability: "open_by_handle_at opaque handle reopen",
            detail: error.to_string(),
        }
    })?;
    let stat = fstat(fd.as_raw_fd(), "opaque handle")?;
    if stat.st_dev != identity.device || stat.st_ino != identity.inode {
        return Err(FsTxError::CapabilityUnavailable {
            capability: "opaque handle identity round-trip",
            detail: "reopened handle did not identify the expected inode".into(),
        });
    }
    Ok(())
}

pub(super) fn delete_contents(fd: RawFd, depth: usize, count: &mut usize) -> Result<(), FsTxError> {
    if depth > MAX_DELETE_DEPTH {
        return Err(FsTxError::DeleteLimit {
            limit_name: "depth",
            limit: MAX_DELETE_DEPTH,
        });
    }
    let directory = fs::read_dir(format!("/proc/self/fd/{fd}"))
        .map_err(|error| io_error("enumerate quarantined directory", "proc fd", error))?;
    for entry in directory {
        let entry = entry.map_err(|error| io_error("read quarantined entry", "proc fd", error))?;
        *count += 1;
        if *count > MAX_DELETE_ENTRIES {
            return Err(FsTxError::DeleteLimit {
                limit_name: "entries",
                limit: MAX_DELETE_ENTRIES,
            });
        }
        let name = entry.file_name();
        let c_name = cstring_os(&name)?;
        let child = openat2(
            fd,
            &c_name,
            (libc::O_PATH | libc::O_CLOEXEC | libc::O_NOFOLLOW) as u64,
            RESOLVE_BENEATH | RESOLVE_NO_MAGICLINKS | RESOLVE_NO_XDEV,
        )?;
        let stat = fstat(child.as_raw_fd(), &name.to_string_lossy())?;
        if stat.st_mode & libc::S_IFMT == libc::S_IFDIR {
            let directory_fd = open_directory_at(fd, &name)?;
            delete_contents(directory_fd.as_raw_fd(), depth + 1, count)?;
            unlinkat_dir(fd, &c_name, &name.to_string_lossy())?;
        } else {
            let result = unsafe { libc::unlinkat(fd, c_name.as_ptr(), 0) };
            if result != 0 {
                return Err(io_error(
                    "unlinkat",
                    &name.to_string_lossy(),
                    io::Error::last_os_error(),
                ));
            }
        }
    }
    sync_fd(fd, "fsync deleted directory contents", "quarantine")
}

pub(super) fn chown_mode(
    fd: RawFd,
    uid: u32,
    gid: u32,
    mode: u32,
    path: &str,
) -> Result<(), FsTxError> {
    if unsafe { libc::fchown(fd, uid, gid) } != 0 {
        return Err(io_error("fchown", path, io::Error::last_os_error()));
    }
    if unsafe { libc::fchmod(fd, mode) } != 0 {
        return Err(io_error("fchmod", path, io::Error::last_os_error()));
    }
    Ok(())
}

pub(super) fn mkdirat(
    parent: RawFd,
    name: &CStr,
    mode: u32,
    display: &str,
) -> Result<(), FsTxError> {
    if unsafe { libc::mkdirat(parent, name.as_ptr(), mode) } == 0 {
        return Ok(());
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::EEXIST) {
        Err(FsTxError::Collision(display.into()))
    } else {
        Err(io_error("mkdirat", display, error))
    }
}

pub(super) fn unlinkat_dir(parent: RawFd, name: &CStr, display: &str) -> Result<(), FsTxError> {
    if unsafe { libc::unlinkat(parent, name.as_ptr(), libc::AT_REMOVEDIR) } == 0 {
        Ok(())
    } else {
        Err(io_error(
            "unlinkat(AT_REMOVEDIR)",
            display,
            io::Error::last_os_error(),
        ))
    }
}

pub(super) fn validate_directory(
    name: &str,
    stat: &libc::stat,
    uid: u32,
    gid: Option<u32>,
    mode: Option<u32>,
) -> Result<(), FsTxError> {
    if stat.st_mode & libc::S_IFMT != libc::S_IFDIR {
        return Err(FsTxError::InvalidSpec(format!("{name} is not a directory")));
    }
    if stat.st_uid != uid || gid.is_some_and(|value| stat.st_gid != value) {
        return Err(FsTxError::InvalidSpec(format!(
            "{name} ownership does not match the broker identity"
        )));
    }
    if mode.is_some_and(|value| stat.st_mode & 0o7777 != value) {
        return Err(FsTxError::InvalidSpec(format!(
            "{name} mode does not match configuration"
        )));
    }
    Ok(())
}

pub(super) fn fstat(fd: RawFd, path: &str) -> Result<libc::stat, FsTxError> {
    let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
    if unsafe { libc::fstat(fd, stat.as_mut_ptr()) } != 0 {
        return Err(io_error("fstat", path, io::Error::last_os_error()));
    }
    Ok(unsafe { stat.assume_init() })
}

pub(super) fn sync_fd(fd: RawFd, operation: &'static str, path: &str) -> Result<(), FsTxError> {
    if unsafe { libc::fsync(fd) } == 0 {
        Ok(())
    } else {
        Err(io_error(operation, path, io::Error::last_os_error()))
    }
}

pub(super) fn cstring(value: &str) -> Result<CString, FsTxError> {
    CString::new(value).map_err(|_| FsTxError::InvalidSpec("path contains NUL".into()))
}

pub(super) fn cstring_os(value: &OsStr) -> Result<CString, FsTxError> {
    CString::new(value.as_bytes()).map_err(|_| FsTxError::InvalidSpec("path contains NUL".into()))
}

pub(super) fn cstr_dot() -> &'static CStr {
    c"."
}

pub(super) fn owned(raw: i32, operation: &'static str, path: &str) -> Result<OwnedFd, FsTxError> {
    if raw < 0 {
        Err(io_error(operation, path, io::Error::last_os_error()))
    } else {
        Ok(unsafe { OwnedFd::from_raw_fd(raw) })
    }
}

pub(super) fn io_error(operation: &'static str, path: &str, source: io::Error) -> FsTxError {
    FsTxError::Io {
        operation,
        path: path.into(),
        source,
    }
}

pub(super) fn mismatch(
    path: &str,
    expected: ObjectIdentity,
    observed: ObjectIdentity,
    disposition: MismatchDisposition,
) -> FsTxError {
    FsTxError::IdentityMismatch {
        path: path.into(),
        expected: Box::new(expected),
        observed: Box::new(observed),
        disposition,
    }
}
