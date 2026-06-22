use calyx_assay::{
    CALYX_TC_INSUFFICIENT_SAMPLES, IISign, TotalCorrelationConfig, interaction_information,
    interaction_information_with_config, min_quorum_tc, total_correlation,
    total_correlation_with_config,
};
use calyx_core::FixedClock;
use proptest::prelude::*;
use serde_json::json;

mod ph52_signal_support;
mod ph52_support;

use ph52_signal_support::noise;
use ph52_support::write_readback;

const READBACK_LABEL: &str = "PH52_TC_READBACK";

fn clock() -> FixedClock {
    FixedClock::new(1_786_100_000)
}

fn fast_config() -> TotalCorrelationConfig {
    TotalCorrelationConfig {
        bootstrap_resamples: 20,
        ..TotalCorrelationConfig::default()
    }
}

#[test]
fn total_correlation_redundant_panel_lowers_n_eff() {
    let panel = redundant_panel(240);
    let result = total_correlation_with_config(&panel, &clock(), &fast_config()).unwrap();
    println!(
        "TC redundant={:.6} n_eff={:.6} ci=({:.6},{:.6}) quorum={}",
        result.tc,
        result.n_eff,
        result.ci_95.0,
        result.ci_95.1,
        min_quorum_tc(panel.len())
    );
    write_readback(
        READBACK_LABEL,
        "ph52-tc-redundant.json",
        json!({ "case": "redundant_panel", "result": result }),
    );

    assert!(!result.provisional);
    assert!(result.tc > 0.1, "{result:?}");
    assert!(result.n_eff < 2.7, "{result:?}");
    assert!(result.ci_95.0 <= result.tc && result.tc <= result.ci_95.1);
}

#[test]
fn total_correlation_independent_panel_preserves_n_eff() {
    let panel = independent_panel(300);
    let result = total_correlation_with_config(&panel, &clock(), &fast_config()).unwrap();
    println!(
        "TC independent={:.6} n_eff={:.6} ci=({:.6},{:.6})",
        result.tc, result.n_eff, result.ci_95.0, result.ci_95.1
    );
    write_readback(
        READBACK_LABEL,
        "ph52-tc-independent.json",
        json!({ "case": "independent_panel", "result": result }),
    );

    assert!(!result.provisional);
    assert!(result.tc <= result.ci_95.1.max(0.5), "{result:?}");
    assert!(result.n_eff >= 2.7, "{result:?}");
}

#[test]
fn interaction_information_classifies_redundant_and_synergistic() {
    let (ra, rb, rc) = redundant_triple(260);
    let redundant =
        interaction_information_with_config(&ra, &rb, &rc, &clock(), &fast_config()).unwrap();
    let (sa, sb, sc) = xor_type_synergy_triple(800);
    let synergy =
        interaction_information_with_config(&sa, &sb, &sc, &clock(), &fast_config()).unwrap();
    println!(
        "IISign redundant={:?} ii={:.6} ci=({:.6},{:.6})",
        redundant.sign, redundant.ii, redundant.ci_95.0, redundant.ci_95.1
    );
    println!(
        "IISign synergy={:?} ii={:.6} ci=({:.6},{:.6})",
        synergy.sign, synergy.ii, synergy.ci_95.0, synergy.ci_95.1
    );
    write_readback(
        READBACK_LABEL,
        "ph52-tc-ii.json",
        json!({
            "case": "interaction_information",
            "redundant": redundant,
            "synergy": synergy,
        }),
    );

    assert_eq!(redundant.sign, IISign::Redundant, "{redundant:?}");
    assert!(redundant.ii > 0.0, "{redundant:?}");
    assert_eq!(synergy.sign, IISign::Synergistic, "{synergy:?}");
    assert!(synergy.ii < 0.0, "{synergy:?}");
}

#[test]
#[ignore = "manual FSV exercises the default 500-resample TC bootstrap"]
fn total_correlation_default_bootstrap_fsv() {
    let panel = redundant_panel(180);
    let result = total_correlation(&panel, &clock()).unwrap();
    println!(
        "default_bootstrap TC={:.6} n_eff={:.6} ci=({:.6},{:.6})",
        result.tc, result.n_eff, result.ci_95.0, result.ci_95.1
    );
    write_readback(
        READBACK_LABEL,
        "ph52-tc-default-bootstrap.json",
        json!({ "case": "default_500_bootstrap_redundant_panel", "result": result }),
    );
    assert!(result.tc > 0.1, "{result:?}");
    assert!(result.n_eff < 2.7, "{result:?}");
    assert!(result.ci_95.0 <= result.tc && result.tc <= result.ci_95.1);
}

