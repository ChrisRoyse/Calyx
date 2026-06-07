use std::collections::BTreeMap;
use std::sync::Arc;

use calyx_core::{CalyxError, Input, Lens, LensId, Result, SlotShape, SlotVector, SparseEntry};

/// Runtime registry for frozen lens measurement instruments.
#[derive(Clone, Default)]
pub struct Registry {
    lenses: BTreeMap<LensId, Arc<dyn Lens>>,
}

impl Registry {
    /// Creates an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a lens by its stable frozen id.
    pub fn register<L>(&mut self, lens: L) -> Result<LensId>
    where
        L: Lens + 'static,
    {
        let id = lens.id();
        if self.lenses.contains_key(&id) {
            return Err(CalyxError::lens_frozen_violation(format!(
                "lens {id} is already registered"
            )));
        }
        self.lenses.insert(id, Arc::new(lens));
        Ok(id)
    }

    /// Returns true when a lens id is registered.
    pub fn contains(&self, id: LensId) -> bool {
        self.lenses.contains_key(&id)
    }

    /// Measures one input with a registered lens.
    pub fn measure(&self, lens_id: LensId, input: &Input) -> Result<SlotVector> {
        let lens = self.lookup(lens_id)?;
        ensure_input_modality(lens.as_ref(), input)?;
        let vector = lens.measure(input)?;
        ensure_vector_shape(lens_id, lens.shape(), &vector)?;
        Ok(vector)
    }

    /// Measures a batch with a registered lens and validates every result.
    pub fn measure_batch(&self, lens_id: LensId, inputs: &[Input]) -> Result<Vec<SlotVector>> {
        let lens = self.lookup(lens_id)?;
        for input in inputs {
            ensure_input_modality(lens.as_ref(), input)?;
        }

        let vectors = lens.measure_batch(inputs)?;
        if vectors.len() != inputs.len() {
            return Err(CalyxError::lens_dim_mismatch(format!(
                "lens {lens_id} returned {} vectors for {} inputs",
                vectors.len(),
                inputs.len()
            )));
        }
        for vector in &vectors {
            ensure_vector_shape(lens_id, lens.shape(), vector)?;
        }
        Ok(vectors)
    }

    fn lookup(&self, lens_id: LensId) -> Result<&Arc<dyn Lens>> {
        self.lenses.get(&lens_id).ok_or_else(|| {
            CalyxError::lens_unreachable(format!("lens {lens_id} is not registered"))
        })
    }
}

/// Verifies that an input matches a lens' declared modality.
pub fn ensure_input_modality(lens: &dyn Lens, input: &Input) -> Result<()> {
    if input.modality == lens.modality() {
        return Ok(());
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

fn ensure_finite(lens_id: LensId, data: &[f32]) -> Result<()> {
    if data.iter().all(|value| value.is_finite()) {
        return Ok(());
    }

    Err(CalyxError::lens_numerical_invariant(format!(
        "lens {lens_id} emitted NaN or Inf"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use calyx_core::{Modality, SlotShape};

    #[test]
    fn registry_measures_registered_lens() {
        let mut registry = Registry::new();
        let id = registry.register(OneDimLens).unwrap();
        let input = Input::new(Modality::Text, b"abc".to_vec());

        let vector = registry.measure(id, &input).unwrap();

        assert_eq!(
            vector,
            SlotVector::Dense {
                dim: 1,
                data: vec![3.0]
            }
        );
    }

    #[test]
    fn registry_rejects_wrong_modality() {
        let mut registry = Registry::new();
        let id = registry.register(OneDimLens).unwrap();
        let input = Input::new(Modality::Image, vec![1, 2, 3]);

        let error = registry.measure(id, &input).unwrap_err();

        assert_eq!(error.code, "CALYX_LENS_DIM_MISMATCH");
    }

    #[test]
    fn registry_rejects_mismatched_batch_count() {
        let mut registry = Registry::new();
        let id = registry.register(ShortBatchLens).unwrap();
        let inputs = [
            Input::new(Modality::Text, b"a".to_vec()),
            Input::new(Modality::Text, b"b".to_vec()),
        ];

        let error = registry.measure_batch(id, &inputs).unwrap_err();

        println!("MISMATCHED_BATCH_ERROR={}", error.code);
        assert_eq!(error.code, "CALYX_LENS_DIM_MISMATCH");
    }

    #[test]
    fn registry_rejects_non_finite_dense_values() {
        let mut registry = Registry::new();
        let id = registry.register(NanLens).unwrap();
        let input = Input::new(Modality::Text, b"x".to_vec());

        let error = registry.measure(id, &input).unwrap_err();

        assert_eq!(error.code, "CALYX_LENS_NUMERICAL_INVARIANT");
    }

    struct OneDimLens;

    impl Lens for OneDimLens {
        fn id(&self) -> LensId {
            LensId::from_bytes([1; 16])
        }

        fn shape(&self) -> SlotShape {
            SlotShape::Dense(1)
        }

        fn modality(&self) -> Modality {
            Modality::Text
        }

        fn measure(&self, input: &Input) -> Result<SlotVector> {
            Ok(SlotVector::Dense {
                dim: 1,
                data: vec![input.bytes.len() as f32],
            })
        }
    }

    struct ShortBatchLens;

    impl Lens for ShortBatchLens {
        fn id(&self) -> LensId {
            LensId::from_bytes([2; 16])
        }

        fn shape(&self) -> SlotShape {
            SlotShape::Dense(1)
        }

        fn modality(&self) -> Modality {
            Modality::Text
        }

        fn measure(&self, _input: &Input) -> Result<SlotVector> {
            Ok(SlotVector::Dense {
                dim: 1,
                data: vec![1.0],
            })
        }

        fn measure_batch(&self, _inputs: &[Input]) -> Result<Vec<SlotVector>> {
            Ok(vec![SlotVector::Dense {
                dim: 1,
                data: vec![1.0],
            }])
        }
    }

    struct NanLens;

    impl Lens for NanLens {
        fn id(&self) -> LensId {
            LensId::from_bytes([3; 16])
        }

        fn shape(&self) -> SlotShape {
            SlotShape::Dense(1)
        }

        fn modality(&self) -> Modality {
            Modality::Text
        }

        fn measure(&self, _input: &Input) -> Result<SlotVector> {
            Ok(SlotVector::Dense {
                dim: 1,
                data: vec![f32::NAN],
            })
        }
    }
}
