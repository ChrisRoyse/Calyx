use serde_json::json;

use crate::error::{CliError, CliResult};
use crate::partitioned_bench::slot_truth_store::{
    self, FORMAT, MODE, ROW_ID_SPACE, SlotTruthRecord, SlotTruthRecordSlot,
};

use super::args::Args;
use super::{BACKEND, SlotEvidence, fail_if_exists, generate, slot_report};

pub(super) fn run(args: &Args) -> CliResult {
    let cf_root = args.cf_root.as_ref().expect("validated");
    fail_if_exists(cf_root)?;
    let (plan_sha256, corpus_rows, slots) = generate(args, None)?;
    let record = SlotTruthRecord {
        format: FORMAT.to_string(),
        mode: MODE.to_string(),
        row_id_space: ROW_ID_SPACE.to_string(),
        plan_sha256: plan_sha256.clone(),
        query_count: args.query_count,
        truth_depth: args.truth_depth,
        corpus_rows,
        reference_backend: BACKEND.to_string(),
        scale_suitable: true,
        slots: slots.iter().map(record_slot).collect(),
    };
    let db_readback = slot_truth_store::write(cf_root, &args.association_key, &record)
        .map_err(CliError::Calyx)?;
    let report = json!({
        "trigger": "calyx bench partitioned-rrf-slot-truth",
        "artifact_mode": "db_only",
        "format": FORMAT,
        "mode": MODE,
        "row_id_space": ROW_ID_SPACE,
        "reference_backend": BACKEND,
        "scale_suitable": true,
        "plan": args.plan,
        "plan_cf_root": args.plan_cf_root,
        "plan_key": args.plan_key,
        "plan_sha256": plan_sha256,
        "cf_root": cf_root,
        "association_key": args.association_key,
        "db_readback": db_readback,
        "query_count": args.query_count,
        "truth_depth": args.truth_depth,
        "corpus_rows": corpus_rows,
        "chunk_rows": args.chunk_rows,
        "slots": slots.iter().map(slot_report).collect::<Vec<_>>(),
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&report)
            .map_err(|error| CliError::runtime(format!("serialize slot-truth report: {error}")))?
    );
    Ok(())
}

fn record_slot(slot: &SlotEvidence) -> SlotTruthRecordSlot {
    SlotTruthRecordSlot {
        slot: slot.slot,
        lens_id: slot.lens_id.clone(),
        weights_sha256: slot.weights_sha256.clone(),
        signal_kind: slot.signal_kind.clone(),
        rows: slot.rank_rows.clone(),
    }
}
