use std::path::PathBuf;
use std::time::Instant;

use calyx_sextant::index::{
    DEFAULT_FINAL_ASSIGNMENT_PROBE, DiskAnnBuildBackend, FbinSource, I8BinSource,
    PartitionBuildParams, PartitionDistanceMetric, VectorSource,
    build_partitioned_vault_from_source_with_backend_and_metric,
    build_partitioned_vault_with_backend,
};
use serde_json::json;

use crate::error::{CliError, CliResult};

use super::{parse, progress};

pub(crate) struct BuildArgs {
    pub(crate) vault: PathBuf,
    /// Real embeddings to ingest (`.fbin` or BigANN `.i8bin`). When set,
    /// `n_cx`/`dim` come from the file and no vectors are synthesised.
    pub(crate) vectors: Option<PathBuf>,
    pub(crate) p: PartitionBuildParams,
    pub(crate) backend: DiskAnnBuildBackend,
    pub(crate) distance_metric: PartitionDistanceMetric,
    pub(crate) progress_file: Option<PathBuf>,
}

struct PreparedBuild {
    source: Option<Box<dyn VectorSource>>,
    params: PartitionBuildParams,
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
        let mut final_assignment_probe = DEFAULT_FINAL_ASSIGNMENT_PROBE;
        let mut final_assignment_cap = None;
        let mut backend = DiskAnnBuildBackend::CpuVamana;
        let mut distance_metric = PartitionDistanceMetric::UnitL2;
        let mut progress_file = None;
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
                "--final-assignment-probe" => {
                    final_assignment_probe = parse(&next()?, "--final-assignment-probe")?
                }
                "--final-assignment-cap" => {
                    final_assignment_cap = Some(parse(&next()?, "--final-assignment-cap")?)
                }
                "--build-backend" => {
                    backend = next()?.parse().map_err(CliError::usage)?;
                }
                "--distance-metric" => {
                    distance_metric = next()?.parse().map_err(CliError::usage)?;
                }
                "--progress-file" => progress_file = Some(PathBuf::from(next()?)),
                other => return Err(CliError::usage(format!("unknown flag: {other}"))),
            }
        }
        let vault = vault.ok_or_else(|| CliError::usage("--vault <dir> is required"))?;
        if regions == 0 {
            return Err(CliError::usage("--regions must be > 0"));
        }
        if final_assignment_probe == 0 {
            return Err(CliError::usage("--final-assignment-probe must be > 0"));
        }
        if final_assignment_cap == Some(0) {
            return Err(CliError::usage("--final-assignment-cap must be > 0"));
        }
        if vectors.is_none() && n_cx == 0 {
            return Err(CliError::usage(
                "provide --vectors <file.fbin|file.i8bin> (real embeddings) or --n-cx (synthetic)",
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
            final_assignment_probe,
            final_assignment_cap,
        };
        Ok(Self {
            vault,
            vectors,
            p,
            backend,
            distance_metric,
            progress_file,
        })
    }
}

