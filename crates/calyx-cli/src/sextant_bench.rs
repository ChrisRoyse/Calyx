use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use calyx_core::SlotId;
use calyx_sextant::index::{
    BwPostcutoffConfig, BwPostcutoffTuner, DiskAnnSearch, DiskAnnSearchParams, FinalCxSearch,
    FunnelParams, KernelFirstSearch, KernelRegion, KernelRegionAnn, RegionPartitions, TunerConfig,
    TunerObservation, build_synthetic_vault, synthetic_dense_rows,
};
use serde::{Deserialize, Serialize};

use crate::error::{CliError, CliResult};
use crate::sextant_bench_guard::require_flat_bench_budget;

const MANIFEST_REL: &str = "bench/bench_manifest.json";
const SEARCH_REPORT_REL: &str = "bench/search-report.json";
const RECALL_REPORT_REL: &str = "bench/recall-report.json";
const TUNER_STATUS_REL: &str = "bench/bw_postcutoff_status.json";

pub(crate) fn run_build(args: &[String]) -> CliResult {
    let args = BuildArgs::parse(args)?;
    require_flat_bench_budget("calyx build-bench-vault", args.n_cx, args.dim)?;
    let vault = build_synthetic_vault(args.n_cx, args.dim, args.slots, args.seed, &args.vault)?;
    let manifest = BenchManifest {
        format: "calyx-sextant-bench-vault-v1".to_string(),
        n_cx: args.n_cx,
        dim: args.dim,
        slots: args.slots,
        seed: args.seed,
        graph: "idx/slot_00.ann/graph.cda".to_string(),
        centroids: "idx/slot_00.sparse/centroids.spn".to_string(),
        postings_dir: "idx/slot_00.sparse".to_string(),
    };
    write_json(&args.vault.join(MANIFEST_REL), &manifest)?;
    let report = BuildReport {
        trigger: "calyx build-bench-vault",
        manifest_path: MANIFEST_REL,
        n_cx: args.n_cx,
        dim: args.dim,
        slots: args.slots,
        seed: args.seed,
        files: vec![
            file_readback(&vault.root, &manifest.graph)?,
            file_readback(&vault.root, &manifest.centroids)?,
        ],
    };
    print_json(&report)
}

pub(crate) fn run_bench(topic: &str, args: &[String]) -> CliResult {
    match topic {
        "search" => bench_search(args),
        "recall" => bench_recall(args),
        other => Err(CliError::usage(format!("unknown bench topic: {other}"))),
    }
}

pub(crate) fn tuner_status(vault: &Path, tuner: &str) -> CliResult {
    if tuner != "bw_postcutoff" {
        return Err(CliError::usage(format!("unknown tuner: {tuner}")));
    }
    let path = vault.join(TUNER_STATUS_REL);
    let text = fs::read_to_string(&path)
        .map_err(|error| CliError::io(format!("read {}: {error}", path.display())))?;
    println!("{text}");
    Ok(())
}

