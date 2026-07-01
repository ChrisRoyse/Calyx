use std::collections::{BTreeMap, BTreeSet};

use calyx_aster::cf::{ColumnFamily, KeyRange};
use calyx_aster::mvcc::Snapshot;
use calyx_aster::vault::AsterVault;
use calyx_aster::vault::encode::{decode_constellation_base, decode_slot_vector};
use calyx_core::{CalyxError, Constellation, CxId, SlotId, SlotVector};
use rayon::prelude::*;

use super::super::rebuild::RebuildProgress;
use super::super::rebuild_plan::SlotBuildPlan;
use super::super::{CliResult, dense, multi, sparse, stale};
use super::{SharedRebuildProgress, emit_shared_progress};

pub(super) fn load_base_docs_at<F>(
    vault: &AsterVault,
    snapshot: Snapshot,
    page_rows: usize,
    progress: &mut F,
) -> CliResult<BTreeMap<CxId, Constellation>>
where
    F: FnMut(RebuildProgress<'_>) -> CliResult,
{
    let range = all_rows();
    let mut docs = BTreeMap::new();
    vault.scan_cf_range_pages_snapshot(
        snapshot,
        ColumnFamily::Base,
        &range,
        page_rows,
        |page| {
            let decoded = page
                .into_par_iter()
                .map(|(key, bytes)| decode_base_row(key, bytes))
                .collect::<calyx_core::Result<Vec<_>>>()?;
            for (cx_id, cx) in decoded {
                if docs.insert(cx_id, cx).is_some() {
                    return Err(stale(format!("base CF repeats row for cx_id {cx_id}")));
                }
            }
            progress(RebuildProgress {
                rows: Some(docs.len()),
                base_seq: Some(snapshot.seq()),
                ..RebuildProgress::phase("base_scan_page")
            })?;
            Ok(())
        },
    )?;
    Ok(docs)
}

fn decode_base_row(key: Vec<u8>, bytes: Vec<u8>) -> calyx_core::Result<(CxId, Constellation)> {
    let cx_id = cx_id_from_cf_key(&key, "base CF")?;
    let cx = decode_constellation_base(&bytes)?;
    if cx.cx_id != cx_id {
        return Err(CalyxError::aster_corrupt_shard(format!(
            "base CF key {cx_id} contains constellation {}",
            cx.cx_id
        )));
    }
    Ok((cx_id, cx))
}

pub(super) enum SlotRows {
    Dense(dense::DenseSlotRows),
    Sparse(sparse::SparseSlotRows),
    Multi(multi::MultiSlotRows),
    AbsentOnly,
}

impl SlotRows {
    pub(super) fn len(&self) -> usize {
        match self {
            Self::Dense(rows) => rows.len(),
            Self::Sparse(rows) => rows.len(),
            Self::Multi(rows) => rows.len(),
            Self::AbsentOnly => 0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SlotRowShape {
    Dense,
    Sparse,
    Multi,
}

pub(super) fn collect_slot_rows_from_cf<F>(
    vault: &AsterVault,
    snapshot: Snapshot,
    plan: &SlotBuildPlan,
    page_rows: usize,
    progress: Option<&SharedRebuildProgress<'_, F>>,
) -> CliResult<SlotRows>
where
    F: FnMut(RebuildProgress<'_>) -> CliResult + Send,
{
    let expected = plan.expected_ids.iter().copied().collect::<BTreeSet<_>>();
    let range = all_rows();
    let mut found = BTreeSet::new();
    let mut shape = None;
    let mut dense_dim = None;
    let mut sparse_dim = None;
    let mut multi_token_dim = None;
    let mut dense_rows = Vec::new();
    let mut sparse_rows = Vec::new();
    let mut multi_rows = Vec::new();
    vault.scan_cf_range_pages_snapshot(
        snapshot,
        ColumnFamily::slot(plan.slot),
        &range,
        page_rows,
        |page| {
            for (key, bytes) in page {
                let cx_id = cx_id_from_cf_key(&key, "slot CF")?;
                if !expected.contains(&cx_id) {
                    continue;
                }
                if !found.insert(cx_id) {
                    return Err(stale(format!(
                        "slot CF repeats row for slot {} cx_id {cx_id}",
                        plan.slot
                    )));
                }
                push_slot_vector(
                    plan,
                    cx_id,
                    decode_slot_vector(&bytes)?,
                    &mut shape,
                    &mut dense_dim,
                    &mut sparse_dim,
                    &mut multi_token_dim,
                    &mut dense_rows,
                    &mut sparse_rows,
                    &mut multi_rows,
                )?;
            }
            if let Some(progress) = progress {
                emit_shared_progress(
                    progress,
                    RebuildProgress::slot(
                        "slot_scan_page",
                        plan.slot,
                        Some(found.len()),
                        Some(snapshot.seq()),
                    ),
                )?;
            }
            Ok(())
        },
    )?;
    if found.len() != expected.len() {
        let missing = expected
            .difference(&found)
            .next()
            .map(ToString::to_string)
            .unwrap_or_else(|| "<unknown>".to_string());
        return Err(CalyxError::aster_corrupt_shard(format!(
            "slot CF row missing for slot {} cx_id {missing}",
            plan.slot
        ))
        .into());
    }
    match shape {
        Some(SlotRowShape::Dense) => Ok(SlotRows::Dense(dense::DenseSlotRows {
            dim: dense_dim.expect("dense shape has dim"),
            rows: dense_rows,
        })),
        Some(SlotRowShape::Sparse) => Ok(SlotRows::Sparse(sparse::SparseSlotRows {
            dim: sparse_dim.expect("sparse shape has dim"),
            rows: sparse_rows,
        })),
        Some(SlotRowShape::Multi) => Ok(SlotRows::Multi(multi::MultiSlotRows {
            token_dim: multi_token_dim.expect("multi shape has token dim"),
            rows: multi_rows,
        })),
        None => Ok(SlotRows::AbsentOnly),
    }
}

#[allow(clippy::too_many_arguments)]
fn push_slot_vector(
    plan: &SlotBuildPlan,
    cx_id: CxId,
    vector: SlotVector,
    shape: &mut Option<SlotRowShape>,
    dense_dim: &mut Option<u32>,
    sparse_dim: &mut Option<u32>,
    multi_token_dim: &mut Option<u32>,
    dense_rows: &mut Vec<(CxId, Vec<f32>)>,
    sparse_rows: &mut Vec<(CxId, Vec<calyx_core::SparseEntry>)>,
    multi_rows: &mut Vec<(CxId, Vec<Vec<f32>>)>,
) -> CliResult {
    vector.validate_schema().map_err(|err| {
        stale(format!(
            "slot {} cx {cx_id} has invalid payload: {}",
            plan.slot, err.message
        ))
    })?;
    match vector {
        SlotVector::Dense { dim, data } => {
            require_shape(shape, SlotRowShape::Dense, plan.slot, cx_id)?;
            dense::validate_dense(plan.slot, cx_id, dim, &data)?;
            match *dense_dim {
                Some(expected_dim) if expected_dim != dim => {
                    return Err(stale(format!(
                        "slot {} has mixed dense dims: {expected_dim} and {dim}",
                        plan.slot
                    )));
                }
                None => *dense_dim = Some(dim),
                _ => {}
            }
            dense_rows.push((cx_id, data));
        }
        SlotVector::Sparse { dim, entries } => {
            require_shape(shape, SlotRowShape::Sparse, plan.slot, cx_id)?;
            match *sparse_dim {
                Some(expected_dim) if expected_dim != dim => {
                    return Err(stale(format!(
                        "slot {} has mixed sparse dims: {expected_dim} and {dim}",
                        plan.slot
                    )));
                }
                None => *sparse_dim = Some(dim),
                _ => {}
            }
            sparse_rows.push((cx_id, entries));
        }
        SlotVector::Multi { token_dim, tokens } => {
            require_shape(shape, SlotRowShape::Multi, plan.slot, cx_id)?;
            match *multi_token_dim {
                Some(expected_dim) if expected_dim != token_dim => {
                    return Err(stale(format!(
                        "slot {} has mixed multi token dims: {expected_dim} and {token_dim}",
                        plan.slot
                    )));
                }
                None => *multi_token_dim = Some(token_dim),
                _ => {}
            }
            multi_rows.push((cx_id, tokens));
        }
        SlotVector::Absent { .. } => {}
    }
    Ok(())
}

fn require_shape(
    current: &mut Option<SlotRowShape>,
    next: SlotRowShape,
    slot: SlotId,
    cx_id: CxId,
) -> CliResult {
    match current {
        Some(existing) if *existing != next => Err(stale(format!(
            "slot {slot} mixes {existing:?} rows with {next:?} row at cx {cx_id}; reingest/backfill the vault"
        ))),
        Some(_) => Ok(()),
        None => {
            *current = Some(next);
            Ok(())
        }
    }
}

fn cx_id_from_cf_key(key: &[u8], cf_name: &str) -> calyx_core::Result<CxId> {
    let bytes: [u8; 16] = key.try_into().map_err(|_| {
        CalyxError::vault_access_denied(format!("{cf_name} key has {} bytes", key.len()))
    })?;
    Ok(CxId::from_bytes(bytes))
}

fn all_rows() -> KeyRange {
    KeyRange {
        start: Vec::new(),
        end: None,
    }
}
