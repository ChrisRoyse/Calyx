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
    PartitionBuildParams, PartitionedSearch, build_partitioned_vault, gen_row,
};
use serde_json::json;

use crate::error::{CliError, CliResult};

fn parse<T: std::str::FromStr>(v: &str, flag: &str) -> CliResult<T> {
    v.parse::<T>()
        .map_err(|_| CliError::usage(format!("{flag} expects a valid value, got {v}")))
}

struct BuildArgs {
    vault: PathBuf,
    /// Real embeddings to ingest (`.fbin`). When set, `n_cx`/`dim` come from the
    /// file and no vectors are synthesised.
    vectors: Option<PathBuf>,
    p: PartitionBuildParams,
}

impl BuildArgs {
    fn parse(args: &[String]) -> CliResult<Self> {
        let mut vault = None;
        let mut vectors = None;
        let (mut n_cx, mut dim, mut regions, mut seed) = (0u64, 512usize, 0usize, 42u64);
        let mut sample: Option<usize> = None;
        let mut chunk: Option<usize> = None;
        let mut m_max = 32usize;
        let mut ef = 96usize;
        let mut it = args.iter();
        while let Some(flag) = it.next() {
            let mut next = || {
                it.next()
                    .cloned()
                    .ok_or_else(|| CliError::usage(format!("{flag} requires a value")))
            };
            match flag.as_str() {
                "--vault" => vault = Some(PathBuf::from(next()?)),
                "--vectors" => vectors = Some(PathBuf::from(next()?)),
                "--n-cx" => n_cx = parse(&next()?, "--n-cx")?,
                "--dim" => dim = parse(&next()?, "--dim")?,
                "--regions" => regions = parse(&next()?, "--regions")?,
                "--seed" => seed = parse(&next()?, "--seed")?,
                "--sample" => sample = Some(parse(&next()?, "--sample")?),
                "--chunk" => chunk = Some(parse(&next()?, "--chunk")?),
                "--m-max" => m_max = parse(&next()?, "--m-max")?,
                "--ef" => ef = parse(&next()?, "--ef")?,
                other => return Err(CliError::usage(format!("unknown flag: {other}"))),
            }
        }
        let vault = vault.ok_or_else(|| CliError::usage("--vault <dir> is required"))?;
        if regions == 0 {
            return Err(CliError::usage("--regions must be > 0"));
        }
        // With real vectors, n_cx/dim are read from the file; without them a synthetic
        // source requires explicit n_cx.
        if vectors.is_none() && n_cx == 0 {
            return Err(CliError::usage(
                "provide --vectors <file.fbin> (real embeddings) or --n-cx (synthetic)",
            ));
        }
        let p = PartitionBuildParams {
            n_cx,
            dim,
            n_regions: regions,
            seed,
            sample: sample.unwrap_or(200_000),
            chunk: chunk.unwrap_or(100_000),
            m_max,
            ef_construction: ef,
        };
        Ok(Self { vault, vectors, p })
    }
}