fn bench_search(args: &[String]) -> CliResult {
    let args = SearchArgs::parse(args)?;
    let manifest = read_manifest(&args.vault)?;
    require_flat_bench_budget("calyx bench search", manifest.n_cx, manifest.dim)?;
    let rows = synthetic_dense_rows(manifest.n_cx, manifest.dim, manifest.seed);
    let search = open_kernel_first(&args.vault, &manifest, &rows, args.beamwidth, args.k)?;
    let params = funnel_params(args.beamwidth, args.k, manifest.n_cx);
    let mut latencies = Vec::with_capacity(args.n);
    let mut self_hits = 0_usize;
    let mut tuner = BwPostcutoffTuner::with_config(
        BwPostcutoffConfig {
            beamwidth: args.beamwidth,
            posting_cutoff: args.posting_cutoff,
        },
        TunerConfig {
            latency_slo_us: args.tuner_slo_us.unwrap_or(25_000),
            ..TunerConfig::default()
        },
    );
    for idx in query_indices(args.n, manifest.n_cx, args.seed) {
        let started = Instant::now();
        let hits = search.search(&rows[idx].1, args.k, &params)?;
        let elapsed = started.elapsed().as_micros().min(u128::from(u64::MAX)) as u64;
        let hit = hits.iter().any(|hit| hit.cx_id as usize == idx);
        self_hits += usize::from(hit);
        latencies.push(elapsed.max(1));
        tuner.observe(TunerObservation {
            query_latency_us: elapsed.max(1),
            recall_at_10: if hit { 1.0 } else { 0.0 },
            beamwidth: args.beamwidth,
            posting_cutoff: args.posting_cutoff,
        });
        let _ = tuner.maybe_adjust();
    }
    let report = SearchReport {
        trigger: "calyx bench search",
        strategy: args.strategy,
        n: args.n,
        k: args.k,
        latency_us: summarize(&latencies),
        self_recall_at_k: self_hits as f32 / args.n as f32,
        tuner_status_path: TUNER_STATUS_REL,
        graph: file_readback(&args.vault, &manifest.graph)?,
        centroids: file_readback(&args.vault, &manifest.centroids)?,
    };
    let status = TunerStatus {
        tuner: "bw_postcutoff",
        current: tuner.current_config(),
        adjustments: tuner.adjustment_history().to_vec(),
        ledger_entries: tuner.ledger_entries().to_vec(),
        warnings: tuner.warnings().to_vec(),
        observations: args.n,
        search_report_path: SEARCH_REPORT_REL,
    };
    write_json(&args.vault.join(SEARCH_REPORT_REL), &report)?;
    write_json(&args.vault.join(TUNER_STATUS_REL), &status)?;
    print_json(&report)
}

fn bench_recall(args: &[String]) -> CliResult {
    let args = RecallArgs::parse(args)?;
    let manifest = read_manifest(&args.vault)?;
    require_flat_bench_budget("calyx bench recall", manifest.n_cx, manifest.dim)?;
    let rows = synthetic_dense_rows(manifest.n_cx, manifest.dim, manifest.seed);
    let ids = rows.iter().map(|(cx, _)| *cx).collect::<Vec<_>>();
    let search = DiskAnnSearch::open(
        SlotId::new(0),
        args.vault.join(&manifest.graph),
        ids,
        None,
        diskann_params(args.k.max(64)),
    )?;
    let mut hits = 0_usize;
    for idx in query_indices(args.n, manifest.n_cx, args.seed) {
        let got = search.search_ids(&rows[idx].1, args.k, &diskann_params(args.k.max(64)))?;
        hits += usize::from(got.iter().any(|(id, _)| *id as usize == idx));
    }
    let report = RecallReport {
        trigger: "calyx bench recall",
        n: args.n,
        k: args.k,
        recall_at_k: hits as f32 / args.n as f32,
        graph: file_readback(&args.vault, &manifest.graph)?,
        centroids: file_readback(&args.vault, &manifest.centroids)?,
    };
    write_json(&args.vault.join(RECALL_REPORT_REL), &report)?;
    print_json(&report)
}

fn open_kernel_first(
    vault: &Path,
    manifest: &BenchManifest,
    rows: &[(calyx_core::CxId, Vec<f32>)],
    beamwidth: usize,
    k: usize,
) -> CliResult<KernelFirstSearch> {
    let ids = rows.iter().map(|(cx, _)| *cx).collect::<Vec<_>>();
    let graph = vault.join(&manifest.graph);
    let region_ann = DiskAnnSearch::open(
        SlotId::new(0),
        graph.clone(),
        ids.clone(),
        None,
        diskann_params(beamwidth.max(k)),
    )?;
    let cx_search = DiskAnnSearch::open(
        SlotId::new(0),
        graph,
        ids,
        None,
        diskann_params(beamwidth.max(k)),
    )?;
    let kernel_rows = rows
        .iter()
        .take(manifest.n_cx.min(64))
        .enumerate()
        .map(|(idx, (_, vector))| KernelRegion {
            id: idx as u32,
            vector: vector.clone(),
        })
        .collect::<Vec<_>>();
    let partitions = RegionPartitions::new((0..manifest.n_cx).map(|idx| (idx as u32, idx as u32)));
    Ok(KernelFirstSearch::new(
        manifest.n_cx as u64,
        Some(KernelRegionAnn::new(kernel_rows)?),
        region_ann,
        FinalCxSearch::DiskAnn(cx_search),
        partitions,
    )
    .with_min_vault_size(1))
}

