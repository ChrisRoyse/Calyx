use calyx_core::{Asymmetry, QuantPolicy};

use crate::frozen::FrozenLensContract;

use super::{LensRuntime, LensSpec, default_recall_delta};

impl LensSpec {
    /// Builds complete registry metadata for an algorithmic content lens.
    pub fn algorithmic_content(
        contract: &FrozenLensContract,
        kind: impl Into<String>,
        axis: Option<String>,
        quant_default: QuantPolicy,
    ) -> Self {
        Self {
            name: contract.name().to_string(),
            runtime: LensRuntime::Algorithmic { kind: kind.into() },
            output: contract.shape(),
            modality: contract.modality(),
            weights_sha256: contract.weights_sha256(),
            corpus_hash: contract.corpus_hash(),
            norm_policy: contract.norm_policy(),
            max_batch: None,
            axis,
            asymmetry: Asymmetry::None,
            quant_default,
            truncate_dim: None,
            recall_delta: default_recall_delta(),
            retrieval_only: false,
            excluded_from_dedup: false,
        }
    }
}
