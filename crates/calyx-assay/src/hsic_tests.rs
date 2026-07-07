use super::*;

fn approx(actual: f32, expected: f32, tol: f32, what: &str) {
    assert!(
        (actual - expected).abs() <= tol,
        "{what}: got {actual}, expected {expected} (tol {tol})"
    );
}

fn fixed(sigma: f64) -> HsicConfig {
    HsicConfig {
        bandwidth_x: Some(sigma),
        bandwidth_y: Some(sigma),
    }
}

#[test]
fn biased_and_unbiased_match_reference_values() {
    // RBF σ=1 regression targets (verified by an independent numpy impl).
    // Case C: X=Y=[1,2,3,4] → HSIC_b=0.1186381508, HSIC_u=0.1135845698.
    let e = hsic_estimators_with_config(&[1.0, 2.0, 3.0, 4.0], &[1.0, 2.0, 3.0, 4.0], fixed(1.0))
        .unwrap();
    approx(e.hsic_biased, 0.118_638_15, 1e-6, "HSIC_b case C");
    approx(e.hsic_unbiased, 0.113_584_57, 1e-6, "HSIC_u case C");

    // Case D: X=Y=[0,1,2,3,4] (n=5) → HSIC_b=0.1304152544, HSIC_u=0.1081209974.
    let d = hsic_estimators_with_config(
        &[0.0, 1.0, 2.0, 3.0, 4.0],
        &[0.0, 1.0, 2.0, 3.0, 4.0],
        fixed(1.0),
    )
    .unwrap();
    approx(d.hsic_biased, 0.130_415_25, 1e-6, "HSIC_b case D");
    approx(d.hsic_unbiased, 0.108_121, 1e-6, "HSIC_u case D");
}

#[test]
fn reversal_invariance() {
    // Case E: X=[1,2,3,4], Y=[4,3,2,1] equals case C (jointly reversing the
    // order leaves HSIC unchanged).
    let e = hsic_estimators_with_config(&[1.0, 2.0, 3.0, 4.0], &[4.0, 3.0, 2.0, 1.0], fixed(1.0))
        .unwrap();
    approx(e.hsic_biased, 0.118_638_15, 1e-6, "HSIC_b case E == C");
}

#[test]
fn gamma_machinery_matches_reference() {
    // n=8, y=x, σ=1 (verified independently):
    //   T=0.99855307, α=83.98182093, β=0.00795453, p=3.403e-05.
    let x = [0.0f32, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0];
    let r = hsic_with_config(&x, &x, fixed(1.0)).unwrap();
    approx(r.test_statistic, 0.998_553, 1e-5, "T");
    approx(r.gamma_shape, 83.981_82, 1e-1, "α");
    approx(r.gamma_scale, 0.007_954_53, 1e-6, "β");
    assert!(r.p_value < 1e-3, "perfect dependence → tiny p: {r:?}");
}

#[test]
fn gamma_test_discriminates_dependence_at_scale() {
    // n=64. Dependent y=x² (Pearson-blind, non-monotone about 0) → reject;
    // independent stream → accept. σ from the median heuristic.
    let n = 64usize;
    let xs: Vec<f32> = (0..n).map(|i| (splitmix(i as u64) * 10.0) as f32).collect();
    let y_dep: Vec<f32> = xs.iter().map(|&v| v * v).collect();
    let y_ind: Vec<f32> = (0..n)
        .map(|i| (splitmix(9000 + i as u64) * 10.0) as f32)
        .collect();
    let dep = hsic(&xs, &y_dep).unwrap();
    let ind = hsic(&xs, &y_ind).unwrap();
    assert!(dep.p_value < 0.01, "dependence rejected: {dep:?}");
    assert!(ind.p_value > 0.05, "independence accepted: {ind:?}");
}

#[test]
fn permutation_test_agrees_on_dependence() {
    let n = 40usize;
    let xs: Vec<f32> = (0..n).map(|i| (splitmix(i as u64) * 6.0) as f32).collect();
    let y_dep: Vec<f32> = xs.iter().map(|&v| v * v).collect();
    let y_ind: Vec<f32> = (0..n)
        .map(|i| (splitmix(555 + i as u64) * 6.0) as f32)
        .collect();
    let dep = hsic_permutation_test(&xs, &y_dep, HsicPermConfig::default()).unwrap();
    let ind = hsic_permutation_test(&xs, &y_ind, HsicPermConfig::default()).unwrap();
    assert!(dep.p_value < 0.01, "dependence rejected: {dep:?}");
    assert!(ind.p_value > 0.05, "independence accepted: {ind:?}");
}

#[test]
fn permutation_test_is_deterministic_for_a_seed() {
    let x = [1.0f32, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0];
    let y = [1.0f32, 4.0, 9.0, 16.0, 25.0, 36.0, 49.0, 64.0];
    let cfg = HsicPermConfig {
        permutations: 300,
        seed: 42,
        ..Default::default()
    };
    let a = hsic_permutation_test(&x, &y, cfg).unwrap();
    let b = hsic_permutation_test(&x, &y, cfg).unwrap();
    assert_eq!(a.p_value, b.p_value);
    assert_eq!(a.ge_count, b.ge_count);
}

#[test]
fn fails_closed_on_bad_input() {
    assert_eq!(
        hsic_estimators(&[1.0, 2.0, 3.0], &[1.0, 2.0])
            .unwrap_err()
            .code,
        "CALYX_ASSAY_INSUFFICIENT_SAMPLES"
    );
    assert_eq!(
        hsic_estimators(&[1.0, 2.0, 3.0], &[1.0, 2.0, 3.0])
            .unwrap_err()
            .code,
        "CALYX_ASSAY_INSUFFICIENT_SAMPLES" // n < 4
    );
    assert_eq!(
        hsic_estimators(&[1.0, f32::NAN, 3.0, 4.0], &[1.0, 2.0, 3.0, 4.0])
            .unwrap_err()
            .code,
        "CALYX_ASSAY_INSUFFICIENT_SAMPLES"
    );
}

#[test]
fn fails_closed_on_constant_column() {
    let e = hsic_estimators(&[5.0, 5.0, 5.0, 5.0, 5.0], &[1.0, 2.0, 3.0, 4.0, 5.0]).unwrap_err();
    assert_eq!(e.code, "CALYX_ASSAY_DEGENERATE_INPUT");
}

#[test]
fn gamma_test_fails_closed_below_six() {
    let e = hsic(&[1.0, 2.0, 3.0, 4.0, 5.0], &[1.0, 4.0, 9.0, 16.0, 25.0]).unwrap_err();
    assert_eq!(e.code, "CALYX_ASSAY_INSUFFICIENT_SAMPLES");
}

/// Deterministic splitmix64 → uniform f64 in [0,1); reproducible, no RNG.
fn splitmix(mut x: u64) -> f64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = x;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    ((z >> 11) as f64) / ((1_u64 << 53) as f64)
}
