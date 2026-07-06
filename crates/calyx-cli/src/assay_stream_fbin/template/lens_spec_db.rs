use calyx_core::{Asymmetry, Modality, QuantPolicy, SlotShape};
use calyx_registry::{LensRuntime, LensSpec, NormPolicy};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(super) struct StoredLensSpec {
    name: String,
    runtime: LensRuntime,
    output: SlotShape,
    modality: Modality,
    weights_sha256: [u8; 32],
    corpus_hash: [u8; 32],
    norm_policy: NormPolicy,
    max_batch: Option<usize>,
    axis: Option<String>,
    asymmetry: Asymmetry,
    quant_default: QuantPolicy,
    truncate_dim: Option<u32>,
    recall_delta: f32,
    retrieval_only: bool,
    excluded_from_dedup: bool,
}

pub(super) fn stored(spec: &LensSpec) -> StoredLensSpec {
    StoredLensSpec {
        name: spec.name.clone(),
        runtime: spec.runtime.clone(),
        output: spec.output,
        modality: spec.modality,
        weights_sha256: spec.weights_sha256,
        corpus_hash: spec.corpus_hash,
        norm_policy: spec.norm_policy,
        max_batch: spec.max_batch,
        axis: spec.axis.clone(),
        asymmetry: spec.asymmetry,
        quant_default: spec.quant_default,
        truncate_dim: spec.truncate_dim,
        recall_delta: spec.recall_delta,
        retrieval_only: spec.retrieval_only,
        excluded_from_dedup: spec.excluded_from_dedup,
    }
}

pub(super) fn serialize<S>(spec: &LensSpec, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    stored(spec).serialize(serializer)
}

pub(super) fn deserialize<'de, D>(deserializer: D) -> Result<LensSpec, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(StoredLensSpec::deserialize(deserializer)?.into())
}

impl From<StoredLensSpec> for LensSpec {
    fn from(stored: StoredLensSpec) -> Self {
        Self {
            name: stored.name,
            runtime: stored.runtime,
            output: stored.output,
            modality: stored.modality,
            weights_sha256: stored.weights_sha256,
            corpus_hash: stored.corpus_hash,
            norm_policy: stored.norm_policy,
            max_batch: stored.max_batch,
            axis: stored.axis,
            asymmetry: stored.asymmetry,
            quant_default: stored.quant_default,
            truncate_dim: stored.truncate_dim,
            recall_delta: stored.recall_delta,
            retrieval_only: stored.retrieval_only,
            excluded_from_dedup: stored.excluded_from_dedup,
        }
    }
}
