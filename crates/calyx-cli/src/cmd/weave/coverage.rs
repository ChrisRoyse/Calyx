use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

use calyx_aster::base_page_index::{read_base_page_index_manifest, read_indexed_base_rows};
use calyx_aster::cf::{ColumnFamily, slot_key};
use calyx_aster::vault::encode;
use calyx_core::{Constellation, CxId, SlotId, SlotVector};
use calyx_lodestar::LodestarError;
use serde::Serialize;

use crate::bounded_progress::Deadline;
use crate::error::{CliError, CliResult};

const EXAMPLE_MISSING_LIMIT: usize = 5;

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

#[derive(Clone, Debug, Serialize)]
pub(super) struct DenseSlotCoverage {
    pub slot_id: u16,
    pub candidate_rows: usize,
    pub dense_rows: usize,
    pub missing_rows: usize,
    pub non_dense_rows: usize,
    pub example_missing_cx_ids: Vec<String>,
}

impl DenseSlotCoverage {
    pub(super) fn has_full_coverage(&self) -> bool {
        self.candidate_rows > 0
            && self.dense_rows == self.candidate_rows
            && self.non_dense_rows == 0
    }
}

pub(super) struct DenseSlotCoverageScan {
    pub constellations_in_vault: usize,
    pub scanned_candidates: Vec<Constellation>,
    pub slot_maps: BTreeMap<SlotId, HashMap<CxId, Vec<f32>>>,
    pub coverage: Vec<DenseSlotCoverage>,
    pub base_page_index_live_entries: usize,
    pub candidate_scan_rows: usize,
}