pub(crate) fn run_build(args: &[String]) -> CliResult {
    let args = BuildArgs::parse(args)?;
    std::fs::create_dir_all(&args.vault)
        .map_err(|e| CliError::io(format!("create vault dir: {e}")))?;
    let started = Instant::now();
    let manifest = match &args.vectors {
        Some(path) => {
            let source = calyx_sextant::index::FbinSource::open(path).map_err(CliError::Calyx)?;
            calyx_sextant::index::build_partitioned_vault_from_source(&args.vault, &source, args.p)
                .map_err(CliError::Calyx)?
        }
        None => build_partitioned_vault(&args.vault, args.p).map_err(CliError::Calyx)?,
    };
    let build_secs = started.elapsed().as_secs_f64();
    let non_empty = manifest.regions.len();
    let total: usize = manifest.regions.iter().map(|r| r.count).sum();
    let max_region = manifest.regions.iter().map(|r| r.count).max().unwrap_or(0);
    let min_region = manifest.regions.iter().map(|r| r.count).min().unwrap_or(0);
    let report = json!({
        "trigger": "calyx build-partitioned-vault",
        "vault": args.vault.to_string_lossy(),
        "n_cx": manifest.n_cx,
        "dim": manifest.dim,
        "n_regions": manifest.n_regions,
        "non_empty_regions": non_empty,
        "assigned_total": total,
        "max_region_count": max_region,
        "min_region_count": min_region,
        "seed": manifest.seed,
        "m_max": manifest.m_max,
        "ef_construction": manifest.ef_construction,
        "root_graph_rel": manifest.root_graph_rel,
        "centroids_rel": manifest.centroids_rel,
        "build_seconds": build_secs,
    });
    if total as u64 != manifest.n_cx {
        return Err(CliError::Calyx(calyx_core::CalyxError {
            code: "CALYX_FSV_PARTITION_COUNT_MISMATCH",
            message: format!("assigned {total} != n_cx {}", manifest.n_cx),
            remediation: "every cx must land in exactly one region",
        }));
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&report).map_err(CliError::from)?
    );
    Ok(())
}

struct SearchArgs {
    vault: PathBuf,
    /// REAL query embeddings (`.fbin`). When set, queries are real vectors, not
    /// synthesised, and `--corpus` supplies the brute-force ground-truth set.
    queries: Option<PathBuf>,
    /// REAL corpus embeddings (`.fbin`) for brute-force ground truth in real mode.
    corpus: Option<PathBuf>,
    n: usize,
    k: usize,
    n_probe: usize,
    region_beam: usize,
    /// If > 0, brute-force the TRUE top-k for the first `ground_truth` queries and
    /// report real recall@k (not just self-recall). Memory-bounded chunked scan.
    ground_truth: usize,
}

impl SearchArgs {
    fn parse(args: &[String]) -> CliResult<Self> {
        let mut vault = None;
        let mut queries = None;
        let mut corpus = None;
        let (mut n, mut k, mut n_probe, mut region_beam) = (1000usize, 10usize, 8usize, 64usize);
        let mut ground_truth = 0usize;
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
                "--n" => n = parse(&next()?, "--n")?,
                "--k" => k = parse(&next()?, "--k")?,
                "--n-probe" => n_probe = parse(&next()?, "--n-probe")?,
                "--region-beam" => region_beam = parse(&next()?, "--region-beam")?,
                "--ground-truth" => ground_truth = parse(&next()?, "--ground-truth")?,
                // --seed and --report are accepted for harness symmetry; the query
                // seed is taken from the vault manifest (must match the build seed).
                "--seed" | "--report" => {
                    let _ = next()?;
                }
                other => return Err(CliError::usage(format!("unknown flag: {other}"))),
            }
        }
        let vault = vault.ok_or_else(|| CliError::usage("--vault <dir> is required"))?;
        Ok(Self {
            vault,
            queries,
            corpus,
            n,
            k,
            n_probe,
            region_beam,
            ground_truth,
        })
    }
}

/// Brute-force the true top-`k` neighbours (by L2) for each query over a REAL corpus
/// `.fbin`. Memory-bounded: scans the mmap'd file in `CHUNK`-sized row windows,
/// parallel per chunk. Returns, per query, the set of true top-k row ids.
fn brute_force_topk_fbin(
    corpus: &calyx_sextant::index::FbinVectors,
    queries: &[Vec<f32>],
    k: usize,
) -> Vec<std::collections::HashSet<u64>> {
    use rayon::prelude::*;
    const CHUNK: u64 = 200_000;
    let dim = corpus.dim();
    let n_cx = corpus.count();
    let mut heaps: Vec<std::collections::BinaryHeap<(ordered_f32::OrdF32, u64)>> = (0..queries
        .len())
        .map(|_| std::collections::BinaryHeap::with_capacity(k + 1))
        .collect();
    let mut start = 0u64;
    while start < n_cx {
        let end = (start + CHUNK).min(n_cx);
        for (qi, q) in queries.iter().enumerate() {
            let scored: Vec<(ordered_f32::OrdF32, u64)> = (start..end)
                .into_par_iter()
                .map(|idx| {
                    let row = corpus.row(idx);
                    let mut d = 0.0f32;
                    for j in 0..dim {
                        let diff = q[j] - row[j];
                        d += diff * diff;
                    }
                    (ordered_f32::OrdF32(d), idx)
                })
                .collect();
            let heap = &mut heaps[qi];
            for item in scored {
                heap.push(item);
                if heap.len() > k {
                    heap.pop();
                }
            }
        }
        start = end;
    }
    heaps
        .into_iter()
        .map(|h| h.into_iter().map(|(_, idx)| idx).collect())
        .collect()
}

