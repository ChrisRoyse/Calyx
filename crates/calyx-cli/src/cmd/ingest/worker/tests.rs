use std::io::{Cursor, Write};

use calyx_core::Modality;

use super::frame::read_frame;
use super::resident::{append_tail, stderr_tail_text};
use super::*;

#[test]
fn resident_binary_request_roundtrips_without_json_shape() {
    let request = ResidentLensWorkerRequest {
        protocol_version: RESIDENT_PROTOCOL_VERSION,
        inputs: vec![
            Input::new(Modality::Text, b"alpha".to_vec()),
            Input::new(Modality::Text, b"beta".to_vec()),
        ],
        runtime_batch_limit: Some(4),
    };

    let bytes = encode_binary(&request).unwrap();
    let decoded: ResidentLensWorkerRequest = decode_binary(&bytes).unwrap();
    println!(
        "resident_binary_request_roundtrip bytes={} inputs={} runtime_batch_limit={:?}",
        bytes.len(),
        decoded.inputs.len(),
        decoded.runtime_batch_limit
    );

    assert_eq!(decoded.protocol_version, RESIDENT_PROTOCOL_VERSION);
    assert_eq!(decoded.inputs, request.inputs);
    assert_eq!(decoded.runtime_batch_limit, Some(4));
    assert!(
        !String::from_utf8_lossy(&bytes).contains("runtime_batch_limit"),
        "binary IPC must not carry JSON field names"
    );
}

#[test]
fn resident_binary_frame_readback_is_length_prefixed() {
    let response = ResidentLensWorkerResponse {
        protocol_version: RESIDENT_PROTOCOL_VERSION,
        result: ResidentLensWorkerResult::Err {
            code: "CALYX_TEST".to_string(),
            message: "synthetic frame edge".to_string(),
            remediation: "fix test input".to_string(),
        },
    };
    let payload = encode_binary(&response).unwrap();
    let mut stream = Cursor::new(Vec::new());

    write_frame(&mut stream, &payload).unwrap();
    let stored = stream.into_inner();
    println!(
        "resident_binary_frame_state header_bytes=8 payload_bytes={} stored_bytes={}",
        payload.len(),
        stored.len()
    );

    assert_eq!(stored.len(), payload.len() + 8);
    assert_eq!(
        u64::from_be_bytes(stored[..8].try_into().unwrap()) as usize,
        payload.len()
    );
    let mut readback = Cursor::new(stored);
    let decoded_payload = read_frame(&mut readback).unwrap();
    let decoded: ResidentLensWorkerResponse = decode_binary(&decoded_payload).unwrap();
    assert_eq!(decoded.protocol_version, RESIDENT_PROTOCOL_VERSION);
    assert!(matches!(
        decoded.result,
        ResidentLensWorkerResult::Err { ref code, .. } if code == "CALYX_TEST"
    ));
}

#[test]
fn resident_binary_frame_truncated_body_fails_loud() {
    let mut stream = Cursor::new(Vec::new());
    stream.write_all(&16_u64.to_be_bytes()).unwrap();
    stream.write_all(b"short").unwrap();
    stream.set_position(0);

    let error = read_frame(&mut stream).unwrap_err();
    println!(
        "resident_binary_frame_truncated_error code={} message={}",
        error.code, error.message
    );
    assert_eq!(error.code, "CALYX_LENS_UNREACHABLE");
    assert!(error.message.contains("read binary frame body"));
}

#[test]
fn stderr_tail_text_is_single_line_for_runtime_logs() {
    let tail = Arc::new(Mutex::new(Vec::new()));
    append_tail(
        &tail,
        b"line one\r\nCALYX_INGEST_RUNTIME phase=child_ready\tok\n",
    );

    let text = stderr_tail_text(&tail);

    println!("stderr_tail_sanitized={text}");
    assert_eq!(
        text,
        "line one\\r\\nCALYX_INGEST_RUNTIME phase=child_ready\\tok"
    );
    assert!(!text.contains('\n'));
    assert!(!text.contains('\r'));
    assert!(!text.contains('\t'));
}
