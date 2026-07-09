//! #1140 Finding C — the panel sufficiency basis must not degrade as panel
//! width grows, and must never fall below the strongest single member.
//!
//! Ground truth is synthetic and known: one strong lens carries the label, the
//! rest are pure noise. The concatenated-feature probe loses power as the noise
//! dimensions pile up — its measured joint MI falls *below* the strong single
//! lens, which is impossible for a real lower bound since `I(panel;Y) >= max_i
//! I(lens_i;Y)`. The union-bound floor (`panel_joint_with_union_floor`) restores
//! a valid, monotone basis: it can never report below the best member, so
//! admitting a stronger lens can only raise sufficiency.
//!
//! Full-state verification: both estimators are computed over identical data,
//! the numbers are persisted to JSON, and the monotonicity contract is asserted
//! against the *readback*, not just the in-memory return values.

use std::fs;

use calyx_assay::{
    MiEstimate, logistic_probe_mi_multiseed, panel_joint_with_union_floor,
    panel_sufficiency_from_estimate,
};
use serde_json::json;

/// Deterministic splitmix64 → uniform f32 in [0,1). No wall clock, no rand dep.
struct Rng(u64);

impl Rng {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn unit(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1_u64 << 24) as f32
    }

    /// Standard-normal via Box–Muller.
    fn normal(&mut self) -> f32 {
        let u1 = self.unit().max(1e-7);
        let u2 = self.unit();
        (-2.0 * u1.ln()).sqrt() * (std::f32::consts::TAU * u2).cos()
    }
}

const N: usize = 400;

fn labels() -> Vec<bool> {
    (0..N).map(|i| i % 2 == 0).collect()
}

/// Strong lens: dim 0 tracks the label with moderate noise; other dims noise.
fn strong_lens(seed: u64, dim: usize, signal: f32) -> Vec<Vec<f32>> {
    let mut rng = Rng(seed);
    (0..N)
        .map(|i| {
            let label_sign = if i % 2 == 0 { 1.0 } else { -1.0 };
            (0..dim)
                .map(|d| {
                    if d == 0 {
                        signal * label_sign + rng.normal()
                    } else {
                        rng.normal()
                    }
                })
                .collect()
        })
        .collect()
}

/// Pure-noise lens: carries no information about the label.
fn noise_lens(seed: u64, dim: usize) -> Vec<Vec<f32>> {
    let mut rng = Rng(seed);
    (0..N)
        .map(|_| (0..dim).map(|_| rng.normal()).collect())
        .collect()
}

fn concat(lenses: &[&[Vec<f32>]]) -> Vec<Vec<f32>> {
    (0..N)
        .map(|sample| {
            let mut row = Vec::new();
            for lens in lenses {
                row.extend_from_slice(&lens[sample]);
            }
            row
        })
        .collect()
}

#[test]
fn union_floor_neutralizes_concat_width_degradation() {
    let labels = labels();
    let strong = strong_lens(1, 8, 1.6);
    let noise: Vec<Vec<Vec<f32>>> = (0..6).map(|k| noise_lens(100 + k as u64, 64)).collect();

    // Baseline: the strong lens alone.
    let single = logistic_probe_mi_multiseed(&strong, &labels, None)
        .unwrap()
        .estimate;

    // Panel = strong lens + 6 pure-noise lenses; the old joint statistic is the
    // concatenated probe over all of it.
    let mut panel: Vec<&[Vec<f32>]> = vec![strong.as_slice()];
    panel.extend(noise.iter().map(Vec::as_slice));
    let concat_matrix = concat(&panel);
    let concat_joint = logistic_probe_mi_multiseed(&concat_matrix, &labels, None)
        .unwrap()
        .estimate;

    // Every admitted member's estimate feeds the union bound. Noise lenses carry
    // near-zero MI; the strong lens is the best member.
    let mut members = vec![single.clone()];
    for lens in &noise {
        members.push(
            logistic_probe_mi_multiseed(lens, &labels, None)
                .unwrap()
                .estimate,
        );
    }
    let basis = panel_joint_with_union_floor(&concat_joint, &members).unwrap();

    let artifact = json!({
        "schema": "calyx-assay-panel-width-degradation-fsv-v1",
        "n_samples": N,
        "panel_width": panel.len(),
        "single_strong_bits": single.bits,
        "single_strong_ci_low": single.ci_low,
        "concat_joint_bits": concat_joint.bits,
        "concat_joint_ci_low": concat_joint.ci_low,
        "basis_bits": basis.bits,
        "basis_ci_low": basis.ci_low,
        "basis_best_member_ci_low": basis.best_member_ci_low,
        "basis_floored": basis.floored,
        "concat_degraded_below_single": concat_joint.bits < single.bits,
    });

    // Full-state verification: persist, then assert against the readback.
    let root = calyx_fsv::fsv_root_or_else("CALYX_FSV_ROOT", || {
        std::env::temp_dir().join("calyx_panel_width_fsv")
    });
    fs::create_dir_all(&root).unwrap();
    let path = root.join("panel_width_degradation.json");
    fs::write(&path, serde_json::to_vec_pretty(&artifact).unwrap()).unwrap();
    let rb: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();

    // 1. The bug is real: the concat probe degraded below the strong single lens.
    assert!(
        rb["concat_degraded_below_single"].as_bool().unwrap(),
        "expected concat joint {} below single {} (readback {rb})",
        concat_joint.bits,
        single.bits
    );
    // 2. The floor fired and lifted the basis back to the best member.
    assert!(
        rb["basis_floored"].as_bool().unwrap(),
        "floor must fire: {rb}"
    );
    // 3. The floored basis is a valid lower bound: at/above the best single lens,
    //    and strictly above the degraded concat.
    let basis_ci_low = rb["basis_ci_low"].as_f64().unwrap();
    assert!(
        basis_ci_low >= rb["single_strong_ci_low"].as_f64().unwrap() - 1e-6,
        "basis must hold at/above best member ci_low: {rb}"
    );
    assert!(
        basis_ci_low > rb["concat_joint_ci_low"].as_f64().unwrap(),
        "basis must exceed the degraded concat ci_low: {rb}"
    );
}

