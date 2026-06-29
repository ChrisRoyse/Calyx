use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::{BufReader, Read, Write};
use std::path::{Path, PathBuf};

use calyx_core::{CalyxError, Constellation, CxId, SlotId, SlotVector};
use calyx_sextant::index::{IndexSearchHit, MaxSimIndex, ranked};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{SearchIndexEntry, rel, sha256_hex, stale, write_atomic_hashed};
use crate::error::{CliError, CliResult};

const MULTI_FORMAT: &str = "calyx-search-multi-maxsim-index-v1";
const MULTI_BINARY_MAGIC: &[u8; 16] = b"CYX_MULTI_BIN_V1";
const DEFAULT_MAX_MULTI_JSON_SIDECAR_BYTES: u64 = 512 * 1024 * 1024;
const UNBOUNDED_MULTI_SIDECAR_CODE: &str = "CALYX_SEARCH_MULTI_SIDECAR_UNBOUNDED";
const UNBOUNDED_MULTI_SIDECAR_REMEDIATION: &str = "rebuild with a bounded/binary multi-vector index or retire the multi-vector lens before search";

#[derive(Clone, Copy, Debug)]
struct BinaryHeader {
    slot: u16,
    token_dim: u32,
    base_seq: u64,
    row_count: u64,
    token_count: u64,
}

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
        "slot_{:05}_seq_{base_seq:020}_n_{:010}.multi.bin",
        slot.get(),
        rows.rows.len()
    ));
    let row_count = rows.rows.len();
    let token_count = rows.rows.iter().map(|row| row.1.len()).sum::<usize>();
    let sha256 = write_binary_atomic_hashed(&path, slot, rows.token_dim, &rows.rows, base_seq)?;
    Ok(SearchIndexEntry::multi(
        slot,
        rows.token_dim,
        row_count,
        token_count,
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
    if entry.require_token_dim(slot)? != *token_dim {
        return Err(stale(format!(
            "persistent multi slot {slot} token_dim {} != query token_dim {token_dim}; reingest/backfill the vault",
            entry.require_token_dim(slot)?
        )));
    }
    if is_binary_sidecar(entry.require_index_rel(slot)?) {
        search_binary(
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
    if is_binary_sidecar(entry.require_index_rel(slot)?) {
        let path = sidecar_path(vault_dir, entry, slot)?;
        let header = read_binary_header_unhashed(&path)?;
        validate_binary_header(&header, entry, entry.built_at_seq, slot)?;
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

fn is_binary_sidecar(index_rel: &str) -> bool {
    index_rel.ends_with(".multi.bin")
}

fn write_binary_atomic_hashed(
    path: &Path,
    slot: SlotId,
    token_dim: u32,
    rows: &[(CxId, Vec<Vec<f32>>)],
    base_seq: u64,
) -> CliResult<String> {
    let token_count = rows.iter().map(|row| row.1.len()).sum::<usize>();
    write_atomic_hashed(path, |writer| {
        writer.write_all(MULTI_BINARY_MAGIC)?;
        write_u16(writer, slot.get())?;
        write_u32(writer, token_dim)?;
        write_u64(writer, base_seq)?;
        write_u64(writer, rows.len() as u64)?;
        write_u64(writer, token_count as u64)?;
        for (cx_id, tokens) in rows {
            writer.write_all(cx_id.as_bytes())?;
            write_u32(
                writer,
                tokens.len().try_into().map_err(|_| {
                    stale(format!(
                        "slot {slot} cx {cx_id} has too many multi tokens for binary sidecar"
                    ))
                })?,
            )?;
            for token in tokens {
                if token.len() != token_dim as usize {
                    return Err(stale(format!(
                        "slot {slot} cx {cx_id} multi token len {} != token_dim {token_dim}",
                        token.len()
                    )));
                }
                for value in token {
                    if !value.is_finite() {
                        return Err(CalyxError::lens_numerical_invariant(format!(
                            "slot {slot} cx {cx_id} has non-finite multi token component"
                        ))
                        .into());
                    }
                    writer.write_all(&value.to_le_bytes())?;
                }
            }
        }
        Ok(())
    })
}

fn search_binary(
    vault_dir: &Path,
    entry: &SearchIndexEntry,
    manifest_base_seq: u64,
    slot: SlotId,
    query: &[Vec<f32>],
    k: usize,
    candidates: Option<&BTreeSet<CxId>>,
) -> CliResult<Vec<IndexSearchHit>> {
    let path = sidecar_path(vault_dir, entry, slot)?;
    let file = File::open(&path)?;
    let mut reader = BufReader::new(file);
    let mut hasher = Sha256::new();
    let header = read_binary_header_hashed(&path, &mut reader, &mut hasher)?;
    validate_binary_header(&header, entry, manifest_base_seq, slot)?;

    let mut seen = BTreeSet::new();
    let mut observed_tokens = 0u64;
    let mut scored = Vec::new();
    for _ in 0..header.row_count {
        let cx_id = read_cx_id(&path, &mut reader, &mut hasher)?;
        if !seen.insert(cx_id) {
            return Err(stale(format!(
                "persistent binary multi sidecar repeats {cx_id}; rebuild the vault search indexes"
            )));
        }
        let row_token_count = read_u32(&path, &mut reader, &mut hasher)? as u64;
        observed_tokens = observed_tokens
            .checked_add(row_token_count)
            .ok_or_else(|| stale("persistent binary multi sidecar token_count overflow"))?;
        if observed_tokens > header.token_count {
            return Err(stale(format!(
                "persistent binary multi sidecar token_count exceeds header {}; rebuild the vault search indexes",
                header.token_count
            )));
        }
        let tokens = read_tokens(
            &path,
            &mut reader,
            &mut hasher,
            slot,
            cx_id,
            header.token_dim,
            row_token_count,
        )?;
        if candidates.is_none_or(|allowed| allowed.contains(&cx_id)) {
            scored.push((cx_id, MaxSimIndex::maxsim(query, &tokens)));
        }
    }
    if observed_tokens != header.token_count {
        return Err(stale(format!(
            "persistent binary multi sidecar token_count {observed_tokens} != header {}; rebuild the vault search indexes",
            header.token_count
        )));
    }
    ensure_no_trailing_bytes(&path, &mut reader, &mut hasher)?;
    let actual = finish_sha256_hex(hasher);
    let expected = entry.require_sha256(slot)?;
    if actual != expected {
        return Err(stale(format!(
            "persistent binary multi sidecar sha256 {actual} != manifest {expected}; rebuild the vault search indexes"
        )));
    }
    Ok(ranked(top_k(scored, k)))
}

fn read_binary_header_unhashed(path: &Path) -> CliResult<BinaryHeader> {
    let file = File::open(path)?;
    let mut reader = BufReader::new(file);
    let mut magic = [0u8; 16];
    reader.read_exact(&mut magic).map_err(|err| {
        stale(format!(
            "persistent binary multi sidecar {} is unreadable: {err}; rebuild the vault search indexes",
            path.display()
        ))
    })?;
    if &magic != MULTI_BINARY_MAGIC {
        return Err(stale(format!(
            "persistent binary multi sidecar {} has invalid magic; rebuild the vault search indexes",
            path.display()
        )));
    }
    let slot = read_u16_unhashed(path, &mut reader)?;
    let token_dim = read_u32_unhashed(path, &mut reader)?;
    let base_seq = read_u64_unhashed(path, &mut reader)?;
    let row_count = read_u64_unhashed(path, &mut reader)?;
    let token_count = read_u64_unhashed(path, &mut reader)?;
    Ok(BinaryHeader {
        slot,
        token_dim,
        base_seq,
        row_count,
        token_count,
    })
}

fn read_binary_header_hashed<R: Read>(
    path: &Path,
    reader: &mut R,
    hasher: &mut Sha256,
) -> CliResult<BinaryHeader> {
    let mut magic = [0u8; 16];
    read_exact_hashed(path, reader, hasher, &mut magic)?;
    if &magic != MULTI_BINARY_MAGIC {
        return Err(stale(format!(
            "persistent binary multi sidecar {} has invalid magic; rebuild the vault search indexes",
            path.display()
        )));
    }
    let slot = read_u16(path, reader, hasher)?;
    let token_dim = read_u32(path, reader, hasher)?;
    let base_seq = read_u64(path, reader, hasher)?;
    let row_count = read_u64(path, reader, hasher)?;
    let token_count = read_u64(path, reader, hasher)?;
    Ok(BinaryHeader {
        slot,
        token_dim,
        base_seq,
        row_count,
        token_count,
    })
}

fn validate_binary_header(
    header: &BinaryHeader,
    entry: &SearchIndexEntry,
    manifest_base_seq: u64,
    slot: SlotId,
) -> CliResult {
    if header.slot != slot.get() || entry.slot != slot.get() {
        return Err(stale(format!(
            "persistent binary multi sidecar slot {} / entry slot {} != query slot {}",
            header.slot,
            entry.slot,
            slot.get()
        )));
    }
    let entry_token_dim = entry.require_token_dim(slot)?;
    if header.token_dim != entry_token_dim {
        return Err(stale(format!(
            "persistent binary multi sidecar token_dim {} != manifest token_dim {entry_token_dim}; rebuild the vault search indexes",
            header.token_dim
        )));
    }
    if header.base_seq != manifest_base_seq || entry.built_at_seq != manifest_base_seq {
        return Err(stale(format!(
            "persistent binary multi sidecar seq {} / entry seq {} != manifest seq {manifest_base_seq}; rebuild the vault search indexes",
            header.base_seq, entry.built_at_seq
        )));
    }
    if header.row_count != entry.len as u64 {
        return Err(stale(format!(
            "persistent binary multi sidecar row len {} != manifest len {}; rebuild the vault search indexes",
            header.row_count, entry.len
        )));
    }
    if entry
        .token_count
        .is_some_and(|count| header.token_count != count as u64)
    {
        return Err(stale(format!(
            "persistent binary multi sidecar token_count {} != manifest token_count {}; rebuild the vault search indexes",
            header.token_count,
            entry.token_count.unwrap_or_default()
        )));
    }
    Ok(())
}

fn read_tokens<R: Read>(
    path: &Path,
    reader: &mut R,
    hasher: &mut Sha256,
    slot: SlotId,
    cx_id: CxId,
    token_dim: u32,
    row_token_count: u64,
) -> CliResult<Vec<Vec<f32>>> {
    let row_token_count_usize = usize::try_from(row_token_count).map_err(|_| {
        stale(format!(
            "persistent binary multi sidecar row {cx_id} token count does not fit usize; rebuild the vault search indexes"
        ))
    })?;
    let token_dim_usize = token_dim as usize;
    let mut tokens = Vec::with_capacity(row_token_count_usize);
    for _ in 0..row_token_count_usize {
        let mut token = Vec::with_capacity(token_dim_usize);
        for _ in 0..token_dim_usize {
            let value = read_f32(path, reader, hasher)?;
            if !value.is_finite() {
                return Err(CalyxError::lens_numerical_invariant(format!(
                    "persistent binary multi row {cx_id} slot {slot} has non-finite component"
                ))
                .into());
            }
            token.push(value);
        }
        tokens.push(token);
    }
    Ok(tokens)
}

fn read_cx_id<R: Read>(path: &Path, reader: &mut R, hasher: &mut Sha256) -> CliResult<CxId> {
    let mut bytes = [0u8; 16];
    read_exact_hashed(path, reader, hasher, &mut bytes)?;
    Ok(CxId::from_bytes(bytes))
}

fn read_u16_unhashed<R: Read>(path: &Path, reader: &mut R) -> CliResult<u16> {
    let mut bytes = [0u8; 2];
    reader.read_exact(&mut bytes).map_err(|err| {
        stale(format!(
            "persistent binary multi sidecar {} is truncated: {err}; rebuild the vault search indexes",
            path.display()
        ))
    })?;
    Ok(u16::from_le_bytes(bytes))
}

fn read_u32_unhashed<R: Read>(path: &Path, reader: &mut R) -> CliResult<u32> {
    let mut bytes = [0u8; 4];
    reader.read_exact(&mut bytes).map_err(|err| {
        stale(format!(
            "persistent binary multi sidecar {} is truncated: {err}; rebuild the vault search indexes",
            path.display()
        ))
    })?;
    Ok(u32::from_le_bytes(bytes))
}

fn read_u64_unhashed<R: Read>(path: &Path, reader: &mut R) -> CliResult<u64> {
    let mut bytes = [0u8; 8];
    reader.read_exact(&mut bytes).map_err(|err| {
        stale(format!(
            "persistent binary multi sidecar {} is truncated: {err}; rebuild the vault search indexes",
            path.display()
        ))
    })?;
    Ok(u64::from_le_bytes(bytes))
}

fn read_u16<R: Read>(path: &Path, reader: &mut R, hasher: &mut Sha256) -> CliResult<u16> {
    let mut bytes = [0u8; 2];
    read_exact_hashed(path, reader, hasher, &mut bytes)?;
    Ok(u16::from_le_bytes(bytes))
}

fn read_u32<R: Read>(path: &Path, reader: &mut R, hasher: &mut Sha256) -> CliResult<u32> {
    let mut bytes = [0u8; 4];
    read_exact_hashed(path, reader, hasher, &mut bytes)?;
    Ok(u32::from_le_bytes(bytes))
}

fn read_u64<R: Read>(path: &Path, reader: &mut R, hasher: &mut Sha256) -> CliResult<u64> {
    let mut bytes = [0u8; 8];
    read_exact_hashed(path, reader, hasher, &mut bytes)?;
    Ok(u64::from_le_bytes(bytes))
}

fn read_f32<R: Read>(path: &Path, reader: &mut R, hasher: &mut Sha256) -> CliResult<f32> {
    let mut bytes = [0u8; 4];
    read_exact_hashed(path, reader, hasher, &mut bytes)?;
    Ok(f32::from_le_bytes(bytes))
}

fn write_u16<W: Write>(writer: &mut W, value: u16) -> CliResult {
    Ok(writer.write_all(&value.to_le_bytes())?)
}

fn write_u32<W: Write>(writer: &mut W, value: u32) -> CliResult {
    Ok(writer.write_all(&value.to_le_bytes())?)
}

fn write_u64<W: Write>(writer: &mut W, value: u64) -> CliResult {
    Ok(writer.write_all(&value.to_le_bytes())?)
}

fn read_exact_hashed<R: Read>(
    path: &Path,
    reader: &mut R,
    hasher: &mut Sha256,
    buf: &mut [u8],
) -> CliResult {
    reader.read_exact(buf).map_err(|err| {
        stale(format!(
            "persistent binary multi sidecar {} is truncated: {err}; rebuild the vault search indexes",
            path.display()
        ))
    })?;
    hasher.update(buf);
    Ok(())
}

fn ensure_no_trailing_bytes<R: Read>(
    path: &Path,
    reader: &mut R,
    hasher: &mut Sha256,
) -> CliResult {
    let mut byte = [0u8; 1];
    match reader.read(&mut byte) {
        Ok(0) => Ok(()),
        Ok(read) => {
            hasher.update(&byte[..read]);
            Err(stale(format!(
                "persistent binary multi sidecar {} has trailing bytes; rebuild the vault search indexes",
                path.display()
            )))
        }
        Err(err) => Err(stale(format!(
            "persistent binary multi sidecar {} could not be checked for trailing bytes: {err}; rebuild the vault search indexes",
            path.display()
        ))),
    }
}

fn finish_sha256_hex(hasher: Sha256) -> String {
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        hex.push_str(&format!("{byte:02x}"));
    }
    hex
}

fn unbounded_multi_sidecar(message: impl Into<String>) -> CliError {
    CalyxError {
        code: UNBOUNDED_MULTI_SIDECAR_CODE,
        message: message.into(),
        remediation: UNBOUNDED_MULTI_SIDECAR_REMEDIATION,
    }
    .into()
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