/// Brute-force the true top-`k` neighbours (by L2) for each query over the whole
/// generated dataset. Memory-bounded: regenerates the dataset in `CHUNK`-sized
/// batches (never materializes all N), parallel per chunk. Returns, per query, the
/// set of true top-k cx indices — the ground truth recall@k is measured against.
fn brute_force_topk(
    seed: u64,
    n_cx: u64,
    dim: usize,
    queries: &[Vec<f32>],
    k: usize,
) -> Vec<std::collections::HashSet<u64>> {
    use rayon::prelude::*;
    const CHUNK: u64 = 200_000;
    // Per-query running top-k as a max-heap keyed by distance (largest at top so we
    // can pop the worst once we exceed k).
    let mut heaps: Vec<std::collections::BinaryHeap<(ordered_f32::OrdF32, u64)>> = (0..queries
        .len())
        .map(|_| std::collections::BinaryHeap::with_capacity(k + 1))
        .collect();
    let mut start = 0u64;
    while start < n_cx {
        let end = (start + CHUNK).min(n_cx);
        // (idx, row) for this chunk, generated in parallel, then scored per query.
        let rows: Vec<(u64, Vec<f32>)> = (start..end)
            .into_par_iter()
            .map(|idx| (idx, gen_row(seed, idx, dim)))
            .collect();
        // For each query, compute distances over the chunk in parallel and fold
        // into that query's running top-k heap.
        for (qi, q) in queries.iter().enumerate() {
            let scored: Vec<(ordered_f32::OrdF32, u64)> = rows
                .par_iter()
                .map(|(idx, row)| {
                    let mut d = 0.0f32;
                    for j in 0..dim {
                        let diff = q[j] - row[j];
                        d += diff * diff;
                    }
                    (ordered_f32::OrdF32(d), *idx)
                })
                .collect();
            let heap = &mut heaps[qi];
            for item in scored {
                heap.push(item);
                if heap.len() > k {
                    heap.pop();
                }
            }
        }
        start = end;
    }
    heaps
        .into_iter()
        .map(|h| h.into_iter().map(|(_, idx)| idx).collect())
        .collect()
}

/// Minimal total-order wrapper over f32 for heap keys (no external dep needed).
mod ordered_f32 {
    #[derive(Clone, Copy, PartialEq)]
    pub struct OrdF32(pub f32);
    impl Eq for OrdF32 {}
    impl PartialOrd for OrdF32 {
        fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
            Some(self.cmp(other))
        }
    }
    impl Ord for OrdF32 {
        fn cmp(&self, other: &Self) -> std::cmp::Ordering {
            self.0.total_cmp(&other.0)
        }
    }
}

pub(crate) fn run_search(args: &[String]) -> CliResult {
    let args = SearchArgs::parse(args)?;
    if args.queries.is_some() {
        run_search_real(&args)
    } else {
        run_search_synthetic(&args)
    }
}

