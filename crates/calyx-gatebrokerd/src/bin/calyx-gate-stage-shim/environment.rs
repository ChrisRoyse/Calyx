//! Environment sanitization authority for the release shim: the payload runs
//! with a fixed, minimal environment plus explicitly requested, non-reserved
//! variables. Loader-, resolver-, and manager-controlling names are refused.

use std::env;
use std::ffi::{CString, OsString};
use std::os::unix::ffi::OsStrExt;

use super::fail;

fn portable_environment_name(name: &[u8]) -> bool {
    name.first()
        .is_some_and(|byte| byte.is_ascii_alphabetic() || *byte == b'_')
        && name[1..]
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
}

fn reserved_environment_name(name: &[u8]) -> bool {
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

fn set_environment(name: &[u8], value: &[u8]) {
    let name = CString::new(name)
        .unwrap_or_else(|_| fail("CALYX_GATE_STAGE_SHIM_ENV", "environment name contains NUL"));
    let value = CString::new(value).unwrap_or_else(|_| {
        fail(
            "CALYX_GATE_STAGE_SHIM_ENV",
            "environment value contains NUL",
        )
    });
    if unsafe { libc::setenv(name.as_ptr(), value.as_ptr(), 1) } != 0 {
        fail("CALYX_GATE_STAGE_SHIM_ENV", std::io::Error::last_os_error());
    }
}

pub(super) fn sanitize_environment(worker_user: &OsString, requested_names: &[OsString]) {
    let worker = worker_user.as_os_str().as_bytes();
    if worker.is_empty()
        || !worker
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        fail("CALYX_GATE_STAGE_SHIM_ENV", "invalid worker account name");
    }
    let invocation = env::var_os("INVOCATION_ID")
        .filter(|value| {
            let bytes = value.as_os_str().as_bytes();
            bytes.len() == 32
                && bytes
                    .iter()
                    .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
        })
        .unwrap_or_else(|| fail("CALYX_GATE_STAGE_SHIM_ENV", "invalid INVOCATION_ID"));
    let mut requested = Vec::with_capacity(requested_names.len());
    for name in requested_names {
        let bytes = name.as_os_str().as_bytes();
        if !portable_environment_name(bytes) || reserved_environment_name(bytes) {
            fail(
                "CALYX_GATE_STAGE_SHIM_ENV",
                "requested environment name is unsafe",
            );
        }
        let value = env::var_os(name)
            .unwrap_or_else(|| fail("CALYX_GATE_STAGE_SHIM_ENV", "requested value is absent"));
        requested.push((name.clone(), value));
    }
    if unsafe { libc::clearenv() } != 0 {
        fail("CALYX_GATE_STAGE_SHIM_ENV", std::io::Error::last_os_error());
    }
    set_environment(
        b"PATH",
        b"/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
    );
    set_environment(b"HOME", b"/nonexistent");
    set_environment(b"USER", worker);
    set_environment(b"LOGNAME", worker);
    set_environment(b"SHELL", b"/usr/sbin/nologin");
    set_environment(b"LANG", b"C.UTF-8");
    set_environment(b"LC_ALL", b"C.UTF-8");
    set_environment(b"INVOCATION_ID", invocation.as_os_str().as_bytes());
    for (name, value) in requested {
        set_environment(name.as_os_str().as_bytes(), value.as_os_str().as_bytes());
    }
}
