#![cfg(target_os = "linux")]

use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::process::{Command, Stdio};

fn inherited_pipe() -> (OwnedFd, OwnedFd) {
    let mut descriptors = [-1; 2];
    assert_eq!(unsafe { libc::pipe(descriptors.as_mut_ptr()) }, 0);
    unsafe {
        (
            OwnedFd::from_raw_fd(descriptors[0]),
            OwnedFd::from_raw_fd(descriptors[1]),
        )
    }
}

#[test]
fn exec_local_failure_uses_response_pipe_and_never_payload_stdout() {
    let (token_read, token_write) = inherited_pipe();
    let (response_read, response_write) = inherited_pipe();
    let mut token_writer = File::from(token_write);
    token_writer
        .write_all(b"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\n")
        .unwrap();
    drop(token_writer);
    let missing_socket = format!(
        "/run/calyx-gatectl-absent-{}-control.sock",
        std::process::id()
    );
    assert!(!std::path::Path::new(&missing_socket).exists());

    let child = Command::new(env!("CARGO_BIN_EXE_calyx-gatectl"))
        .args([
            "--socket",
            &missing_socket,
            "--response-fd",
            &response_write.as_raw_fd().to_string(),
            "exec-stage",
            "--run-id",
            "11111111111111111111111111111111",
            "--token-fd",
            &token_read.as_raw_fd().to_string(),
            "--cwd-root",
            "source",
            "--",
            "/bin/true",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    drop(response_write);
    drop(token_read);
    let mut response_bytes = Vec::new();
    File::from(response_read)
        .read_to_end(&mut response_bytes)
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(125));
    assert_eq!(output.stdout, b"", "payload stdout was polluted");
    assert_eq!(output.stderr, b"");

    let response: serde_json::Value = serde_json::from_slice(&response_bytes).unwrap();
    println!(
        "SOURCE_OF_TRUTH payload_stdout_bytes={} response_pipe={}",
        output.stdout.len(),
        response
    );
    assert_eq!(response["status"], "error");
    assert_eq!(response["code"], "LOCAL_IO_FAILED");
    assert!(response["message"].as_str().unwrap().contains("connect"));
}

#[test]
fn regular_response_file_is_rejected_before_request_construction() {
    let file = tempfile::tempfile().unwrap();
    let flags = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETFD) };
    assert!(flags >= 0);
    assert_eq!(
        unsafe { libc::fcntl(file.as_raw_fd(), libc::F_SETFD, flags & !libc::FD_CLOEXEC) },
        0
    );
    let output = Command::new(env!("CARGO_BIN_EXE_calyx-gatectl"))
        .args(["--response-fd", &file.as_raw_fd().to_string(), "health"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(64));
    let response: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(response["code"], "INVALID_ARGUMENT");
    assert!(
        response["message"]
            .as_str()
            .unwrap()
            .contains("must be a Linux pipe/FIFO")
    );
    assert_eq!(file.metadata().unwrap().len(), 0);
}
