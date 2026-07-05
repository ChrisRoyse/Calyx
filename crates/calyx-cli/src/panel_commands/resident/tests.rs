use std::io::{Cursor, Write};

use calyx_core::Placement;

use super::codec::{decode_binary, encode_binary, read_frame, write_frame};
use super::server::resolve_home_with;
use super::*;

fn resident_env_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

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

#[test]
fn resident_measure_window_decouples_runtime_limit_from_outer_chunk() {
    let _lock = resident_env_lock();
    let old = std::env::var_os(stream::MEASURE_WINDOW_ENV);
    unsafe { std::env::remove_var(stream::MEASURE_WINDOW_ENV) };

    assert_eq!(
        stream::resident_measure_chunk_size(1_000, Some(4)).unwrap(),
        128
    );
    assert_eq!(
        stream::resident_measure_chunk_size(16, Some(4)).unwrap(),
        16
    );
    assert_eq!(
        stream::resident_measure_chunk_size(1_000, None).unwrap(),
        1_000
    );

    unsafe {
        match old {
            Some(value) => std::env::set_var(stream::MEASURE_WINDOW_ENV, value),
            None => std::env::remove_var(stream::MEASURE_WINDOW_ENV),
        }
    }
}

#[test]
fn resident_measure_window_env_fails_closed_on_invalid_values() {
    let _lock = resident_env_lock();
    let old = std::env::var_os(stream::MEASURE_WINDOW_ENV);
    unsafe { std::env::set_var(stream::MEASURE_WINDOW_ENV, "bad") };

    let error = stream::resident_measure_chunk_size(10, Some(2)).unwrap_err();
    assert_eq!(error.code(), "CALYX_PANEL_RESIDENT_MEASURE_WINDOW_INVALID");

    unsafe {
        match old {
            Some(value) => std::env::set_var(stream::MEASURE_WINDOW_ENV, value),
            None => std::env::remove_var(stream::MEASURE_WINDOW_ENV),
        }
    }
}

/// #1153/#1154 — the parallel fan-out and the never-sequential invariant.
mod parallel_invariant {
    use std::sync::{Mutex, MutexGuard};

    use super::super::parallel::{
        OVERLAP_FLOOR_ENV, REQUIRE_PARALLEL_ENV, RequireParallelPolicy, enforce_overlap, fan_out,
        overlap_floor_ms, require_parallel_policy,
    };
    use super::*;

    /// `std::env` is process-global; every test touching the parallel knobs
    /// serializes on this lock.
    static PARALLEL_ENV_LOCK: Mutex<()> = Mutex::new(());

    fn parallel_env_lock() -> MutexGuard<'static, ()> {
        PARALLEL_ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    #[test]
    fn fan_out_runs_slots_concurrently_and_preserves_order() {
        let epoch = Instant::now();
        let outcomes = fan_out(4, |index| {
            std::thread::sleep(Duration::from_millis(60));
            index * 10
        });
        let wall = epoch.elapsed();
        assert_eq!(outcomes.len(), 4);
        for (index, outcome) in outcomes.iter().enumerate() {
            assert_eq!(*outcome.result.as_ref().unwrap(), index * 10);
            assert!(outcome.ended_us - outcome.started_us >= 60_000);
        }
        // Four 60ms workloads serialized would take >= 240ms.
        assert!(
            wall < Duration::from_millis(200),
            "fan_out executed sequentially: wall={wall:?}"
        );
        let spans: Vec<_> = outcomes
            .iter()
            .map(|outcome| (outcome.started_us, outcome.ended_us))
            .collect();
        enforce_overlap(&spans, RequireParallelPolicy::Error, 25_000).unwrap();
    }

    #[test]
    fn fan_out_single_item_runs_inline_with_a_span() {
        let outcomes = fan_out(1, |index| index + 7);
        assert_eq!(outcomes.len(), 1);
        assert_eq!(*outcomes[0].result.as_ref().unwrap(), 7);
        assert!(outcomes[0].ended_us >= outcomes[0].started_us);
    }

    #[test]
    fn fan_out_captures_a_panicking_slot_without_aborting_the_rest() {
        let outcomes = fan_out(3, |index| {
            assert!(index != 1, "synthetic slot failure");
            index
        });
        assert_eq!(*outcomes[0].result.as_ref().unwrap(), 0);
        assert!(outcomes[1].result.is_err());
        assert_eq!(*outcomes[2].result.as_ref().unwrap(), 2);
    }

    #[test]
    fn zero_overlap_between_significant_spans_fails_loud() {
        // Two 30ms spans laid end to end: textbook sequential execution.
        let spans = [(0_u128, 30_000_u128), (30_000, 60_000)];
        let error = enforce_overlap(&spans, RequireParallelPolicy::Error, 25_000).unwrap_err();
        assert_eq!(error.code, "CALYX_EMBED_SEQUENTIAL_EXECUTION");
        enforce_overlap(&spans, RequireParallelPolicy::Warn, 25_000).unwrap();
        enforce_overlap(&spans, RequireParallelPolicy::Off, 25_000).unwrap();
    }

    #[test]
    fn overlapping_or_sub_floor_spans_never_trip_the_invariant() {
        let overlapping = [(0_u128, 40_000_u128), (30_000, 70_000)];
        enforce_overlap(&overlapping, RequireParallelPolicy::Error, 25_000).unwrap();
        // Disjoint but below the floor: scheduling noise, not serialization.
        let tiny = [(0_u128, 10_000_u128), (10_000, 20_000)];
        enforce_overlap(&tiny, RequireParallelPolicy::Error, 25_000).unwrap();
        enforce_overlap(&[], RequireParallelPolicy::Error, 25_000).unwrap();
    }

    #[test]
    fn require_parallel_env_defaults_to_error_and_fails_closed_on_garbage() {
        let _lock = parallel_env_lock();
        // SAFETY: single-threaded within the env lock; restored before unlock.
        unsafe { std::env::remove_var(REQUIRE_PARALLEL_ENV) };
        assert_eq!(
            require_parallel_policy().unwrap(),
            RequireParallelPolicy::Error
        );
        unsafe { std::env::set_var(REQUIRE_PARALLEL_ENV, "warn") };
        assert_eq!(
            require_parallel_policy().unwrap(),
            RequireParallelPolicy::Warn
        );
        unsafe { std::env::set_var(REQUIRE_PARALLEL_ENV, "sometimes") };
        let error = require_parallel_policy().unwrap_err();
        assert_eq!(error.code, "CALYX_EMBED_REQUIRE_PARALLEL_INVALID");
        unsafe { std::env::remove_var(REQUIRE_PARALLEL_ENV) };
    }

    #[test]
    fn overlap_floor_env_applies_and_fails_closed_on_garbage() {
        let _lock = parallel_env_lock();
        // SAFETY: single-threaded within the env lock; restored before unlock.
        unsafe { std::env::remove_var(OVERLAP_FLOOR_ENV) };
        assert_eq!(overlap_floor_ms().unwrap(), 25);
        unsafe { std::env::set_var(OVERLAP_FLOOR_ENV, "40") };
        assert_eq!(overlap_floor_ms().unwrap(), 40);
        unsafe { std::env::set_var(OVERLAP_FLOOR_ENV, "-1") };
        let error = overlap_floor_ms().unwrap_err();
        assert_eq!(error.code, "CALYX_EMBED_OVERLAP_FLOOR_INVALID");
        unsafe { std::env::remove_var(OVERLAP_FLOOR_ENV) };
    }
}
