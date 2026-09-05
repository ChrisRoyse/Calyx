#[cfg(test)]
use calyx_core::{Input, Lens, Modality, SlotVector};

mod batch;
#[cfg(all(test, feature = "cuda"))]
mod benchmark;
mod contract;
mod cpu;
mod encoder;
mod gdelt;
mod legal_document;
mod lens;
mod runtime_kind;
mod structured;

pub use batch::{
    AlgorithmicBatchProvider, AlgorithmicBatchStats, BYTE_FEATURES_CUDA_MIN_INPUT_BYTES,
    SPARSE_KEYWORDS_CUDA_MIN_TOKENS, TOKEN_HASH_CUDA_MIN_WORDS,
};
pub(crate) use contract::algorithmic_contract;
use cpu::hash_part;
use encoder::BYTE_FEATURE_DIM;
pub use encoder::{
    AlgorithmicEncoder, LEGAL_DOCUMENT_STRUCTURE_DIM, LEGAL_DOCUMENT_STRUCTURE_POLICY,
    SPARSE_KEYWORDS_RAW_TF_POLICY,
};
pub use lens::AlgorithmicLens;
pub(crate) use structured::structured_runtime_error;
pub use structured::{
    CALYX_STRUCTURED_BINDING_INVALID, CALYX_STRUCTURED_CATEGORY_UNKNOWN,
    CALYX_STRUCTURED_DATE_INVALID, CALYX_STRUCTURED_DERIVATION_INVALID, DateComponent,
    NumericDerivation, NumericScaling, NumericSourceType, StructuredBinding, StructuredEncoder,
    StructuredSelector, StructuredSourceType,
};

#[cfg(test)]
mod tests;
