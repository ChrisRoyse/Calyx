use std::fs::File;
use std::io::Read;
use std::os::fd::{FromRawFd, IntoRawFd, OwnedFd};

use calyx_gatebrokerd::protocol::{Response, ResponseOutcome, RunId, RunToken};

use super::*;

fn pipe() -> (OwnedFd, OwnedFd) {
    let mut descriptors = [-1; 2];
    assert_eq!(
        unsafe { libc::pipe2(descriptors.as_mut_ptr(), libc::O_CLOEXEC) },
        0
    );
    unsafe {
        (
            OwnedFd::from_raw_fd(descriptors[0]),
            OwnedFd::from_raw_fd(descriptors[1]),
        )
    }
}

#[test]
fn begun_run_json_never_contains_token_and_pipe_contains_exact_token() {
    let (read_end, write_end) = pipe();
    let token =
        RunToken::new("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef").unwrap();
    let (json, exit) = render(
        ResponseOutcome::Ok(Response::RunBegun {
            run_id: RunId::new("11111111111111111111111111111111").unwrap(),
            run_token: token.clone(),
        }),
        &ProfileName::new("test").unwrap(),
        Some(TokenWriter::new(write_end.into_raw_fd(), "test-token-out").unwrap()),
    )
    .unwrap();
    let encoded = serde_json::to_string(&json).unwrap();
    assert_eq!(exit, 0);
    assert!(
        !encoded.contains(token.as_str()),
        "secret leaked: {encoded}"
    );
    assert_eq!(json["run"]["token_transport"], "fd");

    let mut observed = String::new();
    File::from(read_end).read_to_string(&mut observed).unwrap();
    assert_eq!(observed, format!("{}\n", token.as_str()));
}

#[test]
fn begun_run_without_pipe_fails_closed() {
    let token =
        RunToken::new("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef").unwrap();
    let error = render(
        ResponseOutcome::Ok(Response::RunBegun {
            run_id: RunId::new("22222222222222222222222222222222").unwrap(),
            run_token: token,
        }),
        &ProfileName::new("test").unwrap(),
        None,
    )
    .unwrap_err();
    assert_eq!(error.code, "BROKER_PROTOCOL_INVALID");
}
