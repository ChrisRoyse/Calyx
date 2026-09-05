use super::manager::manager_command;
use super::validation::{duplicate_fd, io_error};
use super::*;

pub(super) fn kernel_random_token() -> Result<[u8; TOKEN_BYTES], SystemdError> {
    let mut token = [0_u8; TOKEN_BYTES];
    let mut offset = 0;
    while offset < token.len() {
        let result = unsafe {
            libc::getrandom(token[offset..].as_mut_ptr().cast(), token.len() - offset, 0)
        };
        if result > 0 {
            offset += result as usize;
            continue;
        }
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::Interrupted {
            token.fill(0);
            return Err(io_error("read kernel release nonce", error));
        }
    }
    Ok(token)
}

pub(super) fn hex_token(token: &[u8; TOKEN_BYTES]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(TOKEN_BYTES * 2);
    for byte in token {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

pub(super) fn shim_argv(spec: &StageSpec, token_hex: &str, cwd_stat: &libc::stat) -> Vec<OsString> {
    let mut argv = vec![
        OsString::from(STAGE_SHIM),
        OsString::from(token_hex),
        spec.execution_root.as_os_str().to_owned(),
        spec.relative_cwd.as_os_str().to_owned(),
        OsString::from(spec.execution_root_uid.to_string()),
        OsString::from(format!("{:04o}", spec.execution_root_mode)),
        OsString::from(cwd_stat.st_dev.to_string()),
        OsString::from(cwd_stat.st_ino.to_string()),
        OsString::from(&spec.worker_user),
        OsString::from(spec.environment.len().to_string()),
    ];
    argv.extend(spec.environment.iter().map(|(name, _)| name.clone()));
    argv.push(OsString::from("--"));
    argv.extend(spec.argv.iter().cloned());
    argv
}

pub(super) fn append_property(command: &mut Command, name: &str, value: &str) {
    command.arg(format!("--property={name}={value}"));
}

pub(super) fn build_systemd_run(
    spec: &StageSpec,
    account: &WorkerAccount,
    shim_arguments: &[OsString],
    stdout_fd: RawFd,
    stderr_fd: RawFd,
) -> Result<Command, SystemdError> {
    let stdout = duplicate_fd(stdout_fd, "duplicate stage stdout")?;
    let stderr = duplicate_fd(stderr_fd, "duplicate stage stderr")?;
    let mut command = manager_command(SYSTEMD_RUN);
    command.args([
        "--system",
        "--quiet",
        "--collect",
        "--expand-environment=no",
        "--wait",
        "--pipe",
        "--service-type=exec",
        "--working-directory=/",
    ]);
    command.args(["--unit", &spec.unit_name, "--slice", STAGE_SLICE_NAME]);
    for (name, value) in [
        ("ExitType", "cgroup"),
        ("KillMode", "control-group"),
        ("SendSIGKILL", "yes"),
        ("Delegate", "no"),
        ("NoNewPrivileges", "yes"),
        ("CapabilityBoundingSet", ""),
        ("AmbientCapabilities", ""),
        ("ProtectControlGroups", "strict"),
        ("ProtectKernelTunables", "yes"),
        ("ProtectKernelModules", "yes"),
        ("ProtectKernelLogs", "yes"),
        ("ProtectClock", "yes"),
        ("ProtectHostname", "yes"),
        ("ProtectProc", "invisible"),
        ("ProcSubset", "pid"),
        ("PrivateDevices", "yes"),
        ("PrivateNetwork", "yes"),
        ("ProtectSystem", "strict"),
        ("ProtectHome", "read-only"),
        ("PrivateTmp", "yes"),
        ("RestrictNamespaces", "yes"),
        ("RestrictRealtime", "yes"),
        ("LockPersonality", "yes"),
        ("SystemCallArchitectures", "native"),
        ("UMask", "0077"),
        ("SupplementaryGroups", ""),
        ("PAMName", ""),
    ] {
        append_property(&mut command, name, value);
    }
    // Do not add RestrictSUIDSGID=yes here. systemd's seccomp implementation
    // deliberately returns ENOSYS for every openat2(2) call because classic
    // seccomp cannot inspect open_how.mode. The release shim requires openat2
    // for its race-free cwd binding. Independent NoNewPrivileges, an empty
    // capability set, ProtectSystem=strict, and exact writable paths remain
    // mandatory above.
    append_property(&mut command, "BindsTo", BROKER_UNIT_NAME);
    append_property(&mut command, "After", BROKER_UNIT_NAME);
    append_property(
        &mut command,
        "Conflicts",
        &format!("user@{}.service", account.uid),
    );
    append_property(&mut command, "User", &spec.worker_user);
    append_property(&mut command, "Group", &spec.worker_user);
    append_property(
        &mut command,
        "ReadOnlyPaths",
        spec.execution_root.to_str().expect("validated UTF-8 root"),
    );
    for inaccessible in [
        PRIVATE_STATE_ROOT,
        "/run/user",
        "/run/systemd/private",
        "/run/systemd/notify",
        "/run/dbus/system_bus_socket",
        "/var/run/dbus/system_bus_socket",
    ] {
        append_property(&mut command, "InaccessiblePaths", inaccessible);
    }
    for path in &spec.writable_paths {
        append_property(
            &mut command,
            "ReadWritePaths",
            path.to_str().expect("validated UTF-8 writable path"),
        );
    }
    for (name, value) in &spec.environment {
        let mut assignment = OsString::from("--setenv=");
        assignment.push(name);
        assignment.push("=");
        assignment.push(value);
        command.arg(assignment);
    }
    command.arg("--");
    command.args(shim_arguments);
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));
    Ok(command)
}
