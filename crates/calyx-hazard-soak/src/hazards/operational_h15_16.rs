use calyx_core::{Input, LensId, Modality};
use calyx_registry::{
    INGEST_MICROBATCH_INPUT_OVERHEAD_BYTES, IngestLensOutcomeStatus, IngestMicrobatchConfig,
    IngestMicrobatchController, Registry,
};
use serde::Serialize;
use serde_json::json;
use std::collections::BTreeSet;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Barrier, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use super::operational_support::{
    LengthLens, MutableClock, SingleFlightCache, TimeoutLens, dense_lengths, ttl_jitter_readback,
};
use super::resource::ProbeResult;
use super::resource_support::{case_dir, err};

const START_TS: u64 = 1_800_100_000_000;
const H15_CALLERS: usize = 100;
const LENS_TIMEOUT_MS: u64 = 50;

pub(super) fn probe_h15_cache_stampede(root: &Path) -> ProbeResult {
    let _dir = case_dir(root, "h15_cache_stampede")?;
    let clock = MutableClock::new(START_TS + 1_500);
    let cache = Arc::new(SingleFlightCache::new(
        8 * 1024,
        Duration::from_millis(50),
        Duration::from_millis(20),
        Arc::new(clock.clone()),
    )?);
    cache.insert_seed("kernel-hot", b"stale-kernel".to_vec(), 12)?;
    let before = cache.stats();
    clock.advance(1_000);

    let recomputes = Arc::new(AtomicUsize::new(0));
    let gate = Arc::new(Barrier::new(H15_CALLERS + 1));
    let (tx, rx) = mpsc::channel();
    let mut handles = Vec::new();
    for _ in 0..H15_CALLERS {
        let cache = Arc::clone(&cache);
        let recomputes = Arc::clone(&recomputes);
        let gate = Arc::clone(&gate);
        let tx = tx.clone();
        handles.push(thread::spawn(move || {
            gate.wait();
            let value = cache
                .get_or_compute("kernel-hot", 12, || {
                    recomputes.fetch_add(1, Ordering::SeqCst);
                    thread::sleep(Duration::from_millis(20));
                    b"kernel-v2:42".to_vec()
                })
                .expect("single-flight get");
            let _ = tx.send(value);
        }));
    }
    drop(tx);
    let started = Instant::now();
    gate.wait();
    let values = rx.into_iter().take(H15_CALLERS).collect::<Vec<_>>();
    for handle in handles {
        handle
            .join()
            .map_err(|_| "cache caller panicked".to_string())?;
    }
    let elapsed_ms = started.elapsed().as_millis();
    let after = cache.stats();
    let unique_values = values
        .iter()
        .map(|value| String::from_utf8_lossy(value).to_string())
        .collect::<BTreeSet<_>>();
    let jitter = ttl_jitter_readback()?;
    let zero_jitter_edge = single_flight_edge_zero_jitter()?;
    let passed = values.len() == H15_CALLERS
        && unique_values.len() == 1
        && unique_values.contains("kernel-v2:42")
        && recomputes.load(Ordering::SeqCst) == 1
        && after.len == 1
        && jitter.spread_observed
        && zero_jitter_edge.recompute_count == 1
        && zero_jitter_edge.values_returned == 20;
    Ok((
        passed,
        json!({
            "trigger": "expired high-value kernel cache entry plus 100 concurrent identical misses",
            "expected": {
                "callers": H15_CALLERS,
                "single_flight_recompute_count": 1,
                "all_callers_receive": "kernel-v2:42",
                "ttl_jitter_configured_gt_ms": 0,
                "zero_jitter_edge_single_flight_count": 1
            },
            "actual": {
                "before": before,
                "after": after,
                "values_returned": values.len(),
                "unique_values": unique_values.into_iter().collect::<Vec<_>>(),
                "recompute_count": recomputes.load(Ordering::SeqCst),
                "elapsed_ms": elapsed_ms,
                "ttl_jitter": jitter,
                "zero_jitter_edge": zero_jitter_edge,
                "panic_free": true
            },
            "metrics_text": format!(
                "cache_stampede_single_flight_count{{suite=\"ph59_t04\",hazard=\"H15\"}} {}\ncalyx_cache_stampede_callers_total{{suite=\"ph59_t04\"}} {}\n",
                recomputes.load(Ordering::SeqCst), values.len()
            )
        }),
    ))
}

