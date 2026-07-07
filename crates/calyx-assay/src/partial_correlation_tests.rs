use super::*;

fn approx(actual: f32, expected: f32, tol: f32, what: &str) {
    assert!(
        (actual - expected).abs() <= tol,
        "{what}: got {actual}, expected {expected} (tol {tol})"
    );
}

#[test]
fn pearson_perfect_positive_is_one() {
    let x = [1.0f32, 2.0, 3.0, 4.0, 5.0];
    let y = [2.0f32, 4.0, 6.0, 8.0, 10.0];
    let r = pearson(&x, &y).unwrap();
    approx(r.r, 1.0, 1e-6, "r");
    assert!(r.p_value < 1e-6, "perfect r must be significant: {r:?}");
    approx(r.ci_high, 1.0, 1e-6, "ci_high");
}

#[test]
fn pearson_perfect_negative_is_minus_one() {
    let x = [1.0f32, 2.0, 3.0, 4.0, 5.0];
    let y = [10.0f32, 8.0, 6.0, 4.0, 2.0];
    let r = pearson(&x, &y).unwrap();
    approx(r.r, -1.0, 1e-6, "r");
}

#[test]
fn pearson_matches_known_value() {
    // x=[1,2,3,4,5], y=[2,1,4,3,6]: numpy corrcoef = 0.8219949. df=3,
    // t = r·√(3/(1−r²)) = 2.5 exactly; two-sided incomplete-beta p = 0.0877066.
    let x = [1.0f32, 2.0, 3.0, 4.0, 5.0];
    let y = [2.0f32, 1.0, 4.0, 3.0, 6.0];
    let r = pearson(&x, &y).unwrap();
    approx(r.r, 0.821_994_9, 1e-6, "r");
    approx(r.t_statistic, 2.5, 1e-4, "t");
    approx(r.p_value, 0.087_706_6, 1e-4, "p");
}

#[test]
fn partial_removes_a_pure_confounder() {
    // Z drives both X and Y: X = 3Z + a, Y = 3Z + b, where a=[+1,−1,…] and
    // b=[+1,+1,−1,−1,…] are mean-zero residuals with corr(a,b)=0 exactly, so
    // once Z is held fixed X and Y share essentially nothing. Independently
    // computed: raw r_xy = 0.9777, partial r_xy·z = −0.1085.
    let z = [0.0f32, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0];
    let x = [1.0f32, 2.0, 7.0, 8.0, 13.0, 14.0, 19.0, 20.0];
    let y = [1.0f32, 4.0, 5.0, 8.0, 13.0, 16.0, 17.0, 20.0];
    let raw = pearson(&x, &y).unwrap();
    let pc = partial_correlation(&x, &y, &z).unwrap();
    approx(raw.r, 0.977_7, 1e-3, "raw r (confounded)");
    approx(
        pc.partial_r,
        -0.108_5,
        1e-3,
        "partial r after controlling Z",
    );
    assert!(
        pc.partial_r.abs() < raw.r - 0.3,
        "controlling for Z must collapse the association: raw={} partial={}",
        raw.r,
        pc.partial_r
    );
    approx(pc.zero_order_r, raw.r, 1e-6, "zero_order echo");
}

#[test]
fn first_order_matches_precision_matrix() {
    // The two independent derivations must agree for a single control.
    let x = [2.0f32, 4.0, 1.0, 7.0, 3.0, 9.0, 5.0, 6.0];
    let y = [1.0f32, 3.0, 2.0, 8.0, 4.0, 7.0, 6.0, 5.0];
    let z = [3.0f32, 1.0, 4.0, 2.0, 8.0, 5.0, 7.0, 6.0];
    let a = partial_correlation(&x, &y, &z).unwrap();
    let b = partial_correlation_controlling(&x, &y, &[&z]).unwrap();
    approx(a.partial_r, b.partial_r, 1e-5, "first-order vs precision");
    approx(a.p_value, b.p_value, 1e-5, "p agreement");
}

