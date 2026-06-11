//! In-memory inverted index with BM25 scoring.

use std::collections::{BTreeMap, BTreeSet};

use calyx_core::{CxId, Result, SlotId, SlotShape, SlotVector};

use super::bm25::Bm25;
use super::tokenizer::tokenize;
use super::{IndexSearchHit, IndexStats, SextantIndex, ranked};
use crate::util::top_k;

#[derive(Clone, Debug)]
pub struct Posting {
    pub cx_id: CxId,
    pub tf: usize,
}

#[derive(Clone, Debug)]
pub struct InvertedIndex {
    slot: SlotId,
    docs: BTreeMap<CxId, String>,
    vectors: BTreeMap<CxId, SlotVector>,
    postings: BTreeMap<String, Vec<Posting>>,
    doc_len: BTreeMap<CxId, usize>,
    built_at_seq: u64,
    base_seq: u64,
    scorer: Bm25,
}

impl InvertedIndex {
    pub fn new(slot: SlotId) -> Self {
        Self {
            slot,
            docs: BTreeMap::new(),
            vectors: BTreeMap::new(),
            postings: BTreeMap::new(),
            doc_len: BTreeMap::new(),
            built_at_seq: 0,
            base_seq: 0,
            scorer: Bm25::default(),
        }
    }

    pub fn term_count(&self) -> usize {
        self.postings.len()
    }

    pub fn total_docs(&self) -> usize {
        self.docs.len()
    }

    pub fn lookup(&self, term: &str) -> Vec<CxId> {
        self.postings
            .get(term)
            .map(|items| items.iter().map(|item| item.cx_id).collect())
            .unwrap_or_default()
    }

    pub fn remove(&mut self, cx_id: CxId) -> bool {
        self.remove_doc(cx_id, true)
    }

    fn remove_existing_doc(&mut self, cx_id: CxId, remove_vector: bool) {
        if self.docs.contains_key(&cx_id) || (remove_vector && self.vectors.contains_key(&cx_id)) {
            self.remove_doc(cx_id, remove_vector);
        }
    }

    fn remove_doc(&mut self, cx_id: CxId, remove_vector: bool) -> bool {
        let existed = self.docs.remove(&cx_id).is_some();
        self.doc_len.remove(&cx_id);
        let vector_existed = if remove_vector {
            self.vectors.remove(&cx_id).is_some()
        } else {
            false
        };
        for postings in self.postings.values_mut() {
            postings.retain(|posting| posting.cx_id != cx_id);
        }
        self.postings.retain(|_, postings| !postings.is_empty());
        existed || vector_existed
    }

    fn index_text(&mut self, cx_id: CxId, text: &str, seq: u64) {
        let terms = tokenize(text);
        self.doc_len.insert(cx_id, terms.len());
        self.docs.insert(cx_id, text.to_string());
        let mut counts = BTreeMap::<String, usize>::new();
        for term in terms {
            *counts.entry(term).or_default() += 1;
        }
        for (term, tf) in counts {
            self.postings
                .entry(term)
                .or_default()
                .push(Posting { cx_id, tf });
        }
        self.built_at_seq = self.built_at_seq.max(seq);
        self.base_seq = self.base_seq.max(seq);
    }

    pub fn search_text(&self, text: &str, k: usize) -> Vec<IndexSearchHit> {
        let terms: BTreeSet<_> = tokenize(text).into_iter().collect();
        let total_docs = self.docs.len();
        let avg_len = if total_docs == 0 {
            0.0
        } else {
            self.doc_len.values().sum::<usize>() as f32 / total_docs as f32
        };
        let mut scores = BTreeMap::<CxId, f32>::new();
        for term in terms {
            let Some(postings) = self.postings.get(&term) else {
                continue;
            };
            let df = postings.len();
            for posting in postings {
                let len = *self.doc_len.get(&posting.cx_id).unwrap_or(&1);
                let score = self
                    .scorer
                    .score_term(posting.tf, len, avg_len, total_docs, df);
                *scores.entry(posting.cx_id).or_default() += score;
            }
        }
        ranked(top_k(scores.into_iter().collect(), k))
    }
}

impl SextantIndex for InvertedIndex {
    fn slot(&self) -> SlotId {
        self.slot
    }

    fn shape(&self) -> SlotShape {
        SlotShape::Sparse(1_000_000)
    }

    fn insert(&mut self, cx_id: CxId, vector: SlotVector, seq: u64) -> Result<()> {
        let SlotVector::Sparse { entries, .. } = &vector else {
            return Err(crate::error::sextant_error(
                crate::error::CALYX_SEXTANT_VECTOR_SHAPE,
                "sparse index received non-sparse vector",
            ));
        };
        let text = entries
            .iter()
            .map(|entry| format!("t{}", entry.idx))
            .collect::<Vec<_>>()
            .join(" ");
        self.remove_existing_doc(cx_id, true);
        self.vectors.insert(cx_id, vector);
        self.index_text(cx_id, &text, seq);
        Ok(())
    }

    fn search(
        &self,
        query: &SlotVector,
        k: usize,
        _ef: Option<usize>,
    ) -> Result<Vec<IndexSearchHit>> {
        let SlotVector::Sparse { entries, .. } = query else {
            return Err(crate::error::sextant_error(
                crate::error::CALYX_SEXTANT_VECTOR_SHAPE,
                "sparse query required",
            ));
        };
        let text = entries
            .iter()
            .map(|entry| format!("t{}", entry.idx))
            .collect::<Vec<_>>()
            .join(" ");
        Ok(self.search_text(&text, k))
    }

    fn rebuild(&mut self) -> Result<()> {
        let docs = self.docs.clone();
        self.postings.clear();
        self.doc_len.clear();
        for (cx, text) in docs {
            self.index_text(cx, &text, self.base_seq);
        }
        self.built_at_seq = self.base_seq;
        Ok(())
    }

    fn vector(&self, cx_id: CxId) -> Option<SlotVector> {
        if let Some(vector) = self.vectors.get(&cx_id) {
            return Some(vector.clone());
        }
        self.docs.get(&cx_id).map(|text| SlotVector::Sparse {
            dim: 1_000_000,
            entries: tokenize(text)
                .into_iter()
                .enumerate()
                .map(|(idx, _)| calyx_core::SparseEntry {
                    idx: idx as u32,
                    val: 1.0,
                })
                .collect(),
        })
    }

    fn set_base_seq(&mut self, seq: u64) {
        self.base_seq = seq;
    }

    fn stats(&self) -> IndexStats {
        IndexStats {
            slot: self.slot,
            shape: self.shape(),
            len: self.docs.len(),
            built_at_seq: self.built_at_seq,
            base_seq: self.base_seq,
            kind: "inverted",
        }
    }

    fn insert_text(&mut self, cx_id: CxId, text: &str, seq: u64) -> Result<()> {
        self.remove_existing_doc(cx_id, true);
        self.index_text(cx_id, text, seq);
        Ok(())
    }

    fn search_text(&self, text: &str, k: usize) -> Result<Vec<IndexSearchHit>> {
        Ok(InvertedIndex::search_text(self, text, k))
    }

    fn candidate_text(&self, cx_id: CxId) -> Option<String> {
        self.docs.get(&cx_id).cloned()
    }
}