pub(super) fn probe_h16_slow_lens_hol(root: &Path) -> ProbeResult {
    let _dir = case_dir(root, "h16_slow_lens_hol")?;
    let mut registry = Registry::new();
    let slow = TimeoutLens::new("ph59-h16-slow", Duration::from_millis(LENS_TIMEOUT_MS));
    let fast_a = LengthLens::new("ph59-h16-fast-a", 0.0);
    let fast_b = LengthLens::new("ph59-h16-fast-b", 10.0);
    let slow_calls = slow.calls();
    let fast_a_calls = fast_a.calls();
    let fast_b_calls = fast_b.calls();
    let slow_id = registry
        .register_frozen(slow.clone(), slow.contract())
        .map_err(err)?;
    let fast_a_id = registry
        .register_frozen(fast_a.clone(), fast_a.contract())
        .map_err(err)?;
    let fast_b_id = registry
        .register_frozen(fast_b.clone(), fast_b.contract())
        .map_err(err)?;
    let inputs = h16_inputs();
    let controller =
        IngestMicrobatchController::new(IngestMicrobatchConfig::new(4096).with_breaker(1, 10_000));
    let before = controller.stats();
    let expected_batch_bytes = 2 * INGEST_MICROBATCH_INPUT_OVERHEAD_BYTES + 2 + 4;

    let started = Instant::now();
    let first = registry
        .measure_ingest_microbatch(&[slow_id, fast_a_id, fast_b_id], &inputs, &controller, 100)
        .map_err(err)?;
    let first_elapsed_ms = started.elapsed().as_millis();
    let after_first = controller.stats();
    let slow_calls_after_first = slow_calls.load(Ordering::SeqCst);
    let open_panel = registry
        .measure_ingest_microbatch(&[slow_id, fast_a_id, fast_b_id], &inputs, &controller, 101)
        .map_err(err)?;
    let after_open = controller.stats();
    let slow_calls_after_open = slow_calls.load(Ordering::SeqCst);
    slow.restore();
    let recovered = registry
        .measure_ingest_microbatch(
            &[slow_id, fast_a_id, fast_b_id],
            &inputs,
            &controller,
            10_200,
        )
        .map_err(err)?;
    let after_recovered = controller.stats();
    let all_slow_edge = all_lenses_slow_edge()?;

    let first_slow_error = outcome_error(&first, slow_id);
    let first_fast_a = dense_lengths(&first, fast_a_id);
    let first_fast_b = dense_lengths(&first, fast_b_id);
    let recovered_slow = dense_lengths(&recovered, slow_id);
    let passed = first_elapsed_ms <= u128::from(LENS_TIMEOUT_MS + 100)
        && first_slow_error.as_deref() == Some("CALYX_LENS_UNREACHABLE")
        && first_fast_a == vec![2.0, 4.0]
        && first_fast_b == vec![12.0, 14.0]
        && after_first.lens_timeouts_total == 1
        && after_first.breaker_trips_total == 1
        && after_open.open_breaker_count == 1
        && slow_calls_after_open == slow_calls_after_first
        && after_recovered.breaker_recoveries_total == 1
        && after_recovered.open_breaker_count == 0
        && recovered_slow == vec![102.0, 104.0]
        && all_slow_edge.passed;
    Ok((
        passed,
        json!({
            "trigger": "one timeout-bounded slow lens plus two fast lenses in one registry microbatch search",
            "expected": {
                "returns_within_ms": LENS_TIMEOUT_MS + 100,
                "slow_error_code": "CALYX_LENS_UNREACHABLE",
                "fast_lenses_present": true,
                "lens_timeout_total_gte": 1,
                "lens_breaker_trips_total_gte": 1,
                "breaker_recoveries_total_gte": 1
            },
            "actual": {
                "lens_ids": [slow_id, fast_a_id, fast_b_id],
                "hand_expected_batch_bytes": expected_batch_bytes,
                "before": before,
                "first_elapsed_ms": first_elapsed_ms,
                "first_panel": first,
                "after_first": after_first,
                "open_panel": open_panel,
                "after_open": after_open,
                "recovered_panel": recovered,
                "after_recovered": after_recovered,
                "slow_calls_after_first": slow_calls_after_first,
                "slow_calls_after_open": slow_calls_after_open,
                "fast_a_calls": fast_a_calls.load(Ordering::SeqCst),
                "fast_b_calls": fast_b_calls.load(Ordering::SeqCst),
                "fast_a_lengths": first_fast_a,
                "fast_b_lengths": first_fast_b,
                "recovered_slow_lengths": recovered_slow,
                "all_slow_edge": all_slow_edge,
                "panic_free": true
            },
            "metrics_text": controller.metrics_text() + &format!(
                "calyx_lens_timeout_total{{suite=\"ph59_t04\",hazard=\"H16\"}} {}\ncalyx_lens_breaker_trips_total{{suite=\"ph59_t04\",hazard=\"H16\"}} {}\n",
                after_recovered.lens_timeouts_total, after_recovered.breaker_trips_total
            )
        }),
    ))
}

