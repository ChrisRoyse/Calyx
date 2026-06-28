use calyx_core::LensId;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::lens::Registry;
use crate::spec::{LensRuntime, LensSpec};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilitySignalKind {
    #[default]
    Unknown,
    LearnedEncoder,
    DeterministicContentFeature,
    Algorithmic,
    Placeholder,
}

impl CapabilitySignalKind {
    pub fn is_learned_encoder(self) -> bool {
        matches!(self, Self::LearnedEncoder)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::LearnedEncoder => "learned_encoder",
            Self::DeterministicContentFeature => "deterministic_content_feature",
            Self::Algorithmic => "algorithmic",
            Self::Placeholder => "placeholder",
        }
    }
}

pub fn signal_kind_from_spec(spec: &LensSpec) -> CapabilitySignalKind {
    match &spec.runtime {
        LensRuntime::Algorithmic { .. } => CapabilitySignalKind::DeterministicContentFeature,
        LensRuntime::TeiHttp { endpoint } => {
            learned_if(has_hash_provenance(spec) && !endpoint.trim().is_empty())
        }
        LensRuntime::CandleLocal {
            model_id, files, ..
        }
        | LensRuntime::Onnx { model_id, files }
        | LensRuntime::OnnxColbert { model_id, files }
        | LensRuntime::FastembedSparse { model_id, files }
        | LensRuntime::FastembedReranker { model_id, files } => learned_if(
            has_hash_provenance(spec) && has_model_id(model_id) && has_artifact_paths(files),
        ),
        LensRuntime::FastembedBgem3 {
            model_id, files, ..
        }
        | LensRuntime::FastembedQwen3 {
            model_id, files, ..
        } => learned_if(
            has_hash_provenance(spec) && has_model_id(model_id) && has_artifact_paths(files),
        ),
        LensRuntime::StaticLookup {
            embeddings_file,
            tokenizer,
            dim,
        } => learned_if(
            has_hash_provenance(spec)
                && *dim > 0
                && has_path_provenance(embeddings_file)
                && has_path_provenance(tokenizer),
        ),
        LensRuntime::MultimodalAdapter {
            axis,
            model_id,
            adapter_config,
            files,
        } => learned_if(
            has_hash_provenance(spec)
                && !axis.trim().is_empty()
                && has_model_id(model_id)
                && adapter_config.as_deref().is_some_and(has_path_provenance)
                && has_artifact_paths(files),
        ),
        LensRuntime::ExternalCmd { .. } => CapabilitySignalKind::Unknown,
    }
}

pub(super) fn registry_signal_kind(registry: &Registry, lens_id: LensId) -> CapabilitySignalKind {
    registry
        .lens_spec(lens_id)
        .map(signal_kind_from_spec)
        .unwrap_or(CapabilitySignalKind::Unknown)
}

fn learned_if(verified: bool) -> CapabilitySignalKind {
    if verified {
        CapabilitySignalKind::LearnedEncoder
    } else {
        CapabilitySignalKind::Unknown
    }
}

fn has_hash_provenance(spec: &LensSpec) -> bool {
    spec.weights_sha256 != [0; 32] && spec.corpus_hash != [0; 32]
}

fn has_model_id(model_id: &str) -> bool {
    !model_id.trim().is_empty()
}

fn has_artifact_paths(files: &[PathBuf]) -> bool {
    !files.is_empty() && files.iter().all(|path| has_path_provenance(path))
}

fn has_path_provenance(path: &Path) -> bool {
    !path.as_os_str().is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frozen::NormPolicy;
    use calyx_core::{Asymmetry, Modality, QuantPolicy, SlotShape};

    #[test]
    fn commissioned_algorithmic_spec_stays_deterministic_feature_signal() {
        let runtime = LensRuntime::Algorithmic {
            kind: "commissioned:facebook/esm2_t6_8M_UR50D".to_string(),
        };
        let spec = learned_spec("commissioned", runtime);

        assert_eq!(
            signal_kind_from_spec(&spec),
            CapabilitySignalKind::DeterministicContentFeature
        );
    }

    #[test]
    fn plain_algorithmic_spec_stays_deterministic_feature_signal() {
        let runtime = LensRuntime::Algorithmic {
            kind: "byte_features".to_string(),
        };
        let spec = learned_spec("plain", runtime);

        assert_eq!(
            signal_kind_from_spec(&spec),
            CapabilitySignalKind::DeterministicContentFeature
        );
    }

    #[test]
    fn learned_spec_requires_hash_model_and_artifact_provenance() {
        let valid = learned_spec(
            "valid-onnx",
            LensRuntime::Onnx {
                model_id: "sentence-transformers/all-MiniLM-L6-v2".to_string(),
                files: vec![PathBuf::from("model.onnx")],
            },
        );
        let missing_model = learned_spec(
            "missing-model",
            LensRuntime::Onnx {
                model_id: String::new(),
                files: vec![PathBuf::from("model.onnx")],
            },
        );
        let missing_hash = LensSpec {
            weights_sha256: [0; 32],
            ..valid.clone()
        };

        assert_eq!(
            signal_kind_from_spec(&valid),
            CapabilitySignalKind::LearnedEncoder
        );
        assert_eq!(
            signal_kind_from_spec(&missing_model),
            CapabilitySignalKind::Unknown
        );
        assert_eq!(
            signal_kind_from_spec(&missing_hash),
            CapabilitySignalKind::Unknown
        );
    }

    fn learned_spec(name: &str, runtime: LensRuntime) -> LensSpec {
        LensSpec {
            name: name.to_string(),
            runtime,
            output: SlotShape::Dense(3),
            modality: Modality::Text,
            weights_sha256: [1; 32],
            corpus_hash: [2; 32],
            norm_policy: NormPolicy::None,
            max_batch: None,
            axis: Some(name.to_string()),
            asymmetry: Asymmetry::None,
            quant_default: QuantPolicy::turboquant_default(),
            truncate_dim: None,
            recall_delta: crate::spec::default_recall_delta(),
            retrieval_only: false,
            excluded_from_dedup: false,
        }
    }
}
