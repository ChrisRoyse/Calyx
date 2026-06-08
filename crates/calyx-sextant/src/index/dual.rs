//! Dual directional index scaffold for asymmetric slots.

use calyx_core::{CxId, Result, SlotId, SlotShape, SlotVector};
use serde::{Deserialize, Serialize};

use super::{HnswIndex, IndexSearchHit, IndexStats, SextantIndex};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DualSide {
    A,
    B,
}

#[derive(Clone, Debug)]
pub struct DualIndex {
    slot: SlotId,
    a: HnswIndex,
    b: HnswIndex,
    boost_a_to_b: f32,
    boost_b_to_a: f32,
}

impl DualIndex {
    pub fn new(slot: SlotId, dim: u32, seed: u64) -> Self {
        Self {
            slot,
            a: HnswIndex::new(slot, dim, seed),
            b: HnswIndex::new(slot, dim, seed ^ 0x9e37),
            boost_a_to_b: 1.0,
            boost_b_to_a: 1.0,
        }
    }

    pub fn with_boosts(mut self, a_to_b: f32, b_to_a: f32) -> Self {
        self.boost_a_to_b = a_to_b;
        self.boost_b_to_a = b_to_a;
        self
    }

    pub fn insert_side(
        &mut self,
        side: DualSide,
        cx_id: CxId,
        vector: SlotVector,
        seq: u64,
    ) -> Result<()> {
        match side {
            DualSide::A => self.a.insert(cx_id, vector, seq),
            DualSide::B => self.b.insert(cx_id, vector, seq),
        }
    }

    pub fn search_side(
        &self,
        side: DualSide,
        query: &SlotVector,
        k: usize,
    ) -> Result<Vec<IndexSearchHit>> {
        let (index, boost) = match side {
            DualSide::A => (&self.a, self.boost_a_to_b),
            DualSide::B => (&self.b, self.boost_b_to_a),
        };
        let mut hits = index.search(query, k, None)?;
        for hit in &mut hits {
            hit.score *= boost;
        }
        Ok(hits)
    }
}

impl SextantIndex for DualIndex {
    fn slot(&self) -> SlotId {
        self.slot
    }

    fn shape(&self) -> SlotShape {
        self.a.shape()
    }

    fn insert(&mut self, cx_id: CxId, vector: SlotVector, seq: u64) -> Result<()> {
        self.insert_side(DualSide::A, cx_id, vector.clone(), seq)?;
        self.insert_side(DualSide::B, cx_id, vector, seq)
    }

    fn search(
        &self,
        query: &SlotVector,
        k: usize,
        _ef: Option<usize>,
    ) -> Result<Vec<IndexSearchHit>> {
        self.search_side(DualSide::A, query, k)
    }

    fn rebuild(&mut self) -> Result<()> {
        self.a.rebuild()?;
        self.b.rebuild()
    }

    fn vector(&self, cx_id: CxId) -> Option<SlotVector> {
        self.a.vector(cx_id).or_else(|| self.b.vector(cx_id))
    }

    fn set_base_seq(&mut self, seq: u64) {
        self.a.set_base_seq(seq);
        self.b.set_base_seq(seq);
    }

    fn stats(&self) -> IndexStats {
        let mut stats = self.a.stats();
        stats.kind = "dual";
        stats
    }
}
