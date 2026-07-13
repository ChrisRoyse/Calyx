use std::ffi::CString;
use std::fs::{self, File};
use std::io::Read;
use std::os::fd::FromRawFd;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Component, Path, PathBuf};

use crate::accounts::{Account, lookup_account, process_identity};
use crate::broker_error::BrokerError;
use crate::config::{BrokerConfig, ValidatedConfig, validate};
use crate::protocol::{AbsolutePath, StableCode};

const MAX_CONFIG_BYTES: u64 = 1024 * 1024;

pub fn load_config(path: &Path) -> Result<ValidatedConfig, BrokerError> {
    // Establish the process identity before consulting a caller-selected
    // pathname. This deliberately rejects setuid-style invocation as well as
    // an unprivileged foreground process.
    require_root_broker()?;
    validate_normalized_absolute_path(path, "configuration path")?;
    let parent = path.parent().ok_or_else(|| {
        config_error(format!(
            "configuration path has no parent: {}",
            path.display()
        ))
    })?;
    validate_root_owned_chain(parent)?;

    let mut file = std::fs::OpenOptions::new()
        .read(true)
        // O_NONBLOCK prevents a malicious special file from hanging startup
        // before fstat can reject it. It has no effect on a regular file.
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_NOCTTY)
        .open(path)
        .map_err(|error| config_error(format!("open {}: {error}", path.display())))?;
    let metadata = file
        .metadata()
        .map_err(|error| config_error(format!("fstat {}: {error}", path.display())))?;
    if !metadata.is_file()
        || metadata.nlink() != 1
        || metadata.uid() != 0
        || metadata.gid() != 0
        || metadata.mode() & 0o7777 != 0o600
    {
        return Err(config_error(format!(
            "{} must be a regular, non-hardlinked root:root file with exact mode 0600; actual uid:gid={}:{} mode={:04o} links={}",
            path.display(),
            metadata.uid(),
            metadata.gid(),
            metadata.mode() & 0o7777,
            metadata.nlink()
        )));
    }
    if metadata.len() > MAX_CONFIG_BYTES {
        return Err(config_error(format!(
            "{} is {} bytes; maximum configuration size is {MAX_CONFIG_BYTES} bytes",
            path.display(),
            metadata.len()
        )));
    }

    let mut text = String::new();
    file.by_ref()
        .take(MAX_CONFIG_BYTES + 1)
        .read_to_string(&mut text)
        .map_err(|error| config_error(format!("read {}: {error}", path.display())))?;
    if text.len() as u64 > MAX_CONFIG_BYTES {
        return Err(config_error(format!(
            "{} grew beyond the {MAX_CONFIG_BYTES}-byte configuration limit while it was read",
            path.display()
        )));
    }

    // The root-owned, non-writable parent chain excludes unprivileged rename
    // races. Comparing the opened inode with a fresh pathname lookup also
    // makes a deployment-time replacement by a privileged actor explicit.
    let path_metadata = path
        .symlink_metadata()
        .map_err(|error| config_error(format!("re-lstat {}: {error}", path.display())))?;
    if path_metadata.file_type().is_symlink()
        || path_metadata.dev() != metadata.dev()
        || path_metadata.ino() != metadata.ino()
    {
        return Err(config_error(format!(
            "configuration pathname {} changed while it was being verified; opened dev:ino={}:{} current dev:ino={}:{}",
            path.display(),
            metadata.dev(),
            metadata.ino(),
            path_metadata.dev(),
            path_metadata.ino()
        )));
    }

    let raw: BrokerConfig = toml::from_str(&text)
        .map_err(|error| config_error(format!("parse {}: {error}", path.display())))?;
    validate(raw).map_err(|error| config_error(error.to_string()))
}

