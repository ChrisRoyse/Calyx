//! Concurrent-read-safe SlotId to index registry.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, RwLock};

use calyx_core::{CxId, Result, SlotId, SlotVector};

use crate::error::{CALYX_SEXTANT_SLOT_MISSING, sextant_error};
use crate::index::{IndexSearchHit, IndexStats, SextantIndex};

type SharedIndex = Arc<RwLock<Box<dyn SextantIndex>>>;

#[derive(Clone, Default)]
pub struct SlotIndexMap {
    indexes: Arc<RwLock<BTreeMap<SlotId, SharedIndex>>>,
}

impl SlotIndexMap {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register<I>(&self, index: I)
    where
        I: SextantIndex + 'static,
    {
        self.indexes
            .write()
            .expect("slot map poisoned")
            .insert(index.slot(), Arc::new(RwLock::new(Box::new(index))));
    }

    pub fn slots(&self) -> Vec<SlotId> {
        self.indexes
            .read()
            .expect("slot map poisoned")
            .keys()
            .copied()
            .collect()
    }

    pub fn stats(&self) -> Vec<IndexStats> {
        self.indexes
            .read()
            .expect("slot map poisoned")
            .values()
            .map(|index| index.read().expect("index poisoned").stats())
            .collect()
    }

    pub fn insert(&self, slot: SlotId, cx_id: CxId, vector: SlotVector, seq: u64) -> Result<()> {
        let index = self.get(slot)?;
        index
            .write()
            .expect("index poisoned")
            .insert(cx_id, vector, seq)
    }

    pub fn insert_text(&self, slot: SlotId, cx_id: CxId, text: &str, seq: u64) -> Result<()> {
        let index = self.get(slot)?;
        index
            .write()
            .expect("index poisoned")
            .insert_text(cx_id, text, seq)
    }

    pub fn search(
        &self,
        slot: SlotId,
        query: &SlotVector,
        k: usize,
        ef: Option<usize>,
    ) -> Result<Vec<IndexSearchHit>> {
        let index = self.get(slot)?;
        index.read().expect("index poisoned").search(query, k, ef)
    }

    pub fn search_text(&self, slot: SlotId, text: &str, k: usize) -> Result<Vec<IndexSearchHit>> {
        let index = self.get(slot)?;
        index.read().expect("index poisoned").search_text(text, k)
    }

    pub fn vector(&self, slot: SlotId, cx_id: CxId) -> Result<Option<SlotVector>> {
        let index = self.get(slot)?;
        Ok(index.read().expect("index poisoned").vector(cx_id))
    }

    pub fn set_base_seq(&self, slot: SlotId, seq: u64) -> Result<()> {
        let index = self.get(slot)?;
        index.write().expect("index poisoned").set_base_seq(seq);
        Ok(())
    }

    pub fn rebuild(&self, slot: SlotId) -> Result<()> {
        let index = self.get(slot)?;
        index.write().expect("index poisoned").rebuild()
    }

    pub fn missing_slot_error(slot: SlotId) -> calyx_core::CalyxError {
        sextant_error(
            CALYX_SEXTANT_SLOT_MISSING,
            format!("slot {slot} is not registered"),
        )
    }

    pub fn assert_isolated(&self, a: SlotId, b: SlotId, query: &SlotVector) -> Result<bool> {
        let left: BTreeSet<_> = self
            .search(a, query, 5, None)?
            .into_iter()
            .map(|h| h.cx_id)
            .collect();
        let right: BTreeSet<_> = self
            .search(b, query, 5, None)?
            .into_iter()
            .map(|h| h.cx_id)
            .collect();
        Ok(left != right)
    }

    fn get(&self, slot: SlotId) -> Result<SharedIndex> {
        self.indexes
            .read()
            .expect("slot map poisoned")
            .get(&slot)
            .cloned()
            .ok_or_else(|| Self::missing_slot_error(slot))
    }
}
