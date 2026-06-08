use calyx_core::{CalyxError, Result};

pub(crate) fn validate_rectangular_finite(name: &str, samples: &[Vec<f32>]) -> Result<usize> {
    let dim = samples.first().map_or(0, Vec::len);
    if dim == 0 {
        return Err(CalyxError::assay_insufficient_samples(format!(
            "{name} sample matrix must have at least one dimension"
        )));
    }
    for (row_idx, row) in samples.iter().enumerate() {
        if row.len() != dim {
            return Err(CalyxError::assay_insufficient_samples(format!(
                "{name} sample row {row_idx} has dim {}, expected {dim}",
                row.len()
            )));
        }
        if row.iter().any(|value| !value.is_finite()) {
            return Err(CalyxError::assay_insufficient_samples(format!(
                "{name} sample row {row_idx} contains NaN or infinity"
            )));
        }
    }
    Ok(dim)
}
