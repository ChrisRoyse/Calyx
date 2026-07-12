//! Effective-rank reporting for panel redundancy.

use calyx_core::{CalyxError, Result};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NeffReport {
    pub n_eff: f32,
    pub trace: f32,
    pub frobenius_sq: f32,
}

/// Computes the stable rank of a square, finite redundancy matrix.
///
/// The empty matrix retains rank zero. Non-empty zero-norm, ragged, non-square,
/// non-finite, or numerically unrepresentable matrices fail closed.
pub fn stable_rank(matrix: &[Vec<f32>]) -> Result<NeffReport> {
    let dimension = matrix.len();
    if dimension == 0 {
        return Ok(NeffReport {
            n_eff: 0.0,
            trace: 0.0,
            frobenius_sq: 0.0,
        });
    }
    for (row_index, row) in matrix.iter().enumerate() {
        if row.len() != dimension {
            return Err(CalyxError::assay_degenerate_input(format!(
                "stable-rank matrix must be square: rows={dimension} row={row_index} columns={}",
                row.len()
            )));
        }
        for (column_index, value) in row.iter().enumerate() {
            if !value.is_finite() {
                return Err(CalyxError::assay_degenerate_input(format!(
                    "stable-rank matrix contains a non-finite value at row={row_index} column={column_index}"
                )));
            }
        }
    }

    // Keep the historical f32 accumulation order so valid serialized reports
    // remain byte-stable; use f64 only for an unrepresentable trace-square.
    let trace = matrix
        .iter()
        .enumerate()
        .map(|(index, row)| row[index])
        .sum::<f32>();
    let frobenius_sq = matrix
        .iter()
        .flatten()
        .map(|value| value * value)
        .sum::<f32>();
    if !trace.is_finite() {
        return Err(unrepresentable("trace", f64::from(trace)));
    }
    if !frobenius_sq.is_finite() {
        return Err(unrepresentable("frobenius_sq", f64::from(frobenius_sq)));
    }
    if frobenius_sq == 0.0 {
        if matrix.iter().flatten().any(|value| *value != 0.0) {
            return Err(unrepresentable("frobenius_sq", 0.0));
        }
        return Err(CalyxError::assay_degenerate_input(
            "stable-rank matrix has zero Frobenius norm",
        ));
    }
    let direct_n_eff = trace * trace / frobenius_sq;
    let n_eff = if direct_n_eff.is_finite() && !(trace != 0.0 && direct_n_eff == 0.0) {
        direct_n_eff
    } else {
        report_value(
            f64::from(trace) * f64::from(trace) / f64::from(frobenius_sq),
            "n_eff",
        )?
    };
    if !n_eff.is_finite() {
        return Err(unrepresentable("n_eff", f64::from(n_eff)));
    }
    Ok(NeffReport {
        n_eff,
        trace,
        frobenius_sq,
    })
}

fn report_value(value: f64, name: &'static str) -> Result<f32> {
    let converted = value as f32;
    if !value.is_finite() || !converted.is_finite() || (value != 0.0 && converted == 0.0) {
        return Err(unrepresentable(name, value));
    }
    Ok(converted)
}

fn unrepresentable(name: &'static str, value: f64) -> CalyxError {
    CalyxError::assay_degenerate_input(format!(
        "stable-rank {name} is not representable as a finite f32: {value}"
    ))
}

#[cfg(test)]
mod tests {
    use super::{NeffReport, stable_rank};

    #[test]
    fn identity_and_empty_matrices_have_known_rank() {
        assert_eq!(
            stable_rank(&[]).unwrap(),
            NeffReport {
                n_eff: 0.0,
                trace: 0.0,
                frobenius_sq: 0.0,
            }
        );
        assert_eq!(
            stable_rank(&[vec![1.0, 0.0], vec![0.0, 1.0]]).unwrap(),
            NeffReport {
                n_eff: 2.0,
                trace: 2.0,
                frobenius_sq: 2.0,
            }
        );
    }

    #[test]
    fn ragged_matrix_fails_closed() {
        let error = stable_rank(&[vec![1.0, 0.0], vec![0.0]]).unwrap_err();
        assert_eq!(error.code, "CALYX_ASSAY_DEGENERATE_INPUT");
        assert!(error.message.contains("must be square"));
    }

    #[test]
    fn non_finite_matrix_fails_closed() {
        for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let error = stable_rank(&[vec![1.0, value], vec![0.0, 1.0]]).unwrap_err();
            assert_eq!(error.code, "CALYX_ASSAY_DEGENERATE_INPUT");
            assert!(error.message.contains("non-finite"));
        }
    }

    #[test]
    fn non_empty_zero_matrix_fails_closed() {
        let error = stable_rank(&[vec![0.0, 0.0], vec![0.0, 0.0]]).unwrap_err();
        assert_eq!(error.code, "CALYX_ASSAY_DEGENERATE_INPUT");
        assert!(error.message.contains("zero Frobenius norm"));
    }

    #[test]
    fn unrepresentable_report_values_fail_closed() {
        for value in [f32::MAX, f32::from_bits(1)] {
            let error = stable_rank(&[vec![value]]).unwrap_err();
            assert_eq!(error.code, "CALYX_ASSAY_DEGENERATE_INPUT");
            assert!(error.message.contains("frobenius_sq"));
            assert!(error.message.contains("not representable"));
        }
    }

    #[test]
    fn trace_square_fallback_preserves_a_representable_rank() {
        let value = 1.0e19_f32;
        let report = stable_rank(&[vec![value, 0.0], vec![0.0, value]]).unwrap();
        assert_eq!(report.n_eff, 2.0);
        assert!(report.trace.is_finite());
        assert!(report.frobenius_sq.is_finite());
    }
}
