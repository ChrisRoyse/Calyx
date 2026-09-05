use calyx_core::SlotShape;

pub(super) const BYTE_FEATURE_DIM: u32 = 16;
pub const LEGAL_DOCUMENT_STRUCTURE_DIM: u32 = 32;
pub const LEGAL_DOCUMENT_STRUCTURE_POLICY: &str = "legal-document-structure-v1;coverage=complete;utf8=strict;features=judgment-layout-citations-parties-procedure-outcome;max_input_bytes=16777216";
pub const SPARSE_KEYWORDS_RAW_TF_POLICY: &str = "sparse-keywords-v2;tokenizer=unicode-whitespace;hash=content-address-u32be;weight=raw-tf;document-length=sum-tf;utf8=strict";

/// Deterministic, data-local feature encoders with no model weights.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlgorithmicEncoder {
    /// Byte and character-class features for text/code/structured inputs.
    ByteFeatures,
    /// Single scalar summary.
    Scalar,
    /// Hash-selected one-hot feature vector.
    OneHot { buckets: u32 },
    /// Small AST/code-style feature vector.
    AstStyle,
    /// Legacy length-normalized hashed whitespace terms.
    SparseKeywords { dim: u32 },
    /// Hashed whitespace terms with exact raw term frequency for BM25.
    SparseKeywordsRawTf { dim: u32 },
    /// Hashed whitespace terms as per-token vectors for MaxSim.
    TokenHash { token_dim: u32 },
    /// Complete-document legal/judicial structure features.
    LegalDocumentStructure,
    /// Dense CAMEO/event-code features from GDELT text rows.
    GdeltCameo,
    /// Sparse actor/country/geography entity features from GDELT text rows.
    GdeltActorGeo { dim: u32 },
    /// Sparse source URL host/path features from GDELT text rows.
    GdeltSourceDomain { dim: u32 },
    /// Sparse event-code/geography interaction features from GDELT text rows.
    GdeltEventGeo { dim: u32 },
    /// Sparse directed actor-pair features from GDELT text rows.
    GdeltActorPair { dim: u32 },
    /// Sparse event-code/actor interaction features from GDELT text rows.
    GdeltEventActor { dim: u32 },
    /// Sparse Goldstein/tone bucket features from GDELT text rows.
    GdeltToneSignal { dim: u32 },
    /// Sparse source-domain/event-code interaction features from GDELT text rows.
    GdeltSourceEvent { dim: u32 },
    /// Sparse action-geo-only features from GDELT text rows.
    GdeltActionGeo { dim: u32 },
    /// Sparse actor-country/presence-only features from GDELT text rows.
    GdeltActorCountry { dim: u32 },
    /// Sparse source-host-only features from GDELT text rows.
    GdeltSourceHost { dim: u32 },
    /// Sparse SQLDATE/date bucket features from GDELT text rows.
    GdeltSqlDate { dim: u32 },
    /// Sparse event-code-only features from GDELT text rows.
    GdeltEventCode { dim: u32 },
}

impl AlgorithmicEncoder {
    /// Returns the primary output dimension.
    pub const fn dim(self) -> u32 {
        match self {
            Self::ByteFeatures => BYTE_FEATURE_DIM,
            Self::Scalar => 1,
            Self::OneHot { buckets } => {
                if buckets == 0 {
                    1
                } else {
                    buckets
                }
            }
            Self::AstStyle => 8,
            Self::LegalDocumentStructure => LEGAL_DOCUMENT_STRUCTURE_DIM,
            Self::SparseKeywords { dim }
            | Self::SparseKeywordsRawTf { dim }
            | Self::GdeltActorGeo { dim }
            | Self::GdeltSourceDomain { dim }
            | Self::GdeltEventGeo { dim }
            | Self::GdeltActorPair { dim }
            | Self::GdeltEventActor { dim }
            | Self::GdeltToneSignal { dim }
            | Self::GdeltSourceEvent { dim }
            | Self::GdeltActionGeo { dim }
            | Self::GdeltActorCountry { dim }
            | Self::GdeltSourceHost { dim }
            | Self::GdeltSqlDate { dim }
            | Self::GdeltEventCode { dim } => {
                if dim == 0 {
                    1
                } else {
                    dim
                }
            }
            Self::TokenHash { token_dim } => {
                if token_dim == 0 {
                    1
                } else {
                    token_dim
                }
            }
            Self::GdeltCameo => 16,
        }
    }

    pub const fn shape(self) -> SlotShape {
        match self {
            Self::SparseKeywords { dim }
            | Self::SparseKeywordsRawTf { dim }
            | Self::GdeltActorGeo { dim }
            | Self::GdeltSourceDomain { dim }
            | Self::GdeltEventGeo { dim }
            | Self::GdeltActorPair { dim }
            | Self::GdeltEventActor { dim }
            | Self::GdeltToneSignal { dim }
            | Self::GdeltSourceEvent { dim }
            | Self::GdeltActionGeo { dim }
            | Self::GdeltActorCountry { dim }
            | Self::GdeltSourceHost { dim }
            | Self::GdeltSqlDate { dim }
            | Self::GdeltEventCode { dim } => SlotShape::Sparse(if dim == 0 { 1 } else { dim }),
            Self::TokenHash { token_dim } => SlotShape::Multi {
                token_dim: if token_dim == 0 { 1 } else { token_dim },
            },
            _ => SlotShape::Dense(self.dim()),
        }
    }
}
