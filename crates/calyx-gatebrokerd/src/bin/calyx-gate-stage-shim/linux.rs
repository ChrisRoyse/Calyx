//! Linux implementation of the release barrier: identity-verified cwd
//! entry, one-use release-token check, environment sanitization, and the
//! final payload exec.

mod environment;
use environment::sanitize_environment;

use std::env;
use std::ffi::{CString, OsString};
use std::io::{self, Read};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::process;

const TOKEN_BYTES: usize = 32;
const FAILURE_STATUS: i32 = 125;
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

fn fail(code: &str, detail: impl std::fmt::Display) -> ! {
    eprintln!("{code}: {detail}");
    process::exit(FAILURE_STATUS);
}

fn decode_expected(raw: &OsString) -> Result<[u8; TOKEN_BYTES], &'static str> {
    let bytes = raw.as_os_str().as_bytes();
    if bytes.len() != TOKEN_BYTES * 2
        || !bytes
            .iter()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        return Err("expected token must be 64 lowercase hexadecimal bytes");
    }
    let mut decoded = [0_u8; TOKEN_BYTES];
    for (index, pair) in bytes.chunks_exact(2).enumerate() {
        decoded[index] = (hex_nibble(pair[0])? << 4) | hex_nibble(pair[1])?;
    }
    Ok(decoded)
}

fn hex_nibble(value: u8) -> Result<u8, &'static str> {
    match value {
        b'0'..=b'9' => Ok(value - b'0'),
        b'a'..=b'f' => Ok(value - b'a' + 10),
        _ => Err("token contains a non-hexadecimal byte"),
    }
}

fn read_release_token() -> io::Result<Vec<u8>> {
    let mut input = io::stdin().lock();
    let mut received = Vec::with_capacity(TOKEN_BYTES + 1);
    let mut buffer = [0_u8; TOKEN_BYTES + 1];
    loop {
        let length = input.read(&mut buffer)?;
        if length == 0 {
            break;
        }
        received.extend_from_slice(&buffer[..length]);
        if received.len() > TOKEN_BYTES {
            break;
        }
    }
    Ok(received)
}

fn constant_time_equal(left: &[u8; TOKEN_BYTES], right: &[u8]) -> bool {
    if right.len() != TOKEN_BYTES {
        return false;
    }
    let mut difference = 0_u8;
    for index in 0..TOKEN_BYTES {
        difference |= left[index] ^ right[index];
    }
    difference == 0
}

fn parse_number<T>(raw: &OsString, label: &str) -> T
where
    T: std::str::FromStr,
{
    raw.to_str()
        .and_then(|value| value.parse().ok())
        .unwrap_or_else(|| fail("CALYX_GATE_STAGE_SHIM_CWD_SPEC", format!("invalid {label}")))
}

fn parse_mode(raw: &OsString) -> u32 {
    let value = raw
        .to_str()
        .filter(|value| value.len() == 4 && value.bytes().all(|byte| matches!(byte, b'0'..=b'7')))
        .and_then(|value| u32::from_str_radix(value, 8).ok())
        .unwrap_or_else(|| fail("CALYX_GATE_STAGE_SHIM_CWD_SPEC", "invalid root mode"));
    if value > 0o7777 {
        fail("CALYX_GATE_STAGE_SHIM_CWD_SPEC", "root mode exceeds 07777");
    }
    value
}

fn normalized_root(value: &OsString) -> CString {
    let bytes = value.as_os_str().as_bytes();
    if !bytes.starts_with(b"/") || bytes.len() <= 1 {
        fail(
            "CALYX_GATE_STAGE_SHIM_CWD_SPEC",
            "execution root must be a non-root absolute path",
        );
    }
    if bytes[1..]
        .split(|byte| *byte == b'/')
        .any(|component| component.is_empty() || matches!(component, b"." | b".."))
    {
        fail(
            "CALYX_GATE_STAGE_SHIM_CWD_SPEC",
            "execution root is not lexically normalized",
        );
    }
    CString::new(&bytes[1..]).unwrap_or_else(|_| {
        fail(
            "CALYX_GATE_STAGE_SHIM_CWD_SPEC",
            "execution root contains NUL",
        )
    })
}

fn normalized_relative(value: &OsString) -> CString {
    let bytes = value.as_os_str().as_bytes();
    if bytes == b"." {
        return CString::new(".").expect("literal");
    }
    if bytes.is_empty()
        || bytes.starts_with(b"/")
        || bytes
            .split(|byte| *byte == b'/')
            .any(|component| component.is_empty() || matches!(component, b"." | b".."))
    {
        fail(
            "CALYX_GATE_STAGE_SHIM_CWD_SPEC",
            "relative cwd is empty, absolute, or contains traversal",
        );
    }
    CString::new(bytes).unwrap_or_else(|_| {
        fail(
            "CALYX_GATE_STAGE_SHIM_CWD_SPEC",
            "relative cwd contains NUL",
        )
    })
}