fn diskann_params(width: usize) -> DiskAnnSearchParams {
    DiskAnnSearchParams {
        beamwidth: width.max(1),
        ef_search: width.max(1),
        rescore_k: width.max(1),
        rescore_from_raw: false,
    }
}
fn funnel_params(beamwidth: usize, k: usize, n_cx: usize) -> FunnelParams {
    FunnelParams {
        n_region_beam: beamwidth.max(k).max(1),
        n_cx_beam: beamwidth.max(k).max(1),
        n_regions_to_expand: k.max(4).min(n_cx).max(1),
        ..FunnelParams::default()
    }
}
fn query_indices(n: usize, n_cx: usize, seed: u64) -> Vec<usize> {
    (0..n)
        .map(|idx| ((seed as usize).wrapping_add(idx * 7_919)) % n_cx)
        .collect()
}

fn summarize(values: &[u64]) -> LatencySummary {
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    LatencySummary {
        p50: percentile(&sorted, 50),
        p99: percentile(&sorted, 99),
        p999: percentile(&sorted, 999),
    }
}

fn percentile(sorted: &[u64], pct: usize) -> u64 {
    let scale = if pct > 100 { 1000 } else { 100 };
    let idx = (sorted.len() * pct).div_ceil(scale).saturating_sub(1);
    sorted[idx.min(sorted.len() - 1)]
}

fn read_manifest(vault: &Path) -> CliResult<BenchManifest> {
    let path = vault.join(MANIFEST_REL);
    let text = fs::read_to_string(&path)
        .map_err(|error| CliError::io(format!("read {}: {error}", path.display())))?;
    Ok(serde_json::from_str(&text)?)
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> CliResult {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, serde_json::to_vec_pretty(value)?)?;
    Ok(())
}

fn print_json<T: Serialize>(value: &T) -> CliResult {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

fn file_readback(root: &Path, relative: &str) -> CliResult<FileReadback> {
    let path = root.join(relative);
    Ok(FileReadback {
        path: relative.to_string(),
        size: fs::metadata(&path)
            .map_err(|error| CliError::io(format!("stat {}: {error}", path.display())))?
            .len(),
    })
}

#[derive(Deserialize, Serialize)]
struct BenchManifest {
    format: String,
    n_cx: usize,
    dim: usize,
    slots: usize,
    seed: u64,
    graph: String,
    centroids: String,
    postings_dir: String,
}

struct BuildArgs {
    vault: PathBuf,
    n_cx: usize,
    dim: usize,
    slots: usize,
    seed: u64,
}

impl BuildArgs {
    fn parse(args: &[String]) -> CliResult<Self> {
        let flags = Flags::new(args);
        let parsed = Self {
            vault: flags.path("--vault")?,
            n_cx: flags.usize("--n-cx")?,
            dim: flags.usize("--dim")?,
            slots: flags.usize("--slots")?,
            seed: flags.u64("--seed")?,
        };
        require_positive(parsed.n_cx, "--n-cx")?;
        require_positive(parsed.dim, "--dim")?;
        require_positive(parsed.slots, "--slots")?;
        Ok(parsed)
    }
}

struct SearchArgs {
    vault: PathBuf,
    strategy: String,
    n: usize,
    k: usize,
    seed: u64,
    beamwidth: usize,
    posting_cutoff: usize,
    tuner_slo_us: Option<u64>,
}

impl SearchArgs {
    fn parse(args: &[String]) -> CliResult<Self> {
        let flags = Flags::new(args);
        let strategy = flags.string("--strategy")?;
        if strategy != "KernelFirst" {
            return Err(CliError::usage("--strategy must be KernelFirst"));
        }
        let parsed = Self {
            vault: flags.path("--vault")?,
            strategy,
            n: flags.usize("--n")?,
            k: flags.optional_usize("--k")?.unwrap_or(10),
            seed: flags.u64("--seed")?,
            beamwidth: flags.optional_usize("--beamwidth")?.unwrap_or(64),
            posting_cutoff: flags.optional_usize("--posting-cutoff")?.unwrap_or(1024),
            tuner_slo_us: flags.optional_u64("--tuner-slo-us")?,
        };
        require_positive(parsed.n, "--n")?;
        require_positive(parsed.k, "--k")?;
        require_positive(parsed.beamwidth, "--beamwidth")?;
        require_positive(parsed.posting_cutoff, "--posting-cutoff")?;
        Ok(parsed)
    }
}

struct RecallArgs {
    vault: PathBuf,
    n: usize,
    k: usize,
    seed: u64,
}

impl RecallArgs {
    fn parse(args: &[String]) -> CliResult<Self> {
        let flags = Flags::new(args);
        let parsed = Self {
            vault: flags.path("--vault")?,
            n: flags.usize("--n")?,
            k: flags.usize("--k")?,
            seed: flags.optional_u64("--seed")?.unwrap_or(42),
        };
        require_positive(parsed.n, "--n")?;
        require_positive(parsed.k, "--k")?;
        Ok(parsed)
    }
}

fn require_positive(value: usize, name: &str) -> CliResult {
    if value == 0 {
        Err(CliError::usage(format!("{name} must be positive")))
    } else {
        Ok(())
    }
}

struct Flags<'a> {
    args: &'a [String],
}

