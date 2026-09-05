use calyx_core::{
    CALYX_STRUCTURED_FIELD_MISSING, CalyxError, Input, Lens, LensId, Result, SlotShape, SlotVector,
    SparseEntry,
};

/// Verifies that an input matches a lens' declared modality.
pub fn ensure_input_modality(lens: &dyn Lens, input: &Input) -> Result<()> {
    if lens.accepts_input(input) {
        return Ok(());
    }

    if lens.requires_structured_record() {
        return Err(CalyxError {
            code: CALYX_STRUCTURED_FIELD_MISSING,
            message: format!(
                "structured lens {} received {:?} input without a canonical typed record",
                lens.id(),
                input.modality
            ),
            remediation: "attach the required canonical structured record and retry",
        });
    }

    Err(CalyxError::lens_dim_mismatch(format!(
        "lens {} accepts {:?}, got {:?}",
        lens.id(),
        lens.modality(),
        input.modality
    )))
}

/// Verifies that a slot vector exactly matches the lens' declared shape.
pub fn ensure_vector_shape(lens_id: LensId, shape: SlotShape, vector: &SlotVector) -> Result<()> {
    match (shape, vector) {
        (SlotShape::Dense(expected), SlotVector::Dense { dim, data }) => {
            ensure_dense_shape(lens_id, expected, *dim, data)
        }
        (SlotShape::Sparse(expected), SlotVector::Sparse { dim, entries }) => {
            ensure_sparse_shape(lens_id, expected, *dim, entries)
        }
        (
            SlotShape::Multi {
                token_dim: expected,
            },
            SlotVector::Multi { token_dim, tokens },
        ) => ensure_multi_shape(lens_id, expected, *token_dim, tokens),
        (
            SlotShape::Multi {
                token_dim: expected,
            },
            SlotVector::SegmentedMulti {
                token_dim,
                segments,
                ..
            },
        ) => ensure_segmented_multi_shape(lens_id, expected, *token_dim, segments),
        (_, SlotVector::Absent { reason }) => Err(CalyxError::lens_dim_mismatch(format!(
            "lens {lens_id} returned absent vector {reason:?}"
        ))),
        (expected, actual) => Err(CalyxError::lens_dim_mismatch(format!(
            "lens {lens_id} returned {actual:?}, expected {expected:?}"
        ))),
    }
}

fn ensure_dense_shape(lens_id: LensId, expected: u32, actual: u32, data: &[f32]) -> Result<()> {
    if actual != expected || data.len() != expected as usize {
        return Err(CalyxError::lens_dim_mismatch(format!(
            "lens {lens_id} dense dim {actual}/{} != expected {expected}",
            data.len()
        )));
    }
    ensure_finite(lens_id, data)
}

fn ensure_sparse_shape(
    lens_id: LensId,
    expected: u32,
    actual: u32,
    entries: &[SparseEntry],
) -> Result<()> {
    if actual != expected {
        return Err(CalyxError::lens_dim_mismatch(format!(
            "lens {lens_id} sparse dim {actual} != expected {expected}"
        )));
    }
    for entry in entries {
        if entry.idx >= expected {
            return Err(CalyxError::lens_dim_mismatch(format!(
                "lens {lens_id} sparse index {} outside dim {expected}",
                entry.idx
            )));
        }
        if !entry.val.is_finite() {
            return Err(CalyxError::lens_numerical_invariant(format!(
                "lens {lens_id} sparse entry {} is non-finite",
                entry.idx
            )));
        }
    }
    Ok(())
}

fn ensure_multi_shape(
    lens_id: LensId,
    expected: u32,
    actual: u32,
    tokens: &[Vec<f32>],
) -> Result<()> {
    if actual != expected {
        return Err(CalyxError::lens_dim_mismatch(format!(
            "lens {lens_id} token dim {actual} != expected {expected}"
        )));
    }
    for token in tokens {
        if token.len() != expected as usize {
            return Err(CalyxError::lens_dim_mismatch(format!(
                "lens {lens_id} token length {} != expected {expected}",
                token.len()
            )));
        }
        ensure_finite(lens_id, token)?;
    }
    Ok(())
}

fn ensure_segmented_multi_shape(
    lens_id: LensId,
    expected: u32,
    actual: u32,
    segments: &[calyx_core::SegmentVector],
) -> Result<()> {
    if actual != expected {
        return Err(CalyxError::lens_dim_mismatch(format!(
            "lens {lens_id} segment dim {actual} != expected {expected}"
        )));
    }
    if segments.is_empty() {
        return Err(CalyxError::lens_dim_mismatch(format!(
            "lens {lens_id} emitted no long-input segments"
        )));
    }
    for segment in segments {
        if segment.data.len() != expected as usize {
            return Err(CalyxError::lens_dim_mismatch(format!(
                "lens {lens_id} segment length {} != expected {expected}",
                segment.data.len()
            )));
        }
        ensure_finite(lens_id, &segment.data)?;
    }
    Ok(())
}

fn ensure_finite(lens_id: LensId, data: &[f32]) -> Result<()> {
    if data.iter().all(|value| value.is_finite()) {
        return Ok(());
    }

    Err(CalyxError::lens_numerical_invariant(format!(
        "lens {lens_id} emitted NaN or Inf"
    )))
}
