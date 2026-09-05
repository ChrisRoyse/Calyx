use std::collections::BTreeMap;
use std::path::Path;

use calyx_aster::base_page_index::{
    read_base_page_index_manifest, read_indexed_base_rows, visit_indexed_base_row_pages,
};
use calyx_aster::cf::{ColumnFamily, slot_key};
use calyx_aster::mvcc::is_tombstone_value;
use calyx_aster::vault::encode;
use calyx_core::{CalyxError, SlotId, SlotVector};

use super::super::scratch::{WeaveVectorScratch, WeaveVectorScratchWriter};
use super::{
    COVERED_SCAN_BATCH_SIZE, CandidateSelectionMode, DenseSlotCoverage, DenseSlotCoverageScan,
    EXAMPLE_MISSING_LIMIT, WeaveScanNode,
};
use crate::bounded_progress::Deadline;
use crate::error::{CliError, CliResult};
use crate::provenance_read::VaultReadContext;

type SlotCoverageRows = Vec<DenseSlotCoverage>;

/// Mutable state threaded through the scan: the out-of-core scratch writer every
/// measured vector is streamed into (#1991), plus the per-ordinal count of how
/// many contracted slots each candidate is present in (drives the complete-panel
/// intersection without holding any decoded vector resident).
struct ScanSink {
    scratch: WeaveVectorScratchWriter,
    present_slot_counts: Vec<u32>,
}

impl ScanSink {
    fn ensure_ordinal(&mut self, ordinal: u64) {
        let needed = ordinal as usize + 1;
        if self.present_slot_counts.len() < needed {
            self.present_slot_counts.resize(needed, 0);
        }
    }
}

pub(crate) fn scan_dense_slot_coverage(
    vault_dir: &Path,
    run_dir: &Path,
    content_slots: &[SlotId],
    requested_slot: Option<SlotId>,
    limit: usize,
    mode: CandidateSelectionMode,
    deadline: &Deadline,
) -> CliResult<DenseSlotCoverageScan> {
    deadline.check("weave-loom", "coverage.base_page_index_manifest", 0)?;
    let manifest = read_base_page_index_manifest(vault_dir)?;
    let mut read_context = VaultReadContext::new(vault_dir);
    let sink = ScanSink {
        scratch: WeaveVectorScratchWriter::create(run_dir, content_slots)?,
        present_slot_counts: Vec::new(),
    };
    match mode {
        CandidateSelectionMode::BasePrefix => scan_base_prefix_coverage(
            vault_dir,
            &mut read_context,
            sink,
            content_slots,
            limit,
            manifest.live_entries,
            deadline,
        ),
        CandidateSelectionMode::Covered => {
            // `requested_slot` is a covered-mode legacy knob the panel-native
            // scan no longer honours (it measures every contracted slot); accept
            // it at the entry point for CLI compatibility and drop it here.
            let _ = requested_slot;
            scan_bounded_covered_coverage(
                vault_dir,
                &mut read_context,
                sink,
                content_slots,
                limit,
                manifest.live_entries,
                deadline,
            )
        }
    }
}

fn finish_scan(
    sink: ScanSink,
    content_slots: &[SlotId],
    candidates: Vec<WeaveScanNode>,
    coverage: SlotCoverageRows,
    live_entries: usize,
    candidate_scan_complete: bool,
) -> CliResult<DenseSlotCoverageScan> {
    let node_count = candidates.len() as u64;
    let ScanSink {
        scratch,
        mut present_slot_counts,
    } = sink;
    present_slot_counts.resize(candidates.len(), 0);
    let scratch: WeaveVectorScratch = scratch.finish(node_count)?;
    Ok(DenseSlotCoverageScan {
        constellations_in_vault: live_entries,
        candidate_scan_rows: candidates.len(),
        candidate_scan_complete,
        scanned_candidates: candidates,
        scanned_slots: content_slots.to_vec(),
        present_slot_counts,
        scratch,
        coverage,
        base_page_index_live_entries: live_entries,
    })
}

fn scan_base_prefix_coverage(
    vault_dir: &Path,
    read_context: &mut VaultReadContext,
    mut sink: ScanSink,
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
        candidates.push(encode::decode_constellation_base(value)?.into());
    }
    sink.present_slot_counts.resize(candidates.len(), 0);
    let coverage = scan_slots_for_candidates(
        vault_dir,
        read_context,
        &mut sink,
        content_slots,
        &candidates,
        deadline,
    )?;
    let candidate_scan_complete = candidates.len() == live_entries;
    finish_scan(
        sink,
        content_slots,
        candidates,
        coverage,
        live_entries,
        candidate_scan_complete,
    )
}

fn scan_bounded_covered_coverage(
    vault_dir: &Path,
    read_context: &mut VaultReadContext,
    sink: ScanSink,
    content_slots: &[SlotId],
    limit: usize,
    live_entries: usize,
    deadline: &Deadline,
) -> CliResult<DenseSlotCoverageScan> {
    scan_covered_slots(
        vault_dir,
        read_context,
        sink,
        content_slots,
        limit,
        live_entries,
        deadline,
    )
}

