//! K-way lens-worker scheduling for `assay stream-fbin` (#1160, child of
//! #1152).
//!
//! `run_staged` historically walked the roster with one blocking worker
//! process at a time. With `--lens-parallelism K` (default 1 = exact
//! sequential behavior), up to K single-lens workers run concurrently:
//! slot numbering, per-slot FBIN/vault outputs, worker report/stderr files,
//! and the roster order are byte-identical to the sequential path — only
//! start order and wall clock change. The first failing worker fails the
//! export with its own lens attribution after in-flight workers drain.

use std::path::Path;
use std::sync::mpsc;

use serde_json::json;

use crate::assay_corpus_build::parallel::{ensure_worker_vram_safety, interleaved_start_order};
use crate::error::CliResult;

use super::super::args::Args;
use super::super::local_error;
use super::super::rows::RowStats;
use super::progress::ProgressLog;
use super::selection::SelectedLens;
use super::worker::{self, StreamWorkerReport};

/// Run every selected lens through its own worker process, at most
/// `args.lens_parallelism` concurrently. Returns reports in slot order.
pub(super) fn run_lenses(
    args: &Args,
    stats: &RowStats,
    lenses: Vec<SelectedLens>,
    staging: &Path,
    worker_root: &Path,
    progress: &mut ProgressLog,
) -> CliResult<Vec<StreamWorkerReport>> {
    if args.lens_parallelism <= 1 {
        let mut reports = Vec::with_capacity(lenses.len());
        for (slot, selected) in lenses.into_iter().enumerate() {
            let meta = worker::lens_meta(args, slot, &selected)?;
            progress.lens_started(&meta)?;
            let report =
                worker::run_one_worker(args, stats, slot, &selected, staging, worker_root)?;
            progress.lens_finished(
                report.corpus_rows_written,
                report.query_rows_written,
                report.elapsed_ms,
            )?;
            reports.push(report);
        }
        return Ok(reports);
    }
    run_lenses_parallel(args, stats, &lenses, staging, worker_root, progress)
}

fn run_lenses_parallel(
    args: &Args,
    stats: &RowStats,
    lenses: &[SelectedLens],
    staging: &Path,
    worker_root: &Path,
    progress: &mut ProgressLog,
) -> CliResult<Vec<StreamWorkerReport>> {
    let manifests: Vec<_> = lenses
        .iter()
        .map(|selected| selected.manifest.clone())
        .collect();
    let vram_safety = ensure_worker_vram_safety(
        args.lens_parallelism,
        args.worker_gpu_mem_limit_mib,
        &manifests,
    )
    .map_err(|message| {
        local_error(
            "CALYX_FSV_ASSAY_STREAM_FBIN_PARALLEL_VRAM",
            message,
            "set CALYX_ONNX_GPU_MEM_LIMIT_MIB or pass --worker-gpu-mem-limit-mib before raising --lens-parallelism",
        )
    });
    vram_safety?;
    let order = interleaved_start_order(&manifests);
    let limit = args.lens_parallelism.min(lenses.len());
    let metas = lenses
        .iter()
        .enumerate()
        .map(|(slot, selected)| worker::lens_meta(args, slot, selected))
        .collect::<CliResult<Vec<_>>>()?;
    eprintln!(
        "{}",
        json!({
            "event": "assay_stream_fbin_lens_parallelism",
            "lens_parallelism": limit,
            "lens_total": lenses.len(),
            "start_order": order,
        })
    );
    let mut reports: Vec<Option<StreamWorkerReport>> = Vec::new();
    reports.resize_with(lenses.len(), || None);
    let mut failures: Vec<(usize, crate::error::CliError)> = Vec::new();
    let mut scheduler_result: CliResult = Ok(());
    std::thread::scope(|scope| {
        let (sender, receiver) = mpsc::channel();
        let mut queue = order.into_iter();
        let mut in_flight = 0usize;
        loop {
            while in_flight < limit && failures.is_empty() && scheduler_result.is_ok() {
                let Some(slot) = queue.next() else { break };
                if let Err(error) = progress.lens_started_slot(&metas[slot]) {
                    scheduler_result = Err(error);
                    break;
                }
                let sender = sender.clone();
                let selected = &lenses[slot];
                scope.spawn(move || {
                    let outcome =
                        worker::run_one_worker(args, stats, slot, selected, staging, worker_root);
                    let _ = sender.send((slot, outcome));
                });
                in_flight += 1;
            }
            if in_flight == 0 {
                break;
            }
            let Ok((slot, outcome)) = receiver.recv() else {
                scheduler_result = Err(local_error(
                    "CALYX_FSV_ASSAY_STREAM_FBIN_PARALLEL_CHANNEL",
                    "lens worker scheduler channel closed unexpectedly",
                    "fix the stream-fbin scheduler before trusting streamed FBIN",
                ));
                break;
            };
            in_flight -= 1;
            let slot_u16 = metas[slot].slot as u16;
            match outcome {
                Ok(report) => {
                    if let Err(error) = progress.lens_finished_slot(
                        slot_u16,
                        report.corpus_rows_written,
                        report.query_rows_written,
                        report.elapsed_ms,
                    ) {
                        scheduler_result = Err(error);
                    }
                    reports[slot] = Some(report);
                }
                Err(error) => {
                    if let Err(progress_error) = progress.lens_failed_slot(slot_u16) {
                        scheduler_result = Err(progress_error);
                    }
                    failures.push((slot, error));
                }
            }
        }
    });
    scheduler_result?;
    if !failures.is_empty() {
        failures.sort_by_key(|(slot, _)| *slot);
        for (slot, error) in failures.iter().skip(1) {
            eprintln!(
                "{}",
                json!({
                    "event": "assay_stream_fbin_worker_also_failed",
                    "slot": slot,
                    "error": error.to_string(),
                })
            );
        }
        let (_, error) = failures.remove(0);
        return Err(error);
    }
    reports
        .into_iter()
        .enumerate()
        .map(|(slot, report)| {
            report.ok_or_else(|| {
                local_error(
                    "CALYX_FSV_ASSAY_STREAM_FBIN_WORKER_MISSING",
                    format!("slot {slot} produced no report and no error"),
                    "fix the stream-fbin scheduler before trusting streamed FBIN",
                )
            })
        })
        .collect()
}
