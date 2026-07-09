use wide::{f32x8, f32x16};

use crate::Result;
use crate::cpu::guard::{check_finite, check_shape_2d};

pub const TILE_M: usize = 64;
pub const TILE_K: usize = 64;

pub fn gemm_f32(a: &[f32], b: &[f32], m: usize, k: usize, n: usize, out: &mut [f32]) -> Result<()> {
    validate_gemm_inputs(a, b, m, k, n, out)?;
    out.fill(0.0);
    if m == 0 || n == 0 {
        return Ok(());
    }

    #[cfg(target_arch = "x86_64")]
    {
        if std::arch::is_x86_feature_detected!("avx512f") {
            return gemm_tiled_f32x16(a, b, m, k, n, out);
        }
    }

    gemm_tiled_f32x8(a, b, m, k, n, out)
}

fn validate_gemm_inputs(
    a: &[f32],
    b: &[f32],
    m: usize,
    k: usize,
    n: usize,
    out: &[f32],
) -> Result<()> {
    check_shape_2d(a, m, k, "gemm A")?;
    check_shape_2d(b, k, n, "gemm B")?;
    check_shape_2d(out, m, n, "gemm output")?;
    check_finite(a, "cpu.gemm")?;
    check_finite(b, "cpu.gemm")?;
    Ok(())
}

fn gemm_tiled_f32x16(
    a: &[f32],
    b: &[f32],
    m: usize,
    k: usize,
    n: usize,
    out: &mut [f32],
) -> Result<()> {
    let mut packed_a = vec![0.0; k];
    for row_tile in (0..m).step_by(TILE_M) {
        let row_end = (row_tile + TILE_M).min(m);
        for row in row_tile..row_end {
            pack_a_row(a, row, m, &mut packed_a);
            for col in 0..n {
                out[col_major(row, col, m)] = dot_packed_f32x16(&packed_a, b, col, k);
            }
        }
    }
    Ok(())
}

fn gemm_tiled_f32x8(
    a: &[f32],
    b: &[f32],
    m: usize,
    k: usize,
    n: usize,
    out: &mut [f32],
) -> Result<()> {
    let mut packed_a = vec![0.0; k];
    for row_tile in (0..m).step_by(TILE_M) {
        let row_end = (row_tile + TILE_M).min(m);
        for row in row_tile..row_end {
            pack_a_row(a, row, m, &mut packed_a);
            for col in 0..n {
                out[col_major(row, col, m)] = dot_packed_f32x8(&packed_a, b, col, k);
            }
        }
    }
    Ok(())
}

fn pack_a_row(a: &[f32], row: usize, m: usize, packed: &mut [f32]) {
    for (depth, slot) in packed.iter_mut().enumerate() {
        *slot = a[col_major(row, depth, m)];
    }
}

fn dot_packed_f32x16(a_row: &[f32], b: &[f32], col: usize, k: usize) -> f32 {
    let b_col = b_col_slice(b, col, k);
    let mut sum = 0.0;
    let mut depth_tile = 0;
    while depth_tile < k {
        let depth_end = (depth_tile + TILE_K).min(k);
        let mut depth = depth_tile;
        while depth + 16 <= depth_end {
            let a_lane: &[f32; 16] = a_row[depth..depth + 16]
                .try_into()
                .expect("16-lane packed A slice");
            let b_lane: &[f32; 16] = b_col[depth..depth + 16]
                .try_into()
                .expect("16-lane B column slice");
            // DETERMINISM: keep the AVX512 lane multiply, then reduce as two
            // explicit f32x8-compatible subtotals. A full f32x16 tree reduction
            // drifts from cuBLAS in near-zero cancellation cells.
            let products = (f32x16::from(*a_lane) * f32x16::from(*b_lane)).to_array();
            for lane_chunk in products.chunks_exact(8) {
                let mut subtotal = 0.0;
                for product in lane_chunk {
                    subtotal += *product;
                }
                sum += subtotal;
            }
            depth += 16;
        }
        while depth < depth_end {
            sum += a_row[depth] * b_col[depth];
            depth += 1;
        }
        depth_tile += TILE_K;
    }
    sum
}

fn dot_packed_f32x8(a_row: &[f32], b: &[f32], col: usize, k: usize) -> f32 {
    let b_col = b_col_slice(b, col, k);
    let mut sum = 0.0;
    let mut depth_tile = 0;
    while depth_tile < k {
        let depth_end = (depth_tile + TILE_K).min(k);
        let mut depth = depth_tile;
        while depth + 8 <= depth_end {
            let a_lane: &[f32; 8] = a_row[depth..depth + 8]
                .try_into()
                .expect("8-lane packed A slice");
            let b_lane: &[f32; 8] = b_col[depth..depth + 8]
                .try_into()
                .expect("8-lane B column slice");
            sum += (f32x8::from(*a_lane) * f32x8::from(*b_lane)).reduce_add();
            depth += 8;
        }
        while depth < depth_end {
            sum += a_row[depth] * b_col[depth];
            depth += 1;
        }
        depth_tile += TILE_K;
    }
    sum
}

fn b_col_slice(b: &[f32], col: usize, k: usize) -> &[f32] {
    let start = col_major(0, col, k);
    &b[start..start + k]
}

