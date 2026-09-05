use std::path::Path;

use calyx_core::{Input, StructuredRecord};

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

    pub(crate) fn runtime_name(&self) -> &str {
        &self.runtime_name
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

    pub(crate) fn measurement_identity_sha256_hex(&self) -> String {
        self.spec
            .corpus_hash
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    pub(crate) fn endpoint_identity_sha256(&self) -> Option<String> {
        self.endpoint_identity_sha256.clone()
    }

    pub(crate) fn model_identity(&self) -> Option<String> {
        self.model_identity.clone()
    }

    pub(crate) fn model_dtype(&self) -> Option<String> {
        self.model_dtype.clone()
    }

    pub(crate) fn prompt_identity_sha256(&self) -> Option<String> {
        self.prompt_identity_sha256.clone()
    }

    pub(crate) fn signal_kind(&self) -> &'static str {
        crate::a35_signal::lens_spec_signal_kind_name(&self.spec)
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
    records: &[Option<StructuredRecord>],
    batch_size: usize,
) -> Result<Vec<Vec<f32>>, String> {
    if texts.len() != records.len() {
        return Err(format!(
            "CALYX_FSV_ASSAY_CORPUS_BUILD_STRUCTURED_ALIGNMENT: lens={} received {} texts but {} structured records; every row must carry an aligned record slot",
            lens.name(),
            texts.len(),
            records.len()
        ));
    }
    let inputs = texts
        .iter()
        .zip(records)
        .map(|(text, record)| {
            // Mirror the ingest `prepared_hybrid_text_input` shape: text bytes plus
            // the optional canonical typed record. A structured lens reads only the
            // attached record, so a hybrid input yields byte-identical slot vectors
            // to the ingest path while text lenses keep measuring the byte payload.
            let input = Input::new(lens.spec.modality, text.as_bytes().to_vec());
            match record {
                Some(record) => input.with_structured(record.clone()),
                None => input,
            }
        })
        .collect::<Vec<_>>();
    let slots = measure_batches(lens, &inputs, lens.effective_batch_size(batch_size))?;
    assay_vectors(lens, slots).map(|(vectors, _)| vectors)
}