fn scan_covered_slots(
    vault_dir: &Path,
    read_context: &mut VaultReadContext,
    mut sink: ScanSink,
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
            let ordinal_base = candidates.len() as u64;
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
                chunk.push(WeaveScanNode::from(encode::decode_constellation_base(
                    value,
                )?));
            }
            for (slot_index, &slot) in measured_slots.iter().enumerate() {
                let accumulator = accumulators.get_mut(&slot).expect("slot accumulator");
                classify_chunk(
                    vault_dir,
                    read_context,
                    slot,
                    &chunk,
                    accumulator,
                    &mut sink,
                    deadline,
                    ordinal_base,
                    (slot_index * candidates.len()) as u64,
                )?;
            }
            candidates.extend(chunk);
            if target_rows != usize::MAX
                && common_measured_count(candidates.len(), &sink, measured_slots) >= target_rows
            {
                stopped_after_target = true;
                return Ok(false);
            }
        }
        Ok(true)
    })?;

    let mut coverage = Vec::new();
    for &slot in measured_slots {
        let accumulator = accumulators.remove(&slot).expect("slot accumulator");
        coverage.push(summarize_slot_coverage(
            slot,
            candidates.len(),
            accumulator,
        )?);
    }
    finish_scan(
        sink,
        measured_slots,
        candidates,
        coverage,
        live_entries,
        !stopped_after_target,
    )
}

/// Number of scanned candidates present in *every* measured slot so far — the
/// same "complete panel" intersection the old per-slot maps expressed, computed
/// from the per-ordinal present-slot counts instead of holding vectors resident.
fn common_measured_count(scanned: usize, sink: &ScanSink, measured_slots: &[SlotId]) -> usize {
    let full = measured_slots.len() as u32;
    sink.present_slot_counts
        .iter()
        .take(scanned)
        .filter(|&&count| count == full)
        .count()
}

/// Per-slot classification state. Every candidate lands in exactly one
/// bucket, so the buckets always sum to the candidate count.
#[derive(Default)]
struct SlotAccumulator {
    dense_rows: usize,
    sparse_rows: usize,
    multi_rows: usize,
    segmented_multi_rows: usize,
    absent_rows: usize,
    tombstoned_rows: usize,
    missing_rows: usize,
    example_missing_cx_ids: Vec<String>,
    read_stats: crate::provenance_read::ProvenanceReadStats,
}

/// Bounded per-slot read chunk for the base-prefix scan (#1991). The prior code
/// passed the ENTIRE candidate set (109,133 rows on the canonical law vault) to a
/// single `latest_cf_rows_for_provenance` call per slot, which materialized every
/// row's encoded value for that slot into one transient `BTreeMap` (~1.4 GB for a
/// chunk-Multi lane). Reading in fixed-size chunks caps that transient read
/// buffer — and the decode churn's allocator high-water — to
/// `chunk x per-row-bytes`, independent of the corpus size.
const BASE_PREFIX_SLOT_READ_CHUNK: usize = 4096;

fn scan_slots_for_candidates(
    vault_dir: &Path,
    read_context: &mut VaultReadContext,
    sink: &mut ScanSink,
    content_slots: &[SlotId],
    candidates: &[WeaveScanNode],
    deadline: &Deadline,
) -> CliResult<SlotCoverageRows> {
    let mut coverage = Vec::new();
    for (slot_index, &slot) in content_slots.iter().enumerate() {
        let mut accumulator = SlotAccumulator::default();
        let slot_offset = (slot_index * candidates.len()) as u64;
        for (chunk_index, chunk) in candidates.chunks(BASE_PREFIX_SLOT_READ_CHUNK).enumerate() {
            let ordinal_base = (chunk_index * BASE_PREFIX_SLOT_READ_CHUNK) as u64;
            classify_chunk(
                vault_dir,
                read_context,
                slot,
                chunk,
                &mut accumulator,
                sink,
                deadline,
                ordinal_base,
                slot_offset + ordinal_base,
            )?;
        }
        coverage.push(summarize_slot_coverage(
            slot,
            candidates.len(),
            accumulator,
        )?);
    }
    Ok(coverage)
}

