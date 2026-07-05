use calyx_core::{CalyxError, Result, SlotVector};

use super::batch::TokenBatch;

pub(super) fn write_slot_rows(
    rows: &mut [Option<SlotVector>],
    batch: &TokenBatch,
    vectors: Vec<SlotVector>,
) -> Result<()> {
    if vectors.len() != batch.batch {
        return Err(CalyxError::lens_dim_mismatch(format!(
            "custom ONNX returned {} rows for a padded batch of {}",
            vectors.len(),
            batch.batch
        )));
    }
    // Rows beyond the real inputs are #1143 padding replicas.
    for (index, vector) in batch.indices.iter().copied().zip(vectors) {
        rows[index] = Some(vector);
    }
    Ok(())
}

pub(super) fn finalize_slot_rows(rows: Vec<Option<SlotVector>>) -> Result<Vec<SlotVector>> {
    rows.into_iter()
        .map(|vector| {
            vector
                .ok_or_else(|| CalyxError::lens_dim_mismatch("custom ONNX omitted a bucketed row"))
        })
        .collect()
}
