#[cfg(test)]
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::{BufReader, Read, Write};
use std::path::{Path, PathBuf};

#[cfg(test)]
use calyx_core::Constellation;
use calyx_core::{CalyxError, CxId, SlotId, SlotVector};
use calyx_sextant::index::{IndexSearchHit, MaxSimIndex, ranked};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{
    SearchIndexEntry, rel, sha256_hex, stale, write_atomic_hashed, write_json_atomic_hashed,
};
use crate::error::{CliError, CliResult};

const MULTI_FORMAT: &str = "calyx-search-multi-maxsim-index-v1";
const DEFAULT_MAX_MULTI_JSON_SIDECAR_BYTES: u64 = 512 * 1024 * 1024;
const UNBOUNDED_MULTI_SIDECAR_CODE: &str = "CALYX_SEARCH_MULTI_SIDECAR_UNBOUNDED";
const UNBOUNDED_MULTI_SIDECAR_REMEDIATION: &str = "rebuild with a bounded/binary multi-vector index or retire the multi-vector lens before search";

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

#[derive(Clone, Debug)]
pub(super) struct MultiSlotRows {
    pub(super) token_dim: u32,
    pub(super) rows: Vec<(CxId, Vec<Vec<f32>>)>,
}

impl MultiSlotRows {
    pub(super) fn len(&self) -> usize {
        self.rows.len()
    }
}

#[cfg(test)]
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

pub(super) use segments::write;

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
    if entry.require_token_dim(slot)? != *token_dim {
        return Err(stale(format!(
            "persistent multi slot {slot} token_dim {} != query token_dim {token_dim}; reingest/backfill the vault",
            entry.require_token_dim(slot)?
        )));
    }
    if binary::is_binary_sidecar(entry.require_index_rel(slot)?) {
        ensure_binary_entry_bounded(vault_dir, entry, slot)?;
        binary::search_binary(
            vault_dir,
            entry,
            manifest_base_seq,
            slot,
            query_tokens,
            k,
            candidates,
        )
    } else if entry.kind == "multi_maxsim_segments" {
        segments::search_segments(
            vault_dir,
            entry,
            manifest_base_seq,
            slot,
            query_tokens,
            k,
            candidates,
        )
    } else {
        let index = read_json(vault_dir, entry, manifest_base_seq, slot)?;
        Ok(ranked(top_k(score(&index, query_tokens, candidates), k)))
    }
}

pub(super) fn ensure_bounded_sidecar(
    vault_dir: &Path,
    entry: &SearchIndexEntry,
    slot: SlotId,
) -> CliResult {
    if entry.kind == "multi_maxsim_segments" {
        let entry_sha256 = entry.require_sha256(slot)?;
        if let Some(files) = pinned::memoized_bounded_segment_files(vault_dir, slot, entry_sha256)?
        {
            return pinned::stat_check_segment_files(slot, &files);
        }
        let manifest =
            segments::read_segments_manifest(vault_dir, entry, entry.built_at_seq, slot)?;
        let mut files = vec![segments_manifest_file(vault_dir, entry, slot)?];
        files.extend(segments::validate_segment_files(
            vault_dir,
            slot,
            entry.require_token_dim(slot)?,
            &manifest,
        )?);
        pinned::memoize_bounded_segment_files(vault_dir, slot, entry_sha256, files)?;
    } else if binary::is_binary_sidecar(entry.require_index_rel(slot)?) {
        ensure_binary_entry_bounded(vault_dir, entry, slot)?;
        let path = sidecar_path(vault_dir, entry, slot)?;
        let header = binary::read_binary_header_unhashed(&path)?;
        binary::validate_binary_header(&header, entry, entry.built_at_seq, slot)?;
    } else {
        let _ = checked_json_sidecar_path(
            vault_dir,
            entry,
            slot,
            DEFAULT_MAX_MULTI_JSON_SIDECAR_BYTES,
        )?;
    }
    Ok(())
}

fn segments_manifest_file(
    vault_dir: &Path,
    entry: &SearchIndexEntry,
    slot: SlotId,
) -> CliResult<pinned::BoundedSegmentFile> {
    let index_rel = entry.require_index_rel(slot)?;
    let path = vault_dir.join(index_rel);
    let expected_bytes = fs::metadata(&path)
        .map_err(|_| {
            stale(format!(
                "persistent segmented multi sidecar missing for slot {slot} at {}; rebuild the vault search indexes",
                path.display()
            ))
        })?
        .len();
    Ok(pinned::BoundedSegmentFile {
        path,
        index_rel: index_rel.to_string(),
        expected_bytes,
    })
}

fn ensure_binary_entry_bounded(
    _vault_dir: &Path,
    entry: &SearchIndexEntry,
    slot: SlotId,
) -> CliResult {
    segments::ensure_entry_bounded(
        slot,
        entry.require_index_rel(slot)?,
        entry.require_token_dim(slot)?,
        entry.len,
        entry.token_count.unwrap_or_default(),
    )
}

