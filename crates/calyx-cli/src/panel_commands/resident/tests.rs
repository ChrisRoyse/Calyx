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

#[test]
fn resident_service_binary_frame_readback_is_length_prefixed() {
    let response = ResidentMeasureBatchBinaryResponse {
        protocol_version: RESIDENT_BINARY_PROTOCOL_VERSION,
        result: ResidentMeasureBatchBinaryResult::Err {
            code: "CALYX_TEST".to_string(),
            message: "synthetic resident frame edge".to_string(),
            remediation: "fix test input".to_string(),
        },
    };
    let payload = encode_binary(&response).unwrap();
    let mut stream = Cursor::new(Vec::new());

    write_frame(&mut stream, &payload).unwrap();
    let stored = stream.into_inner();
    println!(
        "resident_service_binary_frame_state header_bytes=8 payload_bytes={} stored_bytes={}",
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
    let decoded: ResidentMeasureBatchBinaryResponse = decode_binary(&decoded_payload).unwrap();
    assert_eq!(decoded.protocol_version, RESIDENT_BINARY_PROTOCOL_VERSION);
    assert!(matches!(
        decoded.result,
        ResidentMeasureBatchBinaryResult::Err { ref code, .. } if code == "CALYX_TEST"
    ));
}

#[test]
fn resident_service_binary_measure_batch_response_roundtrips_with_vectors() {
    let response = ResidentMeasureBatchBinaryResponse {
        protocol_version: RESIDENT_BINARY_PROTOCOL_VERSION,
        result: ResidentMeasureBatchBinaryResult::Ok(MeasureBatchResponse {
            schema: MEASURE_BATCH_SCHEMA.to_string(),
            ready: true,
            process_id: 42,
            template_source: "synthetic-template".to_string(),
            modality: Modality::Text,
            input_count: 1,
            elapsed_ms: 7,
            runtime_batch_limit: Some(4),
            rows: vec![ResidentMeasuredInput {
                input_index: 0,
                input_len: 24,
                measured_slot_count: 1,
                absent_slot_count: 0,
                slots: vec![ResidentSlotMeasure {
                    slot: 0,
                    key: "dense".to_string(),
                    lens_id: "00000000000000000000000000000000".to_string(),
                    modality: Modality::Text,
                    placement: Placement::Gpu,
                    measured: true,
                    vector: Some(SlotVector::Dense {
                        dim: 2,
                        data: vec![0.25, 0.75],
                    }),
                    absent_reason: None,
                }],
            }],
        }),
    };

    let bytes = encode_binary(&response).unwrap();
    println!("resident_service_binary_response bytes={}", bytes.len());
    let decoded: ResidentMeasureBatchBinaryResponse = decode_binary(&bytes).unwrap();

    assert_eq!(decoded.protocol_version, RESIDENT_BINARY_PROTOCOL_VERSION);
    match decoded.result {
        ResidentMeasureBatchBinaryResult::Ok(parsed) => {
            assert_eq!(parsed.schema, MEASURE_BATCH_SCHEMA);
            assert_eq!(parsed.modality, Modality::Text);
            assert_eq!(parsed.rows.len(), 1);
            assert_eq!(parsed.rows[0].slots.len(), 1);
            assert!(matches!(
                parsed.rows[0].slots[0].vector,
                Some(SlotVector::Dense { dim: 2, .. })
            ));
        }
        ResidentMeasureBatchBinaryResult::Err { code, message, .. } => {
            panic!("expected ok response, got {code}: {message}");
        }
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
