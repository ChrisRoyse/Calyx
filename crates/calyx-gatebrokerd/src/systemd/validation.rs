use super::*;

pub(super) fn io_error(operation: &'static str, source: io::Error) -> SystemdError {
    SystemdError::Io { operation, source }
}

pub(super) fn valid_unit_component(value: &str, suffix: &str) -> bool {
    value.ends_with(suffix)
        && value.len() <= 240
        && !value.starts_with('.')
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

pub(super) fn normalized_absolute(path: &Path) -> bool {
    path.is_absolute()
        && path.parent().is_some()
        && path
            .components()
            .all(|component| matches!(component, Component::RootDir | Component::Normal(_)))
        && path.to_str().is_some()
}

pub(super) fn normalized_relative(path: &Path) -> bool {
    if path == Path::new(".") {
        return true;
    }
    !path.as_os_str().is_empty()
        && !path.is_absolute()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
        && path.to_str().is_some()
}

pub(super) fn is_exact_published_object(path: &Path) -> bool {
    let Ok(relative) = path.strip_prefix(OBJECT_ROOT) else {
        return false;
    };
    let components: Vec<_> = relative.components().collect();
    components.len() == 2
        && matches!(components[0], Component::Normal(name) if name == "tmp" || name == "target")
        && matches!(components[1], Component::Normal(_))
}

pub(super) fn reserved_environment_name(name: &[u8]) -> bool {
    matches!(
        name,
        b"PATH"
            | b"HOME"
            | b"USER"
            | b"LOGNAME"
            | b"SHELL"
            | b"LANG"
            | b"LC_ALL"
            | b"XDG_RUNTIME_DIR"
            | b"NOTIFY_SOCKET"
            | b"INVOCATION_ID"
            | b"JOURNAL_STREAM"
            | b"GLIBC_TUNABLES"
            | b"GCONV_PATH"
            | b"LOCPATH"
            | b"NLSPATH"
            | b"HOSTALIASES"
            | b"RES_OPTIONS"
            | b"LOCALDOMAIN"
    ) || name.starts_with(b"LD_")
        || name.starts_with(b"DYLD_")
        || name.starts_with(b"DBUS_")
        || name.starts_with(b"LISTEN_")
        || name.starts_with(b"SYSTEMD_")
}

pub(super) fn validate_spec(spec: &StageSpec) -> Result<(), SystemdError> {
    if !valid_unit_component(&spec.unit_name, ".service") {
        return Err(SystemdError::InvalidSpec(
            "invalid service unit name".into(),
        ));
    }
    if !spec
        .worker_user
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        || spec.worker_user.is_empty()
    {
        return Err(SystemdError::InvalidSpec(
            "invalid worker account name".into(),
        ));
    }
    if spec.worker_uid == 0 {
        return Err(SystemdError::InvalidSpec(
            "worker uid must be nonzero".into(),
        ));
    }
    if !normalized_absolute(&spec.execution_root) {
        return Err(SystemdError::InvalidSpec(
            "execution root must be a normalized UTF-8 absolute path".into(),
        ));
    }
    if !normalized_relative(&spec.relative_cwd) {
        return Err(SystemdError::InvalidSpec(
            "relative cwd must be traversal-free UTF-8".into(),
        ));
    }
    if spec.execution_root_mode > 0o7777 {
        return Err(SystemdError::InvalidSpec(
            "invalid execution-root mode".into(),
        ));
    }
    if spec.cwd_fd < 0 {
        return Err(SystemdError::InvalidSpec(
            "cwd descriptor is invalid".into(),
        ));
    }
    if spec.argv.is_empty() || spec.argv[0].as_os_str().as_bytes().first() != Some(&b'/') {
        return Err(SystemdError::InvalidSpec(
            "argv[0] must be an absolute executable path".into(),
        ));
    }
    let mut argv_bytes = 0_usize;
    for value in &spec.argv {
        let bytes = value.as_os_str().as_bytes();
        if bytes.contains(&0) {
            return Err(SystemdError::InvalidSpec("argv contains NUL".into()));
        }
        argv_bytes = argv_bytes.saturating_add(bytes.len());
    }
    if argv_bytes > 65_536 {
        return Err(SystemdError::InvalidSpec("argv exceeds 65536 bytes".into()));
    }
    let mut names = BTreeSet::new();
    for (name, value) in &spec.environment {
        let name = name.as_os_str().as_bytes();
        let value = value.as_os_str().as_bytes();
        let portable = name
            .first()
            .is_some_and(|byte| byte.is_ascii_alphabetic() || *byte == b'_')
            && name[1..]
                .iter()
                .all(|byte| byte.is_ascii_alphanumeric() || *byte == b'_');
        if !portable || value.contains(&0) || reserved_environment_name(name) {
            return Err(SystemdError::InvalidSpec(format!(
                "unsafe environment assignment: {}",
                String::from_utf8_lossy(name)
            )));
        }
        if !names.insert(name.to_vec()) {
            return Err(SystemdError::InvalidSpec(
                "duplicate environment name".into(),
            ));
        }
    }
    for path in &spec.writable_paths {
        if !normalized_absolute(path)
            || !is_exact_published_object(path)
            || path.starts_with(PRIVATE_STATE_ROOT)
        {
            return Err(SystemdError::InvalidSpec(format!(
                "writable path is not an exact published object: {}",
                path.display()
            )));
        }
    }
    Ok(())
}

pub(super) fn duplicate_fd(fd: RawFd, operation: &'static str) -> Result<OwnedFd, SystemdError> {
    let duplicate = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 3) };
    if duplicate < 0 {
        Err(io_error(operation, io::Error::last_os_error()))
    } else {
        Ok(unsafe { OwnedFd::from_raw_fd(duplicate) })
    }
}