impl<'a> Flags<'a> {
    fn new(args: &'a [String]) -> Self {
        Self { args }
    }

    fn value(&self, name: &str) -> CliResult<&'a str> {
        self.optional_value(name)?
            .ok_or_else(|| CliError::usage(format!("missing {name}")))
    }

    fn optional_value(&self, name: &str) -> CliResult<Option<&'a str>> {
        let mut idx = 0;
        while idx < self.args.len() {
            if self.args[idx] == name {
                return self
                    .args
                    .get(idx + 1)
                    .map(String::as_str)
                    .ok_or_else(|| CliError::usage(format!("{name} requires a value")))
                    .map(Some);
            }
            idx += 2;
        }
        Ok(None)
    }

    fn path(&self, name: &str) -> CliResult<PathBuf> {
        Ok(PathBuf::from(self.value(name)?))
    }

    fn string(&self, name: &str) -> CliResult<String> {
        Ok(self.value(name)?.to_string())
    }

    fn usize(&self, name: &str) -> CliResult<usize> {
        self.value(name)?
            .parse()
            .map_err(|error| CliError::usage(format!("invalid {name}: {error}")))
    }

    fn u64(&self, name: &str) -> CliResult<u64> {
        self.value(name)?
            .parse()
            .map_err(|error| CliError::usage(format!("invalid {name}: {error}")))
    }

    fn optional_usize(&self, name: &str) -> CliResult<Option<usize>> {
        self.optional_value(name)?
            .map(|value| {
                value
                    .parse()
                    .map_err(|error| CliError::usage(format!("invalid {name}: {error}")))
            })
            .transpose()
    }

    fn optional_u64(&self, name: &str) -> CliResult<Option<u64>> {
        self.optional_value(name)?
            .map(|value| {
                value
                    .parse()
                    .map_err(|error| CliError::usage(format!("invalid {name}: {error}")))
            })
            .transpose()
    }
}

#[derive(Serialize)]
struct BuildReport {
    trigger: &'static str,
    manifest_path: &'static str,
    n_cx: usize,
    dim: usize,
    slots: usize,
    seed: u64,
    files: Vec<FileReadback>,
}

#[derive(Serialize)]
struct SearchReport {
    trigger: &'static str,
    strategy: String,
    n: usize,
    k: usize,
    latency_us: LatencySummary,
    self_recall_at_k: f32,
    tuner_status_path: &'static str,
    graph: FileReadback,
    centroids: FileReadback,
}

#[derive(Serialize)]
struct RecallReport {
    trigger: &'static str,
    n: usize,
    k: usize,
    recall_at_k: f32,
    graph: FileReadback,
    centroids: FileReadback,
}

#[derive(Serialize)]
struct TunerStatus {
    tuner: &'static str,
    current: BwPostcutoffConfig,
    adjustments: Vec<calyx_sextant::TunerAdjustment>,
    ledger_entries: Vec<calyx_sextant::TunerLedgerEntry>,
    warnings: Vec<calyx_sextant::TunerWarning>,
    observations: usize,
    search_report_path: &'static str,
}

#[derive(Serialize)]
struct LatencySummary {
    p50: u64,
    p99: u64,
    p999: u64,
}

#[derive(Serialize)]
struct FileReadback {
    path: String,
    size: u64,
}