#[derive(Clone, Debug, Serialize)]
struct SingleFlightEdge {
    recompute_count: usize,
    values_returned: usize,
    unique_values: Vec<String>,
}

fn single_flight_edge_zero_jitter() -> Result<SingleFlightEdge, String> {
    let clock = MutableClock::new(START_TS + 1_550);
    let cache = Arc::new(SingleFlightCache::new(
        4 * 1024,
        Duration::from_millis(10),
        Duration::ZERO,
        Arc::new(clock.clone()),
    )?);
    cache.insert_seed("edge-hot", b"old".to_vec(), 4)?;
    clock.advance(100);
    let recomputes = Arc::new(AtomicUsize::new(0));
    let gate = Arc::new(Barrier::new(21));
    let (tx, rx) = mpsc::channel();
    let mut handles = Vec::new();
    for _ in 0..20 {
        let cache = Arc::clone(&cache);
        let recomputes = Arc::clone(&recomputes);
        let gate = Arc::clone(&gate);
        let tx = tx.clone();
        handles.push(thread::spawn(move || {
            gate.wait();
            let value = cache
                .get_or_compute("edge-hot", 4, || {
                    recomputes.fetch_add(1, Ordering::SeqCst);
                    thread::sleep(Duration::from_millis(20));
                    b"edge".to_vec()
                })
                .expect("edge get");
            let _ = tx.send(value);
        }));
    }
    drop(tx);
    gate.wait();
    let values = rx.into_iter().take(20).collect::<Vec<_>>();
    for handle in handles {
        handle
            .join()
            .map_err(|_| "edge caller panicked".to_string())?;
    }
    let unique_values = values
        .iter()
        .map(|value| String::from_utf8_lossy(value).to_string())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    Ok(SingleFlightEdge {
        recompute_count: recomputes.load(Ordering::SeqCst),
        values_returned: values.len(),
        unique_values,
    })
}

#[derive(Clone, Debug, Serialize)]
struct AllSlowEdge {
    passed: bool,
    elapsed_ms: u128,
    timeout_ms: u64,
    outcomes: Vec<Option<String>>,
    stats_after: calyx_registry::IngestMicrobatchStats,
}

fn all_lenses_slow_edge() -> Result<AllSlowEdge, String> {
    let mut registry = Registry::new();
    let slow_a = TimeoutLens::new("ph59-h16-all-slow-a", Duration::from_millis(20));
    let slow_b = TimeoutLens::new("ph59-h16-all-slow-b", Duration::from_millis(20));
    let a = registry
        .register_frozen(slow_a.clone(), slow_a.contract())
        .map_err(err)?;
    let b = registry
        .register_frozen(slow_b.clone(), slow_b.contract())
        .map_err(err)?;
    let controller =
        IngestMicrobatchController::new(IngestMicrobatchConfig::new(4096).with_breaker(1, 1_000));
    let inputs = h16_inputs();
    let started = Instant::now();
    let panel = registry
        .measure_ingest_microbatch(&[a, b], &inputs, &controller, 7)
        .map_err(err)?;
    let elapsed_ms = started.elapsed().as_millis();
    let outcomes = [a, b]
        .into_iter()
        .map(|lens_id| outcome_error(&panel, lens_id))
        .collect::<Vec<_>>();
    let stats_after = controller.stats();
    let passed = elapsed_ms <= 100
        && outcomes
            .iter()
            .all(|code| code.as_deref() == Some("CALYX_LENS_UNREACHABLE"))
        && panel
            .outcomes
            .iter()
            .all(|outcome| outcome.status == IngestLensOutcomeStatus::Degraded)
        && stats_after.lens_timeouts_total == 2
        && stats_after.breaker_trips_total == 2;
    Ok(AllSlowEdge {
        passed,
        elapsed_ms,
        timeout_ms: 20,
        outcomes,
        stats_after,
    })
}

fn h16_inputs() -> [Input; 2] {
    [
        Input::new(Modality::Text, b"aa".to_vec()),
        Input::new(Modality::Text, b"bbbb".to_vec()),
    ]
}

fn outcome_error(readout: &calyx_registry::IngestPanelReadout, lens_id: LensId) -> Option<String> {
    readout
        .outcomes
        .iter()
        .find(|outcome| outcome.lens_id == lens_id)
        .and_then(|outcome| outcome.error_code.clone())
}
