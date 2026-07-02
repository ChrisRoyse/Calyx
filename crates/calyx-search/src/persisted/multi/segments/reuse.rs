//! Opportunistic reuse of prior multi-vector segments during a rebuild.
//!
//! Prior sidecars are DERIVED data (#1109): the rebuild's source of truth is
//! the Base CF rows it was handed, so a missing, corrupt, or otherwise
//! unreadable prior artifact must never fail the rebuild — it only forfeits
//! the append optimization (#1015). The 2026-07-02 calyx15000 incident was a
//! post-commit rebuild that hard-failed `CALYX_STALE_DERIVED` because the
//! previous manifest's slot-22 `.multi.segments.json` was physically absent;
//! the 103 committed rows were fine and a fresh build would have succeeded.
//!
//! Reuse therefore evaluates the previous entry in a fallible probe: any
//! error DECLINES reuse, emits a structured `multi_segment_reuse_declined`
//! progress event naming the unusable artifact and the reason, and the
//! caller builds every segment fresh from the source rows. Fail-closed
//! validation of the NEW artifacts is unaffected — the staged-manifest gate
//! and search-time readback still hard-fail on anything the new manifest
//! references.

use super::*;

use crate::persisted::RebuildProgress;

pub(super) fn reusable_segments(
    vault_dir: &Path,
    slot: SlotId,
    token_dim: u32,
    current_ids: &BTreeSet<CxId>,
    previous: Option<&SearchIndexEntry>,
    on_event: &mut dyn FnMut(RebuildProgress<'_>) -> CliResult,
) -> CliResult<Option<ReusedMultiSegments>> {
    let Some(previous) = previous else {
        return Ok(None);
    };
    // A slot mismatch is a caller bug (the rebuild selected the wrong
    // previous entry), not a stale-derived condition: stay fail-closed.
    if previous.slot != slot.get() {
        return Err(stale(format!(
            "previous persistent multi slot {} cannot be reused for slot {slot}",
            previous.slot
        )));
    }
    match evaluate_previous_entry(vault_dir, slot, token_dim, current_ids, previous) {
        Ok(reused) => Ok(reused),
        Err(error) => {
            on_event(RebuildProgress::slot_detail(
                "multi_segment_reuse_declined",
                slot,
                format!(
                    "prior {} entry {} is unusable for reuse; rebuilding slot {slot} fresh from source rows: {error}",
                    previous.kind,
                    previous.index_rel.as_deref().unwrap_or("<no index_rel>"),
                ),
            ))?;
            Ok(None)
        }
    }
}

/// Probes whether the previous manifest entry's artifacts are present,
/// intact, and a subset of the current rows. `Ok(None)` means "valid but not
/// reusable" (e.g. rows were removed); `Err` means the prior artifacts are
/// unusable and the caller declines reuse.
fn evaluate_previous_entry(
    vault_dir: &Path,
    slot: SlotId,
    token_dim: u32,
    current_ids: &BTreeSet<CxId>,
    previous: &SearchIndexEntry,
) -> CliResult<Option<ReusedMultiSegments>> {
    match previous.kind.as_str() {
        "multi_maxsim" => {
            let token_count = previous.token_count.unwrap_or_default();
            if bounds::ensure_entry_bounded(
                slot,
                previous.require_index_rel(slot)?,
                token_dim,
                previous.len,
                token_count,
            )
            .is_err()
            {
                return Ok(None);
            }
            let summary = binary::summarize_binary_entry(vault_dir, previous, slot)?;
            if summary.ids.iter().any(|cx_id| !current_ids.contains(cx_id)) {
                return Ok(None);
            }
            let row_count = usize::try_from(summary.row_count).map_err(|_| {
                stale(format!(
                    "persistent binary multi sidecar row_count {} does not fit usize",
                    summary.row_count
                ))
            })?;
            let token_count = usize::try_from(summary.token_count).map_err(|_| {
                stale(format!(
                    "persistent binary multi sidecar token_count {} does not fit usize",
                    summary.token_count
                ))
            })?;
            Ok(Some(ReusedMultiSegments {
                refs: vec![MultiSegmentRef {
                    index_rel: previous.require_index_rel(slot)?.to_string(),
                    sha256: summary.sha256,
                    base_seq: summary.base_seq,
                    row_count,
                    token_count,
                    ids: summary.ids.iter().copied().collect(),
                }],
                ids: summary.ids,
                token_count,
            }))
        }
        "multi_maxsim_segments" => {
            let manifest =
                read_segments_manifest(vault_dir, previous, previous.built_at_seq, slot)?;
            if manifest
                .segments
                .iter()
                .any(|segment| !bounds::segment_ref_is_bounded(slot, token_dim, segment))
            {
                return Ok(None);
            }
            let reused = summarize_segment_files(vault_dir, slot, token_dim, &manifest, false)?;
            if reused.ids.iter().any(|cx_id| !current_ids.contains(cx_id)) {
                return Ok(None);
            }
            Ok(Some(reused))
        }
        _ => Ok(None),
    }
}
