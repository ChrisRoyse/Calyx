use super::*;

#[derive(Debug)]
pub(in crate::persisted::multi) struct BinarySidecarSummary {
    pub(in crate::persisted::multi) base_seq: u64,
    pub(in crate::persisted::multi) row_count: u64,
    pub(in crate::persisted::multi) token_count: u64,
    pub(in crate::persisted::multi) ids: BTreeSet<CxId>,
    pub(in crate::persisted::multi) sha256: String,
}

pub(in crate::persisted::multi) fn summarize_binary_entry(
    vault_dir: &Path,
    entry: &SearchIndexEntry,
    slot: SlotId,
) -> CliResult<BinarySidecarSummary> {
    entry.require_kind("multi_maxsim", slot)?;
    let path = sidecar_path(vault_dir, entry, slot)?;
    let summary = summarize_binary_path(
        &path,
        entry.require_sha256(slot)?,
        slot,
        entry.require_token_dim(slot)?,
        Some(entry.len as u64),
        entry.token_count.map(|count| count as u64),
    )?;
    if summary.base_seq != entry.built_at_seq {
        return Err(stale(format!(
            "persistent binary multi sidecar seq {} != manifest seq {}; rebuild the vault search indexes",
            summary.base_seq, entry.built_at_seq
        )));
    }
    Ok(summary)
}

pub(in crate::persisted::multi) fn summarize_binary_path(
    path: &Path,
    expected_sha256: &str,
    slot: SlotId,
    expected_token_dim: u32,
    expected_row_count: Option<u64>,
    expected_token_count: Option<u64>,
) -> CliResult<BinarySidecarSummary> {
    let file = File::open(path)?;
    let mut reader = BufReader::new(file);
    let mut hasher = Sha256::new();
    let header = read_binary_header_hashed(path, &mut reader, &mut hasher)?;
    validate_binary_segment_header(
        &header,
        slot,
        expected_token_dim,
        expected_row_count,
        expected_token_count,
    )?;
    let mut seen = BTreeSet::new();
    let mut observed_tokens = 0u64;
    for _ in 0..header.row_count {
        let cx_id = read_cx_id(path, &mut reader, &mut hasher)?;
        if !seen.insert(cx_id) {
            return Err(stale(format!(
                "persistent binary multi sidecar repeats {cx_id}; rebuild the vault search indexes"
            )));
        }
        let row_token_count = read_u32(path, &mut reader, &mut hasher)? as u64;
        observed_tokens = observed_tokens
            .checked_add(row_token_count)
            .ok_or_else(|| stale("persistent binary multi sidecar token_count overflow"))?;
        skip_token_payload_hashed(
            path,
            &mut reader,
            &mut hasher,
            header.token_dim,
            row_token_count,
        )?;
    }
    if observed_tokens != header.token_count {
        return Err(stale(format!(
            "persistent binary multi sidecar token_count {observed_tokens} != header {}; rebuild the vault search indexes",
            header.token_count
        )));
    }
    ensure_no_trailing_bytes(path, &mut reader, &mut hasher)?;
    let actual = finish_sha256_hex(hasher);
    if actual != expected_sha256 {
        return Err(stale(format!(
            "persistent binary multi sidecar sha256 {actual} != manifest {expected_sha256}; rebuild the vault search indexes"
        )));
    }
    Ok(BinarySidecarSummary {
        base_seq: header.base_seq,
        row_count: header.row_count,
        token_count: header.token_count,
        ids: seen,
        sha256: actual,
    })
}

fn validate_binary_segment_header(
    header: &BinaryHeader,
    slot: SlotId,
    expected_token_dim: u32,
    expected_row_count: Option<u64>,
    expected_token_count: Option<u64>,
) -> CliResult {
    if header.slot != slot.get() {
        return Err(stale(format!(
            "persistent binary multi sidecar slot {} != expected slot {}; rebuild the vault search indexes",
            header.slot,
            slot.get()
        )));
    }
    if header.token_dim != expected_token_dim {
        return Err(stale(format!(
            "persistent binary multi sidecar token_dim {} != expected token_dim {expected_token_dim}; rebuild the vault search indexes",
            header.token_dim
        )));
    }
    if expected_row_count.is_some_and(|row_count| header.row_count != row_count) {
        return Err(stale(format!(
            "persistent binary multi sidecar row len {} != expected {}; rebuild the vault search indexes",
            header.row_count,
            expected_row_count.unwrap_or_default()
        )));
    }
    if expected_token_count.is_some_and(|token_count| header.token_count != token_count) {
        return Err(stale(format!(
            "persistent binary multi sidecar token_count {} != expected {}; rebuild the vault search indexes",
            header.token_count,
            expected_token_count.unwrap_or_default()
        )));
    }
    Ok(())
}

fn skip_token_payload_hashed<R: Read>(
    path: &Path,
    reader: &mut R,
    hasher: &mut Sha256,
    token_dim: u32,
    row_token_count: u64,
) -> CliResult {
    let bytes = row_token_count
        .checked_mul(token_dim as u64)
        .and_then(|components| components.checked_mul(4))
        .ok_or_else(|| stale("persistent binary multi sidecar token byte count overflow"))?;
    let mut remaining = bytes;
    let mut buf = vec![0u8; 1024 * 1024];
    while remaining > 0 {
        let take = remaining.min(buf.len() as u64) as usize;
        read_exact_hashed(path, reader, hasher, &mut buf[..take])?;
        remaining -= take as u64;
    }
    Ok(())
}
