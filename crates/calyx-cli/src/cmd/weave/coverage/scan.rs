use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use calyx_aster::base_page_index::{
    read_base_page_index_manifest, read_indexed_base_rows, visit_indexed_base_row_pages,
};
use calyx_aster::cf::{ColumnFamily, slot_key};
use calyx_aster::vault::encode;
use calyx_core::{Constellation, CxId, SlotId, SlotVector};

use super::{
    COVERED_SCAN_BATCH_SIZE, CandidateSelectionMode, DenseSlotCoverage, DenseSlotCoverageScan,
    EXAMPLE_MISSING_LIMIT,
};
use crate::bounded_progress::Deadline;
use crate::error::{CliError, CliResult};

type SlotCoverageMaps = BTreeMap<SlotId, HashMap<CxId, Vec<f32>>>;
type SlotCoverageRows = Vec<DenseSlotCoverage>;

pub(crate) fn scan_dense_slot_coverage(
    vault_dir: &Path,
    content_slots: &[SlotId],
    requested_slot: Option<SlotId>,
    limit: usize,
    mode: CandidateSelectionMode,
    deadline: &Deadline,
) -> CliResult<DenseSlotCoverageScan> {
    deadline.check("weave-loom", "coverage.base_page_index_manifest", 0)?;
    let manifest = read_base_page_index_manifest(vault_dir)?;
    match mode {
        CandidateSelectionMode::BasePrefix => scan_base_prefix_coverage(
            vault_dir,
            content_slots,
            limit,
            manifest.live_entries,
            deadline,
        ),
        CandidateSelectionMode::Covered => scan_bounded_covered_coverage(
            vault_dir,
            requested_slot,
            content_slots,
            limit,
            manifest.live_entries,
            deadline,
        ),
    }
}

