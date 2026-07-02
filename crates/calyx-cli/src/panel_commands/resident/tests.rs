use std::io::{Cursor, Write};

use calyx_core::Placement;

use super::codec::{decode_binary, encode_binary, read_frame, write_frame};
use super::server::resolve_home_with;
use super::*;

#[test]
fn provided_home_does_not_evaluate_env_fallback() {
    let home = PathBuf::from(r"C:\calyx");
    let resolved = resolve_home_with(Some(home.clone()), || {
        panic!("explicit --home must not read CALYX_HOME")
    })
    .unwrap();

    assert_eq!(resolved, home);
}

#[test]
fn resident_service_binary_request_roundtrips_without_json_shape() {
    let request = ResidentMeasureBatchBinaryRequest {
        protocol_version: RESIDENT_BINARY_PROTOCOL_VERSION,
        modality: Modality::Text,
        inputs: vec![b"alpha".to_vec(), b"beta".to_vec()],
        runtime_batch_limit: Some(4),
    };

    let bytes = encode_binary(&request).unwrap();
    let decoded: ResidentMeasureBatchBinaryRequest = decode_binary(&bytes).unwrap();
    println!(
        "resident_service_binary_request bytes={} inputs={} runtime_batch_limit={:?}",
        bytes.len(),
        decoded.inputs.len(),
        decoded.runtime_batch_limit
    );

    assert_eq!(decoded.protocol_version, RESIDENT_BINARY_PROTOCOL_VERSION);
    assert_eq!(decoded.modality, Modality::Text);
    assert_eq!(decoded.inputs, request.inputs);
    assert_eq!(decoded.runtime_batch_limit, Some(4));
    let lossy = String::from_utf8_lossy(&bytes);
    assert!(
        !lossy.contains("inputs_hex") && !lossy.contains("runtime_batch_limit"),
        "resident service binary IPC must not carry JSON field names"
    );
}

fn sample_row(input_index: usize) -> ResidentMeasuredInput {
    ResidentMeasuredInput {
        input_index,
        input_len: 24,
        measured_slot_count: 1,
        absent_slot_count: 0,
        slots: vec![ResidentSlotMeasure {
            slot: 0,
            key: "multi".to_string(),
            lens_id: "00000000000000000000000000000000".to_string(),
            modality: Modality::Text,
            placement: Placement::Gpu,
            measured: true,
            vector: Some(SlotVector::Multi {
                token_dim: 2,
                tokens: vec![vec![0.25, 0.75], vec![0.5, 0.5]],
            }),
            absent_reason: None,
        }],
    }
}

/// #1002: the response is a stream of Header/Row/End frames — each row is its
/// own length-prefixed frame, never one giant response frame.
#[test]
fn resident_service_binary_stream_frames_roundtrip_per_row() {
    let mut stream = Cursor::new(Vec::new());
    let frames = [
        ResidentMeasureBatchStreamFrame::Header(ResidentMeasureBatchStreamHeader {
            protocol_version: RESIDENT_BINARY_PROTOCOL_VERSION,
            schema: MEASURE_BATCH_SCHEMA.to_string(),
            ready: true,
            process_id: 42,
            template_source: "synthetic-template".to_string(),
            modality: Modality::Text,
            input_count: 2,
            runtime_batch_limit: Some(4),
        }),
        ResidentMeasureBatchStreamFrame::Row(Box::new(sample_row(0))),
        ResidentMeasureBatchStreamFrame::Row(Box::new(sample_row(1))),
        ResidentMeasureBatchStreamFrame::End(ResidentMeasureBatchStreamEnd {
            row_count: 2,
            elapsed_ms: 7,
        }),
    ];
    let mut frame_sizes = Vec::new();
    for frame in &frames {
        let bytes = encode_binary(frame).unwrap();
        frame_sizes.push(bytes.len());
        write_frame(&mut stream, &bytes).unwrap();
    }
    println!("resident_stream_frame_sizes={frame_sizes:?}");

    let stored = stream.into_inner();
    let lossy = String::from_utf8_lossy(&stored);
    assert!(
        !lossy.contains("input_index") && !lossy.contains("token_dim"),
        "resident stream frames must not carry JSON field names"
    );
    let mut readback = Cursor::new(stored);
    let mut decoded_kinds = Vec::new();
    let mut rows = Vec::new();
    loop {
        let payload = read_frame(&mut readback).unwrap();
        match decode_binary::<ResidentMeasureBatchStreamFrame>(&payload).unwrap() {
            ResidentMeasureBatchStreamFrame::Header(header) => {
                assert_eq!(header.protocol_version, RESIDENT_BINARY_PROTOCOL_VERSION);
                assert_eq!(header.schema, MEASURE_BATCH_SCHEMA);
                decoded_kinds.push("header");
            }
            ResidentMeasureBatchStreamFrame::Row(row) => {
                assert_eq!(row.input_index, rows.len());
                assert!(matches!(
                    row.slots[0].vector,
                    Some(SlotVector::Multi { token_dim: 2, .. })
                ));
                rows.push(*row);
                decoded_kinds.push("row");
            }
            ResidentMeasureBatchStreamFrame::End(end) => {
                assert_eq!(end.row_count, rows.len());
                decoded_kinds.push("end");
                break;
            }
            ResidentMeasureBatchStreamFrame::Err { code, message, .. } => {
                panic!("unexpected err frame {code}: {message}");
            }
        }
    }
    assert_eq!(decoded_kinds, ["header", "row", "row", "end"]);
}

#[test]
fn resident_service_binary_stream_err_frame_carries_structured_cause() {
    let frame = ResidentMeasureBatchStreamFrame::Err {
        code: "CALYX_TEST".to_string(),
        message: "synthetic resident stream edge".to_string(),
        remediation: "fix test input".to_string(),
    };
    let payload = encode_binary(&frame).unwrap();
    let mut stream = Cursor::new(Vec::new());
    write_frame(&mut stream, &payload).unwrap();
    let stored = stream.into_inner();
    assert_eq!(stored.len(), payload.len() + 8);
    assert_eq!(
        u64::from_be_bytes(stored[..8].try_into().unwrap()) as usize,
        payload.len()
    );
    let mut readback = Cursor::new(stored);
    let decoded_payload = read_frame(&mut readback).unwrap();
    match decode_binary::<ResidentMeasureBatchStreamFrame>(&decoded_payload).unwrap() {
        ResidentMeasureBatchStreamFrame::Err { code, .. } => assert_eq!(code, "CALYX_TEST"),
        other => panic!("expected err frame, got {other:?}"),
    }
}

#[test]
fn resident_service_binary_truncated_frame_fails_loud() {
    let mut stream = Cursor::new(Vec::new());
    stream.write_all(&16_u64.to_be_bytes()).unwrap();
    stream.write_all(b"short").unwrap();
    stream.set_position(0);

    let error = read_frame(&mut stream).unwrap_err();
    println!(
        "resident_service_binary_truncated_error code={} message={}",
        error.code, error.message
    );

    assert_eq!(error.code, "CALYX_PANEL_RESIDENT_BINARY_FRAME");
    assert!(
        error
            .message
            .contains("read resident service binary frame body")
    );
}
