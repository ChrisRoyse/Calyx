use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use calyx_core::{Constellation, CxId, SlotId, SlotVector};
use calyx_sextant::index::{IndexSearchHit, MaxSimIndex, ranked};
use serde::{Deserialize, Serialize};

use super::{SearchIndexEntry, rel, sha256_hex, stale, write_bytes_atomic};
use crate::error::CliResult;

const MULTI_FORMAT: &str = "calyx-search-multi-maxsim-index-v1";

#[derive(Clone, Debug)]
pub(super) struct MultiSlotRows {
    token_dim: u32,
    rows: Vec<(CxId, Vec<Vec<f32>>)>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct MultiIndex {
    format: String,
    slot: u16,
    token_dim: u32,
    base_seq: u64,
    token_count: usize,
    rows: Vec<MultiRow>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct MultiRow {
    cx_id: CxId,
    tokens: Vec<Vec<f32>>,
}

impl MultiSlotRows {
    pub(super) fn len(&self) -> usize {
        self.rows.len()
    }
}

pub(super) fn collect(
    docs: &BTreeMap<CxId, Constellation>,
) -> CliResult<BTreeMap<SlotId, MultiSlotRows>> {
    let mut out = BTreeMap::<SlotId, MultiSlotRows>::new();
    for cx in docs.values() {
        for (slot, vector) in &cx.slots {
            let SlotVector::Multi { token_dim, tokens } = vector else {
                continue;
            };
            vector.validate_schema().map_err(|err| {
                stale(format!(
                    "slot {slot} cx {} has invalid multi-vector payload: {}",
                    cx.cx_id, err.message
                ))
            })?;
            let entry = out.entry(*slot).or_insert_with(|| MultiSlotRows {
                token_dim: *token_dim,
                rows: Vec::new(),
            });
            if entry.token_dim != *token_dim {
                return Err(stale(format!(
                    "slot {slot} has mixed multi token dims: {} and {token_dim}",
                    entry.token_dim
                )));
            }
            entry.rows.push((cx.cx_id, tokens.clone()));
        }
    }
    Ok(out)
}

pub(super) fn write(
    vault_dir: &Path,
    root: &Path,
    slot: SlotId,
    rows: MultiSlotRows,
    base_seq: u64,
) -> CliResult<SearchIndexEntry> {
    let path = root.join(format!(
        "slot_{:05}_seq_{base_seq:020}_n_{:010}.multi.json",
        slot.get(),
        rows.rows.len()
    ));
    let index = build_index(slot, rows.token_dim, rows.rows, base_seq);
    let bytes = serde_json::to_vec_pretty(&index)?;
    let sha256 = sha256_hex(&bytes);
    write_bytes_atomic(&path, &bytes)?;
    Ok(SearchIndexEntry::multi(
        slot,
        index.token_dim,
        index.rows.len(),
        index.token_count,
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
    let SlotVector::Multi {
        token_dim,
        tokens: query_tokens,
    } = query
    else {
        return Err(stale(format!(
            "persistent multi search slot {slot} received non-multi query"
        )));
    };
    query.validate_schema().map_err(|err| {
        stale(format!(
            "persistent multi search slot {slot} received invalid query: {}",
            err.message
        ))
    })?;
    let index = read(vault_dir, entry, manifest_base_seq, slot)?;
    if index.token_dim != *token_dim {
        return Err(stale(format!(
            "persistent multi slot {slot} token_dim {} != query token_dim {token_dim}; reingest/backfill the vault",
            index.token_dim
        )));
    }
    Ok(ranked(top_k(score(&index, query_tokens, candidates), k)))
}

fn build_index(
    slot: SlotId,
    token_dim: u32,
    source_rows: Vec<(CxId, Vec<Vec<f32>>)>,
    base_seq: u64,
) -> MultiIndex {
    let rows = source_rows
        .into_iter()
        .map(|(cx_id, tokens)| MultiRow { cx_id, tokens })
        .collect::<Vec<_>>();
    let token_count = rows.iter().map(|row| row.tokens.len()).sum();
    MultiIndex {
        format: MULTI_FORMAT.to_string(),
        slot: slot.get(),
        token_dim,
        base_seq,
        token_count,
        rows,
    }
}

fn read(
    vault_dir: &Path,
    entry: &SearchIndexEntry,
    manifest_base_seq: u64,
    slot: SlotId,
) -> CliResult<MultiIndex> {
    entry.require_kind("multi_maxsim", slot)?;
    let path = vault_dir.join(entry.require_index_rel(slot)?);
    if !path.is_file() {
        return Err(stale(format!(
            "persistent multi sidecar missing at {}; rebuild the vault search indexes",
            path.display()
        )));
    }
    let bytes = fs::read(&path)?;
    let actual = sha256_hex(&bytes);
    let expected = entry.require_sha256(slot)?;
    if actual != expected {
        return Err(stale(format!(
            "persistent multi sidecar sha256 {actual} != manifest {expected}; rebuild the vault search indexes"
        )));
    }
    let index: MultiIndex = serde_json::from_slice(&bytes).map_err(|err| {
        stale(format!(
            "persistent multi sidecar {} is not valid JSON: {err}; rebuild the vault search indexes",
            path.display()
        ))
    })?;
    validate(&index, entry, manifest_base_seq, slot)?;
    Ok(index)
}

fn validate(
    index: &MultiIndex,
    entry: &SearchIndexEntry,
    manifest_base_seq: u64,
    slot: SlotId,
) -> CliResult {
    if index.format != MULTI_FORMAT {
        return Err(stale(format!(
            "persistent multi sidecar has format {}; expected {MULTI_FORMAT}",
            index.format
        )));
    }
    if index.slot != slot.get() || entry.slot != slot.get() {
        return Err(stale(format!(
            "persistent multi sidecar slot {} / entry slot {} != query slot {}",
            index.slot,
            entry.slot,
            slot.get()
        )));
    }
    let entry_token_dim = entry.require_token_dim(slot)?;
    if index.token_dim != entry_token_dim {
        return Err(stale(format!(
            "persistent multi sidecar token_dim {} != manifest token_dim {entry_token_dim}; rebuild the vault search indexes",
            index.token_dim
        )));
    }
    if index.base_seq != manifest_base_seq || entry.built_at_seq != manifest_base_seq {
        return Err(stale(format!(
            "persistent multi sidecar seq {} / entry seq {} != manifest seq {manifest_base_seq}; rebuild the vault search indexes",
            index.base_seq, entry.built_at_seq
        )));
    }
    if index.rows.len() != entry.len {
        return Err(stale(format!(
            "persistent multi sidecar row len {} != manifest len {}; rebuild the vault search indexes",
            index.rows.len(),
            entry.len
        )));
    }
    if entry
        .token_count
        .is_some_and(|count| count != index.token_count)
    {
        return Err(stale(format!(
            "persistent multi sidecar token_count {} != manifest token_count {}; rebuild the vault search indexes",
            index.token_count,
            entry.token_count.unwrap_or_default()
        )));
    }
    let mut seen = BTreeSet::new();
    let mut token_count = 0usize;
    for row in &index.rows {
        if !seen.insert(row.cx_id) {
            return Err(stale(format!(
                "persistent multi sidecar repeats {}; rebuild the vault search indexes",
                row.cx_id
            )));
        }
        token_count += row.tokens.len();
        SlotVector::Multi {
            token_dim: index.token_dim,
            tokens: row.tokens.clone(),
        }
        .validate_schema()
        .map_err(|err| {
            stale(format!(
                "persistent multi row {} has invalid payload: {}; rebuild the vault search indexes",
                row.cx_id, err.message
            ))
        })?;
    }
    if token_count != index.token_count {
        return Err(stale(format!(
            "persistent multi sidecar token_count {} != row token count {token_count}; rebuild the vault search indexes",
            index.token_count
        )));
    }
    Ok(())
}

fn score(
    index: &MultiIndex,
    query: &[Vec<f32>],
    candidates: Option<&BTreeSet<CxId>>,
) -> Vec<(CxId, f32)> {
    index
        .rows
        .iter()
        .filter(|row| candidates.is_none_or(|allowed| allowed.contains(&row.cx_id)))
        .map(|row| (row.cx_id, MaxSimIndex::maxsim(query, &row.tokens)))
        .collect()
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
