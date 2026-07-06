use std::fs;
use std::path::PathBuf;

use sha2::{Digest, Sha256};

use crate::error::{CliError, CliResult};
use crate::partitioned_bench::{rrf_plan, timeline_store};

use super::super::{args::Args, io_error};
use super::StagedExport;
use super::paths::display;

pub(super) fn persist_after_promotion(args: &Args, staged: &mut StagedExport) -> CliResult {
    persist_plan(args, staged)?;
    persist_timeline(args, staged)?;
    if args.emit_artifacts {
        fs::write(
            args.out_dir.join("stream_fbin_report.json"),
            serde_json::to_vec_pretty(&staged.evidence).map_err(|error| {
                CliError::runtime(format!("serialize stream_fbin_report.json: {error}"))
            })?,
        )
        .map_err(io_error)?;
    }
    Ok(())
}

pub(super) fn remove_json_artifacts(args: &Args) -> CliResult {
    for name in [
        "partitioned_rrf_plan.json",
        "timeline.jsonl",
        "stream_fbin_progress.json",
        "stream_fbin_report.json",
    ] {
        let path = args.out_dir.join(name);
        if path.exists() {
            fs::remove_file(&path).map_err(io_error)?;
        }
    }
    Ok(())
}

fn persist_plan(args: &Args, staged: &mut StagedExport) -> CliResult {
    let cf_root = args.out_dir.join("partitioned_rrf_plan_cf");
    let association_key = rrf_plan::DEFAULT_ASSOCIATION_KEY;
    let plan_path = args.out_dir.join("partitioned_rrf_plan.json");
    let plan_bytes = fs::read(&plan_path).map_err(io_error)?;
    let readback = rrf_plan::write(
        &cf_root,
        association_key,
        &rrf_plan::PartitionedRrfPlanRecord {
            format: rrf_plan::FORMAT.to_string(),
            mode: rrf_plan::MODE.to_string(),
            imported_plan_sha256: hex_sha256(&plan_bytes),
            base_dir: PathBuf::new(),
            plan: staged.plan.clone(),
        },
    )
    .map_err(CliError::from)?;
    staged.evidence.plan_cf_root = display(&cf_root);
    staged.evidence.plan_association_key = association_key.to_string();
    staged.evidence.plan_db_readback = Some(readback);
    Ok(())
}

fn persist_timeline(args: &Args, staged: &mut StagedExport) -> CliResult {
    let cf_root = args.out_dir.join("partitioned_rrf_timeline_cf");
    let association_key = timeline_store::DEFAULT_ASSOCIATION_KEY;
    let timeline_path = args.out_dir.join("timeline.jsonl");
    let import =
        timeline_store::load_rows_from_jsonl(&timeline_path, Some(staged.evidence.rows.rows))
            .map_err(CliError::from)?;
    let readback = timeline_store::write(
        &cf_root,
        association_key,
        &import.source_sha256,
        &import.rows,
        timeline_store::DEFAULT_CHUNK_ROWS,
    )
    .map_err(CliError::from)?;
    staged.evidence.timeline_cf_root = display(&cf_root);
    staged.evidence.timeline_association_key = association_key.to_string();
    staged.evidence.timeline_db_readback = Some(readback);
    Ok(())
}

fn hex_sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}