pub(super) fn require_root_broker() -> Result<Account, BrokerError> {
    let (mut real_uid, mut effective_uid, mut saved_uid) = (u32::MAX, u32::MAX, u32::MAX);
    let (mut real_gid, mut effective_gid, mut saved_gid) = (u32::MAX, u32::MAX, u32::MAX);
    if unsafe { libc::getresuid(&mut real_uid, &mut effective_uid, &mut saved_uid) } != 0 {
        return Err(BrokerError::system(
            "read broker uid identity",
            std::io::Error::last_os_error(),
        ));
    }
    if unsafe { libc::getresgid(&mut real_gid, &mut effective_gid, &mut saved_gid) } != 0 {
        return Err(BrokerError::system(
            "read broker gid identity",
            std::io::Error::last_os_error(),
        ));
    }
    if (real_uid, effective_uid, saved_uid) != (0, 0, 0)
        || (real_gid, effective_gid, saved_gid) != (0, 0, 0)
    {
        return Err(BrokerError::new(
            StableCode::PermissionDenied,
            format!(
                "broker must run with real/effective/saved uid and gid 0; actual uid={real_uid}/{effective_uid}/{saved_uid} gid={real_gid}/{effective_gid}/{saved_gid}"
            ),
            "Start the checked-in system service; do not grant setuid or file capabilities to the binary.",
        ));
    }
    // /proc exposes the fourth Linux credential slot (filesystem uid/gid),
    // which getresuid/getresgid do not. accounts::process_identity rejects the
    // status record unless all four values agree.
    let identity = process_identity(std::process::id())
        .map_err(|error| BrokerError::system("verify broker filesystem uid/gid identity", error))?;
    if identity.uid != 0 || identity.gid != 0 {
        return Err(BrokerError::new(
            StableCode::PermissionDenied,
            format!(
                "broker filesystem uid/gid must be 0/0; actual={}/{}",
                identity.uid, identity.gid
            ),
            "Start the checked-in system service with real/effective/saved/filesystem uid and gid all set to root.",
        ));
    }
    let account = lookup_account("root").map_err(|error| config_error(error.to_string()))?;
    if account.uid != 0 || account.gid != 0 {
        return Err(config_error("root account does not resolve to uid/gid 0/0"));
    }
    Ok(account)
}

pub(super) fn validate_state_paths(config: &ValidatedConfig) -> Result<(), BrokerError> {
    let state = config.state();
    let raw = state.raw();
    validate_root_owned_chain(&raw.anchor)?;
    validate_root_owned_chain(&raw.private)?;
    validate_root_owned_chain(&raw.journal_directory)?;
    validate_exact_directory(&raw.anchor, 0, 0, state.anchor_mode(), "state.anchor")?;
    validate_exact_directory(&raw.private, 0, 0, state.private_mode(), "state.private")?;
    validate_exact_directory(
        &raw.journal_directory,
        0,
        0,
        state.journal_directory_mode(),
        "state.journal_directory",
    )?;
    Ok(())
}

pub(super) fn validate_root_owned_chain(path: &Path) -> Result<(), BrokerError> {
    validate_normalized_absolute_path(path, "authority path")?;
    let mut current = PathBuf::from("/");
    validate_root_owned_component(&current, path)?;
    for component in path.components() {
        match component {
            Component::RootDir => continue,
            Component::Normal(value) => current.push(value),
            _ => {
                return Err(config_error(format!(
                    "{} contains a non-normal path component",
                    path.display()
                )));
            }
        }
        validate_root_owned_component(&current, path)?;
    }
    Ok(())
}

fn validate_root_owned_component(current: &Path, authority: &Path) -> Result<(), BrokerError> {
    let metadata = current
        .symlink_metadata()
        .map_err(|error| config_error(format!("lstat {}: {error}", current.display())))?;
    if !metadata.is_dir()
        || metadata.file_type().is_symlink()
        || metadata.uid() != 0
        || metadata.gid() != 0
        || metadata.mode() & 0o022 != 0
    {
        return Err(config_error(format!(
            "authority path {} has unsafe component {}: expected a real root:root directory without group/other write bits; actual uid:gid={}:{} mode={:04o}",
            authority.display(),
            current.display(),
            metadata.uid(),
            metadata.gid(),
            metadata.mode() & 0o7777
        )));
    }
    Ok(())
}

