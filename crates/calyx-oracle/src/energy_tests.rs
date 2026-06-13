use super::*;
use proptest::prelude::*;

const TOL: f32 = 1.0e-4;

#[test]
fn known_energy_matches_two_attractor_formula() {
    let a = [1.0, 0.0];
    let b = [0.0, 1.0];
    let actual = energy(&a, &[&a, &b], 2.0).expect("energy");
    let expected = -((2.0_f32).exp() + 1.0).ln();
    println!("PH51_ENERGY_KNOWN actual={actual:.8} expected={expected:.8}");
    assert!((actual - expected).abs() <= TOL);
}

#[test]
fn descent_step_reaches_symmetric_midpoint() {
    let a = [1.0, 0.0];
    let b = [0.0, 1.0];
    let inv_sqrt2 = 1.0 / 2.0_f32.sqrt();
    let mut free = [inv_sqrt2, inv_sqrt2];
    for _ in 0..5 {
        descent_step(&mut free, &[&a, &b], 2.0).expect("descent step");
    }
    println!("PH51_DESCENT_MIDPOINT free={free:?}");
    assert!((free[0] - inv_sqrt2).abs() <= 1.0e-3);
    assert!((free[1] - inv_sqrt2).abs() <= 1.0e-3);
}

#[test]
fn descend_converges_within_default_steps() {
    let near = [1.0, 0.0];
    let far = [0.0, 1.0];
    let mut free = [0.98, 0.2];
    let result = descend(
        &mut free,
        &[&near, &far],
        DEFAULT_BETA,
        MAX_STEPS,
        DEFAULT_EPS,
    )
    .expect("descend");
    println!(
        "PH51_DESCEND_RESULT steps={} converged={} final_energy={:.8} free={:?}",
        result.steps_taken, result.converged, result.final_energy, free
    );
    assert!(result.converged);
    assert!(result.steps_taken <= MAX_STEPS);
    assert!(free[0] > free[1]);
}

#[test]
fn single_member_converges_in_one_step_to_that_member() {
    let member = [0.6, 0.8];
    let mut free = [1.0, 0.0];
    let result = descend(&mut free, &[&member], 3.0, MAX_STEPS, DEFAULT_EPS).expect("descend");
    println!(
        "PH51_SINGLE_MEMBER steps={} final_energy={:.8} free={:?}",
        result.steps_taken, result.final_energy, free
    );
    assert_eq!(result.steps_taken, 1);
    assert!(result.converged);
    assert!((free[0] - member[0]).abs() <= TOL);
    assert!((free[1] - member[1]).abs() <= TOL);
}

#[test]
fn beta_zero_uses_uniform_centroid() {
    let a = [1.0, 0.0];
    let b = [0.0, 1.0];
    let mut free = [1.0, 0.0];
    descent_step(&mut free, &[&a, &b], 0.0).expect("uniform descent");
    let inv_sqrt2 = 1.0 / 2.0_f32.sqrt();
    println!("PH51_BETA_ZERO free={free:?}");
    assert!((free[0] - inv_sqrt2).abs() <= TOL);
    assert!((free[1] - inv_sqrt2).abs() <= TOL);
}

#[test]
fn empty_region_fails_closed_with_code() {
    let mut free = [1.0, 0.0];
    let err = descent_step(&mut free, &[], DEFAULT_BETA).expect_err("empty region");
    println!("PH51_EMPTY_REGION_ERROR {}", err.code());
    assert_eq!(err.code(), CALYX_ORACLE_ENERGY_EMPTY_REGION);
}

#[test]
fn invalid_shape_and_nonfinite_inputs_fail_closed() {
    let bad_shape = energy(&[1.0, 0.0], &[&[1.0][..]], DEFAULT_BETA).expect_err("shape mismatch");
    assert_eq!(bad_shape.code(), CALYX_ORACLE_ENERGY_INVALID_INPUT);

    let bad_beta = energy(&[1.0, 0.0], &[&[1.0, 0.0][..]], f32::NAN).expect_err("bad beta");
    assert_eq!(bad_beta.code(), CALYX_ORACLE_ENERGY_INVALID_INPUT);
}

#[test]
fn beta_lookup_uses_tuned_value_or_default() {
    struct Fixture;
    impl AnnealConfig for Fixture {
        fn energy_beta(&self, domain: &DomainId) -> Option<f32> {
            match domain.as_str() {
                "tuned" => Some(2.5),
                "bad" => Some(f32::INFINITY),
                _ => None,
            }
        }
    }
    assert_eq!(get_beta(DomainId::from("tuned"), &Fixture), 2.5);
    assert_eq!(get_beta(DomainId::from("missing"), &Fixture), DEFAULT_BETA);
    assert_eq!(get_beta(DomainId::from("bad"), &Fixture), DEFAULT_BETA);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn finite_energy_and_softmax_sums_to_one(beta in 0.01f32..8.0) {
        let x = [0.6, 0.8];
        let a = [1.0, 0.0];
        let b = [0.0, 1.0];
        let value = energy(&x, &[&a, &b], beta).expect("energy");
        prop_assert!(value.is_finite());
        let weights = energy_softmax_weights(&x, &[&a, &b], beta).expect("weights");
        let sum: f32 = weights.iter().sum();
        prop_assert!((sum - 1.0).abs() <= 1.0e-5, "sum={sum}");
        prop_assert!(weights.iter().all(|weight| weight.is_finite()));
    }
}
