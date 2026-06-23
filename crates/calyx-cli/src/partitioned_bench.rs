//! `calyx build-partitioned-vault` + `calyx bench partitioned-search` (#550).
//!
//! Non-materializing CLI surfaces over the memory-bounded partitioned vault
//! (`calyx_sextant::index::partitioned`). The builder streams rows per region; the
//! search generates query vectors on the fly via `gen_row` and routes to a few
//! region graphs — neither holds the full dataset. This is the path to the 1e8
//! KernelFirst SLO soak (the flat `build-bench-vault`/`bench` paths materialize
//! everything and cannot scale — see #703).

use std::time::Instant;

use calyx_sextant::index::{
    DenseVectorFile, I32BinMatrix, PartitionDistanceMetric, PartitionedSearch, gen_row,
};
use serde_json::json;

use crate::error::{CliError, CliResult};

#[path = "partitioned_bench/args.rs"]
mod args;
mod brute_force;
#[path = "partitioned_bench/build.rs"]
mod build;
#[path = "partitioned_bench/multi_rrf.rs"]
mod multi_rrf;
#[path = "partitioned_bench/progress.rs"]
mod progress;
#[path = "partitioned_bench/slot_truth_generate.rs"]
mod slot_truth_generate;
#[path = "partitioned_bench/summary.rs"]
mod summary;
#[path = "partitioned_bench/tuner_status.rs"]
mod tuner_status;
use args::{SearchArgs, parse, parse_recall_floor};
use brute_force::{brute_force_topk, brute_force_topk_vecfile};
#[cfg(test)]
pub(crate) use build::BuildArgs;
pub(crate) use build::run as run_build;
use summary::{percentiles, summarize_u64};

const METRIC_CLASS_ANN_CORRECTNESS: &str = "ann_correctness";
const GROUNDED_PHASE_EXIT_ELIGIBLE: bool = false;

fn partitioned_error(
    code: &'static str,
    message: impl Into<String>,
    remediation: &'static str,
) -> CliError {
    CliError::Calyx(calyx_core::CalyxError {
        code,
        message: message.into(),
        remediation,
    })
}

fn enforce_recall_floor(floor: Option<f32>, gt_n: usize, recall: Option<f32>) -> CliResult {
    let Some(floor) = floor else {
        return Ok(());
    };
    let Some(measured) = recall.filter(|_| gt_n > 0) else {
        return Err(partitioned_error(
            "CALYX_FSV_PARTITIONED_GROUND_TRUTH_REQUIRED",
            format!("--recall-floor {floor:.6} requires --ground-truth > 0"),
            "rerun with --ground-truth N and read ground_truth_recall_at_k",
        ));
    };
    if measured + f32::EPSILON < floor {
        return Err(partitioned_error(
            "CALYX_FSV_PARTITIONED_RECALL_BELOW_FLOOR",
            format!("ground_truth_recall_at_k={measured:.6} below recall_floor={floor:.6}"),
            "increase n-probe/region-beam or rebuild/tune before claiming the recall gate",
        ));
    }
    Ok(())
}

fn row_for_metric(
    vectors: &DenseVectorFile,
    idx: u64,
    distance_metric: PartitionDistanceMetric,
) -> Vec<f32> {
    match distance_metric {
        PartitionDistanceMetric::UnitL2 => vectors.row_f32(idx),
        PartitionDistanceMetric::RawL2 => vectors.row_f32_raw(idx),
    }
}

fn recall_from_i32bin_ground_truth(
    path: &std::path::Path,
    ann: &[Vec<u64>],
    k: usize,
) -> CliResult<f32> {
    let truth = I32BinMatrix::open(path).map_err(CliError::Calyx)?;
    if truth.count() < ann.len() as u64 {
        return Err(CliError::usage(format!(
            "ground-truth file has {} rows, need {}",
            truth.count(),
            ann.len()
        )));
    }
    if truth.width() < k {
        return Err(CliError::usage(format!(
            "ground-truth file width {} is smaller than k {k}",
            truth.width()
        )));
    }
    let truth_sets = (0..ann.len())
        .map(|idx| {
            let row = truth.row(idx as u64);
            let mut set = std::collections::HashSet::with_capacity(k);
            for value in row.into_iter().take(k) {
                if value < 0 {
                    return Err(CliError::usage(format!(
                        "ground-truth row {idx} contains negative id {value}"
                    )));
                }
                set.insert(value as u64);
            }
            Ok(set)
        })
        .collect::<CliResult<Vec<_>>>()?;
    Ok(recall_from_truth_sets(ann, &truth_sets))
}

