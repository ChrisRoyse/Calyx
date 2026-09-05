use std::io::{Read, Write};
use std::os::fd::{FromRawFd, IntoRawFd, OwnedFd};

use super::*;

fn pipe() -> (OwnedFd, OwnedFd) {
    let mut descriptors = [-1; 2];
    assert_eq!(
        unsafe { libc::pipe2(descriptors.as_mut_ptr(), libc::O_CLOEXEC) },
        0,
        "pipe2: {}",
        io::Error::last_os_error()
    );
    unsafe {
        (
            OwnedFd::from_raw_fd(descriptors[0]),
            OwnedFd::from_raw_fd(descriptors[1]),
        )
    }
}

#[test]
fn token_reader_consumes_exact_real_pipe_frame() {
    let (read_end, write_end) = pipe();
    let mut writer = File::from(write_end);
    writer
        .write_all(b"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\n")
        .unwrap();
    drop(writer);

    let value = TokenReader::new(read_end.into_raw_fd(), "test-token")
        .unwrap()
        .read()
        .unwrap();
    assert_eq!(
        value,
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
    );
}

#[test]
fn regular_file_and_wrong_pipe_direction_are_rejected() {
    let file = tempfile::tempfile().unwrap();
    let error = TokenReader::new(file.into_raw_fd(), "test-token")
        .err()
        .unwrap();
    assert!(error.contains("must be a Linux pipe/FIFO"), "{error}");

    let (read_end, write_end) = pipe();
    drop(read_end);
    let error = TokenReader::new(write_end.into_raw_fd(), "test-token")
        .err()
        .unwrap();
    assert!(error.contains("read-only pipe endpoint"), "{error}");
}

#[test]
fn oversized_token_frame_is_rejected_after_real_pipe_read() {
    let (read_end, write_end) = pipe();
    let mut writer = File::from(write_end);
    writer.write_all(&[b'a'; 66]).unwrap();
    drop(writer);
    let error = TokenReader::new(read_end.into_raw_fd(), "test-token")
        .unwrap()
        .read()
        .unwrap_err();
    assert!(error.contains("exceeds the 65-byte"), "{error}");
}

#[test]
fn token_writer_emits_canonical_secret_only_to_pipe() {
    let (read_end, write_end) = pipe();
    let token =
        RunToken::new("abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789").unwrap();
    TokenWriter::new(write_end.into_raw_fd(), "test-token-out")
        .unwrap()
        .write(&token)
        .unwrap();
    let mut observed = String::new();
    File::from(read_end).read_to_string(&mut observed).unwrap();
    assert_eq!(observed, format!("{}\n", token.as_str()));
}

#[test]
fn response_pipe_is_physically_separate_from_arbitrary_payload_stream() {
    let (payload_read, payload_write) = pipe();
    let (response_read, response_write) = pipe();
    let payload = b"\0binary-stage-output\n{not-control-json}";
    let mut payload_writer = File::from(payload_write);
    payload_writer.write_all(payload).unwrap();
    drop(payload_writer);

    let mut response = PipeWriter::new(response_write.into_raw_fd(), "test-response").unwrap();
    response.write_frame(br#"{"status":"ok"}"#).unwrap();
    drop(response);

    let mut observed_payload = Vec::new();
    File::from(payload_read)
        .read_to_end(&mut observed_payload)
        .unwrap();
    let mut observed_response = Vec::new();
    File::from(response_read)
        .read_to_end(&mut observed_response)
        .unwrap();
    assert_eq!(observed_payload, payload);
    assert_eq!(observed_response, b"{\"status\":\"ok\"}\n");
}
