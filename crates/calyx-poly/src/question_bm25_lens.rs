//! Deterministic question/tags sparse lexical lens for Poly (#46).

use std::collections::BTreeMap;

use calyx_core::{AbsentReason, SlotId, SlotShape, SlotVector, SparseEntry};

use crate::lenses::SignalLens;
use crate::model::MarketSnapshot;

pub const QUESTION_BM25_KEY: &str = "question_bm25";
pub const QUESTION_BM25_DIM: u32 = 1_000_000;

pub struct QuestionBm25Lens {
    slot: SlotId,
    key: String,
    dim: u32,
}

impl QuestionBm25Lens {
    pub fn new(slot: u16, key: impl Into<String>) -> Self {
        Self {
            slot: SlotId::new(slot),
            key: key.into(),
            dim: QUESTION_BM25_DIM,
        }
    }
}

impl SignalLens for QuestionBm25Lens {
    fn slot(&self) -> SlotId {
        self.slot
    }

    fn key(&self) -> &str {
        &self.key
    }

    fn shape(&self) -> SlotShape {
        SlotShape::Sparse(self.dim)
    }

    fn measure(&self, snapshot: &MarketSnapshot) -> SlotVector {
        compute_question_bm25_vector(snapshot.question.as_deref(), &snapshot.tags, self.dim)
    }
}

pub fn question_bm25_text(question: Option<&str>, tags: &[String]) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(question) = question.map(str::trim)
        && !question.is_empty()
    {
        parts.push(question.to_string());
    }
    parts.extend(
        tags.iter()
            .map(|tag| tag.trim())
            .filter(|tag| !tag.is_empty())
            .map(ToString::to_string),
    );
    (!parts.is_empty()).then(|| parts.join(" "))
}

pub fn compute_question_bm25_vector(
    question: Option<&str>,
    tags: &[String],
    dim: u32,
) -> SlotVector {
    let Some(text) = question_bm25_text(question, tags) else {
        return SlotVector::Absent {
            reason: AbsentReason::LensUnavailable,
        };
    };
    let mut counts = BTreeMap::<u32, f32>::new();
    for term in calyx_sextant::index::tokenizer::tokenize(&text) {
        let idx = term_index(&term, dim);
        *counts.entry(idx).or_default() += 1.0;
    }
    if counts.is_empty() {
        return SlotVector::Absent {
            reason: AbsentReason::NotApplicable,
        };
    }
    let norm = counts
        .values()
        .map(|tf| (1.0 + tf.ln()).powi(2))
        .sum::<f32>()
        .sqrt()
        .max(f32::EPSILON);
    let entries = counts
        .into_iter()
        .map(|(idx, tf)| SparseEntry {
            idx,
            val: (1.0 + tf.ln()) / norm,
        })
        .collect();
    SlotVector::Sparse { dim, entries }
}

fn term_index(term: &str, dim: u32) -> u32 {
    let hash = blake3::hash(term.as_bytes());
    let bytes = hash.as_bytes();
    let raw = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    raw % dim.max(1)
}
