use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use calyx_core::{Constellation, CxId, SlotId, SlotVector, SparseEntry};
use calyx_sextant::index::bm25::Bm25;
use calyx_sextant::index::{IndexSearchHit, ranked};
use serde::{Deserialize, Serialize};

use super::{SearchIndexEntry, rel, sha256_hex, stale, write_bytes_atomic};
use crate::error::CliResult;

const SPARSE_FORMAT: &str = "calyx-search-sparse-index-v1";

#[derive(Clone, Debug)]
pub(super) struct SparseSlotRows {
    dim: u32,
    rows: Vec<(CxId, Vec<SparseEntry>)>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct SparseIndex {
    format: String,
    slot: u16,
    dim: u32,
    base_seq: u64,
    rows: Vec<SparseRow>,
    postings: BTreeMap<u32, Vec<SparsePosting>>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct SparseRow {
    cx_id: CxId,
    doc_len: usize,
    entries: Vec<SparseEntry>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct SparsePosting {
    cx_id: CxId,
    tf: usize,
}

impl SparseSlotRows {
    pub(super) fn len(&self) -> usize {
        self.rows.len()
    }
}

pub(super) fn collect(
    docs: &BTreeMap<CxId, Constellation>,
) -> CliResult<BTreeMap<SlotId, SparseSlotRows>> {
    let mut out = BTreeMap::<SlotId, SparseSlotRows>::new();
    for cx in docs.values() {
        for (slot, vector) in &cx.slots {
            let SlotVector::Sparse { dim, entries } = vector else {
                continue;
            };
            vector.validate_schema().map_err(|err| {
                stale(format!(
                    "slot {slot} cx {} has invalid sparse payload: {}",
                    cx.cx_id, err.message
                ))
            })?;
            let entry = out.entry(*slot).or_insert_with(|| SparseSlotRows {
                dim: *dim,
                rows: Vec::new(),
            });
            if entry.dim != *dim {
                return Err(stale(format!(
                    "slot {slot} has mixed sparse dims: {} and {dim}",
                    entry.dim
                )));
            }
            entry.rows.push((cx.cx_id, entries.clone()));
        }
    }
    Ok(out)
}

pub(super) fn write(
    vault_dir: &Path,
    root: &Path,
    slot: SlotId,
    rows: SparseSlotRows,
    base_seq: u64,
) -> CliResult<SearchIndexEntry> {
    let path = root.join(format!(
        "slot_{:05}_seq_{base_seq:020}_n_{:010}.sparse.json",
        slot.get(),
        rows.rows.len()
    ));
    let index = build_index(slot, rows.dim, rows.rows, base_seq);
    let bytes = serde_json::to_vec_pretty(&index)?;
    let sha256 = sha256_hex(&bytes);
    write_bytes_atomic(&path, &bytes)?;
    Ok(SearchIndexEntry::sparse(
        slot,
        index.dim,
        index.rows.len(),
        base_seq,
        rel(vault_dir, &path)?,
        sha256,
    ))
}

pub(super) fn search(
    vault_dir: &Path,
    entry: &SearchIndexEntry,
    manifest_base_seq: u64,
    slot: SlotId,
    query: &SlotVector,
    k: usize,
    candidates: Option<&BTreeSet<CxId>>,
) -> CliResult<Vec<IndexSearchHit>> {
    if k == 0 {
        return Ok(Vec::new());
    }
    let SlotVector::Sparse {
        dim: query_dim,
        entries,
    } = query
    else {
        return Err(stale(format!(
            "persistent sparse search slot {slot} received non-sparse query"
        )));
    };
    query.validate_schema().map_err(|err| {
        stale(format!(
            "persistent sparse search slot {slot} received invalid query: {}",
            err.message
        ))
    })?;
    let index = read(vault_dir, entry, manifest_base_seq, slot)?;
    if index.dim != *query_dim {
        return Err(stale(format!(
            "persistent sparse slot {slot} index dim {} != query dim {query_dim}; reingest/backfill the vault",
            index.dim
        )));
    }
    if entries.is_empty() {
        return Ok(Vec::new());
    }
    Ok(ranked(top_k(score(&index, entries, candidates), k)))
}

fn build_index(
    slot: SlotId,
    dim: u32,
    source_rows: Vec<(CxId, Vec<SparseEntry>)>,
    base_seq: u64,
) -> SparseIndex {
    let rows = source_rows
        .into_iter()
        .map(|(cx_id, entries)| SparseRow {
            cx_id,
            doc_len: entries.len(),
            entries,
        })
        .collect::<Vec<_>>();
    let postings = postings_from_rows(&rows);
    SparseIndex {
        format: SPARSE_FORMAT.to_string(),
        slot: slot.get(),
        dim,
        base_seq,
        rows,
        postings,
    }
}

fn read(
    vault_dir: &Path,
    entry: &SearchIndexEntry,
    manifest_base_seq: u64,
    slot: SlotId,
) -> CliResult<SparseIndex> {
    entry.require_kind("sparse_inverted", slot)?;
    let path = vault_dir.join(entry.require_index_rel(slot)?);
    if !path.is_file() {
        return Err(stale(format!(
            "persistent sparse sidecar missing at {}; rebuild the vault search indexes",
            path.display()
        )));
    }
    let bytes = fs::read(&path)?;
    let actual = sha256_hex(&bytes);
    let expected = entry.require_sha256(slot)?;
    if actual != expected {
        return Err(stale(format!(
            "persistent sparse sidecar sha256 {actual} != manifest {expected}; rebuild the vault search indexes"
        )));
    }
    let index: SparseIndex = serde_json::from_slice(&bytes).map_err(|err| {
        stale(format!(
            "persistent sparse sidecar {} is not valid JSON: {err}; rebuild the vault search indexes",
            path.display()
        ))
    })?;
    validate(&index, entry, manifest_base_seq, slot)?;
    Ok(index)
}

fn validate(
    index: &SparseIndex,
    entry: &SearchIndexEntry,
    manifest_base_seq: u64,
    slot: SlotId,
) -> CliResult {
    if index.format != SPARSE_FORMAT {
        return Err(stale(format!(
            "persistent sparse sidecar has format {}; expected {SPARSE_FORMAT}",
            index.format
        )));
    }
    if index.slot != slot.get() || entry.slot != slot.get() {
        return Err(stale(format!(
            "persistent sparse sidecar slot {} / entry slot {} != query slot {}",
            index.slot,
            entry.slot,
            slot.get()
        )));
    }
    let entry_dim = entry.require_dim(slot)?;
    if index.dim != entry_dim {
        return Err(stale(format!(
            "persistent sparse sidecar dim {} != manifest dim {entry_dim}; rebuild the vault search indexes",
            index.dim
        )));
    }
    if index.base_seq != manifest_base_seq || entry.built_at_seq != manifest_base_seq {
        return Err(stale(format!(
            "persistent sparse sidecar seq {} / entry seq {} != manifest seq {manifest_base_seq}; rebuild the vault search indexes",
            index.base_seq, entry.built_at_seq
        )));
    }
    if index.rows.len() != entry.len {
        return Err(stale(format!(
            "persistent sparse sidecar row len {} != manifest len {}; rebuild the vault search indexes",
            index.rows.len(),
            entry.len
        )));
    }
    let mut seen = BTreeSet::new();
    for row in &index.rows {
        if !seen.insert(row.cx_id) {
            return Err(stale(format!(
                "persistent sparse sidecar repeats {}; rebuild the vault search indexes",
                row.cx_id
            )));
        }
        if row.doc_len != row.entries.len() {
            return Err(stale(format!(
                "persistent sparse row {} doc_len {} != entries {}; rebuild the vault search indexes",
                row.cx_id,
                row.doc_len,
                row.entries.len()
            )));
        }
        SlotVector::Sparse {
            dim: index.dim,
            entries: row.entries.clone(),
        }
        .validate_schema()
        .map_err(|err| {
            stale(format!(
                "persistent sparse row {} has invalid payload: {}; rebuild the vault search indexes",
                row.cx_id, err.message
            ))
        })?;
    }
    let expected = postings_from_rows(&index.rows);
    if expected != index.postings {
        return Err(stale(
            "persistent sparse postings do not match row payloads; rebuild the vault search indexes",
        ));
    }
    Ok(())
}

fn postings_from_rows(rows: &[SparseRow]) -> BTreeMap<u32, Vec<SparsePosting>> {
    let mut out = BTreeMap::<u32, Vec<SparsePosting>>::new();
    for row in rows {
        let mut counts = BTreeMap::<u32, usize>::new();
        for entry in &row.entries {
            *counts.entry(entry.idx).or_default() += 1;
        }
        for (idx, tf) in counts {
            out.entry(idx).or_default().push(SparsePosting {
                cx_id: row.cx_id,
                tf,
            });
        }
    }
    out
}

fn score(
    index: &SparseIndex,
    query: &[SparseEntry],
    candidates: Option<&BTreeSet<CxId>>,
) -> Vec<(CxId, f32)> {
    let query_terms = query.iter().map(|entry| entry.idx).collect::<BTreeSet<_>>();
    let doc_len = index
        .rows
        .iter()
        .map(|row| (row.cx_id, row.doc_len))
        .collect::<BTreeMap<_, _>>();
    let total_docs = index.rows.len();
    let avg_len = if total_docs == 0 {
        0.0
    } else {
        doc_len.values().sum::<usize>() as f32 / total_docs as f32
    };
    let scorer = Bm25::default();
    let mut scores = BTreeMap::<CxId, f32>::new();
    for term in query_terms {
        let Some(postings) = index.postings.get(&term) else {
            continue;
        };
        let df = postings.len();
        for posting in postings {
            if candidates.is_some_and(|allowed| !allowed.contains(&posting.cx_id)) {
                continue;
            }
            let len = *doc_len.get(&posting.cx_id).unwrap_or(&1);
            let score = scorer.score_term(posting.tf, len, avg_len, total_docs, df);
            *scores.entry(posting.cx_id).or_default() += score;
        }
    }
    scores.into_iter().collect()
}

fn top_k(mut scored: Vec<(CxId, f32)>, k: usize) -> Vec<(CxId, f32)> {
    scored.sort_by(|left, right| {
        right
            .1
            .total_cmp(&left.1)
            .then_with(|| left.0.to_string().cmp(&right.0.to_string()))
    });
    scored.truncate(k);
    scored
}
