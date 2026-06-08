#![deny(warnings)]

//! Lodestar grounding-kernel discovery and maintenance.

pub mod dfvs;
mod error;
pub mod grounding_gaps;
pub mod incremental;
pub mod kernel;
pub mod kernel_answer;
pub mod kernel_graph;
pub mod kernel_index;
pub mod loom_assoc;
pub mod recall_test;

pub use dfvs::{
    DfvsMethod, DfvsResult, bounded_genus_approx, dfvs_approx, genus_estimate, is_tournament,
    tournament_2approx,
};
pub use error::{LodestarError, Result};
pub use grounding_gaps::{CALYX_KERNEL_UNGROUNDED, GroundingGapReport, grounding_gaps};
pub use incremental::{IncrementalKernelEval, IncrementalResult, NodeAddEdge};
pub use kernel::{GroundednessReport, Kernel, KernelParams, RecallReport, build_kernel_pipeline};
pub use kernel_answer::{AnswerHop, AnswerPath, kernel_answer};
pub use kernel_graph::{
    KernelGraph, KernelGraphParams, LpRoundParams, NodeScore, groundedness_distance,
    lp_round_kernel_graph, lp_round_kernel_graph_from_solution, select_kernel_graph,
};
pub use kernel_index::{
    EmbeddingStore, FsKernelStore, KernelIndex, KernelStore, KernelVectorRow, build_kernel_index,
    kernel_search, load_kernel_index, write_kernel_index,
};
pub use loom_assoc::{
    LoomAssocEdgeProvenance, LoomAssocGraphInput, LoomDirectionalConfidence, LoomSlotNode,
    build_assoc_graph_from_loom, loom_assoc_graph_input,
};
pub use recall_test::{
    AnnIndex, CALYX_KERNEL_RECALL_BELOW_GATE, CorpusReader, InMemoryAnnIndex, InMemoryCorpus,
    RecallQuery, RecallTestParams, RecallTestReport, kernel_recall_test,
    kernel_recall_test_with_clock,
};