pub(super) fn referenced_segment_artifacts(
    vault_dir: &Path,
    entry: &SearchIndexEntry,
    slot: SlotId,
) -> CliResult<Vec<PathBuf>> {
    segments::referenced_segment_artifacts(vault_dir, entry, slot)
}

fn read_json(
    vault_dir: &Path,
    entry: &SearchIndexEntry,
    manifest_base_seq: u64,
    slot: SlotId,
) -> CliResult<MultiIndex> {
    read_with_sidecar_limit(
        vault_dir,
        entry,
        manifest_base_seq,
        slot,
        DEFAULT_MAX_MULTI_JSON_SIDECAR_BYTES,
    )
}

fn read_with_sidecar_limit(
    vault_dir: &Path,
    entry: &SearchIndexEntry,
    manifest_base_seq: u64,
    slot: SlotId,
    max_sidecar_bytes: u64,
) -> CliResult<MultiIndex> {
    entry.require_kind("multi_maxsim", slot)?;
    let path = checked_json_sidecar_path(vault_dir, entry, slot, max_sidecar_bytes)?;
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

fn checked_json_sidecar_path(
    vault_dir: &Path,
    entry: &SearchIndexEntry,
    slot: SlotId,
    max_sidecar_bytes: u64,
) -> CliResult<PathBuf> {
    let path = sidecar_path(vault_dir, entry, slot)?;
    let sidecar_bytes = fs::metadata(&path)?.len();
    if sidecar_bytes > max_sidecar_bytes {
        return Err(unbounded_multi_sidecar(format!(
            "persistent multi sidecar for slot {slot} is {sidecar_bytes} bytes at {}; exceeds search JSON sidecar limit {max_sidecar_bytes} bytes (rows={}, tokens={})",
            path.display(),
            entry.len,
            entry.token_count.unwrap_or_default()
        )));
    }
    Ok(path)
}

fn sidecar_path(vault_dir: &Path, entry: &SearchIndexEntry, slot: SlotId) -> CliResult<PathBuf> {
    entry.require_kind("multi_maxsim", slot)?;
    let path = vault_dir.join(entry.require_index_rel(slot)?);
    if !path.is_file() {
        return Err(stale(format!(
            "persistent multi sidecar missing at {}; rebuild the vault search indexes",
            path.display()
        )));
    }
    Ok(path)
}

fn unbounded_multi_sidecar(message: impl Into<String>) -> CliError {
    CalyxError {
        code: UNBOUNDED_MULTI_SIDECAR_CODE,
        message: message.into(),
        remediation: UNBOUNDED_MULTI_SIDECAR_REMEDIATION,
    }
    .into()
}

#[path = "multi/binary.rs"]
mod binary;

#[path = "multi/pinned.rs"]
mod pinned;

#[path = "multi/segments.rs"]
mod segments;

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

pub(super) fn validate_entry(
    vault_dir: &Path,
    entry: &SearchIndexEntry,
    manifest_base_seq: u64,
    slot: SlotId,
) -> CliResult {
    if entry.kind == "multi_maxsim_segments" {
        return ensure_bounded_sidecar(vault_dir, entry, slot);
    }
    if binary::is_binary_sidecar(entry.require_index_rel(slot)?) {
        return ensure_binary_entry_bounded(vault_dir, entry, slot);
    }
    let _ = read_json(vault_dir, entry, manifest_base_seq, slot)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    #[test]
    fn oversized_multi_sidecar_fails_before_reading_json_payload() {
        let root = temp_root("oversized-multi-sidecar");
        let sidecar_rel = "idx/search/slot_00022.multi.json";
        let sidecar_path = root.join(sidecar_rel);
        fs::create_dir_all(sidecar_path.parent().unwrap()).unwrap();
        fs::write(&sidecar_path, b"not json, but too large").unwrap();

        let slot = SlotId::new(22);
        let entry = SearchIndexEntry::multi(
            slot,
            384,
            1026,
            513_767,
            3348,
            sidecar_rel.to_string(),
            "unused-because-size-check-runs-first".to_string(),
        );
        let err = read_with_sidecar_limit(&root, &entry, 3348, slot, 4).unwrap_err();
        let message = err.message();

        assert_eq!(err.code(), UNBOUNDED_MULTI_SIDECAR_CODE);
        assert!(message.contains("persistent multi sidecar for slot 22"));
        assert!(message.contains("exceeds search JSON sidecar limit 4 bytes"));
        assert!(message.contains("rows=1026"));
        assert!(message.contains("tokens=513767"));
        let CliError::Calyx(calyx) = err else {
            panic!("expected structured Calyx error");
        };
        assert_eq!(calyx.remediation, UNBOUNDED_MULTI_SIDECAR_REMEDIATION);
    }

    fn temp_root(tag: &str) -> std::path::PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("calyx-search-{tag}-{stamp}"));
        if root.exists() {
            fs::remove_dir_all(&root).unwrap();
        }
        fs::create_dir_all(&root).unwrap();
        root
    }
}
