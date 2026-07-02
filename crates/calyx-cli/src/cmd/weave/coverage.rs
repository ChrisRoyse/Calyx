use std::collections::{BTreeMap, HashMap, HashSet};

use calyx_core::{Constellation, CxId, SlotId};
use calyx_lodestar::LodestarError;
use serde::Serialize;

const EXAMPLE_MISSING_LIMIT: usize = 5;
const COVERED_SCAN_BATCH_SIZE: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub(crate) enum CandidateSelectionMode {
    Covered,
    BasePrefix,
}

impl CandidateSelectionMode {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Covered => "covered",
            Self::BasePrefix => "base-prefix",
        }
    }
}

/// Exact per-slot coverage accounting. The counters are additive by
/// construction: `dense + non_dense + absent + tombstoned + missing ==
/// candidate_rows`. `missing_rows` counts only candidates whose base row does
/// not list the slot — a base-listed slot with no physical row fails the scan
/// closed instead of being counted here (issue #1096).
#[derive(Clone, Debug, Serialize)]
pub(super) struct DenseSlotCoverage {
    pub slot_id: u16,
    pub candidate_rows: usize,
    pub dense_rows: usize,
    pub missing_rows: usize,
    pub non_dense_rows: usize,
    pub absent_rows: usize,
    pub tombstoned_rows: usize,
    pub example_missing_cx_ids: Vec<String>,
    pub read_stats: crate::provenance_read::ProvenanceReadStats,
}

impl DenseSlotCoverage {
    pub(super) fn has_full_coverage(&self) -> bool {
        self.candidate_rows > 0 && self.dense_rows == self.candidate_rows
    }

    /// Candidates that are not usable dense rows for this slot.
    pub(super) fn uncovered_rows(&self) -> usize {
        self.candidate_rows.saturating_sub(self.dense_rows)
    }
}

pub(super) struct DenseSlotCoverageScan {
    pub constellations_in_vault: usize,
    pub scanned_candidates: Vec<Constellation>,
    pub slot_maps: BTreeMap<SlotId, HashMap<CxId, Vec<f32>>>,
    pub coverage: Vec<DenseSlotCoverage>,
    pub base_page_index_live_entries: usize,
    pub candidate_scan_rows: usize,
    pub candidate_scan_complete: bool,
}

