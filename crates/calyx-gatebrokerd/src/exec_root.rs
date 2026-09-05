//! Descriptor-relative, read-only working-directory capabilities.

use std::ffi::{CString, OsStr};
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;

use thiserror::Error;

use crate::protocol::{ExecutionRootAlias, RelativePath};

const RESOLVE_NO_XDEV: u64 = 0x01;
const RESOLVE_NO_MAGICLINKS: u64 = 0x02;
const RESOLVE_NO_SYMLINKS: u64 = 0x04;
const RESOLVE_BENEATH: u64 = 0x08;

#[repr(C)]
struct OpenHow {
    flags: u64,
    mode: u64,
    resolve: u64,
}

#[derive(Debug)]
pub struct ExecutionRoot {
    alias: ExecutionRootAlias,
    path: PathBuf,
    expected_uid: u32,
    expected_mode: u32,
    fd: OwnedFd,
}

#[derive(Debug)]
pub struct ResolvedExecutionDirectory {
    path: PathBuf,
    fd: OwnedFd,
    device: u64,
    inode: u64,
}

#[derive(Debug, Error)]
pub enum ExecutionRootError {
    #[error("invalid execution root: {0}")]
    Invalid(String),
    #[error("execution-root operation {operation} failed for {path}: {source}")]
    Io {
        operation: &'static str,
        path: String,
        #[source]
        source: io::Error,
    },
    #[error("required openat2 execution-root authority is unavailable: {0}")]
    Capability(String),
}

impl ExecutionRoot {
    pub fn open(
        alias: ExecutionRootAlias,
        path: PathBuf,
        expected_uid: u32,
        expected_mode: u32,
    ) -> Result<Self, ExecutionRootError> {
        if !path.is_absolute() || path.parent().is_none() {
            return Err(ExecutionRootError::Invalid(format!(
                "{} is not a normalized absolute directory",
                path.display()
            )));
        }
        let slash = c"/";
        let raw = unsafe {
            libc::open(
                slash.as_ptr(),
                libc::O_PATH | libc::O_DIRECTORY | libc::O_CLOEXEC,
            )
        };
        let root = owned(raw, "open filesystem root", "/")?;
        let relative = path
            .strip_prefix("/")
            .map_err(|_| ExecutionRootError::Invalid(path.display().to_string()))?;
        let fd = open_directory(root.as_raw_fd(), relative.as_os_str())?;
        let stat = fstat(fd.as_raw_fd(), &path.display().to_string())?;
        if stat.st_mode & libc::S_IFMT != libc::S_IFDIR
            || stat.st_uid != expected_uid
            || stat.st_mode & 0o7777 != expected_mode
        {
            return Err(ExecutionRootError::Invalid(format!(
                "{} must be a directory with uid={} mode={expected_mode:04o}; actual uid={} mode={:04o}",
                path.display(),
                expected_uid,
                stat.st_uid,
                stat.st_mode & 0o7777
            )));
        }
        // A second open with NO_XDEV proves the kernel supports every resolve
        // flag the broker relies on. The configured root itself may cross a
        // mount while being opened from `/`; descendants may not cross again.
        let _ = openat2(
            fd.as_raw_fd(),
            c".",
            (libc::O_PATH | libc::O_DIRECTORY | libc::O_CLOEXEC) as u64,
            RESOLVE_BENEATH | RESOLVE_NO_SYMLINKS | RESOLVE_NO_MAGICLINKS | RESOLVE_NO_XDEV,
        )?;
        Ok(Self {
            alias,
            path,
            expected_uid,
            expected_mode,
            fd,
        })
    }

    pub fn alias(&self) -> &ExecutionRootAlias {
        &self.alias
    }

    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    pub fn expected_uid(&self) -> u32 {
        self.expected_uid
    }

    pub fn expected_mode(&self) -> u32 {
        self.expected_mode
    }