fn validate_normalized_absolute_path(path: &Path, field: &str) -> Result<(), BrokerError> {
    let value = path
        .to_str()
        .ok_or_else(|| config_error(format!("{field} must be valid UTF-8: {}", path.display())))?;
    if !value.starts_with('/')
        || value.contains("//")
        || (value.len() > 1 && value.ends_with('/'))
        || value
            .bytes()
            .any(|byte| byte.is_ascii_control() || byte == 0x7f)
        || value
            .split('/')
            .skip(1)
            .any(|component| matches!(component, "." | ".."))
    {
        return Err(config_error(format!(
            "{field} must be a normalized absolute path without traversal: {}",
            path.display()
        )));
    }
    Ok(())
}

pub(super) fn validate_exact_directory(
    path: &Path,
    uid: u32,
    gid: u32,
    mode: u32,
    field: &str,
) -> Result<(), BrokerError> {
    let metadata = path
        .symlink_metadata()
        .map_err(|error| config_error(format!("lstat {}: {error}", path.display())))?;
    if !metadata.is_dir()
        || metadata.file_type().is_symlink()
        || metadata.uid() != uid
        || metadata.gid() != gid
        || metadata.mode() & 0o7777 != mode
    {
        return Err(config_error(format!(
            "{field}={} must be a real directory uid:gid={uid}:{gid} mode={mode:04o}; actual uid:gid={}:{} mode={:04o}",
            path.display(),
            metadata.uid(),
            metadata.gid(),
            metadata.mode() & 0o7777
        )));
    }
    Ok(())
}

pub(super) fn broker_cgroup() -> Result<AbsolutePath, BrokerError> {
    let raw = fs::read_to_string("/proc/self/cgroup")
        .map_err(|error| BrokerError::system("read broker cgroup", error))?;
    let lines = raw
        .lines()
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    let value = match lines.as_slice() {
        [line] => line.strip_prefix("0::").ok_or_else(|| {
            BrokerError::new(
                StableCode::CapabilityUnavailable,
                format!("broker cgroup record is not unified cgroup v2: {line:?}"),
                "Boot with cgroup v2 and start calyx-gatebrokerd through the system manager.",
            )
        })?,
        _ => {
            return Err(BrokerError::new(
                StableCode::CapabilityUnavailable,
                format!(
                    "broker must have exactly one unified cgroup v2 membership record; actual record count={}",
                    lines.len()
                ),
                "Boot with a pure unified cgroup v2 hierarchy; hybrid and legacy hierarchies are unsupported.",
            ));
        }
    };
    let cgroup = AbsolutePath::new(value)
        .map_err(|error| BrokerError::system("parse broker cgroup", error))?;

    let mount = c"/sys/fs/cgroup";
    let mut stat = std::mem::MaybeUninit::<libc::statfs>::uninit();
    if unsafe { libc::statfs(mount.as_ptr(), stat.as_mut_ptr()) } != 0 {
        return Err(BrokerError::system(
            "statfs cgroup hierarchy",
            std::io::Error::last_os_error(),
        ));
    }
    let stat = unsafe { stat.assume_init() };
    if stat.f_type != libc::CGROUP2_SUPER_MAGIC {
        return Err(BrokerError::new(
            StableCode::CapabilityUnavailable,
            format!(
                "/sys/fs/cgroup is not a cgroup v2 filesystem; f_type={:#x}",
                stat.f_type
            ),
            "Mount the unified cgroup v2 hierarchy at /sys/fs/cgroup before starting the broker.",
        ));
    }
    let membership_path = if value == "/" {
        PathBuf::from("/sys/fs/cgroup")
    } else {
        PathBuf::from(format!("/sys/fs/cgroup{value}"))
    };
    let metadata = membership_path.symlink_metadata().map_err(|error| {
        BrokerError::system(
            "inspect broker cgroup membership directory",
            format!("{}: {error}", membership_path.display()),
        )
    })?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(BrokerError::new(
            StableCode::CapabilityUnavailable,
            format!(
                "broker cgroup membership is not a real directory: {}",
                membership_path.display()
            ),
            "Start the broker as a system service in the mounted unified cgroup v2 hierarchy.",
        ));
    }
    Ok(cgroup)
}