/// Reads and classifies one chunk of candidates against one slot CF, streaming
/// each measured vector's canonical encoded bytes into the out-of-core scratch
/// store keyed by scan ordinal (`ordinal_base + position`). A candidate whose
/// base row lists the slot but has no physical slot row in any resolution stage
/// fails closed as `CALYX_ASTER_CORRUPT_SHARD` — it is never silently counted as
/// missing coverage (issue #1096).
#[allow(clippy::too_many_arguments)]
fn classify_chunk(
    vault_dir: &Path,
    read_context: &mut VaultReadContext,
    slot: SlotId,
    chunk: &[WeaveScanNode],
    accumulator: &mut SlotAccumulator,
    sink: &mut ScanSink,
    deadline: &Deadline,
    ordinal_base: u64,
    processed_offset: u64,
) -> CliResult<()> {
    if chunk.is_empty() {
        return Ok(());
    }
    let keys = chunk
        .iter()
        .map(|cx| (slot_key(cx.meta.cx_id), cx.provenance_seq))
        .collect::<Vec<_>>();
    let batch = read_context.latest_cf_rows_for_provenance(ColumnFamily::slot(slot), &keys)?;
    accumulator.read_stats.accumulate(batch.stats);
    for (candidate_index, cx) in chunk.iter().enumerate() {
        if candidate_index == 0 || (candidate_index + 1) % 256 == 0 {
            deadline.check(
                "weave-loom",
                "coverage.slot_point_read",
                processed_offset + candidate_index as u64,
            )?;
        }
        let ordinal = ordinal_base + candidate_index as u64;
        sink.ensure_ordinal(ordinal);
        let key = slot_key(cx.meta.cx_id);
        let Some(Some(resolved)) = batch.rows.get(key.as_slice()) else {
            if cx.listed_slots.binary_search(&slot).is_ok() {
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
                    .push(cx.meta.cx_id.to_string());
            }
            sink.scratch.push(slot, ordinal, None)?;
            continue;
        };
        if is_tombstone_value(&resolved.value) {
            accumulator.tombstoned_rows += 1;
            sink.scratch.push(slot, ordinal, None)?;
            continue;
        }
        match encode::decode_slot_vector(&resolved.value)? {
            SlotVector::Absent { .. } => {
                accumulator.absent_rows += 1;
                sink.scratch.push(slot, ordinal, None)?;
            }
            vector => {
                vector.validate_schema()?;
                match &vector {
                    SlotVector::Dense { .. } => accumulator.dense_rows += 1,
                    SlotVector::Sparse { .. } => accumulator.sparse_rows += 1,
                    SlotVector::Multi { .. } => accumulator.multi_rows += 1,
                    SlotVector::SegmentedMulti { .. } => accumulator.segmented_multi_rows += 1,
                    SlotVector::Absent { .. } => unreachable!("handled above"),
                }
                // Stream the canonical on-disk encoding (the exact bytes read
                // from the slot CF) into the scratch store — residence changes,
                // bytes do not, so #1960 evidence stays byte-identical.
                sink.scratch.push(slot, ordinal, Some(&resolved.value))?;
                sink.present_slot_counts[ordinal as usize] += 1;
            }
        }
    }
    Ok(())
}

fn missing_listed_slot_row_error(
    vault_dir: &Path,
    cx: &WeaveScanNode,
    slot: SlotId,
    read_stats: &crate::provenance_read::ProvenanceReadStats,
) -> CliError {
    CalyxError::aster_corrupt_shard(format!(
        "weave-loom typed coverage fail-closed: base row for cx {} in {} lists slot {} \
         (provenance seq {}) but no physical slot row exists in the commit batch, the full \
         SST level, or the WAL tail; read stats so far: {:?}",
        cx.meta.cx_id,
        vault_dir.display(),
        slot.get(),
        cx.provenance_seq,
        read_stats,
    ))
    .into()
}

fn summarize_slot_coverage(
    slot: SlotId,
    candidate_rows: usize,
    accumulator: SlotAccumulator,
) -> CliResult<DenseSlotCoverage> {
    let measured_rows = accumulator.dense_rows
        + accumulator.sparse_rows
        + accumulator.multi_rows
        + accumulator.segmented_multi_rows;
    let classified = measured_rows
        + accumulator.absent_rows
        + accumulator.tombstoned_rows
        + accumulator.missing_rows;
    if classified != candidate_rows {
        return Err(CliError::io(format!(
            "weave-loom typed coverage accounting bug for slot {}: {classified} classified rows \
             != {candidate_rows} candidate rows (measured={measured_rows} dense={} sparse={} \
             multi={} segmented_multi={} absent={} tombstoned={} missing={})",
            slot.get(),
            accumulator.dense_rows,
            accumulator.sparse_rows,
            accumulator.multi_rows,
            accumulator.segmented_multi_rows,
            accumulator.absent_rows,
            accumulator.tombstoned_rows,
            accumulator.missing_rows,
        )));
    }
    Ok(DenseSlotCoverage {
        slot_id: slot.get(),
        candidate_rows,
        dense_rows: accumulator.dense_rows,
        sparse_rows: accumulator.sparse_rows,
        multi_rows: accumulator.multi_rows,
        segmented_multi_rows: accumulator.segmented_multi_rows,
        measured_rows,
        missing_rows: accumulator.missing_rows,
        absent_rows: accumulator.absent_rows,
        tombstoned_rows: accumulator.tombstoned_rows,
        example_missing_cx_ids: accumulator.example_missing_cx_ids,
        read_stats: accumulator.read_stats,
    })
}
