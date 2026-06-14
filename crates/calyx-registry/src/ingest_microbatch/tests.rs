use std::sync::atomic::{AtomicUsize, Ordering};

use calyx_core::{Modality, SlotVector};

use super::*;

#[test]
fn admission_accounts_bytes_until_permit_drop() {
    let inputs = [
        Input::new(Modality::Text, b"aa".to_vec()).with_pointer("ptr-a"),
        Input::new(Modality::Text, b"bbbb".to_vec()).with_pointer("p"),
    ];
    let expected_bytes = 2 * INGEST_MICROBATCH_INPUT_OVERHEAD_BYTES + 2 + 4 + 5 + 1;
    let controller =
        IngestMicrobatchController::new(IngestMicrobatchConfig::new(160).with_high_water(100));

    assert_eq!(estimate_microbatch_bytes(&inputs), expected_bytes);
    assert_eq!(controller.stats().current_buffer_bytes, 0);

    {
        let permit = controller.admit(&inputs).unwrap();
        let stats = controller.stats();
        assert_eq!(permit.bytes(), expected_bytes);
        assert_eq!(stats.current_buffer_bytes, expected_bytes);
        assert_eq!(stats.buffer_high_water_bytes, expected_bytes);
        assert_eq!(stats.high_water_events_total, 1);
    }

    let stats = controller.stats();
    assert_eq!(stats.current_buffer_bytes, 0);
    assert_eq!(stats.admitted_total, 1);
}

#[test]
fn over_cap_microbatch_fails_closed_with_backpressure() {
    let exact = [Input::new(Modality::Text, vec![0; 16])];
    let expected_bytes = INGEST_MICROBATCH_INPUT_OVERHEAD_BYTES + 16;
    let controller = IngestMicrobatchController::new(IngestMicrobatchConfig::new(expected_bytes));

    let permit = controller.admit(&exact).unwrap();
    assert_eq!(controller.stats().current_buffer_bytes, expected_bytes);

    let error = controller
        .admit(&[Input::new(Modality::Text, b"x".to_vec())])
        .unwrap_err();
    assert_eq!(error.code, "CALYX_BACKPRESSURE");
    assert_eq!(controller.stats().current_buffer_bytes, expected_bytes);
    assert_eq!(controller.stats().backpressure_total, 1);

    drop(permit);
    assert_eq!(controller.stats().current_buffer_bytes, 0);
}

#[test]
fn zero_cap_allows_empty_batch_but_rejects_non_empty_batch() {
    let controller = IngestMicrobatchController::new(IngestMicrobatchConfig::new(0));

    let empty = controller.admit(&[]).unwrap();
    assert_eq!(empty.bytes(), 0);
    drop(empty);

    let error = controller
        .admit(&[Input::new(Modality::Text, b"x".to_vec())])
        .unwrap_err();
    assert_eq!(error.code, "CALYX_BACKPRESSURE");
    assert_eq!(controller.stats().current_buffer_bytes, 0);
}

#[test]
fn lens_timeout_trips_breaker_and_skips_open_lens() {
    let lens_id = LensId::from_bytes([9; 16]);
    let inputs = [
        Input::new(Modality::Text, b"aa".to_vec()),
        Input::new(Modality::Text, b"bbbb".to_vec()),
    ];
    let controller =
        IngestMicrobatchController::new(IngestMicrobatchConfig::new(4096).with_breaker(1, 1_000));

    let first = controller
        .measure_lens_batch(lens_id, &inputs, 10, |_| {
            Err(CalyxError::lens_unreachable("synthetic stalled TEI lens"))
        })
        .unwrap();

    assert_eq!(first.status, IngestLensOutcomeStatus::Degraded);
    assert_eq!(first.error_code.as_deref(), Some("CALYX_LENS_UNREACHABLE"));
    assert_eq!(first.breaker_open_until_ms, Some(1_010));
    assert!(first.vectors.iter().all(SlotVector::is_absent));

    let calls = AtomicUsize::new(0);
    let second = controller
        .measure_lens_batch(lens_id, &inputs, 11, |_| {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok(Vec::new())
        })
        .unwrap();

    assert_eq!(second.status, IngestLensOutcomeStatus::Degraded);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let stats = controller.stats();
    assert_eq!(stats.current_buffer_bytes, 0);
    assert_eq!(stats.lens_timeouts_total, 1);
    assert_eq!(stats.breaker_trips_total, 1);
    assert_eq!(stats.degraded_lenses_total, 2);
    assert_eq!(stats.open_breaker_count, 1);
}

#[test]
fn half_open_success_resets_breaker() {
    let lens_id = LensId::from_bytes([10; 16]);
    let input = [Input::new(Modality::Text, b"aa".to_vec())];
    let controller =
        IngestMicrobatchController::new(IngestMicrobatchConfig::new(4096).with_breaker(1, 10));
    controller
        .measure_lens_batch(lens_id, &input, 20, |_| {
            Err(CalyxError::lens_unreachable("synthetic timeout"))
        })
        .unwrap();

    let recovered = controller
        .measure_lens_batch(lens_id, &input, 31, |_| {
            Ok(vec![SlotVector::Dense {
                dim: 1,
                data: vec![2.0],
            }])
        })
        .unwrap();

    assert_eq!(recovered.status, IngestLensOutcomeStatus::Measured);
    let stats = controller.stats();
    assert_eq!(stats.breaker_recoveries_total, 1);
    assert_eq!(stats.open_breaker_count, 0);
}

#[test]
fn non_timeout_lens_error_propagates_without_degrade() {
    let lens_id = LensId::from_bytes([11; 16]);
    let input = [Input::new(Modality::Text, b"aa".to_vec())];
    let controller = IngestMicrobatchController::new(IngestMicrobatchConfig::new(4096));

    let error = controller
        .measure_lens_batch(lens_id, &input, 40, |_| {
            Err(CalyxError::lens_dim_mismatch("synthetic bad vector"))
        })
        .unwrap_err();

    let stats = controller.stats();
    assert_eq!(error.code, "CALYX_LENS_DIM_MISMATCH");
    assert_eq!(stats.current_buffer_bytes, 0);
    assert_eq!(stats.degraded_lenses_total, 0);
}