fn col_major(row: usize, col: usize, rows: usize) -> usize {
    col * rows + row
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Backend, CpuBackend, ForgeError};
    use proptest::prelude::*;

    fn finite_f32() -> impl Strategy<Value = f32> {
        -10.0f32..10.0
    }

    fn identity(size: usize) -> Vec<f32> {
        let mut id = vec![0.0; size * size];
        for i in 0..size {
            id[col_major(i, i, size)] = 1.0;
        }
        id
    }

    fn scalar_dot(a: &[f32], b: &[f32], row: usize, col: usize, m: usize, k: usize) -> f32 {
        let mut sum = 0.0;
        for depth in 0..k {
            sum += a[col_major(row, depth, m)] * b[col_major(depth, col, k)];
        }
        sum
    }

    #[test]
    fn gemm_2x2_exact() -> Result<()> {
        let cpu = CpuBackend::new();
        println!(
            "CPU_GEMM_FEATURE avx512f={} path={}",
            cpu.avx512_available(),
            cpu.simd_path()
        );
        let a = vec![1.0, 3.0, 2.0, 4.0];
        let b = vec![5.0, 7.0, 6.0, 8.0];
        let mut out = vec![0.0; 4];

        cpu.gemm(&a, &b, 2, 2, 2, &mut out)?;

        println!("GEMM_2X2 C[0]={:.1}, C[3]={:.1}", out[0], out[3]);
        assert_eq!(out, vec![19.0, 43.0, 22.0, 50.0]);
        Ok(())
    }

    #[test]
    fn gemm_degenerate_1x1_and_dot_shape() -> Result<()> {
        let mut single = vec![0.0; 1];
        gemm_f32(&[3.0], &[7.0], 1, 1, 1, &mut single)?;
        assert_eq!(single, vec![21.0]);

        let a = vec![1.0, 2.0, 3.0];
        let b = vec![4.0, 5.0, 6.0];
        let mut out = vec![0.0; 1];
        gemm_f32(&a, &b, 1, 3, 1, &mut out)?;
        assert_eq!(out, vec![32.0]);
        Ok(())
    }

    #[test]
    fn gemm_edges_empty_outer_product_and_tile() -> Result<()> {
        let mut empty = Vec::new();
        gemm_f32(&[], &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0], 0, 3, 2, &mut empty)?;
        assert!(empty.is_empty());

        let a = vec![2.0, 3.0];
        let b = vec![5.0, 7.0, 11.0];
        let mut outer = vec![0.0; 6];
        gemm_f32(&a, &b, 2, 1, 3, &mut outer)?;
        assert_eq!(outer, vec![10.0, 15.0, 14.0, 21.0, 22.0, 33.0]);

        let a = vec![1.0; TILE_M * TILE_K];
        let b = vec![1.0; TILE_K * TILE_M];
        let mut out = vec![0.0; TILE_M * TILE_M];
        gemm_f32(&a, &b, TILE_M, TILE_K, TILE_M, &mut out)?;
        println!(
            "GEMM_TILE C[0]={:.1}, C[last]={:.1}",
            out[0],
            out[out.len() - 1]
        );
        assert!(out.iter().all(|value| *value == TILE_K as f32));
        Ok(())
    }

    #[test]
    fn gemm_packed_strided_a_matches_scalar_tail() -> Result<()> {
        let (m, k, n) = (3, 19, 5);
        let a: Vec<_> = (0..m * k)
            .map(|idx| (idx % 11) as f32 * 0.25 - 1.0)
            .collect();
        let b: Vec<_> = (0..k * n)
            .map(|idx| (idx % 13) as f32 * 0.125 - 0.5)
            .collect();
        let mut out = vec![0.0; m * n];

        gemm_f32(&a, &b, m, k, n, &mut out)?;

        for col in 0..n {
            for row in 0..m {
                let actual = out[col_major(row, col, m)];
                let expected = scalar_dot(&a, &b, row, col, m, k);
                assert!((actual - expected).abs() <= 1e-5);
            }
        }
        println!("GEMM_PACKED_STRIDED_A m={m} k={k} n={n} tail={}", k % 8);
        Ok(())
    }

    #[test]
    fn gemm_fail_closed_nan_and_shape_mismatch() {
        let mut out = vec![0.0; 1];
        let err = gemm_f32(&[f32::NAN], &[1.0], 1, 1, 1, &mut out)
            .expect_err("NaN input must fail closed");
        assert!(matches!(err, ForgeError::NumericalInvariant { .. }));

        let mut short_out = Vec::new();
        let err = gemm_f32(&[1.0], &[1.0], 1, 1, 1, &mut short_out)
            .expect_err("short output must fail closed");
        assert!(matches!(err, ForgeError::ShapeMismatch { .. }));
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(32))]

        #[test]
        fn gemm_identity_proptest(
            (m, k, a) in (1usize..=64, 1usize..=64)
                .prop_flat_map(|(m, k)| {
                    let len = m * k;
                    (Just(m), Just(k), proptest::collection::vec(finite_f32(), len))
                })
        ) {
            let mut out = vec![0.0; m * k];
            gemm_f32(&a, &identity(k), m, k, k, &mut out)?;
            for (actual, expected) in out.iter().zip(a.iter()) {
                prop_assert!((actual - expected).abs() <= 1e-5);
            }
        }

        #[test]
        fn gemm_scalar_dot_matches_each_cell(
            a in proptest::collection::vec(finite_f32(), 16),
            b in proptest::collection::vec(finite_f32(), 16)
        ) {
            let mut out = vec![0.0; 16];
            gemm_f32(&a, &b, 4, 4, 4, &mut out)?;
            for col in 0..4 {
                for row in 0..4 {
                    let actual = out[col_major(row, col, 4)];
                    let expected = scalar_dot(&a, &b, row, col, 4, 4);
                    prop_assert!((actual - expected).abs() <= 1e-5);
                }
            }
        }
    }
}
