//! Inverted secondary-index key encoding and BM25-style term queries (PH54 T03).
//!
//! Posting key schema:
//! ```text
//! 0x11 | collection_id (8B BE) | index_id (4B BE) | term_hash (8B BE) | pk_bytes
//! ```
//! Posting value is the stored BM25 term-frequency component as `f32` BE. IDF is
//! applied at query time. The reserved key with `term_hash = u64::MAX` and no
//! primary key stores `doc_count (u64 BE) || avgdl (f32 BE)`.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

use calyx_core::{CalyxError, Clock, Result, Seq};

use super::{
    FieldValue, IndexKind, IndexSpec, SecondaryIndex, field_value_type, invalid_index_input,
};
use crate::cf::{ColumnFamily, prefix_range};
use crate::collection::{Collection, FieldType};
use crate::layers::relational::{collection_id, record_key};
use crate::layers::{RecordKey, RecordValue};
use crate::vault::AsterVault;

use super::terms::{term_frequencies, term_hash, tokenize};

/// Key-space discriminant for inverted secondary-index keys.
pub const DISC_INVERTED_INDEX: u8 = 0x11;

/// On-disk name of the Aster CF that stores inverted secondary-index entries.
pub const CF_INDEX_INVERTED: &str = "index_inverted";

const PREFIX_BYTES: usize = 1 + 8 + 4;
const TERM_HASH_BYTES: usize = 8;
const POSTING_VALUE_BYTES: usize = 4;
const STATS_VALUE_BYTES: usize = 12;
const STATS_TERM_HASH: u64 = u64::MAX;
const BM25_K1: f32 = 1.2;
const BM25_B: f32 = 0.75;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InvertedQueryMode {
    Or,
    And,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InvertedStats {
    pub doc_count: u64,
    pub avgdl: f32,
}

impl Default for InvertedStats {
    fn default() -> Self {
        Self {
            doc_count: 0,
            avgdl: 0.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvertedIndex {
    collection_id: u64,
    spec: IndexSpec,
}

impl InvertedIndex {
    pub fn new(collection_id: u64, spec: IndexSpec) -> Self {
        Self {
            collection_id,
            spec,
        }
    }

    pub fn spec(&self) -> &IndexSpec {
        &self.spec
    }

    pub fn index_key_prefix(&self) -> Vec<u8> {
        self.prefix()
    }

    fn prefix(&self) -> Vec<u8> {
        let mut prefix = Vec::with_capacity(PREFIX_BYTES);
        prefix.push(DISC_INVERTED_INDEX);
        prefix.extend_from_slice(&self.collection_id.to_be_bytes());
        prefix.extend_from_slice(&self.spec.index_id.to_be_bytes());
        prefix
    }

    fn term_prefix(&self, term: &str) -> Vec<u8> {
        let mut prefix = self.prefix();
        prefix.extend_from_slice(&term_hash(term).to_be_bytes());
        prefix
    }

    fn stats_key(&self) -> Vec<u8> {
        let mut key = self.prefix();
        key.extend_from_slice(&STATS_TERM_HASH.to_be_bytes());
        key
    }

    fn posting_key(&self, term: &str, pk: &RecordKey) -> Vec<u8> {
        let mut key = self.term_prefix(term);
        key.extend_from_slice(pk.as_bytes());
        key
    }

    pub(crate) fn decode_posting_key(&self, key: &[u8]) -> Result<(u64, RecordKey)> {
        let prefix = self.prefix();
        if key.len() <= prefix.len() + TERM_HASH_BYTES || key[..prefix.len()] != prefix[..] {
            return Err(corrupt(
                "inverted index key prefix does not match this index",
            ));
        }
        let term_hash_bytes = key
            .get(prefix.len()..prefix.len() + TERM_HASH_BYTES)
            .expect("length checked");
        let hash = u64::from_be_bytes(term_hash_bytes.try_into().expect("8-byte slice"));
        let pk_bytes = &key[prefix.len() + TERM_HASH_BYTES..];
        let pk = RecordKey::from_bytes(pk_bytes.to_vec())
            .map_err(|error| corrupt(format!("inverted index key primary key: {error}")))?;
        Ok((hash, pk))
    }

    fn encode_entries(
        &self,
        field_val: &FieldValue,
        pk: &RecordKey,
        stats: InvertedStats,
    ) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        let text = text_value(field_val)?;
        let (counts, doc_len) = term_frequencies(text);
        if counts.is_empty() {
            return Ok(Vec::new());
        }
        let avgdl = updated_stats(stats, doc_len).avgdl;
        let mut rows = Vec::with_capacity(counts.len());
        for (term, tf) in counts {
            let weight = bm25_tf_weight(tf, doc_len, avgdl);
            rows.push((self.posting_key(&term, pk), encode_weight(weight)));
        }
        Ok(rows)
    }
}

impl SecondaryIndex for InvertedIndex {
    fn kind(&self) -> IndexKind {
        self.spec.kind
    }

    fn encode_index_key(&self, field_val: &FieldValue, pk: &RecordKey) -> Result<Vec<u8>> {
        let text = text_value(field_val)?;
        let terms = tokenize(text).into_iter().collect::<BTreeSet<_>>();
        if terms.len() != 1 {
            return Err(invalid_index_input(
                "inverted encode_index_key requires exactly one normalized term",
            ));
        }
        Ok(self.posting_key(terms.iter().next().expect("one term"), pk))
    }

    fn encode_scan_prefix(&self, field_val: &FieldValue) -> Result<Vec<u8>> {
        let text = text_value(field_val)?;
        let term = single_query_term(text)?;
        Ok(term.map_or_else(|| self.index_key_prefix(), |term| self.term_prefix(&term)))
    }
}

pub fn inverted_update_avgdl<C: Clock>(
    vault: &AsterVault<C>,
    col: &Collection,
    spec: &IndexSpec,
    new_dl: u32,
) -> Result<Seq> {
    if new_dl == 0 {
        return Ok(vault.latest_seq());
    }
    let idx = index_for(col, spec)?;
    let stats = read_stats(vault, vault.latest_seq(), &idx)?;
    let updated = updated_stats(stats, new_dl);
    vault.write_cf(
        ColumnFamily::IndexInverted,
        idx.stats_key(),
        encode_stats(updated),
    )
}

pub fn inverted_put<C: Clock>(
    vault: &AsterVault<C>,
    col: &Collection,
    spec: &IndexSpec,
    field_val: &FieldValue,
    pk: &RecordKey,
) -> Result<Seq> {
    let idx = index_for(col, spec)?;
    let stats = read_stats(vault, vault.latest_seq(), &idx)?;
    let rows = idx.encode_put_entries(field_val, pk, stats)?;
    if rows.is_empty() {
        return Ok(vault.latest_seq());
    }
    vault.write_cf_batch(
        rows.into_iter()
            .map(|(key, value)| (ColumnFamily::IndexInverted, key, value)),
    )
}

pub fn inverted_match<C: Clock>(
    vault: &AsterVault<C>,
    col: &Collection,
    spec: &IndexSpec,
    term: &str,
) -> Result<Vec<(RecordKey, f32)>> {
    inverted_match_at(vault, vault.latest_seq(), col, spec, term)
}

pub fn inverted_match_at<C: Clock>(
    vault: &AsterVault<C>,
    snapshot: Seq,
    col: &Collection,
    spec: &IndexSpec,
    term: &str,
) -> Result<Vec<(RecordKey, f32)>> {
    let idx = index_for(col, spec)?;
    let Some(term) = single_query_term(term)? else {
        return Ok(Vec::new());
    };
    let range = prefix_range(&idx.term_prefix(&term));
    let mut out = Vec::new();
    for (key, value) in vault.scan_cf_range_at(snapshot, ColumnFamily::IndexInverted, &range)? {
        let (_hash, pk) = idx.decode_posting_key(&key)?;
        let data_key = record_key(col, &pk)?;
        if vault
            .read_cf_at(snapshot, ColumnFamily::Relational, &data_key)?
            .is_some()
        {
            out.push((pk, decode_weight(&value)?));
        }
    }
    out.sort_by(descending_weight);
    Ok(out)
}

pub fn inverted_bm25<C: Clock>(
    vault: &AsterVault<C>,
    col: &Collection,
    spec: &IndexSpec,
    terms: &[&str],
    n_docs: u64,
    limit: usize,
) -> Result<Vec<(RecordKey, f32)>> {
    inverted_bm25_with_mode(
        vault,
        col,
        spec,
        terms,
        n_docs,
        limit,
        InvertedQueryMode::Or,
    )
}

pub fn inverted_bm25_and<C: Clock>(
    vault: &AsterVault<C>,
    col: &Collection,
    spec: &IndexSpec,
    terms: &[&str],
    n_docs: u64,
    limit: usize,
) -> Result<Vec<(RecordKey, f32)>> {
    inverted_bm25_with_mode(
        vault,
        col,
        spec,
        terms,
        n_docs,
        limit,
        InvertedQueryMode::And,
    )
}

pub fn inverted_bm25_with_mode<C: Clock>(
    vault: &AsterVault<C>,
    col: &Collection,
    spec: &IndexSpec,
    terms: &[&str],
    n_docs: u64,
    limit: usize,
    mode: InvertedQueryMode,
) -> Result<Vec<(RecordKey, f32)>> {
    let query_terms = normalized_query_terms(terms);
    if query_terms.is_empty() || n_docs == 0 {
        return Ok(Vec::new());
    }
    let mut scores = BTreeMap::<RecordKey, f32>::new();
    let mut hits = BTreeMap::<RecordKey, usize>::new();
    for term in &query_terms {
        let postings = inverted_match(vault, col, spec, term)?;
        if postings.is_empty() {
            continue;
        }
        let idf = bm25_idf(n_docs, postings.len() as u64);
        for (pk, weight) in postings {
            *scores.entry(pk.clone()).or_default() += weight * idf;
            *hits.entry(pk).or_default() += 1;
        }
    }
    let required = query_terms.len();
    let mut ranked = scores
        .into_iter()
        .filter(|(pk, _score)| {
            mode == InvertedQueryMode::Or || hits.get(pk).copied().unwrap_or(0) == required
        })
        .collect::<Vec<_>>();
    ranked.sort_by(descending_weight);
    if limit != 0 {
        ranked.truncate(limit);
    }
    Ok(ranked)
}

pub fn inverted_stats<C: Clock>(
    vault: &AsterVault<C>,
    col: &Collection,
    spec: &IndexSpec,
) -> Result<InvertedStats> {
    let idx = index_for(col, spec)?;
    read_stats(vault, vault.latest_seq(), &idx)
}

fn index_for(col: &Collection, spec: &IndexSpec) -> Result<InvertedIndex> {
    spec.validate()?;
    if spec.kind != IndexKind::Inverted {
        return Err(invalid_index_input("inverted index requires kind Inverted"));
    }
    if spec.field_type != FieldType::Text {
        return Err(invalid_index_input(
            "inverted index requires a Text field type",
        ));
    }
    Ok(InvertedIndex::new(collection_id(col), spec.clone()))
}

fn text_value(field_val: &FieldValue) -> Result<&str> {
    let actual = field_value_type(field_val)
        .ok_or_else(|| invalid_index_input("cannot index a NULL value in an inverted key"))?;
    if actual != FieldType::Text {
        return Err(invalid_index_input(format!(
            "field value type {actual:?} does not match inverted Text field type"
        )));
    }
    match field_val {
        RecordValue::Text(value) => Ok(value),
        _ => Err(invalid_index_input("inverted index requires Text values")),
    }
}

fn single_query_term(input: &str) -> Result<Option<String>> {
    let terms = tokenize(input);
    match terms.len() {
        0 => Ok(None),
        1 => Ok(terms.into_iter().next()),
        _ => Err(invalid_index_input(
            "term-match accepts exactly one normalized term",
        )),
    }
}

fn normalized_query_terms(terms: &[&str]) -> Vec<String> {
    terms
        .iter()
        .flat_map(|term| tokenize(term))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn updated_stats(stats: InvertedStats, new_dl: u32) -> InvertedStats {
    let doc_count = stats.doc_count + 1;
    let total = stats.avgdl * stats.doc_count as f32 + new_dl as f32;
    InvertedStats {
        doc_count,
        avgdl: total / doc_count as f32,
    }
}

fn bm25_tf_weight(tf: u32, doc_len: u32, avgdl: f32) -> f32 {
    if tf == 0 || doc_len == 0 {
        return 0.0;
    }
    let len_norm = if avgdl <= 0.0 {
        1.0
    } else {
        doc_len as f32 / avgdl
    };
    let tf = tf as f32;
    tf / (tf + BM25_K1 * (1.0 - BM25_B + BM25_B * len_norm))
}

fn bm25_idf(total_docs: u64, doc_freq: u64) -> f32 {
    if total_docs == 0 || doc_freq == 0 {
        return 0.0;
    }
    (((total_docs as f32 - doc_freq as f32 + 0.5) / (doc_freq as f32 + 0.5)) + 1.0).ln()
}

fn read_stats<C: Clock>(
    vault: &AsterVault<C>,
    snapshot: Seq,
    idx: &InvertedIndex,
) -> Result<InvertedStats> {
    vault
        .read_cf_at(snapshot, ColumnFamily::IndexInverted, &idx.stats_key())?
        .map(|bytes| decode_stats(&bytes))
        .transpose()
        .map(|stats| stats.unwrap_or_default())
}

fn encode_stats(stats: InvertedStats) -> Vec<u8> {
    let mut out = Vec::with_capacity(STATS_VALUE_BYTES);
    out.extend_from_slice(&stats.doc_count.to_be_bytes());
    out.extend_from_slice(&stats.avgdl.to_be_bytes());
    out
}

fn decode_stats(bytes: &[u8]) -> Result<InvertedStats> {
    if bytes.len() != STATS_VALUE_BYTES {
        return Err(corrupt(format!(
            "inverted stats value has {} bytes, expected {STATS_VALUE_BYTES}",
            bytes.len()
        )));
    }
    let doc_count = u64::from_be_bytes(bytes[..8].try_into().expect("8-byte slice"));
    let avgdl = f32::from_be_bytes(bytes[8..12].try_into().expect("4-byte slice"));
    if !avgdl.is_finite() || avgdl < 0.0 {
        return Err(corrupt(
            "inverted stats avgdl is not finite and non-negative",
        ));
    }
    Ok(InvertedStats { doc_count, avgdl })
}

fn encode_weight(weight: f32) -> Vec<u8> {
    weight.to_be_bytes().to_vec()
}

fn decode_weight(bytes: &[u8]) -> Result<f32> {
    if bytes.len() != POSTING_VALUE_BYTES {
        return Err(corrupt(format!(
            "inverted posting value has {} bytes, expected {POSTING_VALUE_BYTES}",
            bytes.len()
        )));
    }
    let weight = f32::from_be_bytes(bytes.try_into().expect("4-byte slice"));
    if !weight.is_finite() || weight < 0.0 {
        return Err(corrupt(
            "inverted posting weight is not finite and non-negative",
        ));
    }
    Ok(weight)
}

fn descending_weight(a: &(RecordKey, f32), b: &(RecordKey, f32)) -> Ordering {
    b.1.partial_cmp(&a.1)
        .unwrap_or(Ordering::Equal)
        .then_with(|| a.0.cmp(&b.0))
}

fn corrupt(message: impl Into<String>) -> CalyxError {
    CalyxError::aster_corrupt_shard(message)
}

#[path = "inverted_maintenance.rs"]
mod maintenance;

#[cfg(test)]
#[path = "inverted_tests.rs"]
mod tests;