pub(super) struct DenseSlotPreflight {
    pub constellations_in_vault: usize,
    pub candidates: Vec<Constellation>,
    pub slot_maps: BTreeMap<SlotId, HashMap<CxId, Vec<f32>>>,
    pub coverage: Vec<DenseSlotCoverage>,
    pub base_page_index_live_entries: usize,
    pub candidate_scan_rows: usize,
    pub candidate_scan_complete: bool,
    pub selected_candidate_rows: usize,
    pub selected_candidate_cx_ids: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(super) struct SlotSelection {
    pub slot: SlotId,
    pub reason: &'static str,
    pub mode: &'static str,
    pub scanned_rows: usize,
    pub selected_rows: usize,
    pub requested_limit: usize,
    pub excluded_uncovered_rows: usize,
}

mod scan;
pub(super) use scan::scan_dense_slot_coverage;
pub(super) fn select_slot_from_coverage(
    requested: Option<SlotId>,
    mode: CandidateSelectionMode,
    limit: usize,
    coverage: &[DenseSlotCoverage],
) -> Result<SlotSelection, String> {
    let candidate_rows = coverage
        .iter()
        .map(|row| row.candidate_rows)
        .max()
        .unwrap_or(0);
    if candidate_rows < 2 {
        return Err(format!(
            "CALYX_WEAVE_LOOM_EMPTY_CANDIDATE_SET: weave-loom needs >=2 candidate constellations; candidate_rows={candidate_rows}"
        ));
    }
    if mode == CandidateSelectionMode::Covered {
        return select_covered_slot(requested, limit, coverage);
    }
    if let Some(slot) = requested {
        let Some(row) = coverage.iter().find(|row| row.slot_id == slot.get()) else {
            return Err(format!(
                "CALYX_WEAVE_LOOM_SLOT_NOT_PREFLIGHTED: content slot {} was not measured in dense-slot coverage preflight",
                slot.get()
            ));
        };
        if row.has_full_coverage() {
            return Ok(SlotSelection {
                slot,
                reason: "requested_slot_full_coverage",
                mode: mode.as_str(),
                scanned_rows: row.candidate_rows,
                selected_rows: candidate_rows,
                requested_limit: limit,
                excluded_uncovered_rows: 0,
            });
        }
        return Err(format!(
            "CALYX_WEAVE_LOOM_DENSE_COVERAGE_INCOMPLETE: requested content slot {} covers {}/{} candidate rows; missing_rows={}; non_dense_rows={}; absent_rows={}; tombstoned_rows={}; example_missing_cx_ids={:?}",
            row.slot_id,
            row.dense_rows,
            row.candidate_rows,
            row.missing_rows,
            row.non_dense_rows,
            row.absent_rows,
            row.tombstoned_rows,
            row.example_missing_cx_ids
        ));
    }

    if let Some(row) = coverage.iter().find(|row| row.has_full_coverage()) {
        return Ok(SlotSelection {
            slot: SlotId::new(row.slot_id),
            reason: "lowest_slot_with_full_candidate_coverage",
            mode: mode.as_str(),
            scanned_rows: row.candidate_rows,
            selected_rows: candidate_rows,
            requested_limit: limit,
            excluded_uncovered_rows: 0,
        });
    }
    Err(format!(
        "CALYX_WEAVE_LOOM_NO_FULL_DENSE_SLOT: no active dense content slot covers all {candidate_rows} candidate rows; coverage={}",
        coverage_summary(coverage)
    ))
}

fn select_covered_slot(
    requested: Option<SlotId>,
    limit: usize,
    coverage: &[DenseSlotCoverage],
) -> Result<SlotSelection, String> {
    let candidate_rows = coverage
        .iter()
        .map(|row| row.candidate_rows)
        .max()
        .unwrap_or(0);
    let target_rows = |dense_rows: usize| {
        if limit == 0 {
            dense_rows
        } else {
            dense_rows.min(limit)
        }
    };
    if let Some(slot) = requested {
        let Some(row) = coverage.iter().find(|row| row.slot_id == slot.get()) else {
            return Err(format!(
                "CALYX_WEAVE_LOOM_SLOT_NOT_PREFLIGHTED: content slot {} was not measured in dense-slot coverage preflight",
                slot.get()
            ));
        };
        let selected_rows = target_rows(row.dense_rows);
        if selected_rows >= 2 {
            return Ok(SlotSelection {
                slot,
                reason: "requested_slot_covered_candidate_set",
                mode: CandidateSelectionMode::Covered.as_str(),
                scanned_rows: row.candidate_rows,
                selected_rows,
                requested_limit: limit,
                excluded_uncovered_rows: row.uncovered_rows(),
            });
        }
        return Err(format!(
            "CALYX_WEAVE_LOOM_COVERED_SET_TOO_SMALL: requested content slot {} has only {} dense candidate rows after scanning {}; selected_rows={}; limit={}; missing_rows={}; non_dense_rows={}; absent_rows={}; tombstoned_rows={}; example_missing_cx_ids={:?}",
            row.slot_id,
            row.dense_rows,
            row.candidate_rows,
            selected_rows,
            limit,
            row.missing_rows,
            row.non_dense_rows,
            row.absent_rows,
            row.tombstoned_rows,
            row.example_missing_cx_ids
        ));
    }

    let best = coverage
        .iter()
        .filter_map(|row| {
            let selected_rows = target_rows(row.dense_rows);
            (selected_rows >= 2).then_some((row, selected_rows))
        })
        .max_by(|(left, left_selected), (right, right_selected)| {
            left_selected
                .cmp(right_selected)
                .then_with(|| left.dense_rows.cmp(&right.dense_rows))
                .then_with(|| right.slot_id.cmp(&left.slot_id))
        });
    if let Some((row, selected_rows)) = best {
        return Ok(SlotSelection {
            slot: SlotId::new(row.slot_id),
            reason: "largest_dense_covered_candidate_set",
            mode: CandidateSelectionMode::Covered.as_str(),
            scanned_rows: row.candidate_rows,
            selected_rows,
            requested_limit: limit,
            excluded_uncovered_rows: row.uncovered_rows(),
        });
    }
    Err(format!(
        "CALYX_WEAVE_LOOM_NO_COVERED_DENSE_SET: no active dense content slot has >=2 covered candidate rows after scanning {candidate_rows}; coverage={}",
        coverage_summary(coverage)
    ))
}

pub(super) fn materialize_selected_preflight(
    scan: DenseSlotCoverageScan,
    selection: &SlotSelection,
) -> DenseSlotPreflight {
    let selected_ids = selected_ids_for(&scan, selection);
    let selected_set = selected_ids.iter().copied().collect::<HashSet<_>>();
    let candidates = scan
        .scanned_candidates
        .into_iter()
        .filter(|cx| selected_set.contains(&cx.cx_id))
        .collect::<Vec<_>>();
    let slot_maps = scan
        .slot_maps
        .into_iter()
        .map(|(slot, map)| {
            let selected_map = map
                .into_iter()
                .filter(|(cx_id, _)| selected_set.contains(cx_id))
                .collect::<HashMap<_, _>>();
            (slot, selected_map)
        })
        .collect::<BTreeMap<_, _>>();
    let selected_candidate_cx_ids = candidates
        .iter()
        .map(|cx| cx.cx_id.to_string())
        .collect::<Vec<_>>();
    DenseSlotPreflight {
        constellations_in_vault: scan.constellations_in_vault,
        selected_candidate_rows: candidates.len(),
        candidate_scan_rows: scan.candidate_scan_rows,
        candidate_scan_complete: scan.candidate_scan_complete,
        candidates,
        slot_maps,
        coverage: scan.coverage,
        base_page_index_live_entries: scan.base_page_index_live_entries,
        selected_candidate_cx_ids,
    }
}

fn selected_ids_for(scan: &DenseSlotCoverageScan, selection: &SlotSelection) -> Vec<CxId> {
    let Some(map) = scan.slot_maps.get(&selection.slot) else {
        return Vec::new();
    };
    scan.scanned_candidates
        .iter()
        .filter(|cx| map.contains_key(&cx.cx_id))
        .take(selection.selected_rows)
        .map(|cx| cx.cx_id)
        .collect()
}

pub(super) fn coverage_summary(coverage: &[DenseSlotCoverage]) -> String {
    coverage
        .iter()
        .map(|row| {
            format!(
                "slot {} dense={}/{} missing={} non_dense={} absent={} tombstoned={} examples={:?}",
                row.slot_id,
                row.dense_rows,
                row.candidate_rows,
                row.missing_rows,
                row.non_dense_rows,
                row.absent_rows,
                row.tombstoned_rows,
                row.example_missing_cx_ids
            )
        })
        .collect::<Vec<_>>()
        .join("; ")
}

pub(super) fn invalid_params(detail: impl Into<String>) -> crate::error::CliError {
    LodestarError::KernelInvalidParams {
        detail: detail.into(),
    }
    .into()
}
