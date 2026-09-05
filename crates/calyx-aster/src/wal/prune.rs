//! Bounded deletion of WAL segments already covered by durable checkpoint/SST
//! state (issue #1992).
//!
//! WAL segments accumulate forever. Checkpointing advances the durable
//! coverage floor and flush-SST compaction (#1985) retires SST debt, but until
//! this module no code path ever *deleted* the covered WAL segments, so
//! production vaults carried tens of GB of history back to segment index `0`.
//! Every open replayed (header-scanned) that whole history. This retires whole
//! segments whose highest seq is at or below a proven coverage floor.
//!
//! Safety invariant (never deletes recoverable data):
//! * The active (append) segment — the highest index — is never touched.
//! * A segment is deletable only when its highest seq is
//!   `<= coverage_floor_seq`, which the caller sets to the *verified durable
//!   coverage* floor (the same `verified_durable_coverage_seq` authority the
//!   #1985 compactor uses). Every row in such a segment is therefore already
//!   represented by the checkpoint/SST state that both open paths replay
//!   *after* (`replay_dir_after(wal, manifest.durable_seq)` in
//!   `manifest::recover_vault`, and the ledger view's `wal_replay_floor_seq`),
//!   so those records would be header-seeked-over anyway.
//! * Only a contiguous prefix from the lowest index is deleted, so the
//!   surviving segments keep the contiguous-index invariant
//!   [`super::segment::list_segments`] enforces, and replay-on-open — seeded
//!   from the manifest floor, never from assuming segment `0` exists — stays
//!   correct with the prefix gone.

use super::{WalSegmentStatus, segment, segment_inventory_locked, storage_error};
use calyx_core::{CalyxError, Result};
use std::fs;
use std::path::{Path, PathBuf};

/// Config error for a WAL-prune round requested with unusable bounds.
pub const CALYX_WAL_PRUNE_CONFIG_INVALID: &str = "CALYX_WAL_PRUNE_CONFIG_INVALID";

/// Bounds on the work a single WAL-prune round performs, so a debt-laden vault
/// (hundreds of covered segments, tens of GB) retires over successive bounded
/// rounds instead of unlinking everything and fsyncing the directory once over
/// a massive batch.
#[derive(Debug, Clone, Copy)]
pub struct WalPruneConfig {
    /// Hard cap on the number of segments unlinked in one round.
    pub max_segments: usize,
    /// Hard cap on total bytes reclaimed in one round. A round always retires
    /// at least one covered segment so progress is guaranteed even when a
    /// single segment exceeds this cap.
    pub max_bytes: u64,
}

impl WalPruneConfig {
    /// Fails closed on unusable bounds so a misconfigured round can never
    /// silently disable pruning or request an empty round.
    pub fn validate(&self) -> Result<()> {
        if self.max_segments == 0 {
            return Err(CalyxError {
                code: CALYX_WAL_PRUNE_CONFIG_INVALID,
                message: "WAL prune max_segments must be >= 1".to_string(),
                remediation: "set the per-round segment cap to a positive integer",
            });
        }
        if self.max_bytes == 0 {
            return Err(CalyxError {
                code: CALYX_WAL_PRUNE_CONFIG_INVALID,
                message: "WAL prune max_bytes must be >= 1".to_string(),
                remediation: "set the per-round byte cap to a positive integer",
            });
        }
        Ok(())
    }
}

/// Outcome of one bounded WAL-prune round. A no-op round (nothing covered, or
/// coverage floor below every segment) returns counts with `segments_pruned`
/// = 0, never an error.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WalPruneOutcome {
    /// Coverage floor the round pruned against (the caller's verified durable
    /// coverage seq).
    pub coverage_floor_seq: u64,
    pub segments_before: usize,
    pub bytes_before: u64,
    /// Contiguous covered prefix available before applying the round caps.
    pub prunable_segments_before: usize,
    pub segments_pruned: usize,
    pub bytes_reclaimed: u64,
    pub segments_after: usize,
    pub bytes_after: u64,
    pub pruned_paths: Vec<PathBuf>,
}

/// Deletes a bounded contiguous prefix of WAL segments whose highest seq is at
/// or below `coverage_floor_seq`, holding the append lock so no writer can
/// rotate a new segment or append mid-prune.
pub fn prune_covered_segments(
    dir: impl AsRef<Path>,
    coverage_floor_seq: u64,
    config: WalPruneConfig,
) -> Result<WalPruneOutcome> {
    config.validate()?;
    let dir = dir.as_ref();
    let _lock = crate::file_lock::FileLockGuard::acquire(&dir.join(".append.lock"))?;
    prune_covered_segments_locked(dir, coverage_floor_seq, config)
}

fn prune_covered_segments_locked(
    dir: &Path,
    coverage_floor_seq: u64,
    config: WalPruneConfig,
) -> Result<WalPruneOutcome> {
    let segments = segment::list_segments(dir)?;
    let active_index = segments.last().map_or(0, |(index, _)| *index);
    let inventory = segment_inventory_locked(dir, active_index)?;
    let bytes_before = inventory.iter().map(|segment| segment.bytes).sum();

    let prunable = collect_covered_prefix(&inventory, coverage_floor_seq);
    let mut outcome = WalPruneOutcome {
        coverage_floor_seq,
        segments_before: inventory.len(),
        bytes_before,
        prunable_segments_before: prunable.len(),
        segments_pruned: 0,
        bytes_reclaimed: 0,
        segments_after: inventory.len(),
        bytes_after: bytes_before,
        pruned_paths: Vec::new(),
    };
    if prunable.is_empty() {
        return Ok(outcome);
    }

    for segment in prunable {
        if outcome.segments_pruned >= config.max_segments {
            break;
        }
        // Always retire the first covered segment; stop before a later one that
        // would push the round past its byte budget.
        if outcome.segments_pruned > 0
            && outcome.bytes_reclaimed.saturating_add(segment.bytes) > config.max_bytes
        {
            break;
        }
        fs::remove_file(&segment.path)
            .map_err(|error| storage_error("unlink covered WAL segment", error))?;
        outcome.segments_pruned += 1;
        outcome.bytes_reclaimed = outcome.bytes_reclaimed.saturating_add(segment.bytes);
        outcome.pruned_paths.push(segment.path.clone());
    }

    if outcome.segments_pruned == 0 {
        return Ok(outcome);
    }

    // Persist the unlinks: fsync the WAL directory once after the batch so a
    // crash cannot resurrect a deleted segment (which would reintroduce a gap
    // below the surviving contiguous suffix on the next replay).
    crate::fsync::sync_dir(dir, "WAL prune")?;

    let after = segment_inventory_locked(dir, active_index)?;
    outcome.segments_after = after.len();
    outcome.bytes_after = after.iter().map(|segment| segment.bytes).sum();
    Ok(outcome)
}

/// Contiguous prefix (lowest index first) of non-active segments whose highest
/// seq is at or below `floor`. Stops at the first segment that is active or
/// holds any row above the floor, so the surviving segments remain a
/// contiguous suffix and no row above the coverage floor is ever unlinked.
fn collect_covered_prefix(inventory: &[WalSegmentStatus], floor: u64) -> Vec<&WalSegmentStatus> {
    let mut covered = Vec::new();
    for segment in inventory {
        if segment.active {
            break;
        }
        let fully_covered = match segment.last_seq {
            // An empty (e.g. recycled) non-active segment carries no rows above
            // the floor and is safe to retire.
            None => true,
            Some(last_seq) => last_seq <= floor,
        };
        if !fully_covered {
            break;
        }
        covered.push(segment);
    }
    covered
}

#[cfg(test)]
mod tests;