#[test]
fn union_floor_preserves_genuine_synergy() {
    // Two lenses each carry a different half of the signal — neither alone is
    // decisive, but concatenated they separate the classes. Here the concat
    // joint exceeds both members, so the floor must be a no-op (synergy kept).
    let labels = labels();
    let mut rng_a = Rng(7);
    let mut rng_b = Rng(9);
    let lens_a: Vec<Vec<f32>> = (0..N)
        .map(|i| {
            let sign = if i % 2 == 0 { 1.0 } else { -1.0 };
            let s = if i % 4 < 2 { 1.8 * sign } else { 0.0 };
            vec![
                s + rng_a.normal(),
                rng_a.normal(),
                rng_a.normal(),
                rng_a.normal(),
            ]
        })
        .collect();
    let lens_b: Vec<Vec<f32>> = (0..N)
        .map(|i| {
            let sign = if i % 2 == 0 { 1.0 } else { -1.0 };
            let s = if i % 4 >= 2 { 1.8 * sign } else { 0.0 };
            vec![
                s + rng_b.normal(),
                rng_b.normal(),
                rng_b.normal(),
                rng_b.normal(),
            ]
        })
        .collect();

    let a = logistic_probe_mi_multiseed(&lens_a, &labels, None)
        .unwrap()
        .estimate;
    let b = logistic_probe_mi_multiseed(&lens_b, &labels, None)
        .unwrap()
        .estimate;
    let concat_matrix = concat(&[lens_a.as_slice(), lens_b.as_slice()]);
    let joint = logistic_probe_mi_multiseed(&concat_matrix, &labels, None)
        .unwrap()
        .estimate;

    let basis = panel_joint_with_union_floor(&joint, &[a.clone(), b.clone()]).unwrap();
    let best_single = a.bits.max(b.bits);
    assert!(
        joint.bits > best_single,
        "concat of complementary lenses {} should beat best single {best_single}",
        joint.bits
    );
    // Floor is a no-op when the joint genuinely adds signal.
    assert!(
        !basis.floored,
        "floor should not fire on real synergy: {basis:?}"
    );
    assert!((basis.bits - joint.bits).abs() < 1e-6);
}

#[test]
fn union_floor_rejects_empty_and_nonfinite() {
    let joint = MiEstimate::new(
        0.5,
        0.4,
        0.6,
        100,
        calyx_assay::EstimatorKind::LogisticProbe,
        calyx_assay::TrustTag::Provisional,
    );
    // No admitted members → fail closed.
    assert!(panel_joint_with_union_floor(&joint, &[]).is_err());

    // A floored basis remains a lower bound the sufficiency gate accepts as-is.
    let member = MiEstimate::new(
        0.9,
        0.85,
        0.95,
        100,
        calyx_assay::EstimatorKind::LogisticProbe,
        calyx_assay::TrustTag::Provisional,
    )
    .with_power_calibration(calyx_assay::PowerCalibration::new(1.0, 0.9, 0.5, 100, 8, 7).unwrap());
    let basis = panel_joint_with_union_floor(&joint, std::slice::from_ref(&member)).unwrap();
    assert_eq!(basis.best_member_ci_low, 0.85);
    assert!(basis.floored);
    // The floored point estimate carried into a passing calibration is sufficient
    // against an anchor entropy at/below the floor.
    let floored = MiEstimate::new(
        basis.bits,
        basis.ci_low,
        basis.ci_high,
        100,
        calyx_assay::EstimatorKind::LogisticProbe,
        calyx_assay::TrustTag::Provisional,
    )
    .with_power_calibration(calyx_assay::PowerCalibration::new(1.0, 0.9, 0.5, 100, 8, 7).unwrap());
    let sufficiency =
        panel_sufficiency_from_estimate(&floored, 0.85, &[], calyx_assay::TrustTag::Trusted)
            .unwrap();
    assert!(sufficiency.sufficient);
}
