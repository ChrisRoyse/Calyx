use std::path::PathBuf;
use std::time::Instant;

use calyx_sextant::index::{
    DiskAnnBuildBackend, PartitionBuildParams, build_partitioned_vault_from_source_with_backend,
    build_partitioned_vault_with_backend,
};
use serde_json::json;

use crate::error::{CliError, CliResult};

use super::parse;

pub(crate) struct BuildArgs {
    pub(crate) vault: PathBuf,
    /// Real embeddings to ingest (`.fbin`). When set, `n_cx`/`dim` come from the
    /// file and no vectors are synthesised.
    pub(crate) vectors: Option<PathBuf>,
    pub(crate) p: PartitionBuildParams,
    pub(crate) backend: DiskAnnBuildBackend,
}

impl BuildArgs {
    pub(crate) fn parse(args: &[String]) -> CliResult<Self> {
        let mut vault = None;
        let mut vectors = None;
        let (mut n_cx, mut dim, mut regions, mut seed) = (0u64, 512usize, 0usize, 42u64);
        let mut sample: Option<usize> = None;
        let mut chunk: Option<usize> = None;
        let mut m_max = 32usize;
        let mut ef = 96usize;
        let mut region_build_parallelism = None;
        let mut backend = DiskAnnBuildBackend::CpuVamana;
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
                "--region-build-parallelism" => {
                    region_build_parallelism = Some(parse(&next()?, "--region-build-parallelism")?)
                }
                "--build-backend" => {
                    backend = next()?.parse().map_err(CliError::usage)?;
                }
                other => return Err(CliError::usage(format!("unknown flag: {other}"))),
            }
        }
        let vault = vault.ok_or_else(|| CliError::usage("--vault <dir> is required"))?;
        if regions == 0 {
            return Err(CliError::usage("--regions must be > 0"));
        }
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
            region_build_parallelism: region_build_parallelism
                .unwrap_or_else(|| PartitionBuildParams::default_region_build_parallelism(regions)),
        };
        Ok(Self {
            vault,
            vectors,
            p,
            backend,
        })
    }
}

pub(crate) fn run(args: &[String]) -> CliResult {
    let args = BuildArgs::parse(args)?;
    std::fs::create_dir_all(&args.vault)
        .map_err(|e| CliError::io(format!("create vault dir: {e}")))?;
    let started = Instant::now();
    let manifest = match &args.vectors {
        Some(path) => {
            let source = calyx_sextant::index::FbinSource::open(path).map_err(CliError::Calyx)?;
            build_partitioned_vault_from_source_with_backend(
                &args.vault,
                &source,
                args.p,
                args.backend,
            )
            .map_err(CliError::Calyx)?
        }
        None => build_partitioned_vault_with_backend(&args.vault, args.p, args.backend)
            .map_err(CliError::Calyx)?,
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
        "region_build_parallelism": manifest.region_build_parallelism,
        "graph_build_backend": manifest.graph_build_backend.as_str(),
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
