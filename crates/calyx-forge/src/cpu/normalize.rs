use wide::f32x16;

use crate::cpu::guard::{check_finite, check_norm_positive, check_shape_2d};
use crate::{ForgeError, Result};

pub fn normalize_f32(vecs: &mut [f32], dim: usize) -> Result<()> {
    if dim == 0 {
        if vecs.is_empty() {
            return Ok(());
        }
        return Err(ForgeError::ShapeMismatch {
            expected: vec![0],
            got: vec![vecs.len()],
            remediation: "dim=0 is valid only for an empty matrix".to_string(),
        });
    }
    if !vecs.len().is_multiple_of(dim) {
        return Err(ForgeError::ShapeMismatch {
            expected: vec![dim],
            got: vec![vecs.len()],
            remediation: "normalize input length must be an integer number of rows".to_string(),
        });
    }
    let rows = vecs.len() / dim;
    check_shape_2d(vecs, rows, dim, "normalize input")?;
    check_finite(vecs, "normalize")?;

    for row in 0..rows {
        let start = row * dim;
        let end = start + dim;
        let norm_sq = sum_squares(&vecs[start..end]);
        let norm = norm_sq.sqrt();
        check_norm_positive(norm, "normalize", row)?;
        scale_row(&mut vecs[start..end], 1.0 / norm);
    }
    Ok(())
}

fn sum_squares(values: &[f32]) -> f32 {
    let mut sum = 0.0;
    let mut offset = 0;
    while offset + 16 <= values.len() {
        let chunk = load16(values, offset);
        // DETERMINISM: row elements are consumed in ascending offset order; each
        // f32x16 chunk contributes one reduce_add() subtotal before scalar tail.
        sum += (chunk * chunk).reduce_add();
        offset += 16;
    }
    while offset < values.len() {
        sum += values[offset] * values[offset];
        offset += 1;
    }
    sum
}

fn scale_row(values: &mut [f32], scale: f32) {
    let scale_vec = f32x16::splat(scale);
    let mut offset = 0;
    while offset + 16 <= values.len() {
        let scaled = load16(values, offset) * scale_vec;
        values[offset..offset + 16].copy_from_slice(&scaled.to_array());
        offset += 16;
    }
    while offset < values.len() {
        values[offset] *= scale;
        offset += 1;
    }
}

fn load16(values: &[f32], offset: usize) -> f32x16 {
    let mut lanes = [0.0; 16];
    lanes.copy_from_slice(&values[offset..offset + 16]);
    f32x16::from(lanes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Backend, CpuBackend};
    use proptest::prelude::*;

    #[test]
    fn normalize_345_exact() -> Result<()> {
        let mut values = vec![3.0, 4.0];
        normalize_f32(&mut values, 2)?;
        println!("NORMALIZE_345 {:?}", values);
        assert_eq!(values, vec![0.6, 0.8]);
        Ok(())
    }

    #[test]
    fn normalize_edges_dim1_and_empty() -> Result<()> {
        let cpu = CpuBackend::new();
        let mut dim_one = vec![5.0, -7.0];
        cpu.normalize(&mut dim_one, 1)?;
        assert_eq!(dim_one, vec![1.0, -1.0]);

        let mut empty = Vec::new();
        normalize_f32(&mut empty, 0)?;
        assert!(empty.is_empty());
        Ok(())
    }

    #[test]
    fn normalize_fail_closed_zero_and_non_finite() {
        let mut zero = vec![0.0, 0.0];
        let zero_err = normalize_f32(&mut zero, 2).expect_err("zero vector must fail closed");
        println!("NORMALIZE_FAIL_ZERO {zero_err}");
        assert!(matches!(zero_err, ForgeError::NumericalInvariant { .. }));

        let mut non_finite = vec![1.0, f32::INFINITY];
        let err = normalize_f32(&mut non_finite, 2).expect_err("non-finite input must fail closed");
        println!("NORMALIZE_FAIL_NONFINITE {err}");
        assert!(matches!(err, ForgeError::NumericalInvariant { .. }));
        assert_eq!(non_finite, vec![1.0, f32::INFINITY]);
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(32))]

        #[test]
        fn normalize_sets_non_zero_vector_norm_to_one(
            mut values in proptest::collection::vec(-1.0f32..1.0, 1..=128)
        ) {
            prop_assume!(values.iter().any(|value| *value != 0.0));
            let dim = values.len();
            let result = normalize_f32(&mut values, dim);
            prop_assert!(result.is_ok(), "normalize failed: {:?}", result.err());
            let norm = values.iter().fold(0.0, |sum, value| sum + value * value).sqrt();
            prop_assert!((norm - 1.0).abs() <= 1e-6, "norm={norm}");
        }
    }
}
