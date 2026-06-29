use super::*;

pub(super) fn validate_segments_manifest_shape(
    manifest: &MultiSegmentsManifest,
    slot: SlotId,
    token_dim: u32,
    base_seq: u64,
    row_count: usize,
    token_count: usize,
) -> CliResult {
    if manifest.format != MULTI_SEGMENTS_FORMAT {
        return Err(stale(format!(
            "persistent segmented multi manifest has format {}; expected {MULTI_SEGMENTS_FORMAT}",
            manifest.format
        )));
    }
    if manifest.slot != slot.get() {
        return Err(stale(format!(
            "persistent segmented multi manifest slot {} != query slot {}",
            manifest.slot,
            slot.get()
        )));
    }
    if manifest.token_dim != token_dim {
        return Err(stale(format!(
            "persistent segmented multi manifest token_dim {} != expected token_dim {token_dim}",
            manifest.token_dim
        )));
    }
    if manifest.base_seq != base_seq {
        return Err(stale(format!(
            "persistent segmented multi manifest seq {} != expected seq {base_seq}; rebuild the vault search indexes",
            manifest.base_seq
        )));
    }
    if manifest.row_count != row_count {
        return Err(stale(format!(
            "persistent segmented multi manifest row_count {} != expected {row_count}; rebuild the vault search indexes",
            manifest.row_count
        )));
    }
    if manifest.token_count != token_count {
        return Err(stale(format!(
            "persistent segmented multi manifest token_count {} != expected {token_count}; rebuild the vault search indexes",
            manifest.token_count
        )));
    }
    if manifest.row_count > 0 && manifest.segments.is_empty() {
        return Err(stale(
            "persistent segmented multi manifest has rows but no segment files; rebuild the vault search indexes",
        ));
    }
    let row_sum = manifest.segments.iter().try_fold(0usize, |sum, segment| {
        checked_rel(&segment.index_rel)?;
        if segment.sha256.len() != 64
            || !segment.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(stale(format!(
                "persistent segmented multi segment {} has invalid sha256",
                segment.index_rel
            )));
        }
        if !segment.ids.is_empty() && segment.ids.len() != segment.row_count {
            return Err(stale(format!(
                "persistent segmented multi segment {} id count {} != row_count {}; rebuild the vault search indexes",
                segment.index_rel,
                segment.ids.len(),
                segment.row_count
            )));
        }
        sum.checked_add(segment.row_count)
            .ok_or_else(|| stale("persistent segmented multi manifest row_count overflow"))
    })?;
    let token_sum = manifest.segments.iter().try_fold(0usize, |sum, segment| {
        sum.checked_add(segment.token_count)
            .ok_or_else(|| stale("persistent segmented multi manifest token_count overflow"))
    })?;
    if row_sum != manifest.row_count {
        return Err(stale(format!(
            "persistent segmented multi manifest row_count {} != segment row sum {row_sum}; rebuild the vault search indexes",
            manifest.row_count
        )));
    }
    if token_sum != manifest.token_count {
        return Err(stale(format!(
            "persistent segmented multi manifest token_count {} != segment token sum {token_sum}; rebuild the vault search indexes",
            manifest.token_count
        )));
    }
    Ok(())
}
