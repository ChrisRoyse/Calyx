use calyx_core::CxId;
use thiserror::Error;

pub type Result<T> = std::result::Result<T, PathsError>;

#[derive(Clone, Debug, PartialEq, Error)]
pub enum PathsError {
    #[error("CALYX_GRAPH_DUPLICATE_NODE: duplicate graph node {id}")]
    GraphDuplicateNode { id: CxId },
    #[error("CALYX_GRAPH_UNKNOWN_NODE: unknown graph node {id}")]
    GraphUnknownNode { id: CxId },
    #[error("CALYX_GRAPH_INVALID_WEIGHT: invalid {field} weight {value}")]
    GraphInvalidWeight { field: &'static str, value: f32 },
    #[error("CALYX_PATHS_MAX_HOPS: path requires {required} hops but max_hops={max_hops}")]
    MaxHops { required: usize, max_hops: usize },
    #[error("CALYX_PATHS_NODE_NOT_FOUND: traversal node {id} is absent")]
    NodeNotFound { id: CxId },
}

impl PathsError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::GraphDuplicateNode { .. } => "CALYX_GRAPH_DUPLICATE_NODE",
            Self::GraphUnknownNode { .. } => "CALYX_GRAPH_UNKNOWN_NODE",
            Self::GraphInvalidWeight { .. } => "CALYX_GRAPH_INVALID_WEIGHT",
            Self::MaxHops { .. } => "CALYX_PATHS_MAX_HOPS",
            Self::NodeNotFound { .. } => "CALYX_PATHS_NODE_NOT_FOUND",
        }
    }
}
