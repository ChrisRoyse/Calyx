use std::collections::{BTreeMap, HashSet};
use std::path::Path;

use calyx_core::SlotId;
use serde_json::{Value, json};

use super::{Plan, report, slot_id};
use crate::error::CliResult;
use crate::partitioned_bench::partitioned_error;
use crate::partitioned_bench::slot_truth_store::{
    self, DEFAULT_ASSOCIATION_KEY, FORMAT, MODE, ROW_ID_SPACE, SlotTruthDbReadback, SlotTruthRecord,
};

pub(super) struct DbSlotTruth {
    rows_by_slot: BTreeMap<SlotId, Vec<Vec<u64>>>,
    source: Value,
    scale_suitable: bool,
}

pub(super) struct Context<'a> {
    pub(super) cf_root: &'a Path,
    pub(super) association_key: &'a str,
    pub(super) plan_path: &'a Path,
    pub(super) plan_sha256: &'a str,
    pub(super) plan: &'a Plan,
    pub(super) truth_n: usize,
    pub(super) truth_depth: usize,
    pub(super) corpus_rows: usize,
}

impl DbSlotTruth {
    pub(super) fn load(ctx: Context<'_>) -> CliResult<Self> {
        let key = if ctx.association_key.trim().is_empty() {
            DEFAULT_ASSOCIATION_KEY
        } else {
            ctx.association_key
        };
        let (record, readback) =
            slot_truth_store::read(ctx.cf_root, key).map_err(crate::error::CliError::Calyx)?;
        validate_record(&record, &ctx)?;
        let mut rows_by_slot = BTreeMap::new();
        for spec in &record.slots {
            validate_rows(spec.slot, &spec.rows, &ctx)?;
            rows_by_slot.insert(slot_id(spec.slot), spec.rows.clone());
        }
        Ok(Self {
            rows_by_slot,
            source: source(&record, &readback, ctx, key),
            scale_suitable: record.scale_suitable,
        })
    }

    pub(super) fn row_ids(&self, slot: SlotId, query_idx: usize) -> &[u64] {
        &self.rows_by_slot[&slot][query_idx]
    }

    pub(super) fn source(&self) -> Value {
        self.source.clone()
    }

    pub(super) fn scale_suitable(&self) -> bool {
        self.scale_suitable
    }
}

fn validate_record(record: &SlotTruthRecord, ctx: &Context<'_>) -> CliResult {
    if record.format != FORMAT || record.mode != MODE || record.row_id_space != ROW_ID_SPACE {
        return Err(partitioned_error(
            "CALYX_FSV_PARTITIONED_RRF_SLOT_TRUTH_DB_NOT_RRF_REFERENCE",
            "DB slot truth row is not a Calyx RRF reference record",
            "write the row with calyx bench partitioned-rrf-slot-truth --db-only",
        ));
    }
    let plan_sha256 = ctx.plan_sha256;
    if record.plan_sha256 != plan_sha256 {
        return Err(partitioned_error(
            "CALYX_FSV_PARTITIONED_RRF_SLOT_TRUTH_DB_STALE",
            format!("plan_sha256 db={} actual={plan_sha256}", record.plan_sha256),
            "regenerate DB slot truth after changing the RRF plan",
        ));
    }
    if record.query_count < ctx.truth_n
        || record.truth_depth < ctx.truth_depth
        || record.corpus_rows != ctx.corpus_rows
    {
        return Err(partitioned_error(
            "CALYX_FSV_PARTITIONED_RRF_SLOT_TRUTH_DB_MISMATCH",
            format!(
                "db query_count/truth_depth/corpus_rows = {}/{}/{} but run needs {}/{}/{}",
                record.query_count,
                record.truth_depth,
                record.corpus_rows,
                ctx.truth_n,
                ctx.truth_depth,
                ctx.corpus_rows
            ),
            "regenerate DB slot truth for this run depth and corpus row space",
        ));
    }
    if record_slots(record) != plan_slots(ctx.plan) {
        return Err(partitioned_error(
            "CALYX_FSV_PARTITIONED_RRF_SLOT_TRUTH_DB_STALE",
            "DB slot truth lens roster does not match the current RRF plan",
            "regenerate DB slot truth after changing lenses, weights, or slot order",
        ));
    }
    Ok(())
}

fn validate_rows(slot: u16, rows: &[Vec<u64>], ctx: &Context<'_>) -> CliResult {
    if rows.len() < ctx.truth_n {
        return Err(partitioned_error(
            "CALYX_FSV_PARTITIONED_RRF_SLOT_TRUTH_DB_MISMATCH",
            format!("slot {slot} DB rows={} needs {}", rows.len(), ctx.truth_n),
            "regenerate DB slot truth with enough query rows",
        ));
    }
    for (row_idx, row) in rows.iter().take(ctx.truth_n).enumerate() {
        if row.len() < ctx.truth_depth {
            return Err(partitioned_error(
                "CALYX_FSV_PARTITIONED_RRF_SLOT_TRUTH_DB_MISMATCH",
                format!(
                    "slot {slot} row {row_idx} width={} needs {}",
                    row.len(),
                    ctx.truth_depth
                ),
                "regenerate DB slot truth with enough rank depth",
            ));
        }
        let mut seen = HashSet::with_capacity(ctx.truth_depth);
        for &id in row.iter().take(ctx.truth_depth) {
            if id >= ctx.corpus_rows as u64 || !seen.insert(id) {
                return Err(partitioned_error(
                    "CALYX_FSV_PARTITIONED_RRF_SLOT_TRUTH_DB_INVALID",
                    format!("slot {slot} row {row_idx} has out-of-range or duplicate id {id}"),
                    "regenerate DB slot truth for the current corpus row space",
                ));
            }
        }
    }
    Ok(())
}

fn record_slots(record: &SlotTruthRecord) -> Vec<(u16, String, String, String)> {
    record
        .slots
        .iter()
        .map(|slot| {
            (
                slot.slot,
                slot.lens_id.clone(),
                slot.weights_sha256.clone(),
                slot.signal_kind.clone(),
            )
        })
        .collect()
}

fn plan_slots(plan: &Plan) -> Vec<(u16, String, String, String)> {
    plan.slots
        .iter()
        .map(|slot| {
            (
                slot.slot,
                slot.lens_id.clone().unwrap_or_default(),
                slot.weights_sha256.clone().unwrap_or_default(),
                slot.signal_kind.clone().unwrap_or_default(),
            )
        })
        .collect()
}

fn source(
    record: &SlotTruthRecord,
    readback: &SlotTruthDbReadback,
    ctx: Context<'_>,
    association_key: &str,
) -> Value {
    json!({
        "mode": "precomputed_slot_rrf_aster_cf",
        "metric_class": report::METRIC_CLASS,
        "metric_scope": report::METRIC_SCOPE,
        "truth_reference_class": report::TRUTH_REFERENCE_CLASS,
        "valid_real_outcome": false,
        "grounded_phase_exit_eligible": false,
        "format": FORMAT,
        "cf_root": ctx.cf_root,
        "association_key": association_key,
        "db_readback": readback,
        "plan": ctx.plan_path,
        "plan_sha256": record.plan_sha256,
        "row_id_space": record.row_id_space,
        "query_count_used": ctx.truth_n,
        "truth_depth": record.truth_depth,
        "corpus_rows": record.corpus_rows,
        "reference_backend": record.reference_backend,
        "scale_suitable": record.scale_suitable,
        "slots": record_slots(record),
    })
}