fn scan_base_prefix_coverage(
    vault_dir: &Path,
    content_slots: &[SlotId],
    limit: usize,
    live_entries: usize,
    deadline: &Deadline,
) -> CliResult<DenseSlotCoverageScan> {
    let candidate_limit = if limit == 0 {
        live_entries
    } else {
        limit.min(live_entries)
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
    let (slot_maps, coverage) =
        scan_slots_for_candidates(vault_dir, content_slots, &candidates, deadline)?;
    Ok(DenseSlotCoverageScan {
        constellations_in_vault: live_entries,
        candidate_scan_rows: candidates.len(),
        candidate_scan_complete: candidates.len() == live_entries,
        scanned_candidates: candidates,
        slot_maps,
        coverage,
        base_page_index_live_entries: live_entries,
    })
}

fn scan_bounded_covered_coverage(
    vault_dir: &Path,
    requested_slot: Option<SlotId>,
    content_slots: &[SlotId],
    limit: usize,
    live_entries: usize,
    deadline: &Deadline,
) -> CliResult<DenseSlotCoverageScan> {
    if requested_slot.is_none() && limit > 0 {
        return scan_auto_bounded_covered_coverage(
            vault_dir,
            content_slots,
            limit,
            live_entries,
            deadline,
        );
    }
    let measured_slots = requested_slot.map_or_else(|| content_slots.to_vec(), |slot| vec![slot]);
    scan_covered_slots(vault_dir, &measured_slots, limit, live_entries, deadline)
}

fn scan_auto_bounded_covered_coverage(
    vault_dir: &Path,
    content_slots: &[SlotId],
    limit: usize,
    live_entries: usize,
    deadline: &Deadline,
) -> CliResult<DenseSlotCoverageScan> {
    let target_rows = limit.max(2);
    let mut measured_coverage = Vec::new();
    let mut last_scan = None;
    for &slot in content_slots {
        let mut scan = scan_covered_slots(vault_dir, &[slot], limit, live_entries, deadline)?;
        let row = scan.coverage.remove(0);
        let reached_target = row.dense_rows >= target_rows;
        measured_coverage.push(row);
        if reached_target {
            scan.coverage = measured_coverage;
            return Ok(scan);
        }
        last_scan = Some(scan);
    }
    let mut scan = last_scan.unwrap_or(DenseSlotCoverageScan {
        constellations_in_vault: live_entries,
        scanned_candidates: Vec::new(),
        slot_maps: BTreeMap::new(),
        coverage: Vec::new(),
        base_page_index_live_entries: live_entries,
        candidate_scan_rows: 0,
        candidate_scan_complete: true,
    });
    scan.coverage = measured_coverage;
    Ok(scan)
}

fn scan_covered_slots(
    vault_dir: &Path,
    measured_slots: &[SlotId],
    limit: usize,
    live_entries: usize,
    deadline: &Deadline,
) -> CliResult<DenseSlotCoverageScan> {
    let target_rows = if limit == 0 { usize::MAX } else { limit.max(2) };
    let mut candidates = Vec::new();
    let mut slot_maps = measured_slots
        .iter()
        .map(|&slot| (slot, HashMap::new()))
        .collect::<BTreeMap<_, _>>();
    let mut non_dense_rows = measured_slots
        .iter()
        .map(|&slot| (slot, 0usize))
        .collect::<BTreeMap<_, _>>();
    let mut stopped_after_target = false;

    visit_indexed_base_row_pages(vault_dir, |_, rows| -> CliResult<bool> {
        for row_chunk in rows.chunks(COVERED_SCAN_BATCH_SIZE) {
            let mut chunk = Vec::with_capacity(row_chunk.len());
            for (_, value) in row_chunk {
                let index = candidates.len() + chunk.len();
                if index == 0 || (index + 1) % 512 == 0 {
                    deadline.check(
                        "weave-loom",
                        "coverage.base_page_index_readback",
                        index as u64,
                    )?;
                }
                chunk.push(encode::decode_constellation_base(value)?);
            }
            for (slot_index, &slot) in measured_slots.iter().enumerate() {
                let keys = chunk
                    .iter()
                    .map(|cx| (slot_key(cx.cx_id), cx.provenance.seq))
                    .collect::<Vec<_>>();
                let slot_rows = crate::cf_read::latest_cf_rows_near_seqs(
                    vault_dir,
                    ColumnFamily::slot(slot),
                    &keys,
                )
                .map_err(|error| {
                    CliError::io(format!(
                        "weave-loom dense coverage grouped readback failed for slot {slot}: {error}"
                    ))
                })?;
                for (candidate_index, cx) in chunk.iter().enumerate() {
                    let processed = (slot_index * candidates.len() + candidate_index) as u64;
                    if candidate_index == 0 || (candidate_index + 1) % 256 == 0 {
                        deadline.check("weave-loom", "coverage.slot_point_read", processed)?;
                    }
                    let Some(Some(bytes)) = slot_rows.get(slot_key(cx.cx_id).as_slice()) else {
                        continue;
                    };
                    match encode::decode_slot_vector(bytes)? {
                        SlotVector::Dense { data, .. } => {
                            slot_maps
                                .get_mut(&slot)
                                .expect("slot accumulator")
                                .insert(cx.cx_id, data);
                        }
                        SlotVector::Absent { .. } => {}
                        _ => *non_dense_rows.get_mut(&slot).expect("slot accumulator") += 1,
                    }
                }
            }
            candidates.extend(chunk);
            if target_rows != usize::MAX
                && measured_slots
                    .iter()
                    .any(|slot| slot_maps.get(slot).map_or(0, HashMap::len) >= target_rows)
            {
                stopped_after_target = true;
                return Ok(false);
            }
        }
        Ok(true)
    })?;

    let coverage = measured_slots
        .iter()
        .map(|&slot| {
            summarize_slot_coverage(
                slot,
                candidates.len(),
                *non_dense_rows.get(&slot).unwrap_or(&0),
                slot_maps.get(&slot).expect("slot accumulator"),
                &candidates,
            )
        })
        .collect::<Vec<_>>();
    Ok(DenseSlotCoverageScan {
        constellations_in_vault: live_entries,
        candidate_scan_rows: candidates.len(),
        candidate_scan_complete: !stopped_after_target,
        scanned_candidates: candidates,
        slot_maps,
        coverage,
        base_page_index_live_entries: live_entries,
    })
}

fn scan_slots_for_candidates(
    vault_dir: &Path,
    content_slots: &[SlotId],
    candidates: &[Constellation],
    deadline: &Deadline,
) -> CliResult<(SlotCoverageMaps, SlotCoverageRows)> {
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
                _ => non_dense_rows += 1,
            }
        }
        coverage.push(summarize_slot_coverage(
            slot,
            candidate_rows,
            non_dense_rows,
            &map,
            candidates,
        ));
        slot_maps.insert(slot, map);
    }
    Ok((slot_maps, coverage))
}

fn summarize_slot_coverage(
    slot: SlotId,
    candidate_rows: usize,
    non_dense_rows: usize,
    map: &HashMap<CxId, Vec<f32>>,
    candidates: &[Constellation],
) -> DenseSlotCoverage {
    let dense_rows = map.len();
    let missing_rows = candidate_rows.saturating_sub(dense_rows + non_dense_rows);
    let example_missing_cx_ids = candidates
        .iter()
        .filter(|cx| !map.contains_key(&cx.cx_id))
        .take(EXAMPLE_MISSING_LIMIT)
        .map(|cx| cx.cx_id.to_string())
        .collect();
    DenseSlotCoverage {
        slot_id: slot.get(),
        candidate_rows,
        dense_rows,
        missing_rows,
        non_dense_rows,
        example_missing_cx_ids,
    }
}