    pub fn resolve(
        &self,
        relative: &RelativePath,
    ) -> Result<ResolvedExecutionDirectory, ExecutionRootError> {
        let name = CString::new(relative.as_str())
            .map_err(|_| ExecutionRootError::Invalid("cwd contains NUL".into()))?;
        let fd = openat2(
            self.fd.as_raw_fd(),
            &name,
            (libc::O_PATH | libc::O_DIRECTORY | libc::O_CLOEXEC) as u64,
            RESOLVE_BENEATH | RESOLVE_NO_SYMLINKS | RESOLVE_NO_MAGICLINKS | RESOLVE_NO_XDEV,
        )?;
        let stat = fstat(fd.as_raw_fd(), relative.as_str())?;
        if stat.st_mode & libc::S_IFMT != libc::S_IFDIR {
            return Err(ExecutionRootError::Invalid(format!(
                "{} beneath {} is not a directory",
                relative,
                self.path.display()
            )));
        }
        let path = if relative.as_str() == "." {
            self.path.clone()
        } else {
            self.path.join(relative.as_str())
        };
        Ok(ResolvedExecutionDirectory {
            path,
            fd,
            device: stat.st_dev,
            inode: stat.st_ino,
        })
    }
}

impl ResolvedExecutionDirectory {
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    pub fn device(&self) -> u64 {
        self.device
    }

    pub fn inode(&self) -> u64 {
        self.inode
    }

    pub fn raw_fd(&self) -> RawFd {
        self.fd.as_raw_fd()
    }
}

fn open_directory(parent: RawFd, path: &OsStr) -> Result<OwnedFd, ExecutionRootError> {
    let path = CString::new(path.as_bytes())
        .map_err(|_| ExecutionRootError::Invalid("execution root contains NUL".into()))?;
    openat2(
        parent,
        &path,
        (libc::O_PATH | libc::O_DIRECTORY | libc::O_CLOEXEC) as u64,
        RESOLVE_BENEATH | RESOLVE_NO_SYMLINKS | RESOLVE_NO_MAGICLINKS,
    )
}

fn openat2(
    parent: RawFd,
    name: &std::ffi::CStr,
    flags: u64,
    resolve: u64,
) -> Result<OwnedFd, ExecutionRootError> {
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
    if raw >= 0 {
        return Ok(unsafe { OwnedFd::from_raw_fd(raw) });
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ENOSYS) {
        Err(ExecutionRootError::Capability(error.to_string()))
    } else {
        Err(io_error("openat2", &name.to_string_lossy(), error))
    }
}

fn fstat(fd: RawFd, path: &str) -> Result<libc::stat, ExecutionRootError> {
    let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
    if unsafe { libc::fstat(fd, stat.as_mut_ptr()) } != 0 {
        return Err(io_error("fstat", path, io::Error::last_os_error()));
    }
    Ok(unsafe { stat.assume_init() })
}

fn owned(raw: RawFd, operation: &'static str, path: &str) -> Result<OwnedFd, ExecutionRootError> {
    if raw < 0 {
        Err(io_error(operation, path, io::Error::last_os_error()))
    } else {
        Ok(unsafe { OwnedFd::from_raw_fd(raw) })
    }
}

fn io_error(operation: &'static str, path: &str, source: io::Error) -> ExecutionRootError {
    ExecutionRootError::Io {
        operation,
        path: path.into(),
        source,
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};

    use super::*;

    #[test]
    fn resolves_real_directory_and_rejects_symlink_escape() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::set_permissions(temp.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::create_dir(temp.path().join("real")).unwrap();
        symlink("/", temp.path().join("escape")).unwrap();
        let metadata = temp.path().metadata().unwrap();
        let root = ExecutionRoot::open(
            ExecutionRootAlias::new("test").unwrap(),
            temp.path().to_path_buf(),
            metadata.uid(),
            0o700,
        )
        .unwrap();
        let resolved = root.resolve(&RelativePath::new("real").unwrap()).unwrap();
        let observed = std::fs::metadata(resolved.path()).unwrap();
        assert_eq!(
            (resolved.device(), resolved.inode()),
            (observed.dev(), observed.ino())
        );
        assert!(root.resolve(&RelativePath::new("escape").unwrap()).is_err());
    }
}