#[test]
fn total_correlation_edges_fail_closed_with_code() {
    let single = vec![base_signal(80)];
    let single_result = total_correlation_with_config(&single, &clock(), &fast_config()).unwrap();
    let identical = identical_panel(180);
    let identical_result =
        total_correlation_with_config(&identical, &clock(), &fast_config()).unwrap();
    let below = independent_panel(40);
    let below_result = total_correlation(&below, &clock()).unwrap();
    let below_ii = interaction_information(&[], &[], &[], &clock()).unwrap();
    let unclear = independent_interaction_sign(180);

    write_readback(
        READBACK_LABEL,
        "ph52-tc-edges.json",
        json!({
            "single_slot": single_result,
            "identical_slots": identical_result,
            "below_quorum": below_result,
            "below_quorum_ii": below_ii,
            "ragged_error": ragged_error_code(),
            "nonfinite_error": nonfinite_error_code(),
            "independent_ii_unclear": unclear,
        }),
    );

    assert_eq!(single_result.tc, 0.0);
    assert_eq!(single_result.n_eff, 1.0);
    assert!(identical_result.tc > 0.1, "{identical_result:?}");
    assert!(identical_result.n_eff <= 1.2, "{identical_result:?}");
    assert!(below_result.provisional);
    assert_eq!(
        below_result.error_code.as_deref(),
        Some(CALYX_TC_INSUFFICIENT_SAMPLES)
    );
    assert!(below_ii.provisional);
    assert_eq!(
        below_ii.error_code.as_deref(),
        Some(CALYX_TC_INSUFFICIENT_SAMPLES)
    );
    assert_eq!(ragged_error_code(), "CALYX_ASSAY_INSUFFICIENT_SAMPLES");
    assert_eq!(nonfinite_error_code(), "CALYX_ASSAY_INSUFFICIENT_SAMPLES");
    assert_eq!(unclear.sign, IISign::Unclear, "{unclear:?}");
    assert!(unclear.ci_95.0 <= 0.0 && 0.0 <= unclear.ci_95.1);
}

proptest! {
    #[test]
    fn n_eff_bounds_and_tc_nonnegative(slot_count in 1usize..5, n in 0usize..90) {
        let panel = generated_panel(slot_count, n);
        let result = total_correlation_with_config(&panel, &clock(), &fast_config()).unwrap();
        prop_assert!(result.tc >= 0.0);
        prop_assert!(result.n_eff >= 1.0);
        prop_assert!(result.n_eff <= slot_count as f32);
    }
}

fn independent_interaction_sign(n: usize) -> calyx_assay::IIResult {
    let a = seeded_signal(n, 211);
    let b = seeded_signal(n, 307);
    let c = seeded_signal(n, 401);
    interaction_information_with_config(&a, &b, &c, &clock(), &fast_config()).unwrap()
}

fn ragged_error_code() -> &'static str {
    let bad = vec![vec![1.0, 2.0], vec![1.0]];
    total_correlation(&bad, &clock()).unwrap_err().code
}

fn nonfinite_error_code() -> &'static str {
    let bad = vec![vec![1.0, f32::NAN], vec![2.0, 3.0]];
    total_correlation(&bad, &clock()).unwrap_err().code
}

fn redundant_panel(n: usize) -> Vec<Vec<f32>> {
    let base = base_signal(n);
    vec![
        base.clone(),
        jittered(&base, 17, 0.015),
        jittered(&base, 29, 0.015),
    ]
}

fn identical_panel(n: usize) -> Vec<Vec<f32>> {
    let base = base_signal(n);
    vec![base.clone(), base.clone(), base]
}

fn independent_panel(n: usize) -> Vec<Vec<f32>> {
    vec![
        seeded_signal(n, 61),
        seeded_signal(n, 113),
        seeded_signal(n, 181),
    ]
}

fn redundant_triple(n: usize) -> (Vec<f32>, Vec<f32>, Vec<f32>) {
    let base = base_signal(n);
    (
        base.clone(),
        jittered(&base, 71, 0.012),
        jittered(&base, 89, 0.012),
    )
}

fn xor_type_synergy_triple(n: usize) -> (Vec<f32>, Vec<f32>, Vec<f32>) {
    let mut a = Vec::with_capacity(n);
    let mut b = Vec::with_capacity(n);
    let mut c = Vec::with_capacity(n);
    for t in 0..n {
        let x = normalish(t as u64, 503);
        let y = normalish(t as u64, 607);
        a.push(x);
        b.push(y);
        c.push(x * y + 0.01 * normalish(t as u64, 709));
    }
    (a, b, c)
}

fn generated_panel(slot_count: usize, n: usize) -> Vec<Vec<f32>> {
    (0..slot_count)
        .map(|slot| seeded_signal(n, 1_000 + slot as u64 * 37))
        .collect()
}

fn base_signal(n: usize) -> Vec<f32> {
    (0..n)
        .map(|t| {
            let phase = t as f32 / 19.0;
            4.0 * normalish(t as u64, 13) + phase.sin() + 0.4 * (2.0 * phase).cos()
        })
        .collect()
}

fn seeded_signal(n: usize, salt: u64) -> Vec<f32> {
    (0..n)
        .map(|t| 4.0 * normalish(t as u64, salt) + 0.3 * ((t as f32 + salt as f32).sin()))
        .collect()
}

fn jittered(base: &[f32], salt: u64, scale: f32) -> Vec<f32> {
    base.iter()
        .enumerate()
        .map(|(t, &value)| value + scale * normalish(t as u64, salt))
        .collect()
}

fn normalish(t: u64, salt: u64) -> f32 {
    (0..6).map(|offset| noise(t, salt + offset)).sum::<f32>() - 3.0
}