pub(super) fn fstat(fd: RawFd, operation: &'static str) -> Result<libc::stat, SystemdError> {
    let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
    if unsafe { libc::fstat(fd, stat.as_mut_ptr()) } != 0 {
        Err(io_error(operation, io::Error::last_os_error()))
    } else {
        Ok(unsafe { stat.assume_init() })
    }
}

pub(super) fn trusted_executable(
    path: &'static str,
    exact_shim: bool,
) -> Result<TrustedIdentity, SystemdError> {
    let path_c = CString::new(path).expect("static executable path");
    let flags = if exact_shim {
        libc::O_PATH | libc::O_CLOEXEC | libc::O_NOFOLLOW
    } else {
        libc::O_PATH | libc::O_CLOEXEC
    };
    let raw = unsafe { libc::open(path_c.as_ptr(), flags) };
    if raw < 0 {
        return Err(SystemdError::ExecutablePolicy {
            path,
            detail: io::Error::last_os_error().to_string(),
        });
    }
    let fd = unsafe { OwnedFd::from_raw_fd(raw) };
    let stat = fstat(fd.as_raw_fd(), "fstat trusted executable")?;
    let mode = stat.st_mode as u32 & 0o7777;
    let valid = stat.st_mode & libc::S_IFMT == libc::S_IFREG
        && stat.st_uid == 0
        && stat.st_gid == 0
        && mode & 0o022 == 0
        && (!exact_shim || (mode == 0o755 && stat.st_nlink == 1));
    if !valid {
        return Err(SystemdError::ExecutablePolicy {
            path,
            detail: format!(
                "expected root:root regular nonwritable{}; actual uid={} gid={} mode={mode:04o} nlink={}",
                if exact_shim { " mode 0755 nlink 1" } else { "" },
                stat.st_uid,
                stat.st_gid,
                stat.st_nlink
            ),
        });
    }
    Ok(TrustedIdentity { stat })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_names_are_bounded_and_not_templates() {
        assert!(valid_unit_component("calyx-gate-a.service", ".service"));
        assert!(!valid_unit_component("../escape.service", ".service"));
        assert!(!valid_unit_component("calyx@evil.service", ".service"));
        assert!(!valid_unit_component("calyx gate.service", ".service"));
    }

    #[test]
    fn published_writable_path_is_exactly_alias_and_leaf() {
        assert!(is_exact_published_object(Path::new(
            "/var/lib/calyx-gatebrokerd/objects/tmp/abc"
        )));
        assert!(!is_exact_published_object(Path::new(
            "/var/lib/calyx-gatebrokerd/objects/tmp"
        )));
        assert!(!is_exact_published_object(Path::new(
            "/var/lib/calyx-gatebrokerd/private/quarantine/tmp/abc"
        )));
    }

    #[test]
    fn loader_and_manager_environment_is_reserved() {
        assert!(reserved_environment_name(b"LD_PRELOAD"));
        assert!(reserved_environment_name(b"DBUS_SYSTEM_BUS_ADDRESS"));
        assert!(reserved_environment_name(b"XDG_RUNTIME_DIR"));
        assert!(!reserved_environment_name(b"CALYX_TEST_MARKER"));
    }

    #[test]
    fn relative_cwd_rejects_traversal() {
        assert!(normalized_relative(Path::new(".")));
        assert!(normalized_relative(Path::new("src/bin")));
        assert!(!normalized_relative(Path::new("../src")));
        assert!(!normalized_relative(Path::new("/src")));
    }
}