pub(super) struct DenseSlotPreflight {
    pub constellations_in_vault: usize,
    pub candidates: Vec<Constellation>,
    pub slot_maps: BTreeMap<SlotId, HashMap<CxId, Vec<f32>>>,
    pub coverage: Vec<DenseSlotCoverage>,
    pub base_page_index_live_entries: usize,
    pub candidate_scan_rows: usize,
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

pub(super) fn scan_dense_slot_coverage(
    vault_dir: &Path,
    content_slots: &[SlotId],
    limit: usize,
    mode: CandidateSelectionMode,
    deadline: &Deadline,
) -> CliResult<DenseSlotCoverageScan> {
    deadline.check("weave-loom", "coverage.base_page_index_manifest", 0)?;
    let manifest = read_base_page_index_manifest(vault_dir)?;
    let candidate_limit = match mode {
        CandidateSelectionMode::Covered => manifest.live_entries,
        CandidateSelectionMode::BasePrefix => {
            if limit == 0 {
                manifest.live_entries
            } else {
                limit.min(manifest.live_entries)
            }
        }
    };
    let indexed_rows = read_indexed_base_rows(vault_dir, candidate_limit)?;
    let mut candidates = Vec::with_capacity(indexed_rows.len());
    for (index, value) in indexed_rows.values().enumerate() {
        if index == 0 || (index + 1) % 512 == 0 {
            deadline.check(
                "weave-loom",
                "coverage.base_page_index_readback",
                index as u64,
            )?;
        }
        candidates.push(encode::decode_constellation_base(value)?);
    }

    let mut slot_maps = BTreeMap::new();
    let mut coverage = Vec::new();
    let candidate_rows = candidates.len();
    for (slot_index, &slot) in content_slots.iter().enumerate() {
        let mut map = HashMap::new();
        let mut non_dense_rows = 0usize;
        let keys = candidates
            .iter()
            .map(|cx| (slot_key(cx.cx_id), cx.provenance.seq))
            .collect::<Vec<_>>();
        let slot_rows =
            crate::cf_read::latest_cf_rows_near_seqs(vault_dir, ColumnFamily::slot(slot), &keys)
                .map_err(|error| {
                    CliError::io(format!(
                        "weave-loom dense coverage grouped readback failed for slot {slot}: {error}"
                    ))
                })?;
        for (candidate_index, cx) in candidates.iter().enumerate() {
            let processed = (slot_index * candidate_rows + candidate_index) as u64;
            if candidate_index == 0 || (candidate_index + 1) % 256 == 0 {
                deadline.check("weave-loom", "coverage.slot_point_read", processed)?;
            }
            let Some(Some(bytes)) = slot_rows.get(slot_key(cx.cx_id).as_slice()) else {
                continue;
            };
            match encode::decode_slot_vector(bytes)? {
                SlotVector::Dense { data, .. } => {
                    map.insert(cx.cx_id, data);
                }
                SlotVector::Absent { .. } => {}
                _ => {
                    non_dense_rows += 1;
                }
            }
        }
        let dense_rows = map.len();
        let missing_rows = candidate_rows.saturating_sub(dense_rows);
        let example_missing_cx_ids = candidates
            .iter()
            .filter(|cx| !map.contains_key(&cx.cx_id))
            .take(EXAMPLE_MISSING_LIMIT)
            .map(|cx| cx.cx_id.to_string())
            .collect();
        coverage.push(DenseSlotCoverage {
            slot_id: slot.get(),
            candidate_rows,
            dense_rows,
            missing_rows,
            non_dense_rows,
            example_missing_cx_ids,
        });
        slot_maps.insert(slot, map);
    }

    Ok(DenseSlotCoverageScan {
        constellations_in_vault: manifest.live_entries,
        candidate_scan_rows: candidates.len(),
        scanned_candidates: candidates,
        slot_maps,
        coverage,
        base_page_index_live_entries: manifest.live_entries,
    })
}

pub(super) fn select_slot_from_coverage(
    requested: Option<SlotId>,
    mode: CandidateSelectionMode,
    limit: usize,
    coverage: &[DenseSlotCoverage],
) -> Result<SlotSelection, String> {
    let candidate_rows = coverage.first().map(|row| row.candidate_rows).unwrap_or(0);
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
                scanned_rows: candidate_rows,
                selected_rows: candidate_rows,
                requested_limit: limit,
                excluded_uncovered_rows: 0,
            });
        }
        return Err(format!(
            "CALYX_WEAVE_LOOM_DENSE_COVERAGE_INCOMPLETE: requested content slot {} covers {}/{} candidate rows; missing_rows={}; non_dense_rows={}; example_missing_cx_ids={:?}",
            row.slot_id,
            row.dense_rows,
            row.candidate_rows,
            row.missing_rows,
            row.non_dense_rows,
            row.example_missing_cx_ids
        ));
    }

    if let Some(row) = coverage.iter().find(|row| row.has_full_coverage()) {
        return Ok(SlotSelection {
            slot: SlotId::new(row.slot_id),
            reason: "lowest_slot_with_full_candidate_coverage",
            mode: mode.as_str(),
            scanned_rows: candidate_rows,
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
    let candidate_rows = coverage.first().map(|row| row.candidate_rows).unwrap_or(0);
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
                scanned_rows: candidate_rows,
                selected_rows,
                requested_limit: limit,
                excluded_uncovered_rows: row.missing_rows + row.non_dense_rows,
            });
        }
        return Err(format!(
            "CALYX_WEAVE_LOOM_COVERED_SET_TOO_SMALL: requested content slot {} has only {} dense candidate rows after scanning {}; selected_rows={}; limit={}; missing_rows={}; non_dense_rows={}; example_missing_cx_ids={:?}",
            row.slot_id,
            row.dense_rows,
            row.candidate_rows,
            selected_rows,
            limit,
            row.missing_rows,
            row.non_dense_rows,
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
            scanned_rows: candidate_rows,
            selected_rows,
            requested_limit: limit,
            excluded_uncovered_rows: row.missing_rows + row.non_dense_rows,
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
                "slot {} dense={}/{} missing={} non_dense={} examples={:?}",
                row.slot_id,
                row.dense_rows,
                row.candidate_rows,
                row.missing_rows,
                row.non_dense_rows,
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
