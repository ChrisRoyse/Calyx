use crate::{ForgeError, Result};

const NUMERICAL_REMEDIATION: &str =
    "Ensure all input vectors are normalized finite f32; check upstream embedding model output";

pub fn check_finite(slice: &[f32], op: &str) -> Result<()> {
    for (index, value) in slice.iter().enumerate() {
        if !value.is_finite() {
            return Err(ForgeError::NumericalInvariant {
                op: op.to_string(),
                detail: format!("non-finite f32 at index {index}: {value}"),
                remediation: NUMERICAL_REMEDIATION.to_string(),
            });
        }
    }
    Ok(())
}

pub fn check_norm_positive(norm: f32, op: &str, row: usize) -> Result<()> {
    if norm > 0.0 && norm.is_finite() {
        return Ok(());
    }
    Err(ForgeError::NumericalInvariant {
        op: op.to_string(),
        detail: format!("zero or non-finite norm at row {row}"),
        remediation: NUMERICAL_REMEDIATION.to_string(),
    })
}

pub fn check_shape_2d(slice: &[f32], rows: usize, cols: usize, name: &str) -> Result<()> {
    let expected_len = rows
        .checked_mul(cols)
        .ok_or_else(|| ForgeError::ShapeMismatch {
            expected: vec![rows, cols],
            got: vec![slice.len()],
            remediation: format!("{name} shape overflows usize"),
        })?;
    if slice.len() == expected_len {
        return Ok(());
    }
    Err(ForgeError::ShapeMismatch {
        expected: vec![rows, cols],
        got: vec![slice.len()],
        remediation: format!("{name} length does not match rows*cols"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cpu::{dot_batch, gemm_f32, normalize_f32, topk_f32};
    use proptest::prelude::*;

    #[test]
    fn numerical_invariant_check_finite_nan_starts_with_code() {
        let err = check_finite(&[1.0, f32::NAN, 2.0], "test").expect_err("NaN must fail");
        let rendered = err.to_string();
        println!("{rendered}");
        assert!(rendered.starts_with("CALYX_FORGE_NUMERICAL_INVARIANT"));
        assert!(rendered.contains("Remediation:"));
    }

    #[test]
    fn numerical_invariant_check_finite_infinity_reports_index() {
        let err = check_finite(&[1.0, f32::INFINITY, 2.0], "test").expect_err("Inf must fail");
        let rendered = err.to_string();
        println!("{rendered}");
        assert!(rendered.contains("index 1"));
    }

    #[test]
    fn numerical_invariant_norm_reports_row() {
        let err = check_norm_positive(0.0, "normalize", 7).expect_err("zero norm must fail");
        let rendered = err.to_string();
        println!("{rendered}");
        assert!(rendered.contains("row 7"));
        assert!(rendered.contains("Remediation:"));
    }

    #[test]
    fn guard_edges_accept_finite_empty_and_reject_negative_infinity() -> Result<()> {
        check_finite(&[1.0, 2.0, 3.0], "finite")?;
        check_finite(&[], "empty")?;
        let err = check_finite(&[f32::NEG_INFINITY], "neg_inf").expect_err("-inf must fail");
        assert!(
            err.to_string()
                .starts_with("CALYX_FORGE_NUMERICAL_INVARIANT")
        );
        Ok(())
    }

    #[test]
    fn guard_shape_mismatch_reports_expected_and_got() {
        let err =
            check_shape_2d(&[1.0, 2.0, 3.0], 2, 2, "matrix").expect_err("shape mismatch must fail");
        let rendered = err.to_string();
        println!("{rendered}");
        assert!(matches!(err, ForgeError::ShapeMismatch { .. }));
        assert!(rendered.contains("expected"));
        assert!(rendered.contains("got"));
    }

    proptest! {
        #[test]
        fn numerical_invariant_from_cpu_kernel_has_prefix_and_remediation(which in 0usize..4) {
            let mut out = vec![0.0; 1];
            let err = match which {
                0 => gemm_f32(&[f32::NAN], &[1.0], 1, 1, 1, &mut out),
                1 => dot_batch(&[f32::NAN], &[1.0], 1, &mut out),
                2 => {
                    let mut values = vec![f32::NAN];
                    normalize_f32(&mut values, 1)
                }
                _ => topk_f32(&[f32::NAN], 1).map(|_| ()),
            }
            .expect_err("NaN input must fail closed");
            let rendered = err.to_string();
            prop_assert!(rendered.starts_with("CALYX_FORGE_"));
            prop_assert!(rendered.contains("Remediation:"));
        }
    }
}
