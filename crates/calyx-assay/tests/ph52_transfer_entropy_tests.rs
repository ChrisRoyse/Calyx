use calyx_assay::{
    CALYX_TE_INSUFFICIENT_SAMPLES, Direction, TransferEntropyConfig, max_transfer_entropy_lag,
    transfer_entropy, transfer_entropy_sweep_with_config, transfer_entropy_with_config,
};
use calyx_core::FixedClock;
use proptest::prelude::*;
use serde_json::json;

mod ph52_signal_support;
mod ph52_support;

use ph52_signal_support::noise;
use ph52_support::write_readback;

type TestStream = Vec<(u64, f32)>;
const READBACK_LABEL: &str = "PH52_TE_READBACK";

fn clock() -> FixedClock {
    FixedClock::new(1_786_000_000)
}

fn fast_config() -> TransferEntropyConfig {
    TransferEntropyConfig {
        bootstrap_resamples: 20,
        ..TransferEntropyConfig::default()
    }
}

#[test]
fn planted_lag_two_a_drives_b_directionally() {
    let lag = 2;
    let (a, b) = planted_a_to_b(140, lag);
    let result = transfer_entropy_with_config(&a, &b, lag, &clock(), &fast_config()).unwrap();
    let sweep = transfer_entropy_sweep_with_config(&a, &b, &[1, 2, 4, 8], &clock(), &fast_config());

    println!(
        "t_a_to_b={:.6} t_b_to_a={:.6} dominant_direction={:?} lag={} ci=({:.6},{:.6})",
        result.t_a_to_b,
        result.t_b_to_a,
        result.dominant_direction,
        result.lag,
        result.ci_95.0,
        result.ci_95.1
    );
    write_readback(
        READBACK_LABEL,
        "ph52-te-planted.json",
        json!({
            "case": "planted_a_to_b_lag2",
            "result": result,
            "sweep_lags": sweep,
            "max_te_lag": max_transfer_entropy_lag(&sweep),
        }),
    );

    assert!(!result.provisional);
    assert_eq!(result.dominant_direction, Direction::AToB);
    assert!(result.t_a_to_b > result.t_b_to_a + 0.1, "{result:?}");
    assert!(result.ci_95.0 <= result.t_a_to_b && result.t_a_to_b <= result.ci_95.1);
    assert!(result.difference_ci_95.0 > 0.0, "{result:?}");
    assert_eq!(max_transfer_entropy_lag(&sweep), Some(2));
}

#[test]
fn independent_streams_are_unclear_and_near_zero() {
    let (a, b) = independent_streams(140);
    let result = transfer_entropy_with_config(&a, &b, 2, &clock(), &fast_config()).unwrap();

    println!(
        "independent t_a_to_b={:.6} t_b_to_a={:.6} dominant_direction={:?}",
        result.t_a_to_b, result.t_b_to_a, result.dominant_direction
    );
    write_readback(
        READBACK_LABEL,
        "ph52-te-independent.json",
        json!({ "case": "independent_streams", "result": result }),
    );

    assert!(!result.provisional);
    assert_eq!(result.dominant_direction, Direction::Unclear);
    assert!(result.t_a_to_b <= 0.2, "{result:?}");
    assert!(result.t_b_to_a <= 0.2, "{result:?}");
}

#[test]
#[ignore = "manual FSV exercises the default 500-resample TE bootstrap"]
fn transfer_entropy_default_bootstrap_fsv() {
    let lag = 2;
    let (a, b) = planted_a_to_b(100, lag);
    let result = transfer_entropy(&a, &b, lag, &clock()).unwrap();
    println!(
        "default_bootstrap t_a_to_b={:.6} t_b_to_a={:.6} dominant_direction={:?}",
        result.t_a_to_b, result.t_b_to_a, result.dominant_direction
    );
    write_readback(
        READBACK_LABEL,
        "ph52-te-default-bootstrap.json",
        json!({
            "case": "default_500_bootstrap_planted_a_to_b",
            "result": result,
        }),
    );
    assert_eq!(result.dominant_direction, Direction::AToB);
    assert!(result.t_a_to_b > result.t_b_to_a + 0.1, "{result:?}");
    assert!(result.ci_95.0 <= result.t_a_to_b && result.t_a_to_b <= result.ci_95.1);
}

#[test]
fn transfer_entropy_edges_fail_closed_with_code() {
    let empty: Vec<(u64, f32)> = Vec::new();
    let single = vec![(0, 1.0)];
    let empty_result = transfer_entropy(&empty, &empty, 2, &clock()).unwrap();
    let single_result = transfer_entropy(&single, &single, 2, &clock()).unwrap();
    let lag_zero_stream = independent_streams(80).0;
    let lag_zero = transfer_entropy_with_config(
        &lag_zero_stream,
        &lag_zero_stream,
        0,
        &clock(),
        &fast_config(),
    )
    .unwrap();

    write_readback(
        READBACK_LABEL,
        "ph52-te-edges.json",
        json!({
            "empty": empty_result,
            "single": single_result,
            "lag_zero": lag_zero,
            "invalid_duplicate_error": duplicate_timestamp_error(),
        }),
    );

    assert_eq!(
        empty_result.error_code.as_deref(),
        Some(CALYX_TE_INSUFFICIENT_SAMPLES)
    );
    assert_eq!(
        single_result.error_code.as_deref(),
        Some(CALYX_TE_INSUFFICIENT_SAMPLES)
    );
    assert!(lag_zero.t_a_to_b.is_finite());
    assert!(!lag_zero.provisional);
    assert_eq!(
        duplicate_timestamp_error(),
        "CALYX_ASSAY_INSUFFICIENT_SAMPLES"
    );
}

proptest! {
    #[test]
    fn below_quorum_is_always_provisional(n in 0usize..30) {
        let a = simple_stream(n, 11);
        let b = simple_stream(n, 29);
        let result = transfer_entropy(&a, &b, 1, &clock()).unwrap();
        prop_assert!(result.provisional);
        prop_assert_eq!(result.error_code.as_deref(), Some(CALYX_TE_INSUFFICIENT_SAMPLES));
    }
}

fn duplicate_timestamp_error() -> &'static str {
    let bad = vec![(1, 0.1), (1, 0.2)];
    transfer_entropy(&bad, &bad, 1, &clock()).unwrap_err().code
}

fn planted_a_to_b(n: usize, lag: usize) -> (TestStream, TestStream) {
    let a = simple_stream(n, 7);
    let mut b = Vec::with_capacity(n);
    for t in 0..n {
        let value = if t >= lag {
            let driver = a[t - lag].1;
            driver + 0.01 * (noise(t as u64, 41) - 0.5)
        } else {
            noise(t as u64, 73)
        };
        b.push((t as u64, value));
    }
    (a, b)
}

fn independent_streams(n: usize) -> (TestStream, TestStream) {
    (simple_stream(n, 17), simple_stream(n, 83))
}

fn simple_stream(n: usize, salt: u64) -> TestStream {
    (0..n)
        .map(|t| (t as u64, 0.2 + 0.6 * noise(t as u64, salt)))
        .collect()
}
