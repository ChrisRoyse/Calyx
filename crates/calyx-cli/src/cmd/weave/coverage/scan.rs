use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use calyx_aster::base_page_index::{
    read_base_page_index_manifest, read_indexed_base_rows, visit_indexed_base_row_pages,
};
use calyx_aster::cf::{ColumnFamily, slot_key};
use calyx_aster::mvcc::is_tombstone_value;
use calyx_aster::vault::encode;
use calyx_core::{CalyxError, Constellation, CxId, SlotId, SlotVector};

use super::{
    COVERED_SCAN_BATCH_SIZE, CandidateSelectionMode, DenseSlotCoverage, DenseSlotCoverageScan,
    EXAMPLE_MISSING_LIMIT,
};
use crate::bounded_progress::Deadline;
use crate::error::{CliError, CliResult};
use crate::provenance_read::VaultReadContext;

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
    let mut read_context = VaultReadContext::new(vault_dir);
    match mode {
        CandidateSelectionMode::BasePrefix => scan_base_prefix_coverage(
            vault_dir,
            &mut read_context,
            content_slots,
            limit,
            manifest.live_entries,
            deadline,
        ),
        CandidateSelectionMode::Covered => scan_bounded_covered_coverage(
            vault_dir,
            &mut read_context,
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
    read_context: &mut VaultReadContext,
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
    let (slot_maps, coverage) = scan_slots_for_candidates(
        vault_dir,
        read_context,
        content_slots,
        &candidates,
        deadline,
    )?;
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
    read_context: &mut VaultReadContext,
    requested_slot: Option<SlotId>,
    content_slots: &[SlotId],
    limit: usize,
    live_entries: usize,
    deadline: &Deadline,
) -> CliResult<DenseSlotCoverageScan> {
    if requested_slot.is_none() && limit > 0 {
        return scan_auto_bounded_covered_coverage(
            vault_dir,
            read_context,
            content_slots,
            limit,
            live_entries,
            deadline,
        );
    }
    let measured_slots = requested_slot.map_or_else(|| content_slots.to_vec(), |slot| vec![slot]);
    scan_covered_slots(
        vault_dir,
        read_context,
        &measured_slots,
        limit,
        live_entries,
        deadline,
    )
}

fn scan_auto_bounded_covered_coverage(
    vault_dir: &Path,
    read_context: &mut VaultReadContext,
    content_slots: &[SlotId],
    limit: usize,
    live_entries: usize,
    deadline: &Deadline,
) -> CliResult<DenseSlotCoverageScan> {
    let target_rows = limit.max(2);
    let mut measured_coverage = Vec::new();
    let mut last_scan = None;
    for &slot in content_slots {
        let mut scan = scan_covered_slots(
            vault_dir,
            read_context,
            &[slot],
            limit,
            live_entries,
            deadline,
        )?;
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
    read_context: &mut VaultReadContext,
    measured_slots: &[SlotId],
    limit: usize,
    live_entries: usize,
    deadline: &Deadline,
) -> CliResult<DenseSlotCoverageScan> {
    let target_rows = if limit == 0 { usize::MAX } else { limit.max(2) };
    let mut candidates = Vec::new();
    let mut accumulators = measured_slots
        .iter()
        .map(|&slot| (slot, SlotAccumulator::default()))
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
                let accumulator = accumulators.get_mut(&slot).expect("slot accumulator");
                classify_chunk(
                    vault_dir,
                    read_context,
                    slot,
                    &chunk,
                    accumulator,
                    deadline,
                    (slot_index * candidates.len()) as u64,
                )?;
            }
            candidates.extend(chunk);
            if target_rows != usize::MAX
                && accumulators
                    .values()
                    .any(|accumulator| accumulator.map.len() >= target_rows)
            {
                stopped_after_target = true;
                return Ok(false);
            }
        }
        Ok(true)
    })?;

    let mut slot_maps = BTreeMap::new();
    let mut coverage = Vec::new();
    for &slot in measured_slots {
        let accumulator = accumulators.remove(&slot).expect("slot accumulator");
        let (map, row) = summarize_slot_coverage(slot, candidates.len(), accumulator)?;
        slot_maps.insert(slot, map);
        coverage.push(row);
    }
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
    read_context: &mut VaultReadContext,
    content_slots: &[SlotId],
    candidates: &[Constellation],
    deadline: &Deadline,
) -> CliResult<(SlotCoverageMaps, SlotCoverageRows)> {
    let mut slot_maps = BTreeMap::new();
    let mut coverage = Vec::new();
    for (slot_index, &slot) in content_slots.iter().enumerate() {
        let mut accumulator = SlotAccumulator::default();
        classify_chunk(
            vault_dir,
            read_context,
            slot,
            candidates,
            &mut accumulator,
            deadline,
            (slot_index * candidates.len()) as u64,
        )?;
        let (map, row) = summarize_slot_coverage(slot, candidates.len(), accumulator)?;
        slot_maps.insert(slot, map);
        coverage.push(row);
    }
    Ok((slot_maps, coverage))
}

