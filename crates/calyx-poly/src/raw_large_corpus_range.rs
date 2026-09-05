use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LargeCorpusRangeState {
    pub chain: String,
    pub address: String,
    pub topics: Vec<String>,
    pub from_block: u64,
    pub to_block: u64,
    pub requested_block_count: u64,
    pub max_blocks_per_chunk: u64,
    pub chunk_index: usize,
    pub chunk_count: usize,
    pub next_from_block: Option<u64>,
    pub range_policy: String,
    pub limit_semantics: String,
    pub provider_limit_evidence: String,
}
