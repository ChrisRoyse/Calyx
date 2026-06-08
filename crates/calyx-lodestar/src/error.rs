use thiserror::Error;

pub type Result<T> = std::result::Result<T, LodestarError>;

#[derive(Clone, Debug, PartialEq, Error)]
pub enum LodestarError {
    #[error("CALYX_KERNEL_EMPTY_GRAPH: kernel graph selection requires at least one node")]
    KernelEmptyGraph,
    #[error("CALYX_KERNEL_INVALID_PARAMS: {detail}")]
    KernelInvalidParams { detail: String },
    #[error("CALYX_KERNEL_LP_UNAVAILABLE: {detail}")]
    KernelLpUnavailable { detail: String },
    #[error("CALYX_KERNEL_LP_INFEASIBLE: {detail}")]
    KernelLpInfeasible { detail: String },
    #[error("CALYX_KERNEL_EMPTY_RESULT: kernel selection returned no nodes")]
    KernelEmptyResult,
    #[error("CALYX_KERNEL_INDEX_NOT_FOUND: kernel index {kernel_id} was not found")]
    KernelIndexNotFound { kernel_id: calyx_core::CxId },
    #[error("CALYX_KERNEL_DIM_MISMATCH: expected dim {expected}, got {actual}")]
    KernelDimMismatch { expected: usize, actual: usize },
    #[error("CALYX_KERNEL_EMBEDDING_MISSING: missing embedding for {cx_id}")]
    KernelEmbeddingMissing { cx_id: calyx_core::CxId },
    #[error("CALYX_KERNEL_INDEX_IO: {detail}")]
    KernelIndexIo { detail: String },
    #[error("CALYX_KERNEL_INDEX_CODEC: {detail}")]
    KernelIndexCodec { detail: String },
    #[error("CALYX_KERNEL_INDEX_BUILD: {detail}")]
    KernelIndexBuild { detail: String },
    #[error("CALYX_DFVS_VERIFICATION_FAILED: {detail}")]
    DfvsVerificationFailed { detail: String },
    #[error("CALYX_DFVS_GENUS_TOO_LARGE: genus {genus} exceeds supported bound")]
    DfvsGenusTooLarge { genus: usize },
    #[error("{code}: {message}")]
    Graph { code: &'static str, message: String },
}

impl LodestarError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::KernelEmptyGraph => "CALYX_KERNEL_EMPTY_GRAPH",
            Self::KernelInvalidParams { .. } => "CALYX_KERNEL_INVALID_PARAMS",
            Self::KernelLpUnavailable { .. } => "CALYX_KERNEL_LP_UNAVAILABLE",
            Self::KernelLpInfeasible { .. } => "CALYX_KERNEL_LP_INFEASIBLE",
            Self::KernelEmptyResult => "CALYX_KERNEL_EMPTY_RESULT",
            Self::KernelIndexNotFound { .. } => "CALYX_KERNEL_INDEX_NOT_FOUND",
            Self::KernelDimMismatch { .. } => "CALYX_KERNEL_DIM_MISMATCH",
            Self::KernelEmbeddingMissing { .. } => "CALYX_KERNEL_EMBEDDING_MISSING",
            Self::KernelIndexIo { .. } => "CALYX_KERNEL_INDEX_IO",
            Self::KernelIndexCodec { .. } => "CALYX_KERNEL_INDEX_CODEC",
            Self::KernelIndexBuild { .. } => "CALYX_KERNEL_INDEX_BUILD",
            Self::DfvsVerificationFailed { .. } => "CALYX_DFVS_VERIFICATION_FAILED",
            Self::DfvsGenusTooLarge { .. } => "CALYX_DFVS_GENUS_TOO_LARGE",
            Self::Graph { code, .. } => code,
        }
    }
}

impl From<calyx_paths::PathsError> for LodestarError {
    fn from(value: calyx_paths::PathsError) -> Self {
        Self::Graph {
            code: value.code(),
            message: value.to_string(),
        }
    }
}

impl From<calyx_mincut::MincutError> for LodestarError {
    fn from(value: calyx_mincut::MincutError) -> Self {
        Self::Graph {
            code: value.code(),
            message: value.to_string(),
        }
    }
}