/// Per-slot classification state. Every candidate lands in exactly one
/// bucket, so the buckets always sum to the candidate count.
#[derive(Default)]
struct SlotAccumulator {
    map: HashMap<CxId, Vec<f32>>,
    non_dense_rows: usize,
    absent_rows: usize,
    tombstoned_rows: usize,
    missing_rows: usize,
    example_missing_cx_ids: Vec<String>,
    read_stats: crate::provenance_read::ProvenanceReadStats,
}

/// Reads and classifies one chunk of candidates against one slot CF. A
/// candidate whose base row lists the slot but has no physical slot row in
/// any resolution stage fails closed as `CALYX_ASTER_CORRUPT_SHARD` — it is
/// never silently counted as missing coverage (issue #1096).
fn classify_chunk(
    vault_dir: &Path,
    read_context: &mut VaultReadContext,
    slot: SlotId,
    chunk: &[Constellation],
    accumulator: &mut SlotAccumulator,
    deadline: &Deadline,
    processed_offset: u64,
) -> CliResult<()> {
    if chunk.is_empty() {
        return Ok(());
    }
    let keys = chunk
        .iter()
        .map(|cx| (slot_key(cx.cx_id), cx.provenance.seq))
        .collect::<Vec<_>>();
    let batch = read_context
        .latest_cf_rows_for_provenance(ColumnFamily::slot(slot), &keys)
        .map_err(|error| {
            CliError::io(format!(
                "weave-loom dense coverage grouped readback failed for slot {slot}: {error}"
            ))
        })?;
    accumulator.read_stats.accumulate(batch.stats);
    for (candidate_index, cx) in chunk.iter().enumerate() {
        if candidate_index == 0 || (candidate_index + 1) % 256 == 0 {
            deadline.check(
                "weave-loom",
                "coverage.slot_point_read",
                processed_offset + candidate_index as u64,
            )?;
        }
        let key = slot_key(cx.cx_id);
        let Some(Some(resolved)) = batch.rows.get(key.as_slice()) else {
            if cx.slots.contains_key(&slot) {
                return Err(missing_listed_slot_row_error(
                    vault_dir,
                    cx,
                    slot,
                    &accumulator.read_stats,
                ));
            }
            accumulator.missing_rows += 1;
            if accumulator.example_missing_cx_ids.len() < EXAMPLE_MISSING_LIMIT {
                accumulator
                    .example_missing_cx_ids
                    .push(cx.cx_id.to_string());
            }
            continue;
        };
        if is_tombstone_value(&resolved.value) {
            accumulator.tombstoned_rows += 1;
            continue;
        }
        match encode::decode_slot_vector(&resolved.value)? {
            SlotVector::Dense { data, .. } => {
                accumulator.map.insert(cx.cx_id, data);
            }
            SlotVector::Absent { .. } => accumulator.absent_rows += 1,
            _ => accumulator.non_dense_rows += 1,
        }
    }
    Ok(())
}

fn missing_listed_slot_row_error(
    vault_dir: &Path,
    cx: &Constellation,
    slot: SlotId,
    read_stats: &crate::provenance_read::ProvenanceReadStats,
) -> CliError {
    CalyxError::aster_corrupt_shard(format!(
        "weave-loom dense coverage fail-closed: base row for cx {} in {} lists slot {} \
         (provenance seq {}) but no physical slot row exists in the commit batch, the full \
         SST level, or the WAL tail; read stats so far: {:?}",
        cx.cx_id,
        vault_dir.display(),
        slot.get(),
        cx.provenance.seq,
        read_stats,
    ))
    .into()
}

fn summarize_slot_coverage(
    slot: SlotId,
    candidate_rows: usize,
    accumulator: SlotAccumulator,
) -> CliResult<(HashMap<CxId, Vec<f32>>, DenseSlotCoverage)> {
    let dense_rows = accumulator.map.len();
    let classified = dense_rows
        + accumulator.non_dense_rows
        + accumulator.absent_rows
        + accumulator.tombstoned_rows
        + accumulator.missing_rows;
    if classified != candidate_rows {
        return Err(CliError::io(format!(
            "weave-loom dense coverage accounting bug for slot {}: {classified} classified rows \
             != {candidate_rows} candidate rows (dense={dense_rows} non_dense={} absent={} \
             tombstoned={} missing={})",
            slot.get(),
            accumulator.non_dense_rows,
            accumulator.absent_rows,
            accumulator.tombstoned_rows,
            accumulator.missing_rows,
        )));
    }
    let row = DenseSlotCoverage {
        slot_id: slot.get(),
        candidate_rows,
        dense_rows,
        missing_rows: accumulator.missing_rows,
        non_dense_rows: accumulator.non_dense_rows,
        absent_rows: accumulator.absent_rows,
        tombstoned_rows: accumulator.tombstoned_rows,
        example_missing_cx_ids: accumulator.example_missing_cx_ids,
        read_stats: accumulator.read_stats,
    };
    Ok((accumulator.map, row))
}