fn map_ann_rows_to_ground_truth_ids(
    path: &std::path::Path,
    ann: &[Vec<u64>],
    expected_rows: u64,
) -> CliResult<Vec<Vec<u64>>> {
    let id_map = I32BinMatrix::open(path).map_err(CliError::Calyx)?;
    if id_map.width() != 1 {
        return Err(CliError::usage(format!(
            "--ground-truth-id-map width must be 1, got {}",
            id_map.width()
        )));
    }
    if id_map.count() != expected_rows {
        return Err(CliError::usage(format!(
            "--ground-truth-id-map row count {} != vault n_cx {expected_rows}",
            id_map.count()
        )));
    }
    ann.iter()
        .map(|row| {
            row.iter()
                .map(|&row_id| {
                    if row_id >= id_map.count() {
                        return Err(CliError::usage(format!(
                            "ANN row id {row_id} is outside --ground-truth-id-map count {}",
                            id_map.count()
                        )));
                    }
                    let external_id = id_map.row(row_id)[0];
                    if external_id < 0 {
                        return Err(CliError::usage(format!(
                            "--ground-truth-id-map row {row_id} contains negative id {external_id}"
                        )));
                    }
                    Ok(external_id as u64)
                })
                .collect()
        })
        .collect()
}

fn recall_from_truth_sets(ann: &[Vec<u64>], truth: &[std::collections::HashSet<u64>]) -> f32 {
    let mut found = 0usize;
    let mut total = 0usize;
    for (ann, truth_set) in ann.iter().zip(truth.iter()) {
        found += ann.iter().filter(|cx| truth_set.contains(cx)).count();
        total += truth_set.len();
    }
    found as f32 / total.max(1) as f32
}

pub(crate) fn run_search(args: &[String]) -> CliResult {
    let args = SearchArgs::parse(args)?;
    if args.queries.is_some() {
        run_search_real(&args)
    } else {
        run_search_synthetic(&args)
    }
}

pub(crate) fn run_rrf(args: &[String]) -> CliResult {
    multi_rrf::run(args)
}

pub(crate) fn run_rrf_slot_truth(args: &[String]) -> CliResult {
    slot_truth_generate::run(args)
}