#[test]
fn partial_matches_known_value() {
    // Independently computed (numpy corrcoef + Numerical-Recipes betai). x,y,z:
    //   r_xy = 0.7917947, r_xz = -0.4857143, r_yz = -0.3464102
    //   partial r_xy·z = (r_xy - r_xz r_yz)/√((1-r_xz²)(1-r_yz²)) = 0.7604172
    //   df = 3, t = 2.0280418, two-sided p = 0.1355998
    let x = [1.0f32, 2.0, 3.0, 4.0, 5.0, 6.0];
    let y = [2.0f32, 1.0, 4.0, 3.0, 7.0, 5.0];
    let z = [5.0f32, 6.0, 2.0, 1.0, 4.0, 3.0];
    let pc = partial_correlation(&x, &y, &z).unwrap();
    approx(pc.partial_r, 0.760_417_2, 1e-4, "partial r");
    approx(pc.t_statistic, 2.028_042, 1e-3, "t");
    approx(pc.p_value, 0.135_599_8, 1e-3, "p");
}

#[test]
fn multi_control_partial_is_defined_and_bounded() {
    let x = [1.0f32, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0];
    let y = [2.0f32, 1.0, 4.0, 3.0, 6.0, 5.0, 8.0, 7.0, 10.0, 9.0];
    let z1 = [5.0f32, 3.0, 6.0, 2.0, 7.0, 4.0, 8.0, 1.0, 9.0, 10.0];
    let z2 = [1.0f32, 4.0, 2.0, 8.0, 3.0, 7.0, 5.0, 6.0, 10.0, 9.0];
    let pc = partial_correlation_controlling(&x, &y, &[&z1, &z2]).unwrap();
    assert_eq!(pc.n_controls, 2);
    assert!(pc.partial_r.abs() <= 1.0, "partial in [-1,1]: {pc:?}");
    assert!(
        pc.ci_low <= pc.partial_r && pc.partial_r <= pc.ci_high,
        "{pc:?}"
    );
}

#[test]
fn fails_closed_on_length_mismatch() {
    assert_eq!(
        pearson(&[1.0, 2.0, 3.0], &[1.0, 2.0]).unwrap_err().code,
        "CALYX_ASSAY_INSUFFICIENT_SAMPLES"
    );
    assert_eq!(
        partial_correlation(&[1.0, 2.0, 3.0, 4.0], &[1.0, 2.0, 3.0, 4.0], &[1.0, 2.0])
            .unwrap_err()
            .code,
        "CALYX_ASSAY_INSUFFICIENT_SAMPLES"
    );
}

#[test]
fn fails_closed_below_min_samples() {
    assert_eq!(
        pearson(&[1.0, 2.0], &[1.0, 2.0]).unwrap_err().code,
        "CALYX_ASSAY_INSUFFICIENT_SAMPLES"
    );
    assert_eq!(
        partial_correlation(&[1.0, 2.0, 3.0], &[1.0, 2.0, 3.0], &[3.0, 2.0, 1.0])
            .unwrap_err()
            .code,
        "CALYX_ASSAY_INSUFFICIENT_SAMPLES" // n=3 < 4
    );
}

#[test]
fn fails_closed_on_non_finite() {
    assert_eq!(
        pearson(&[1.0, f32::NAN, 3.0], &[1.0, 2.0, 3.0])
            .unwrap_err()
            .code,
        "CALYX_ASSAY_INSUFFICIENT_SAMPLES"
    );
}

#[test]
fn fails_closed_on_constant_column() {
    assert_eq!(
        pearson(&[5.0, 5.0, 5.0, 5.0], &[1.0, 2.0, 3.0, 4.0])
            .unwrap_err()
            .code,
        "CALYX_ASSAY_DEGENERATE_INPUT"
    );
    assert_eq!(
        partial_correlation(
            &[1.0, 2.0, 3.0, 4.0],
            &[2.0, 4.0, 6.0, 8.0],
            &[1.0, 2.0, 3.0, 4.0]
        )
        .unwrap_err()
        .code,
        "CALYX_ASSAY_DEGENERATE_INPUT" // Z perfectly explains X and Y
    );
}

#[test]
fn fails_closed_on_collinear_controls() {
    // z2 = 2·z1 → correlation matrix singular.
    let x = [1.0f32, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0];
    let y = [2.0f32, 1.0, 4.0, 3.0, 6.0, 5.0, 7.0];
    let z1 = [1.0f32, 3.0, 2.0, 5.0, 4.0, 7.0, 6.0];
    let z2 = [2.0f32, 6.0, 4.0, 10.0, 8.0, 14.0, 12.0];
    assert_eq!(
        partial_correlation_controlling(&x, &y, &[&z1, &z2])
            .unwrap_err()
            .code,
        "CALYX_ASSAY_DEGENERATE_INPUT"
    );
}
