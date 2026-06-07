use wide::f32x16;

use crate::Result;
use crate::cpu::guard::{check_finite, check_norm_positive, check_shape_2d};

pub fn cosine_batch(query: &[f32], candidates: &[f32], dim: usize, out: &mut [f32]) -> Result<()> {
    validate_batch("cosine_batch", query, candidates, dim, out)?;
    if out.is_empty() {
        return Ok(());
    }

    let query_norm = sum_squares(query).sqrt();
    check_norm_positive(query_norm, "cosine_batch", 0)?;

    for (row, score) in out.iter_mut().enumerate() {
        let candidate = candidate_row(candidates, dim, row);
        let (dot, candidate_norm_sq) = dot_and_norm(query, candidate);
        let candidate_norm = candidate_norm_sq.sqrt();
        check_norm_positive(candidate_norm, "cosine_batch", row)?;
        *score = dot / (query_norm * candidate_norm);
    }
    Ok(())
}

pub fn dot_batch(query: &[f32], candidates: &[f32], dim: usize, out: &mut [f32]) -> Result<()> {
    validate_batch("dot_batch", query, candidates, dim, out)?;
    for (row, score) in out.iter_mut().enumerate() {
        *score = dot(query, candidate_row(candidates, dim, row));
    }
    Ok(())
}

pub fn l2_batch(query: &[f32], candidates: &[f32], dim: usize, out: &mut [f32]) -> Result<()> {
    validate_batch("l2_batch", query, candidates, dim, out)?;
    for (row, score) in out.iter_mut().enumerate() {
        *score = l2_squared(query, candidate_row(candidates, dim, row));
    }
    Ok(())
}

fn validate_batch(
    op: &'static str,
    query: &[f32],
    candidates: &[f32],
    dim: usize,
    out: &[f32],
) -> Result<()> {
    check_shape_2d(query, 1, dim, "distance query")?;
    check_shape_2d(candidates, out.len(), dim, "distance candidates")?;
    check_finite(query, op)?;
    check_finite(candidates, op)?;
    Ok(())
}

fn candidate_row(candidates: &[f32], dim: usize, row: usize) -> &[f32] {
    let start = row * dim;
    &candidates[start..start + dim]
}

fn sum_squares(values: &[f32]) -> f32 {
    let mut sum = 0.0;
    let mut offset = 0;
    while offset + 16 <= values.len() {
        let chunk = load16(values, offset);
        // DETERMINISM: f32x16 chunks are reduced in ascending input-offset
        // order; each chunk contributes exactly one reduce_add() subtotal.
        sum += (chunk * chunk).reduce_add();
        offset += 16;
    }
    while offset < values.len() {
        sum += values[offset] * values[offset];
        offset += 1;
    }
    sum
}

fn dot(query: &[f32], candidate: &[f32]) -> f32 {
    let mut sum = 0.0;
    let mut offset = 0;
    while offset + 16 <= query.len() {
        sum += (load16(query, offset) * load16(candidate, offset)).reduce_add();
        offset += 16;
    }
    while offset < query.len() {
        sum += query[offset] * candidate[offset];
        offset += 1;
    }
    sum
}

fn dot_and_norm(query: &[f32], candidate: &[f32]) -> (f32, f32) {
    let mut dot_sum = 0.0;
    let mut norm_sum = 0.0;
    let mut offset = 0;
    while offset + 16 <= query.len() {
        let q = load16(query, offset);
        let c = load16(candidate, offset);
        dot_sum += (q * c).reduce_add();
        norm_sum += (c * c).reduce_add();
        offset += 16;
    }
    while offset < query.len() {
        dot_sum += query[offset] * candidate[offset];
        norm_sum += candidate[offset] * candidate[offset];
        offset += 1;
    }
    (dot_sum, norm_sum)
}

fn l2_squared(query: &[f32], candidate: &[f32]) -> f32 {
    let mut sum = 0.0;
    let mut offset = 0;
    while offset + 16 <= query.len() {
        let diff = load16(query, offset) - load16(candidate, offset);
        sum += (diff * diff).reduce_add();
        offset += 16;
    }
    while offset < query.len() {
        let diff = query[offset] - candidate[offset];
        sum += diff * diff;
        offset += 1;
    }
    sum
}