/// REAL-data search: real query embeddings + brute-force ground truth over the REAL
/// corpus `.fbin`. This is the path that actually validates the system as used.
fn run_search_real(args: &SearchArgs) -> CliResult {
    if args.anneal_vault.is_some() {
        return Err(partitioned_error(
            "CALYX_FSV_PARTITIONED_SEARCH_TUNER_REAL_RECALL_REQUIRED",
            "partitioned-search tuner status needs per-query recall; real multi-lens gates should use partitioned-rrf",
            "rerun real fused evidence with bench partitioned-rrf --anneal-vault and ground truth",
        ));
    }
    let search = PartitionedSearch::open(&args.vault).map_err(CliError::Calyx)?;
    let manifest = search.manifest().clone();
    let distance_metric = manifest.distance_metric;
    let queries_path = args.queries.as_ref().expect("real mode");
    let q_vecs = DenseVectorFile::open(queries_path).map_err(CliError::Calyx)?;
    if q_vecs.dim() != manifest.dim {
        return Err(CliError::usage(format!(
            "query dim {} != vault dim {}",
            q_vecs.dim(),
            manifest.dim
        )));
    }
    let n = args.n.min(q_vecs.count() as usize);
    let mut latencies_us: Vec<u64> = Vec::with_capacity(n);
    let mut region_touch_counts = Vec::with_capacity(n);
    let mut first_touched_regions = Vec::new();
    let gt_n = args.ground_truth.min(n);
    let mut gt_queries: Vec<Vec<f32>> = Vec::with_capacity(gt_n);
    let mut gt_ann: Vec<Vec<u64>> = Vec::with_capacity(gt_n);
    for i in 0..n {
        let q = row_for_metric(&q_vecs, i as u64, distance_metric);
        let started = Instant::now();
        let readback = search
            .search_with_readback(&q, args.k, args.n_probe, args.region_beam)
            .map_err(CliError::Calyx)?;
        let hits = readback.hits;
        if first_touched_regions.is_empty() {
            first_touched_regions = readback.touched_regions.clone();
        }
        region_touch_counts.push(readback.touched_regions.len() as u64);
        latencies_us.push((started.elapsed().as_micros().min(u128::from(u64::MAX)) as u64).max(1));
        if i < gt_n {
            gt_ann.push(hits.iter().map(|(cx, _)| *cx).collect());
            gt_queries.push(q);
        }
    }
    let summary = percentiles(&latencies_us);

    let ground_truth_recall = if gt_n > 0 {
        Some(if let Some(path) = &args.ground_truth_file {
            let mapped_ann;
            let ann = if let Some(id_map) = &args.ground_truth_id_map {
                mapped_ann = map_ann_rows_to_ground_truth_ids(id_map, &gt_ann, manifest.n_cx)?;
                &mapped_ann
            } else {
                &gt_ann
            };
            recall_from_i32bin_ground_truth(path, ann, args.k)?
        } else {
            let corpus_path = args.corpus.as_ref().ok_or_else(|| {
                CliError::usage(
                    "--corpus <file.fbin|file.i8bin> or --ground-truth-file <file.i32bin> is required with --ground-truth in real mode",
                )
            })?;
            let corpus = DenseVectorFile::open(corpus_path).map_err(CliError::Calyx)?;
            if corpus.dim() != manifest.dim {
                return Err(CliError::usage(format!(
                    "corpus dim {} != vault dim {}",
                    corpus.dim(),
                    manifest.dim
                )));
            }
            let truth = brute_force_topk_vecfile(&corpus, &gt_queries, args.k, distance_metric);
            recall_from_truth_sets(&gt_ann, &truth)
        })
    } else {
        None
    };
    enforce_recall_floor(args.recall_floor, gt_n, ground_truth_recall)?;

    let report = json!({
        "trigger": "calyx bench partitioned-search",
        "mode": "real",
        "metric_class": METRIC_CLASS_ANN_CORRECTNESS,
        "grounded_phase_exit_eligible": GROUNDED_PHASE_EXIT_ELIGIBLE,
        "vault": args.vault.to_string_lossy(),
        "queries_file": queries_path.to_string_lossy(),
        "n_cx": manifest.n_cx,
        "dim": manifest.dim,
        "n_regions": manifest.n_regions,
        "queries": n,
        "k": args.k,
        "n_probe": args.n_probe,
        "region_beam": args.region_beam,
        "strategy": "KernelFirstPartitioned",
        "distance_metric": distance_metric.as_str(),
        "region_touch_count": summarize_u64(&region_touch_counts),
        "max_touched_regions": region_touch_counts.iter().copied().max().unwrap_or(0),
        "first_touched_regions": first_touched_regions,
        "region_touch_bound": args.n_probe.min(manifest.n_regions),
        "latency_us": summary,
        "ground_truth_queries": gt_n,
        "ground_truth_file": args.ground_truth_file.as_ref().map(|path| path.to_string_lossy()),
        "ground_truth_id_map": args.ground_truth_id_map.as_ref().map(|path| path.to_string_lossy()),
        "ground_truth_recall_at_k": ground_truth_recall,
        "recall_floor": args.recall_floor,
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&report).map_err(CliError::from)?
    );
    Ok(())
}

