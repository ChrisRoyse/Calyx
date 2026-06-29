use calyx_core::CxId;
use thiserror::Error;

pub type Result<T> = std::result::Result<T, MincutError>;

#[derive(Clone, Debug, PartialEq, Error)]
pub enum MincutError {
    #[error("CALYX_SCC_GRAPH_MISMATCH: {detail}")]
    SccGraphMismatch { detail: String },
    #[error("CALYX_BETWEENNESS_EMPTY_GRAPH: betweenness requires at least one node")]
    BetweennessEmptyGraph,
    #[error("CALYX_LP_INVALID: {detail}")]
    LpInvalid { detail: String },
    #[error("CALYX_LP_SOLVER_LIMIT: {detail}")]
    LpSolverLimit { detail: String },
    #[error("CALYX_LP_SOLVE_FAILED: {detail}")]
    LpSolveFailed { detail: String },
    #[error("CALYX_MINCUT_NODE_NOT_FOUND: node {id} is absent")]
    NodeNotFound { id: CxId },
}

impl MincutError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::SccGraphMismatch { .. } => "CALYX_SCC_GRAPH_MISMATCH",
            Self::BetweennessEmptyGraph => "CALYX_BETWEENNESS_EMPTY_GRAPH",
            Self::LpInvalid { .. } => "CALYX_LP_INVALID",
            Self::LpSolverLimit { .. } => "CALYX_LP_SOLVER_LIMIT",
            Self::LpSolveFailed { .. } => "CALYX_LP_SOLVE_FAILED",
            Self::NodeNotFound { .. } => "CALYX_MINCUT_NODE_NOT_FOUND",
        }
    }

    pub fn lp_invalid(detail: impl Into<String>) -> Self {
        Self::LpInvalid {
            detail: detail.into(),
        }
    }

    pub fn lp_solver_limit(detail: impl Into<String>) -> Self {
        Self::LpSolverLimit {
            detail: detail.into(),
        }
    }

    pub fn lp_solve_failed(detail: impl Into<String>) -> Self {
        Self::LpSolveFailed {
            detail: detail.into(),
        }
    }
}