fn load16(values: &[f32], offset: usize) -> f32x16 {
    let mut lanes = [0.0; 16];
    lanes.copy_from_slice(&values[offset..offset + 16]);
    f32x16::from(lanes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Backend, CpuBackend, ForgeError};
    use proptest::prelude::*;

    fn finite_f32() -> impl Strategy<Value = f32> {
        -0.1f32..0.1
    }

    fn scalar_dot(query: &[f32], candidate: &[f32]) -> f32 {
        query
            .iter()
            .zip(candidate.iter())
            .fold(0.0, |sum, (q, c)| sum + q * c)
    }

    #[test]
    fn cosine_orthogonal_exact() -> Result<()> {
        let mut out = vec![99.0];
        cosine_batch(&[1.0, 0.0], &[0.0, 1.0], 2, &mut out)?;
        println!("COSINE_ORTHOGONAL {:.8}", out[0]);
        assert_eq!(out, vec![0.0]);
        Ok(())
    }

    #[test]
    fn cosine_parallel_exact() -> Result<()> {
        let mut out = vec![0.0, 0.0];
        cosine_batch(&[1.0, 0.0], &[1.0, 0.0, -1.0, 0.0], 2, &mut out)?;
        println!("COSINE_PARALLEL {:.8}", out[0]);
        println!("COSINE_ANTIPARALLEL {:.8}", out[1]);
        assert_eq!(out, vec![1.0, -1.0]);
        Ok(())
    }

    #[test]
    fn l2_pythagorean_exact() -> Result<()> {
        let mut out = vec![0.0];
        l2_batch(&[0.0, 0.0], &[3.0, 4.0], 2, &mut out)?;
        println!("L2_PYTHAGOREAN {:.1}", out[0]);
        assert_eq!(out, vec![25.0]);
        Ok(())
    }

    #[test]
    fn backend_delegates_distance_batches() -> Result<()> {
        let cpu = CpuBackend::new();
        let mut out = vec![0.0];
        cpu.dot(&[2.0, 3.0], &[5.0, 7.0], 2, &mut out)?;
        assert_eq!(out, vec![31.0]);
        cpu.l2(&[2.0, 3.0], &[5.0, 7.0], 2, &mut out)?;
        assert_eq!(out, vec![25.0]);
        cpu.cosine(&[1.0, 0.0], &[1.0, 0.0], 2, &mut out)?;
        assert_eq!(out, vec![1.0]);
        Ok(())
    }

    #[test]
    fn distance_edges_single_dim_and_model_dim() -> Result<()> {
        let mut single = vec![0.0];
        dot_batch(&[3.0], &[4.0], 1, &mut single)?;
        assert_eq!(single, vec![12.0]);

        let dim = 1536;
        let query = vec![1.0; dim];
        let candidate = vec![2.0; dim];
        let mut out = vec![0.0];
        dot_batch(&query, &candidate, dim, &mut out)?;
        assert_eq!(out, vec![3072.0]);
        l2_batch(&query, &candidate, dim, &mut out)?;
        assert_eq!(out, vec![1536.0]);
        cosine_batch(&query, &candidate, dim, &mut out)?;
        println!("DISTANCE_DIM1536_COSINE {:.8}", out[0]);
        assert!((out[0] - 1.0).abs() <= 1e-6);

        let mut empty = Vec::new();
        dot_batch(&[], &[], 0, &mut empty)?;
        assert!(empty.is_empty());
        Ok(())
    }

    #[test]
    fn distance_fail_closed_infinity_zero_norm_and_shape() {
        let mut out = vec![0.0];
        let inf_err = dot_batch(&[f32::INFINITY], &[1.0], 1, &mut out)
            .expect_err("infinity query must fail closed");
        println!("DIST_FAIL_INF {inf_err}");
        assert!(matches!(inf_err, ForgeError::NumericalInvariant { .. }));

        let zero_err = cosine_batch(&[0.0, 0.0], &[1.0, 0.0], 2, &mut out)
            .expect_err("zero norm query must fail closed");
        println!("DIST_FAIL_ZERO {zero_err}");
        assert!(matches!(zero_err, ForgeError::NumericalInvariant { .. }));

        let shape_err = l2_batch(&[1.0, 2.0], &[1.0], 2, &mut out)
            .expect_err("candidate shape mismatch must fail closed");
        assert!(matches!(shape_err, ForgeError::ShapeMismatch { .. }));
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(32))]

        #[test]
        fn cosine_self_is_one_for_non_zero_vector(
            values in proptest::collection::vec(-1.0f32..1.0, 1..=128)
        ) {
            prop_assume!(values.iter().any(|value| *value != 0.0));
            let mut out = vec![0.0];
            let result = cosine_batch(&values, &values, values.len(), &mut out);
            prop_assert!(result.is_ok(), "cosine failed: {:?}", result.err());
            prop_assert!((out[0] - 1.0).abs() <= 1e-6, "cosine self={}", out[0]);
        }

        #[test]
        fn dot_batch_matches_scalar_reference_dim128(
            query in proptest::collection::vec(finite_f32(), 128),
            candidates in proptest::collection::vec(finite_f32(), 128 * 100)
        ) {
            let mut out = vec![0.0; 100];
            let result = dot_batch(&query, &candidates, 128, &mut out);
            prop_assert!(result.is_ok(), "dot failed: {:?}", result.err());
            for (row, actual) in out.iter().enumerate().take(100) {
                let start = row * 128;
                let expected = scalar_dot(&query, &candidates[start..start + 128]);
                prop_assert!((*actual - expected).abs() <= 1e-5);
            }
        }
    }
}