/// Synthetic search (builder-logic / latency harness only — NOT a recall claim).
fn run_search_synthetic(args: &SearchArgs) -> CliResult {
    let search = PartitionedSearch::open(&args.vault).map_err(CliError::Calyx)?;
    let manifest = search.manifest().clone();
    let dim = manifest.dim;
    let n_cx = manifest.n_cx;
    let seed = manifest.seed;

    let mut latencies_us: Vec<u64> = Vec::with_capacity(args.n);
    let mut region_touch_counts = Vec::with_capacity(args.n);
    let mut first_touched_regions = Vec::new();
    let mut self_hits = 0usize;
    let mut self_recall_rows = Vec::with_capacity(args.n);
    let gt_n = args.ground_truth.min(args.n);
    let mut gt_queries: Vec<Vec<f32>> = Vec::with_capacity(gt_n);
    let mut gt_ann: Vec<Vec<u64>> = Vec::with_capacity(gt_n);
    for i in 0..args.n {
        let idx = (seed.wrapping_add(i as u64 * 7919)) % n_cx;
        let q = gen_row(seed, idx, dim);
        let started = Instant::now();
        let readback = search
            .search_with_readback(&q, args.k, args.n_probe, args.region_beam)
            .map_err(CliError::Calyx)?;
        let hits = readback.hits;
        if first_touched_regions.is_empty() {
            first_touched_regions = readback.touched_regions.clone();
        }
        region_touch_counts.push(readback.touched_regions.len() as u64);
        let us = started.elapsed().as_micros().min(u128::from(u64::MAX)) as u64;
        latencies_us.push(us.max(1));
        let self_hit = hits.iter().any(|(cx, _)| *cx == idx);
        self_hits += usize::from(self_hit);
        self_recall_rows.push(if self_hit { 1.0 } else { 0.0 });
        if i < gt_n {
            gt_ann.push(hits.iter().map(|(cx, _)| *cx).collect());
            gt_queries.push(q);
        }
    }
    let summary = percentiles(&latencies_us);
    let self_recall = self_hits as f32 / args.n.max(1) as f32;

    let ground_truth_recall = if gt_n > 0 {
        let truth = brute_force_topk(seed, n_cx, dim, &gt_queries, args.k);
        let mut found = 0usize;
        let mut total = 0usize;
        for (ann, truth_set) in gt_ann.iter().zip(truth.iter()) {
            found += ann.iter().filter(|cx| truth_set.contains(cx)).count();
            total += truth_set.len();
        }
        Some(found as f32 / total.max(1) as f32)
    } else {
        None
    };
    enforce_recall_floor(args.recall_floor, gt_n, ground_truth_recall)?;
    let tuner_status_path = if let Some(vault) = &args.anneal_vault {
        Some(tuner_status::write(tuner_status::Request {
            vault,
            latencies_us: &latencies_us,
            per_query_recall: &self_recall_rows,
            region_beam: args.region_beam,
            n_probe: args.n_probe,
            tuner_slo_us: args.tuner_slo_us,
            recall_floor: args.recall_floor,
            aggregate_recall: self_recall,
            latency_us: &summary,
            queries: args.n,
            k: args.k,
        })?)
    } else {
        None
    };

    let report = json!({
        "trigger": "calyx bench partitioned-search",
        "mode": "synthetic",
        "metric_class": METRIC_CLASS_ANN_CORRECTNESS,
        "grounded_phase_exit_eligible": GROUNDED_PHASE_EXIT_ELIGIBLE,
        "vault": args.vault.to_string_lossy(),
        "n_cx": n_cx,
        "dim": dim,
        "n_regions": manifest.n_regions,
        "queries": args.n,
        "k": args.k,
        "n_probe": args.n_probe,
        "region_beam": args.region_beam,
        "strategy": "KernelFirstPartitioned",
        "region_touch_count": summarize_u64(&region_touch_counts),
        "max_touched_regions": region_touch_counts.iter().copied().max().unwrap_or(0),
        "first_touched_regions": first_touched_regions,
        "region_touch_bound": args.n_probe.min(manifest.n_regions),
        "latency_us": summary,
        "self_recall_at_k": self_recall,
        "ground_truth_queries": gt_n,
        "ground_truth_recall_at_k": ground_truth_recall,
        "recall_floor": args.recall_floor,
        "tuner_status_path": tuner_status_path,
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&report).map_err(CliError::from)?
    );
    Ok(())
}

#[cfg(test)]
#[path = "partitioned_bench/progress_tests.rs"]
mod partitioned_bench_progress_tests;
#[cfg(test)]
#[path = "partitioned_bench_tests.rs"]
mod partitioned_bench_tests;