fn openat2_directory(parent: RawFd, path: &std::ffi::CStr, resolve: u64) -> OwnedFd {
    let how = OpenHow {
        flags: (libc::O_PATH | libc::O_DIRECTORY | libc::O_CLOEXEC) as u64,
        mode: 0,
        resolve,
    };
    let raw = unsafe {
        libc::syscall(
            libc::SYS_openat2,
            parent,
            path.as_ptr(),
            &how,
            std::mem::size_of::<OpenHow>(),
        ) as RawFd
    };
    if raw < 0 {
        fail(
            "CALYX_GATE_STAGE_SHIM_OPENAT2",
            format!("{}: {}", path.to_string_lossy(), io::Error::last_os_error()),
        );
    }
    unsafe { OwnedFd::from_raw_fd(raw) }
}

fn fstat(fd: RawFd, label: &str) -> libc::stat {
    let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
    if unsafe { libc::fstat(fd, stat.as_mut_ptr()) } != 0 {
        fail(
            "CALYX_GATE_STAGE_SHIM_FSTAT",
            format!("{label}: {}", io::Error::last_os_error()),
        );
    }
    unsafe { stat.assume_init() }
}

fn enter_verified_cwd(
    root_path: &OsString,
    relative_path: &OsString,
    expected_root_uid: u32,
    expected_root_mode: u32,
    expected_cwd_device: u64,
    expected_cwd_inode: u64,
) {
    let slash = c"/";
    let raw_root = unsafe {
        libc::open(
            slash.as_ptr(),
            libc::O_PATH | libc::O_DIRECTORY | libc::O_CLOEXEC,
        )
    };
    if raw_root < 0 {
        fail(
            "CALYX_GATE_STAGE_SHIM_OPEN_ROOT",
            io::Error::last_os_error(),
        );
    }
    let filesystem_root = unsafe { OwnedFd::from_raw_fd(raw_root) };
    let root_name = normalized_root(root_path);
    let execution_root = openat2_directory(
        filesystem_root.as_raw_fd(),
        &root_name,
        RESOLVE_BENEATH | RESOLVE_NO_SYMLINKS | RESOLVE_NO_MAGICLINKS,
    );
    let root_stat = fstat(execution_root.as_raw_fd(), "execution root");
    if root_stat.st_mode & libc::S_IFMT != libc::S_IFDIR
        || root_stat.st_uid != expected_root_uid
        || root_stat.st_mode as u32 & 0o7777 != expected_root_mode
    {
        fail(
            "CALYX_GATE_STAGE_SHIM_ROOT_POLICY",
            format!(
                "expected uid={expected_root_uid} mode={expected_root_mode:04o}; actual uid={} mode={:04o}",
                root_stat.st_uid,
                root_stat.st_mode as u32 & 0o7777
            ),
        );
    }
    let relative = normalized_relative(relative_path);
    let cwd = openat2_directory(
        execution_root.as_raw_fd(),
        &relative,
        RESOLVE_BENEATH | RESOLVE_NO_SYMLINKS | RESOLVE_NO_MAGICLINKS | RESOLVE_NO_XDEV,
    );
    let cwd_stat = fstat(cwd.as_raw_fd(), "resolved cwd");
    if cwd_stat.st_dev as u64 != expected_cwd_device || cwd_stat.st_ino as u64 != expected_cwd_inode
    {
        fail(
            "CALYX_GATE_STAGE_SHIM_CWD_IDENTITY",
            format!(
                "expected dev={expected_cwd_device} ino={expected_cwd_inode}; actual dev={} ino={}",
                cwd_stat.st_dev, cwd_stat.st_ino
            ),
        );
    }
    if unsafe { libc::fchdir(cwd.as_raw_fd()) } != 0 {
        fail("CALYX_GATE_STAGE_SHIM_FCHDIR", io::Error::last_os_error());
    }
}

