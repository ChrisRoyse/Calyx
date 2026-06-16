use super::{
    ANN_INDEX_CAP, DIM, GC_SWEEP_EVERY_GC_TICKS, KEY_SPACE, MIB, SoakCounts, SoakSample,
    WAL_BATCH_RECORDS, WAL_RECYCLE_EVERY_GC_TICKS, err,
};
use calyx_anneal::{BudgetEnforcer, BudgetProbe};
use calyx_aster::cf::{CfRouter, ColumnFamily};
use calyx_aster::gc::{
    CompactionGcReclaimer, CompactionGcTarget, TombstoneInventory, WalRecycler,
    scan_tombstone_inventory,
};
use calyx_aster::mvcc::tombstone_value;
use calyx_aster::resource::heap_rss_bytes;
use calyx_aster::wal::Wal;
use calyx_core::{CxId, Result as CalyxResult, SlotVector};
use calyx_forge::{Category, VramBudgeter, VramProbe};
use calyx_sextant::{HnswIndex, SextantIndex};
use std::fs;
use std::path::Path;

pub(super) struct WriteOpState<'a> {
    pub(super) router: &'a mut CfRouter,
    pub(super) index: &'a mut HnswIndex,
    pub(super) value: &'a mut [u8],
    pub(super) wal: &'a mut Wal,
    pub(super) wal_payloads: &'a mut Vec<Vec<u8>>,
    pub(super) durable_wal_seq: &'a mut u64,
    pub(super) live_values: &'a mut u64,
    pub(super) tombstone_values: &'a mut u64,
    pub(super) counts: &'a mut SoakCounts,
}

pub(super) fn write_op(op: u64, state: WriteOpState<'_>) -> Result<(), String> {
    let key = row_key(op);
    state.value[0] = op as u8;
    state.value[state.value.len() - 1] = (op >> 8) as u8;
    let row_len = 64 + (op as usize % 128);
    state
        .router
        .put(ColumnFamily::Base, &key, &state.value[..row_len])
        .map_err(err)?;
    *state.live_values += 1;
    if op.is_multiple_of(4) {
        let tombstone = tombstone_value();
        state
            .router
            .put(ColumnFamily::Base, &tombstone_key(op), &tombstone)
            .map_err(err)?;
        *state.tombstone_values += 1;
    }
    if state.index.live_len() < ANN_INDEX_CAP {
        state
            .index
            .insert(cx(op), dense_vector(op), op.saturating_add(1))
            .map_err(err)?;
    }
    state.wal_payloads.push(wal_payload(op, row_len));
    if state.wal_payloads.len() >= WAL_BATCH_RECORDS {
        flush_wal_batch(state.wal, state.wal_payloads, state.durable_wal_seq)?;
    }
    state.counts.writes += 1;
    Ok(())
}

pub(super) fn read_op(op: u64, router: &CfRouter, counts: &mut SoakCounts) -> Result<(), String> {
    let key = row_key(op.saturating_sub(1));
    let _ = router.get(ColumnFamily::Base, &key).map_err(err)?;
    counts.reads += 1;
    Ok(())
}

pub(super) fn ann_search_op(
    op: u64,
    index: &HnswIndex,
    counts: &mut SoakCounts,
) -> Result<(), String> {
    if index.live_len() > 0 {
        let _ = index.search(&dense_vector(op), 3, Some(8)).map_err(err)?;
    }
    counts.ann_searches += 1;
    Ok(())
}

pub(super) fn gc_tick_op(
    op: u64,
    vault_dir: &Path,
    router: &mut CfRouter,
    wal: &mut Wal,
    wal_recycler: &WalRecycler,
    durable_wal_seq: u64,
    counts: &mut SoakCounts,
) -> Result<(), String> {
    counts.gc_ticks += 1;
    if counts.gc_ticks.is_multiple_of(WAL_RECYCLE_EVERY_GC_TICKS) {
        let _ = wal_recycler.run_once_at(wal, durable_wal_seq, op);
    }
    if counts.gc_ticks.is_multiple_of(GC_SWEEP_EVERY_GC_TICKS) {
        router.flush_pending().map_err(err)?;
        let reclaimer = CompactionGcReclaimer::with_limits(0.5, 1, 1_000_000_000, op);
        let target = SoakGcTarget { vault_dir };
        let _ = reclaimer.maybe_trigger_at(&target, 1.0, op);
    }
    Ok(())
}

