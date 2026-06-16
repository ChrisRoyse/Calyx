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
    p: PartitionBuildParams,
}

impl BuildArgs {
    fn parse(args: &[String]) -> CliResult<Self> {
        let mut vault = None;
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
        if n_cx == 0 || regions == 0 {
            return Err(CliError::usage("--n-cx and --regions must be > 0"));
        }
        let p = PartitionBuildParams {
            n_cx,
            dim,
            n_regions: regions,
            seed,
            sample: sample.unwrap_or_else(|| (n_cx as usize).min(200_000)),
            chunk: chunk.unwrap_or(100_000),
            m_max,
            ef_construction: ef,
        };
        Ok(Self { vault, p })
    }
}

pub(crate) fn run_build(args: &[String]) -> CliResult {
    let args = BuildArgs::parse(args)?;
    std::fs::create_dir_all(&args.vault)
        .map_err(|e| CliError::io(format!("create vault dir: {e}")))?;
    let started = Instant::now();
    let manifest = build_partitioned_vault(&args.vault, args.p).map_err(CliError::Calyx)?;
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
    n: usize,
    k: usize,
    n_probe: usize,
    region_beam: usize,
}

impl SearchArgs {
    fn parse(args: &[String]) -> CliResult<Self> {
        let mut vault = None;
        let (mut n, mut k, mut n_probe, mut region_beam) = (1000usize, 10usize, 8usize, 64usize);
        let mut it = args.iter();
        while let Some(flag) = it.next() {
            let mut next = || {
                it.next()
                    .cloned()
                    .ok_or_else(|| CliError::usage(format!("{flag} requires a value")))
            };
            match flag.as_str() {
                "--vault" => vault = Some(PathBuf::from(next()?)),
                "--n" => n = parse(&next()?, "--n")?,
                "--k" => k = parse(&next()?, "--k")?,
                "--n-probe" => n_probe = parse(&next()?, "--n-probe")?,
                "--region-beam" => region_beam = parse(&next()?, "--region-beam")?,
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
            n,
            k,
            n_probe,
            region_beam,
        })
    }
}

pub(crate) fn run_search(args: &[String]) -> CliResult {
    let args = SearchArgs::parse(args)?;
    let search = PartitionedSearch::open(&args.vault).map_err(CliError::Calyx)?;
    let manifest = search.manifest().clone();
    let dim = manifest.dim;
    let n_cx = manifest.n_cx;
    let seed = manifest.seed;

    let mut latencies_us: Vec<u64> = Vec::with_capacity(args.n);
    let mut self_hits = 0usize;
    for i in 0..args.n {
        // Query an existing cx (deterministic) so self-recall is well-defined; the
        // query vector is generated on the fly — nothing is materialized.
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
    }
    let summary = percentiles(&latencies_us);
    let self_recall = self_hits as f32 / args.n.max(1) as f32;
    let report = json!({
        "trigger": "calyx bench partitioned-search",
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
