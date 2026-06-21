use calyx_core::LensId;
use serde::{Deserialize, Serialize};

use crate::lens::Registry;
use crate::spec::LensRuntime;

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

pub fn signal_kind_from_runtime(runtime: &LensRuntime) -> CapabilitySignalKind {
    match runtime {
        LensRuntime::Algorithmic { kind } if kind.starts_with("commissioned:") => {
            CapabilitySignalKind::LearnedEncoder
        }
        LensRuntime::Algorithmic { .. } => CapabilitySignalKind::DeterministicContentFeature,
        LensRuntime::TeiHttp { .. }
        | LensRuntime::CandleLocal { .. }
        | LensRuntime::Onnx { .. }
        | LensRuntime::OnnxColbert { .. }
        | LensRuntime::FastembedSparse { .. }
        | LensRuntime::FastembedBgem3 { .. }
        | LensRuntime::FastembedReranker { .. }
        | LensRuntime::FastembedQwen3 { .. }
        | LensRuntime::StaticLookup { .. }
        | LensRuntime::MultimodalAdapter { .. } => CapabilitySignalKind::LearnedEncoder,
        LensRuntime::ExternalCmd { .. } => CapabilitySignalKind::Unknown,
    }
}

pub(super) fn registry_signal_kind(registry: &Registry, lens_id: LensId) -> CapabilitySignalKind {
    registry
        .lens_spec(lens_id)
        .map(|spec| signal_kind_from_runtime(&spec.runtime))
        .unwrap_or(CapabilitySignalKind::Unknown)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commissioned_algorithmic_runtime_is_learned_encoder_signal() {
        let runtime = LensRuntime::Algorithmic {
            kind: "commissioned:facebook/esm2_t6_8M_UR50D".to_string(),
        };

        assert_eq!(
            signal_kind_from_runtime(&runtime),
            CapabilitySignalKind::LearnedEncoder
        );
    }

    #[test]
    fn plain_algorithmic_runtime_stays_deterministic_feature_signal() {
        let runtime = LensRuntime::Algorithmic {
            kind: "byte_features".to_string(),
        };

        assert_eq!(
            signal_kind_from_runtime(&runtime),
            CapabilitySignalKind::DeterministicContentFeature
        );
    }
}
