//! In-memory inverted index with BM25 scoring.

use std::collections::{BTreeMap, BTreeSet};

use calyx_core::{CxId, Result, SlotId, SlotShape, SlotVector, SparseEntry};

use super::bm25::Bm25;
use super::tokenizer::{TEXT_SPARSE_DIM, text_sparse_entries, token_sparse_key, tokenize};
use super::{IndexSearchHit, IndexStats, SextantIndex, ranked};
use crate::util::top_k;

#[derive(Clone, Debug)]
pub struct Posting {
    pub cx_id: CxId,
    pub tf: f32,
}

#[derive(Clone, Debug)]
pub struct InvertedIndex {
    slot: SlotId,
    docs: BTreeMap<CxId, String>,
    vectors: BTreeMap<CxId, SlotVector>,
    postings: BTreeMap<String, Vec<Posting>>,
    doc_len: BTreeMap<CxId, f32>,
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
        let encoded = token_sparse_key(term);
        self.postings
            .get(term)
            .or_else(|| self.postings.get(&encoded))
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
        let removed_doc = self.docs.remove(&cx_id);
        let existed = removed_doc.is_some();
        self.doc_len.remove(&cx_id);
        let vector_existed = if remove_vector {
            self.vectors.remove(&cx_id).is_some()
        } else {
            false
        };
        if let Some(text) = removed_doc {
            for term in text_terms(&text) {
                if let Some(postings) = self.postings.get_mut(&term) {
                    postings.retain(|posting| posting.cx_id != cx_id);
                }
            }
        }
        self.postings.retain(|_, postings| !postings.is_empty());
        existed || vector_existed
    }

    fn index_text(&mut self, cx_id: CxId, text: &str, seq: u64) {
        let terms = text_terms(text);
        self.doc_len.insert(cx_id, terms.len() as f32);
        self.docs.insert(cx_id, text.to_string());
        let mut counts = BTreeMap::<String, usize>::new();
        for term in terms {
            *counts.entry(term).or_default() += 1;
        }
        for (term, tf) in counts {
            self.postings.entry(term).or_default().push(Posting {
                cx_id,
                tf: tf as f32,
            });
        }
        self.update_seq(seq);
    }

    fn index_sparse(&mut self, cx_id: CxId, entries: &[SparseEntry], seq: u64) {
        let doc_len = entries.iter().map(|entry| entry.val).sum();
        self.doc_len.insert(cx_id, doc_len);
        self.docs.insert(cx_id, sparse_text(entries));
        for entry in entries {
            self.postings
                .entry(sparse_term(entry.idx))
                .or_default()
                .push(Posting {
                    cx_id,
                    tf: entry.val,
                });
        }
        self.update_seq(seq);
    }

    fn update_seq(&mut self, seq: u64) {
        self.built_at_seq = self.built_at_seq.max(seq);
        self.base_seq = self.base_seq.max(seq);
    }

    pub fn search_text(&self, text: &str, k: usize) -> Vec<IndexSearchHit> {
        let terms: BTreeSet<_> = text_terms(text).into_iter().collect();
        self.search_terms(terms.into_iter().map(|term| (term, 1.0)), k)
    }

    fn search_sparse(&self, entries: &[SparseEntry], k: usize) -> Result<Vec<IndexSearchHit>> {
        let mut total_doc_len = 0.0_f32;
        for doc_len in self.doc_len.values() {
            total_doc_len += doc_len;
            if !total_doc_len.is_finite() {
                return Err(crate::error::sextant_error(
                    crate::error::CALYX_SEXTANT_VECTOR_SHAPE,
                    "sparse BM25 corpus length overflowed",
                ));
            }
        }
        let hits = self.search_terms(
            entries
                .iter()
                .map(|entry| (sparse_term(entry.idx), entry.val)),
            k,
        );
        if let Some(hit) = hits.iter().find(|hit| !hit.score.is_finite()) {
            return Err(crate::error::sextant_error(
                crate::error::CALYX_SEXTANT_VECTOR_SHAPE,
                format!("sparse BM25 score overflowed for {}", hit.cx_id),
            ));
        }
        Ok(hits)
    }

    fn search_terms(
        &self,
        terms: impl IntoIterator<Item = (String, f32)>,
        k: usize,
    ) -> Vec<IndexSearchHit> {
        let total_docs = self.docs.len();
        let avg_len = if total_docs == 0 {
            0.0
        } else {
            self.doc_len.values().sum::<f32>() / total_docs as f32
        };
        let mut scores = BTreeMap::<CxId, f32>::new();
        for (term, query_weight) in terms {
            let Some(postings) = self.postings.get(&term) else {
                continue;
            };
            let df = postings.len();
            for posting in postings {
                let len = *self.doc_len.get(&posting.cx_id).unwrap_or(&1.0);
                let score = self
                    .scorer
                    .score_term(posting.tf, len, avg_len, total_docs, df)
                    * query_weight;
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
        SlotShape::Sparse(TEXT_SPARSE_DIM)
    }

    fn insert(&mut self, cx_id: CxId, vector: SlotVector, seq: u64) -> Result<()> {
        let SlotVector::Sparse { entries, .. } = &vector else {
            return Err(crate::error::sextant_error(
                crate::error::CALYX_SEXTANT_VECTOR_SHAPE,
                "sparse index received non-sparse vector",
            ));
        };
        vector.validate_schema()?;
        validate_sparse_weights(entries)?;
        self.remove_existing_doc(cx_id, true);
        self.index_sparse(cx_id, entries, seq);
        self.vectors.insert(cx_id, vector);
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
        query.validate_schema()?;
        validate_sparse_weights(entries)?;
        self.search_sparse(entries, k)
    }

    fn rebuild(&mut self) -> Result<()> {
        let docs = self.docs.clone();
        let vectors = self.vectors.clone();
        self.docs.clear();
        self.postings.clear();
        self.doc_len.clear();
        for (cx, text) in docs {
            if let Some(SlotVector::Sparse { entries, .. }) = vectors.get(&cx) {
                validate_sparse_weights(entries)?;
                self.index_sparse(cx, entries, self.base_seq);
            } else {
                self.index_text(cx, &text, self.base_seq);
            }
        }
        self.built_at_seq = self.base_seq;
        Ok(())
    }

    fn vector(&self, cx_id: CxId) -> Option<SlotVector> {
        if let Some(vector) = self.vectors.get(&cx_id) {
            return Some(vector.clone());
        }
        self.docs.get(&cx_id).map(|text| SlotVector::Sparse {
            dim: TEXT_SPARSE_DIM,
            entries: text_sparse_entries(text),
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

fn text_terms(text: &str) -> Vec<String> {
    tokenize(text)
        .into_iter()
        .map(|term| token_sparse_key(&term))
        .collect()
}

fn sparse_term(idx: u32) -> String {
    token_sparse_key(&format!("t{idx}"))
}

fn sparse_text(entries: &[SparseEntry]) -> String {
    entries
        .iter()
        .map(|entry| format!("t{}", entry.idx))
        .collect::<Vec<_>>()
        .join(" ")
}

fn validate_sparse_weights(entries: &[SparseEntry]) -> Result<()> {
    let mut total = 0.0_f32;
    for entry in entries {
        if !entry.val.is_finite() || entry.val <= 0.0 {
            return Err(crate::error::sextant_error(
                crate::error::CALYX_SEXTANT_VECTOR_SHAPE,
                format!(
                    "sparse BM25 weight at index {} must be finite and greater than zero",
                    entry.idx
                ),
            ));
        }
        total += entry.val;
        if !total.is_finite() {
            return Err(crate::error::sextant_error(
                crate::error::CALYX_SEXTANT_VECTOR_SHAPE,
                "sparse BM25 weight sum overflowed",
            ));
        }
    }
    Ok(())
}
