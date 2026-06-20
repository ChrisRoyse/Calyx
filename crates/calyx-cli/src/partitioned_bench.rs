//! `calyx build-partitioned-vault` + `calyx bench partitioned-search` (#550).
//!
//! Non-materializing CLI surfaces over the memory-bounded partitioned vault
//! (`calyx_sextant::index::partitioned`). The builder streams rows per region; the
//! search generates query vectors on the fly via `gen_row` and routes to a few
//! region graphs — neither holds the full dataset. This is the path to the 1e8
//! KernelFirst SLO soak (the flat `build-bench-vault`/`bench` paths materialize
//! everything and cannot scale — see #703).

use std::path::PathBuf;
use std::time::Instant;

use calyx_sextant::index::{
    DenseVectorFile, I32BinMatrix, PartitionDistanceMetric, PartitionedSearch, gen_row,
};
use serde_json::json;

use crate::error::{CliError, CliResult};

mod brute_force;
#[path = "partitioned_bench/build.rs"]
mod build;
#[path = "partitioned_bench/multi_rrf.rs"]
mod multi_rrf;
#[path = "partitioned_bench/slot_truth_generate.rs"]
mod slot_truth_generate;
use brute_force::{brute_force_topk, brute_force_topk_vecfile};
#[cfg(test)]
pub(crate) use build::BuildArgs;
pub(crate) use build::run as run_build;

const METRIC_CLASS_ANN_CORRECTNESS: &str = "ann_correctness";
const GROUNDED_PHASE_EXIT_ELIGIBLE: bool = false;

fn parse<T: std::str::FromStr>(v: &str, flag: &str) -> CliResult<T> {
    v.parse::<T>()
        .map_err(|_| CliError::usage(format!("{flag} expects a valid value, got {v}")))
}

fn parse_recall_floor(v: &str) -> CliResult<f32> {
    let value: f32 = parse(v, "--recall-floor")?;
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        return Err(CliError::usage(
            "--recall-floor expects a finite value in [0, 1]",
        ));
    }
    Ok(value)
}

struct SearchArgs {
    vault: PathBuf,
    queries: Option<PathBuf>,
    corpus: Option<PathBuf>,
    ground_truth_file: Option<PathBuf>,
    ground_truth_id_map: Option<PathBuf>,
    n: usize,
    k: usize,
    n_probe: usize,
    region_beam: usize,
    ground_truth: usize,
    recall_floor: Option<f32>,
}

impl SearchArgs {
    fn parse(args: &[String]) -> CliResult<Self> {
        let mut vault = None;
        let mut queries = None;
        let mut corpus = None;
        let mut ground_truth_file = None;
        let mut ground_truth_id_map = None;
        let (mut n, mut k, mut n_probe, mut region_beam) = (1000usize, 10usize, 8usize, 64usize);
        let mut ground_truth = 0usize;
        let mut recall_floor = None;
        let mut it = args.iter();
        while let Some(flag) = it.next() {
            let mut next = || {
                it.next()
                    .cloned()
                    .ok_or_else(|| CliError::usage(format!("{flag} requires a value")))
            };
            match flag.as_str() {
                "--vault" => vault = Some(PathBuf::from(next()?)),
                "--queries" => queries = Some(PathBuf::from(next()?)),
                "--corpus" => corpus = Some(PathBuf::from(next()?)),
                "--ground-truth-file" => ground_truth_file = Some(PathBuf::from(next()?)),
                "--ground-truth-id-map" => ground_truth_id_map = Some(PathBuf::from(next()?)),
                "--n" => n = parse(&next()?, "--n")?,
                "--k" => k = parse(&next()?, "--k")?,
                "--n-probe" => n_probe = parse(&next()?, "--n-probe")?,
                "--region-beam" => region_beam = parse(&next()?, "--region-beam")?,
                "--ground-truth" => ground_truth = parse(&next()?, "--ground-truth")?,
                "--recall-floor" => recall_floor = Some(parse_recall_floor(&next()?)?),
                // --seed and --report are accepted for harness symmetry; the query
                // seed is taken from the vault manifest (must match the build seed).
                "--seed" | "--report" => {
                    let _ = next()?;
                }
                other => return Err(CliError::usage(format!("unknown flag: {other}"))),
            }
        }
        let vault = vault.ok_or_else(|| CliError::usage("--vault <dir> is required"))?;
        if n == 0 {
            return Err(CliError::usage("--n must be > 0"));
        }
        if k == 0 {
            return Err(CliError::usage("--k must be > 0"));
        }
        if n_probe == 0 {
            return Err(CliError::usage("--n-probe must be > 0"));
        }
        if region_beam == 0 {
            return Err(CliError::usage("--region-beam must be > 0"));
        }
        if ground_truth_id_map.is_some() && ground_truth_file.is_none() {
            return Err(CliError::usage(
                "--ground-truth-id-map requires --ground-truth-file",
            ));
        }
        Ok(Self {
            vault,
            queries,
            corpus,
            ground_truth_file,
            ground_truth_id_map,
            n,
            k,
            n_probe,
            region_beam,
            ground_truth,
            recall_floor,
        })
    }
}

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
        if hits.iter().any(|(cx, _)| *cx == idx) {
            self_hits += 1;
        }
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
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&report).map_err(CliError::from)?
    );
    Ok(())
}

fn percentiles(values: &[u64]) -> serde_json::Value {
    summarize_u64(values)
}

fn summarize_u64(values: &[u64]) -> serde_json::Value {
    let mut s = values.to_vec();
    s.sort_unstable();
    let pct = |p: usize| -> u64 {
        if s.is_empty() {
            return 0;
        }
        // p in tenths-of-percent (e.g. 999 = 99.9th). idx = ceil(p/1000 * n) - 1.
        let rank = ((p as f64 / 1000.0) * s.len() as f64).ceil() as usize;
        s[rank.saturating_sub(1).min(s.len() - 1)]
    };
    json!({ "p50": pct(500), "p99": pct(990), "p999": pct(999), "max": s.last().copied().unwrap_or(0) })
}

#[cfg(test)]
#[path = "partitioned_bench_tests.rs"]
mod partitioned_bench_tests;