fn replace_stdin_with_dev_null() -> io::Result<()> {
    let path = c"/dev/null";
    let fd = unsafe { libc::open(path.as_ptr(), libc::O_RDONLY | libc::O_CLOEXEC) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    let result = unsafe { libc::dup2(fd, libc::STDIN_FILENO) };
    let error = io::Error::last_os_error();
    if fd != libc::STDIN_FILENO {
        unsafe { libc::close(fd) };
    }
    if result < 0 { Err(error) } else { Ok(()) }
}

fn exec_payload(argv: Vec<OsString>) -> io::Error {
    let mut c_argv = Vec::with_capacity(argv.len());
    for value in argv {
        match CString::new(value.as_os_str().as_bytes()) {
            Ok(value) => c_argv.push(value),
            Err(_) => return io::Error::new(io::ErrorKind::InvalidInput, "argv contains NUL"),
        }
    }
    let mut pointers: Vec<*const libc::c_char> =
        c_argv.iter().map(|value| value.as_ptr()).collect();
    pointers.push(std::ptr::null());
    unsafe { libc::execvp(c_argv[0].as_ptr(), pointers.as_ptr()) };
    io::Error::last_os_error()
}

pub fn run() {
    let mut args = env::args_os();
    let _program = args.next();
    let expected_raw = args
        .next()
        .unwrap_or_else(|| fail("CALYX_GATE_STAGE_SHIM_USAGE", "missing release token"));
    let execution_root = args
        .next()
        .unwrap_or_else(|| fail("CALYX_GATE_STAGE_SHIM_USAGE", "missing execution root"));
    let relative_cwd = args
        .next()
        .unwrap_or_else(|| fail("CALYX_GATE_STAGE_SHIM_USAGE", "missing relative cwd"));
    let root_uid_raw = args
        .next()
        .unwrap_or_else(|| fail("CALYX_GATE_STAGE_SHIM_USAGE", "missing root uid"));
    let root_mode_raw = args
        .next()
        .unwrap_or_else(|| fail("CALYX_GATE_STAGE_SHIM_USAGE", "missing root mode"));
    let cwd_device_raw = args
        .next()
        .unwrap_or_else(|| fail("CALYX_GATE_STAGE_SHIM_USAGE", "missing cwd device"));
    let cwd_inode_raw = args
        .next()
        .unwrap_or_else(|| fail("CALYX_GATE_STAGE_SHIM_USAGE", "missing cwd inode"));
    let worker_user = args
        .next()
        .unwrap_or_else(|| fail("CALYX_GATE_STAGE_SHIM_USAGE", "missing worker user"));
    let environment_count_raw = args
        .next()
        .unwrap_or_else(|| fail("CALYX_GATE_STAGE_SHIM_USAGE", "missing environment count"));
    let environment_count: usize = parse_number(&environment_count_raw, "environment count");
    if environment_count > 256 {
        fail(
            "CALYX_GATE_STAGE_SHIM_USAGE",
            "environment count exceeds 256",
        );
    }
    let mut environment_names = Vec::with_capacity(environment_count);
    for _ in 0..environment_count {
        environment_names
            .push(args.next().unwrap_or_else(|| {
                fail("CALYX_GATE_STAGE_SHIM_USAGE", "missing environment name")
            }));
    }
    let separator = args
        .next()
        .unwrap_or_else(|| fail("CALYX_GATE_STAGE_SHIM_USAGE", "missing -- separator"));
    if separator.as_os_str().as_bytes() != b"--" {
        fail("CALYX_GATE_STAGE_SHIM_USAGE", "expected -- separator");
    }
    let payload: Vec<OsString> = args.collect();
    if payload.is_empty() || payload[0].as_os_str().as_bytes().first() != Some(&b'/') {
        fail(
            "CALYX_GATE_STAGE_SHIM_USAGE",
            "payload executable must be absolute",
        );
    }
    let mut expected = decode_expected(&expected_raw)
        .unwrap_or_else(|detail| fail("CALYX_GATE_STAGE_SHIM_TOKEN_FORMAT", detail));
    enter_verified_cwd(
        &execution_root,
        &relative_cwd,
        parse_number(&root_uid_raw, "root uid"),
        parse_mode(&root_mode_raw),
        parse_number(&cwd_device_raw, "cwd device"),
        parse_number(&cwd_inode_raw, "cwd inode"),
    );
    let mut received = read_release_token()
        .unwrap_or_else(|error| fail("CALYX_GATE_STAGE_SHIM_TOKEN_READ", error));
    if !constant_time_equal(&expected, &received) {
        expected.fill(0);
        received.fill(0);
        fail(
            "CALYX_GATE_STAGE_SHIM_TOKEN_MISMATCH",
            "release token was absent, short, long, or incorrect",
        );
    }
    expected.fill(0);
    received.fill(0);
    sanitize_environment(&worker_user, &environment_names);
    replace_stdin_with_dev_null()
        .unwrap_or_else(|error| fail("CALYX_GATE_STAGE_SHIM_STDIN", error));
    let error = exec_payload(payload);
    fail("CALYX_GATE_STAGE_SHIM_EXEC", error);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_decoder_requires_canonical_lower_hex() {
        assert!(decode_expected(&OsString::from("00".repeat(TOKEN_BYTES))).is_ok());
        assert!(decode_expected(&OsString::from("AA".repeat(TOKEN_BYTES))).is_err());
        assert!(decode_expected(&OsString::from("0".repeat(TOKEN_BYTES))).is_err());
    }

    #[test]
    fn comparison_rejects_wrong_and_non_exact_tokens() {
        let expected = [7_u8; TOKEN_BYTES];
        assert!(constant_time_equal(&expected, &[7_u8; TOKEN_BYTES]));
        assert!(!constant_time_equal(&expected, &[7_u8; TOKEN_BYTES - 1]));
        let mut wrong = [7_u8; TOKEN_BYTES];
        wrong[TOKEN_BYTES - 1] = 8;
        assert!(!constant_time_equal(&expected, &wrong));
    }
}