/// REAL-data search: real query embeddings + brute-force ground truth over the REAL
/// corpus `.fbin`. This is the path that actually validates the system as used.
fn run_search_real(args: &SearchArgs) -> CliResult {
    let search = PartitionedSearch::open(&args.vault).map_err(CliError::Calyx)?;
    let manifest = search.manifest().clone();
    let queries_path = args.queries.as_ref().expect("real mode");
    let q_vecs = calyx_sextant::index::FbinVectors::open(queries_path).map_err(CliError::Calyx)?;
    if q_vecs.dim() != manifest.dim {
        return Err(CliError::usage(format!(
            "query dim {} != vault dim {}",
            q_vecs.dim(),
            manifest.dim
        )));
    }
    let n = args.n.min(q_vecs.count() as usize);
    let mut latencies_us: Vec<u64> = Vec::with_capacity(n);
    let gt_n = args.ground_truth.min(n);
    let mut gt_queries: Vec<Vec<f32>> = Vec::with_capacity(gt_n);
    let mut gt_ann: Vec<Vec<u64>> = Vec::with_capacity(gt_n);
    for i in 0..n {
        let q = q_vecs.row(i as u64).to_vec();
        let started = Instant::now();
        let hits = search
            .search(&q, args.k, args.n_probe, args.region_beam)
            .map_err(CliError::Calyx)?;
        latencies_us.push((started.elapsed().as_micros().min(u128::from(u64::MAX)) as u64).max(1));
        if i < gt_n {
            gt_ann.push(hits.iter().map(|(cx, _)| *cx).collect());
            gt_queries.push(q);
        }
    }
    let summary = percentiles(&latencies_us);

    let ground_truth_recall = if gt_n > 0 {
        let corpus_path = args.corpus.as_ref().ok_or_else(|| {
            CliError::usage("--corpus <file.fbin> is required with --ground-truth in real mode")
        })?;
        let corpus =
            calyx_sextant::index::FbinVectors::open(corpus_path).map_err(CliError::Calyx)?;
        if corpus.dim() != manifest.dim {
            return Err(CliError::usage(format!(
                "corpus dim {} != vault dim {}",
                corpus.dim(),
                manifest.dim
            )));
        }
        let truth = brute_force_topk_fbin(&corpus, &gt_queries, args.k);
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

    let report = json!({
        "trigger": "calyx bench partitioned-search",
        "mode": "real",
        "vault": args.vault.to_string_lossy(),
        "queries_file": queries_path.to_string_lossy(),
        "n_cx": manifest.n_cx,
        "dim": manifest.dim,
        "n_regions": manifest.n_regions,
        "queries": n,
        "k": args.k,
        "n_probe": args.n_probe,
        "region_beam": args.region_beam,
        "latency_us": summary,
        "ground_truth_queries": gt_n,
        "ground_truth_recall_at_k": ground_truth_recall,
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
    let mut self_hits = 0usize;
    let gt_n = args.ground_truth.min(args.n);
    let mut gt_queries: Vec<Vec<f32>> = Vec::with_capacity(gt_n);
    let mut gt_ann: Vec<Vec<u64>> = Vec::with_capacity(gt_n);
    for i in 0..args.n {
        let idx = (seed.wrapping_add(i as u64 * 7919)) % n_cx;
        let q = gen_row(seed, idx, dim);
        let started = Instant::now();
        let hits = search
            .search(&q, args.k, args.n_probe, args.region_beam)
            .map_err(CliError::Calyx)?;
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

    let report = json!({
        "trigger": "calyx bench partitioned-search",
        "mode": "synthetic",
        "vault": args.vault.to_string_lossy(),
        "n_cx": n_cx,
        "dim": dim,
        "n_regions": manifest.n_regions,
        "queries": args.n,
        "k": args.k,
        "n_probe": args.n_probe,
        "region_beam": args.region_beam,
        "latency_us": summary,
        "self_recall_at_k": self_recall,
        "ground_truth_queries": gt_n,
        "ground_truth_recall_at_k": ground_truth_recall,
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&report).map_err(CliError::from)?
    );
    Ok(())
}

fn percentiles(values: &[u64]) -> serde_json::Value {
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