pub(super) fn vram_dispatch_op<P: VramProbe>(
    budgeter: &VramBudgeter<P>,
    counts: &mut SoakCounts,
) -> Result<(), String> {
    {
        let _guard = budgeter
            .reserve_category(MIB, Category::Serving)
            .map_err(err)?;
        let _readback = budgeter.stats().allocated_bytes;
    }
    counts.vram_dispatches += 1;
    Ok(())
}

pub(super) fn anneal_tick_op<P: BudgetProbe>(
    anneal: &BudgetEnforcer<'_, P>,
    counts: &mut SoakCounts,
) -> Result<(), String> {
    let status = anneal.tick().map_err(err)?;
    if status.handles_active != 0 {
        return Err("anneal budget handle leak detected".to_string());
    }
    counts.anneal_ticks += 1;
    Ok(())
}

pub(super) fn sample<P: VramProbe>(
    op: u64,
    budgeter: &VramBudgeter<P>,
    wal_dir: &Path,
    tombstone_ratio: f64,
    oldest_pinned_seq_gap: u64,
) -> Result<SoakSample, String> {
    Ok(SoakSample {
        op,
        rss_kib: heap_rss_bytes().map_err(err)? / 1024,
        vram_mib: (budgeter.stats().allocated_bytes / MIB) as u64,
        tombstone_ratio,
        wal_bytes_active: dir_bytes(wal_dir),
        oldest_pinned_seq_gap,
    })
}

pub(super) fn flush_wal_batch(
    wal: &mut Wal,
    payloads: &mut Vec<Vec<u8>>,
    durable_wal_seq: &mut u64,
) -> Result<(), String> {
    if payloads.is_empty() {
        return Ok(());
    }
    let refs = payloads.iter().map(Vec::as_slice).collect::<Vec<&[u8]>>();
    let acks = wal.append_batch(&refs).map_err(err)?;
    if let Some(ack) = acks.last() {
        *durable_wal_seq = ack.seq;
    }
    payloads.clear();
    Ok(())
}

pub(super) fn running_tombstone_ratio(tombstones: u64, live: u64) -> f64 {
    let total = tombstones.saturating_add(live);
    if total == 0 {
        0.0
    } else {
        tombstones as f64 / total as f64
    }
}

pub(super) fn physical_tombstone_ratio(vault_dir: &Path) -> Result<f64, String> {
    scan_tombstone_inventory(vault_dir)
        .map(|inventory| inventory.tombstone_ratio())
        .map_err(err)
}

fn dense_vector(op: u64) -> SlotVector {
    SlotVector::Dense {
        dim: DIM as u32,
        data: (0..DIM)
            .map(|idx| ((op.wrapping_mul(31) + idx as u64 * 17) % 997) as f32 / 997.0)
            .collect(),
    }
}

fn cx(op: u64) -> CxId {
    let mut bytes = [0_u8; 16];
    bytes[..8].copy_from_slice(&op.to_be_bytes());
    bytes[8..].copy_from_slice(&op.rotate_left(17).to_be_bytes());
    CxId::from_bytes(bytes)
}

fn tombstone_key(op: u64) -> [u8; 8] {
    KEY_SPACE.saturating_add(op % KEY_SPACE).to_be_bytes()
}

fn row_key(op: u64) -> [u8; 8] {
    (op % KEY_SPACE).to_be_bytes()
}

fn wal_payload(op: u64, row_len: usize) -> Vec<u8> {
    let mut payload = Vec::with_capacity(16);
    payload.extend_from_slice(&op.to_be_bytes());
    payload.extend_from_slice(&(row_len as u64).to_be_bytes());
    payload
}

fn dir_bytes(path: &Path) -> u64 {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return 0;
    };
    if metadata.is_file() {
        return metadata.len();
    }
    fs::read_dir(path)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|entry| dir_bytes(&entry.path()))
                .sum()
        })
        .unwrap_or(0)
}

struct SoakGcTarget<'a> {
    vault_dir: &'a Path,
}

impl CompactionGcTarget for SoakGcTarget<'_> {
    fn tombstone_inventory(&self) -> CalyxResult<TombstoneInventory> {
        scan_tombstone_inventory(self.vault_dir)
    }

    fn compact_tombstoned_cfs(&self, _cfs: &[ColumnFamily]) -> CalyxResult<()> {
        Ok(())
    }
}
