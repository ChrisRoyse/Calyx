//! Per-slot index trait and implementations.

use calyx_core::{CxId, Result, SlotId, SlotShape, SlotVector};
use serde::{Deserialize, Serialize};

pub mod bm25;
pub mod diskann;
pub mod dual;
pub mod hnsw;
pub mod inverted;
pub mod multi;
pub mod quant_config;
pub mod tokenizer;

pub use diskann::{
    ConcatCrossTermDiskAnn, ConcatCrossTermHit, ConcatCrossTermKey, DiskAnnBuildParams,
    DiskAnnGraphReader, DiskAnnGraphWriter, DiskAnnHeader, DiskAnnNodeRef, DiskAnnSearch,
    DiskAnnSearchParams, TokenDiskAnnMaxSim, build_diskann_graph, node_block_size,
    open_diskann_graph,
};
pub use dual::{DualIndex, DualSide};
pub use hnsw::HnswIndex;
pub use inverted::InvertedIndex;
pub use multi::MaxSimIndex;
pub use quant_config::{QuantConfig, QuantKind, QuantizedVector};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct IndexSearchHit {
    pub cx_id: CxId,
    pub score: f32,
    pub rank: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexStats {
    pub slot: SlotId,
    pub shape: SlotShape,
    pub len: usize,
    pub built_at_seq: u64,
    pub base_seq: u64,
    pub kind: &'static str,
}

pub trait SextantIndex: Send + Sync {
    fn slot(&self) -> SlotId;
    fn shape(&self) -> SlotShape;
    fn insert(&mut self, cx_id: CxId, vector: SlotVector, seq: u64) -> Result<()>;
    fn search(
        &self,
        query: &SlotVector,
        k: usize,
        ef: Option<usize>,
    ) -> Result<Vec<IndexSearchHit>>;
    fn rebuild(&mut self) -> Result<()>;
    fn vector(&self, cx_id: CxId) -> Option<SlotVector>;
    fn set_base_seq(&mut self, seq: u64);
    fn stats(&self) -> IndexStats;
    fn insert_text(&mut self, _cx_id: CxId, _text: &str, _seq: u64) -> Result<()> {
        Err(crate::error::sextant_error(
            crate::error::CALYX_SEXTANT_VECTOR_SHAPE,
            "index does not accept text",
        ))
    }

    fn search_text(&self, _text: &str, _k: usize) -> Result<Vec<IndexSearchHit>> {
        Err(crate::error::sextant_error(
            crate::error::CALYX_SEXTANT_VECTOR_SHAPE,
            "index does not search text",
        ))
    }

    fn candidate_text(&self, _cx_id: CxId) -> Option<String> {
        None
    }
}

pub fn ranked(scored: Vec<(CxId, f32)>) -> Vec<IndexSearchHit> {
    scored
        .into_iter()
        .enumerate()
        .map(|(idx, (cx_id, score))| IndexSearchHit {
            cx_id,
            score,
            rank: idx + 1,
        })
        .collect()
}
