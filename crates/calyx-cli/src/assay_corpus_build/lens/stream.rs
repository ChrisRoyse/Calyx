use std::path::Path;

use calyx_core::Input;

use crate::lens_commands::support::dim;

use super::projection::{projected_slot_dim, slot_projection_name};
use super::{BuildLens, assay_vectors, measure_batches};

impl BuildLens {
    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    pub(crate) fn manifest(&self) -> &Path {
        &self.manifest
    }

    pub(crate) fn dim(&self) -> usize {
        projected_slot_dim(self.spec.output) as usize
    }

    pub(crate) fn native_dim(&self) -> usize {
        dim(self.spec.output) as usize
    }

    pub(crate) fn assay_projection(&self) -> &'static str {
        slot_projection_name(self.spec.output)
    }

    pub(crate) fn max_batch(&self) -> Option<usize> {
        self.spec.max_batch
    }

    pub(crate) fn lens_id(&self) -> String {
        self.spec.lens_id().to_string()
    }

    pub(crate) fn weights_sha256_hex(&self) -> String {
        self.spec
            .weights_sha256
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    pub(crate) fn signal_kind(&self) -> &'static str {
        crate::a35_signal::runtime_signal_kind_name(&self.spec.runtime)
    }

    pub(crate) fn effective_batch_size(&self, requested: usize) -> usize {
        self.max_batch()
            .filter(|value| *value > 0)
            .map(|value| value.min(requested))
            .unwrap_or(requested)
    }
}

pub(crate) fn measure_text_batch(
    lens: &BuildLens,
    texts: &[String],
    batch_size: usize,
) -> Result<Vec<Vec<f32>>, String> {
    let inputs = texts
        .iter()
        .map(|text| Input::new(lens.spec.modality, text.as_bytes().to_vec()))
        .collect::<Vec<_>>();
    let slots = measure_batches(lens, &inputs, lens.effective_batch_size(batch_size))?;
    assay_vectors(lens, slots).map(|(vectors, _)| vectors)
}