pub(crate) fn run(args: &[String]) -> CliResult {
    let args = BuildArgs::parse(args)?;
    let prepared = match prepare_build(&args) {
        Ok(prepared) => prepared,
        Err(error) => {
            progress::write_failure(
                args.progress_file.as_deref(),
                progress_config(&args, args.p),
                &error,
            )?;
            return Err(error);
        }
    };
    let progress = progress::BuildProgress::start(
        args.progress_file.as_deref(),
        progress_config(&args, prepared.params),
    )?;
    let started = Instant::now();
    let manifest = match run_prepared_build(&args, &prepared) {
        Ok(manifest) => manifest,
        Err(error) => {
            progress.fail(&error)?;
            return Err(error);
        }
    };
    progress.complete()?;
    let build_secs = started.elapsed().as_secs_f64();
    let non_empty = manifest.regions.len();
    let total: usize = manifest.regions.iter().map(|r| r.count).sum();
    let max_replication = manifest.final_assignment_max_replication.max(1);
    let max_total = (manifest.n_cx as usize).saturating_mul(max_replication);
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
        "stored_region_members": manifest.stored_region_members,
        "assignment_replication_factor": total as f64 / manifest.n_cx.max(1) as f64,
        "max_region_count": max_region,
        "min_region_count": min_region,
        "seed": manifest.seed,
        "m_max": manifest.m_max,
        "ef_construction": manifest.ef_construction,
        "distance_metric": manifest.distance_metric.as_str(),
        "region_build_parallelism": manifest.region_build_parallelism,
        "graph_build_backend": manifest.graph_build_backend.as_str(),
        "provisional_assignment_routing": manifest.provisional_assignment_routing,
        "final_assignment_routing": manifest.final_assignment_routing,
        "final_assignment_probe": manifest.final_assignment_probe,
        "final_assignment_cap": manifest.final_assignment_cap,
        "final_assignment_boundary_epsilon": manifest.final_assignment_boundary_epsilon,
        "final_assignment_max_replication": manifest.final_assignment_max_replication,
        "root_graph_rel": manifest.root_graph_rel,
        "centroids_rel": manifest.centroids_rel,
        "build_seconds": build_secs,
    });
    if total < manifest.n_cx as usize
        || total > max_total
        || total != manifest.stored_region_members
    {
        return Err(CliError::Calyx(calyx_core::CalyxError {
            code: "CALYX_FSV_PARTITION_COUNT_MISMATCH",
            message: format!(
                "stored_region_members={total} outside [{}, {max_total}] or manifest mismatch",
                manifest.n_cx
            ),
            remediation: "every cx must land at least once and bounded replication must not exceed max_replication",
        }));
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&report).map_err(CliError::from)?
    );
    Ok(())
}

fn prepare_build(args: &BuildArgs) -> CliResult<PreparedBuild> {
    let mut params = args.p;
    let source = match &args.vectors {
        Some(path) => {
            let source = open_vector_source(path, args.distance_metric)?;
            params.n_cx = source.len();
            params.dim = source.dim();
            Some(source)
        }
        None => None,
    };
    Ok(PreparedBuild { source, params })
}

fn progress_config(
    args: &BuildArgs,
    params: PartitionBuildParams,
) -> progress::BuildProgressConfig {
    progress::BuildProgressConfig {
        vault: args.vault.clone(),
        params,
        backend: args.backend,
        distance_metric: args.distance_metric,
    }
}

fn run_prepared_build(
    args: &BuildArgs,
    prepared: &PreparedBuild,
) -> CliResult<calyx_sextant::index::PartitionedManifest> {
    match prepared.source.as_deref() {
        Some(source) => build_partitioned_vault_from_source_with_backend_and_metric(
            &args.vault,
            source,
            prepared.params,
            args.backend,
            args.distance_metric,
        )
        .map_err(CliError::Calyx),
        None => build_partitioned_vault_with_backend(&args.vault, prepared.params, args.backend)
            .map_err(CliError::Calyx),
    }
}

fn open_vector_source(
    path: &std::path::Path,
    distance_metric: PartitionDistanceMetric,
) -> CliResult<Box<dyn VectorSource>> {
    match path.extension().and_then(|ext| ext.to_str()) {
        Some("fbin") => Ok(Box::new(FbinSource::open(path).map_err(CliError::Calyx)?)),
        Some("i8bin") => {
            let source = match distance_metric {
                PartitionDistanceMetric::UnitL2 => I8BinSource::open(path),
                PartitionDistanceMetric::RawL2 => I8BinSource::open_raw(path),
            }
            .map_err(CliError::Calyx)?;
            Ok(Box::new(source))
        }
        _ => Err(CliError::usage(format!(
            "--vectors {} must end in .fbin or .i8bin",
            path.display()
        ))),
    }
}