pub(super) fn validate_output_fd(fd: i32, label: &str) -> Result<(), BrokerError> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 {
        return Err(BrokerError::invalid(format!(
            "{label} descriptor is invalid: {}",
            std::io::Error::last_os_error()
        )));
    }
    if matches!(flags & libc::O_ACCMODE, libc::O_RDONLY) {
        return Err(BrokerError::invalid(format!(
            "{label} descriptor is not writable"
        )));
    }
    let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
    if unsafe { libc::fstat(fd, stat.as_mut_ptr()) } != 0 {
        return Err(BrokerError::invalid(format!(
            "fstat {label} descriptor: {}",
            std::io::Error::last_os_error()
        )));
    }
    let stat = unsafe { stat.assume_init() };
    if stat.st_mode & libc::S_IFMT == libc::S_IFDIR {
        return Err(BrokerError::invalid(format!(
            "{label} descriptor must not be a directory"
        )));
    }
    Ok(())
}

pub(super) fn require_absolute_regular_executable(path: &Path) -> Result<(), BrokerError> {
    if !path.is_absolute() {
        return Err(BrokerError::invalid(
            "argv[0] must be an absolute executable path",
        ));
    }
    let path_c = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| BrokerError::invalid("argv[0] contains NUL"))?;
    let fd = unsafe {
        libc::open(
            path_c.as_ptr(),
            libc::O_PATH | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(BrokerError::invalid(format!(
            "open executable {}: {}",
            path.display(),
            std::io::Error::last_os_error()
        )));
    }
    let file = unsafe { File::from_raw_fd(fd) };
    let metadata = file
        .metadata()
        .map_err(|error| BrokerError::invalid(format!("fstat executable: {error}")))?;
    if !metadata.is_file() || metadata.mode() & 0o111 == 0 {
        return Err(BrokerError::invalid(format!(
            "{} is not an executable regular file",
            path.display()
        )));
    }
    Ok(())
}

fn config_error(message: impl Into<String>) -> BrokerError {
    BrokerError::new(
        StableCode::ConfigInvalid,
        message,
        "Correct the root-owned configuration or filesystem policy; no compatibility fallback is available.",
    )
}

#[cfg(test)]
mod tests {
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

    use super::*;

    #[test]
    fn authority_paths_are_strictly_normalized() {
        validate_normalized_absolute_path(Path::new("/etc/calyx-gatebrokerd/config.toml"), "test")
            .unwrap();
        for invalid in [
            "etc/calyx-gatebrokerd/config.toml",
            "/etc//calyx-gatebrokerd/config.toml",
            "/etc/calyx-gatebrokerd/../config.toml",
            "/etc/calyx-gatebrokerd/./config.toml",
            "/etc/calyx-gatebrokerd/",
            "/etc/calyx-gatebrokerd/config\n.toml",
        ] {
            assert_eq!(
                validate_normalized_absolute_path(Path::new(invalid), "test")
                    .unwrap_err()
                    .code,
                StableCode::ConfigInvalid
            );
        }
    }

    #[test]
    fn output_fd_policy_reads_actual_kernel_descriptor_state() {
        let mut descriptors = [-1; 2];
        assert_eq!(
            unsafe { libc::pipe2(descriptors.as_mut_ptr(), libc::O_CLOEXEC) },
            0
        );
        let read_end = unsafe { OwnedFd::from_raw_fd(descriptors[0]) };
        let write_end = unsafe { OwnedFd::from_raw_fd(descriptors[1]) };
        assert_eq!(
            validate_output_fd(read_end.as_raw_fd(), "read end")
                .unwrap_err()
                .code,
            StableCode::InvalidRequest
        );
        validate_output_fd(write_end.as_raw_fd(), "write end").unwrap();
    }

    #[test]
    fn broker_cgroup_resolves_to_the_live_unified_hierarchy() {
        let cgroup = broker_cgroup().unwrap();
        let path = if cgroup.as_str() == "/" {
            PathBuf::from("/sys/fs/cgroup")
        } else {
            PathBuf::from(format!("/sys/fs/cgroup{}", cgroup.as_str()))
        };
        let metadata = path.symlink_metadata().unwrap();
        assert!(metadata.is_dir());
        assert!(!metadata.file_type().is_symlink());
    }
}
