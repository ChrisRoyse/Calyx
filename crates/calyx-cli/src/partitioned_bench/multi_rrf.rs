use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use calyx_core::SlotId;
use calyx_sextant::fusion;
use calyx_sextant::index::{DenseVectorFile, PartitionedSearch};
use calyx_sextant::{FusionContext, FusionStrategy, IndexSearchHit};
use rayon::prelude::*;
use serde::Deserialize;
use serde_json::json;

use super::{enforce_recall_floor, percentiles, row_for_metric};
use crate::error::{CliError, CliResult};
#[cfg(test)]
use ids::low_u64;
use ids::{fused_hit_ids, hit_ids, slot_id, to_index_hits};

#[path = "multi_rrf/a35.rs"]
mod a35;
#[path = "multi_rrf/ensemble.rs"]
mod ensemble;
#[path = "multi_rrf/ground_truth.rs"]
mod ground_truth;
#[path = "multi_rrf/ids.rs"]
mod ids;
#[path = "multi_rrf/io.rs"]
mod io;
#[path = "multi_rrf/recall.rs"]
mod recall;
#[path = "multi_rrf/report.rs"]
mod report;
#[path = "multi_rrf/slot_truth.rs"]
mod slot_truth;
#[path = "multi_rrf/timeline.rs"]
mod timeline;
#[path = "multi_rrf/truth_gate.rs"]
mod truth_gate;
#[path = "multi_rrf/tuner.rs"]
mod tuner;

const DEFAULT_TRUTH_DEPTH: usize = 64;

#[derive(Clone, Debug)]
struct Args {
    plan: PathBuf,
    n: usize,
    k: usize,
    n_probe: usize,
    region_beam: usize,
    ground_truth: usize,
    recall_floor: Option<f32>,
    truth_depth: Option<usize>,
    fused_ground_truth_file: Option<PathBuf>,
    fused_ground_truth_manifest: Option<PathBuf>,
    slot_ground_truth_manifest: Option<PathBuf>,
    ensemble_card: Option<PathBuf>,
    write_fused_ground_truth_file: Option<PathBuf>,
    write_fused_ground_truth_manifest: Option<PathBuf>,
    out: Option<PathBuf>,
    anneal_vault: Option<PathBuf>,
    tuner_slo_us: Option<u64>,
}

#[derive(Clone, Debug, Deserialize)]
struct Plan {
    #[serde(default)]
    timeline: Option<PathBuf>,
    slots: Vec<PlanSlot>,
}

#[derive(Clone, Debug, Deserialize)]
struct PlanSlot {
    slot: u16,
    name: Option<String>,
    lens_id: Option<String>,
    weights_sha256: Option<String>,
    signal_kind: Option<String>,
    bits_about: Option<f32>,
    vault: PathBuf,
    queries: PathBuf,
    corpus: PathBuf,
}

struct OpenSlot {
    spec: PlanSlot,
    search: PartitionedSearch,
    queries: DenseVectorFile,
    corpus: DenseVectorFile,
    distance_metric: calyx_sextant::index::PartitionDistanceMetric,
}

impl Args {
    fn parse(raw: &[String]) -> CliResult<Self> {
        let mut plan = None;
        let (mut n, mut k, mut n_probe, mut region_beam) = (1000, 10, 8, 64);
        let mut ground_truth = 0;
        let mut recall_floor = None;
        let mut truth_depth = None;
        let mut fused_ground_truth_file = None;
        let mut fused_ground_truth_manifest = None;
        let mut slot_ground_truth_manifest = None;
        let mut ensemble_card = None;
        let mut write_fused_ground_truth_file = None;
        let mut write_fused_ground_truth_manifest = None;
        let mut out = None;
        let mut anneal_vault = None;
        let mut tuner_slo_us = None;
        let mut it = raw.iter();
        while let Some(flag) = it.next() {
            let mut next = || {
                it.next()
                    .cloned()
                    .ok_or_else(|| CliError::usage(format!("{flag} requires a value")))
            };
            match flag.as_str() {
                "--plan" => plan = Some(PathBuf::from(next()?)),
                "--n" => n = parse(&next()?, "--n")?,
                "--k" => k = parse(&next()?, "--k")?,
                "--n-probe" => n_probe = parse(&next()?, "--n-probe")?,
                "--region-beam" => region_beam = parse(&next()?, "--region-beam")?,
                "--ground-truth" => ground_truth = parse(&next()?, "--ground-truth")?,
                "--recall-floor" => recall_floor = Some(super::parse_recall_floor(&next()?)?),
                "--truth-depth" => truth_depth = Some(parse(&next()?, "--truth-depth")?),
                "--fused-ground-truth-file" => {
                    fused_ground_truth_file = Some(PathBuf::from(next()?))
                }
                "--fused-ground-truth-manifest" => {
                    fused_ground_truth_manifest = Some(PathBuf::from(next()?))
                }
                "--slot-ground-truth-manifest" => {
                    slot_ground_truth_manifest = Some(PathBuf::from(next()?))
                }
                "--ensemble-card" => ensemble_card = Some(PathBuf::from(next()?)),
                "--write-fused-ground-truth-file" => {
                    write_fused_ground_truth_file = Some(PathBuf::from(next()?))
                }
                "--write-fused-ground-truth-manifest" => {
                    write_fused_ground_truth_manifest = Some(PathBuf::from(next()?))
                }
                "--out" => out = Some(PathBuf::from(next()?)),
                "--anneal-vault" => anneal_vault = Some(PathBuf::from(next()?)),
                "--tuner-slo-us" => {
                    let value = parse(&next()?, "--tuner-slo-us")?;
                    if value == 0 {
                        return Err(CliError::usage("--tuner-slo-us must be > 0"));
                    }
                    tuner_slo_us = Some(value);
                }
                other => return Err(CliError::usage(format!("unknown flag: {other}"))),
            }
        }
        let plan = plan.ok_or_else(|| CliError::usage("--plan <json> is required"))?;
        if k == 0 {
            return Err(CliError::usage("--k must be > 0"));
        }
        validate_truth_args(
            fused_ground_truth_file.as_ref(),
            fused_ground_truth_manifest.as_ref(),
            slot_ground_truth_manifest.as_ref(),
            write_fused_ground_truth_file.as_ref(),
            write_fused_ground_truth_manifest.as_ref(),
        )?;
        Ok(Self {
            plan,
            n,
            k,
            n_probe,
            region_beam,
            ground_truth,
            recall_floor,
            truth_depth,
            fused_ground_truth_file,
            fused_ground_truth_manifest,
            slot_ground_truth_manifest,
            ensemble_card,
            write_fused_ground_truth_file,
            write_fused_ground_truth_manifest,
            out,
            anneal_vault,
            tuner_slo_us,
        })
    }
}

fn validate_truth_args(
    fused_file: Option<&PathBuf>,
    fused_manifest: Option<&PathBuf>,
    slot_manifest: Option<&PathBuf>,
    write_file: Option<&PathBuf>,
    write_manifest: Option<&PathBuf>,
) -> CliResult {
    if fused_file.is_some() != fused_manifest.is_some() {
        return Err(CliError::usage(
            "--fused-ground-truth-file requires --fused-ground-truth-manifest",
        ));
    }
    if write_file.is_some() != write_manifest.is_some() {
        return Err(CliError::usage(
            "--write-fused-ground-truth-file requires --write-fused-ground-truth-manifest",
        ));
    }
    if fused_file.is_some() && write_file.is_some() {
        return Err(CliError::usage(
            "precomputed and generated fused ground truth are mutually exclusive in one run",
        ));
    }
    if fused_file.is_some() && slot_manifest.is_some() {
        return Err(CliError::usage(
            "--fused-ground-truth-file and --slot-ground-truth-manifest are mutually exclusive",
        ));
    }
    Ok(())
}

pub(crate) fn run(raw: &[String]) -> CliResult {
    let args = Args::parse(raw)?;
    let plan = load_plan(&args.plan)?;
    a35::validate_plan(&plan)?;
    let ensemble_readback = ensemble::load(
        args.ensemble_card.as_deref(),
        &plan,
        args.recall_floor.is_some(),
    )?;
    let slots = open_slots(&plan)?;
    let n = slots
        .iter()
        .fold(args.n, |acc, slot| acc.min(slot.queries.count() as usize));
    let corpus_rows = slots
        .iter()
        .fold(u64::MAX, |acc, slot| acc.min(slot.corpus.count())) as usize;
    let timeline = plan
        .timeline
        .as_ref()
        .map(|path| {
            timeline::Timeline::load(&timeline::resolve_plan_path(&args.plan, path), corpus_rows)
        })
        .transpose()?;
    let truth_n = args.ground_truth.min(n);
    timeline::enforce_gate(args.recall_floor.is_some(), timeline.as_ref(), truth_n)?;
    let truth_depth = args
        .truth_depth
        .unwrap_or_else(|| DEFAULT_TRUTH_DEPTH.max(args.k * 8))
        .max(args.k);
    let precomputed_truth = match (
        args.fused_ground_truth_file.as_ref(),
        args.fused_ground_truth_manifest.as_ref(),
    ) {
        (Some(file), Some(manifest)) if truth_n > 0 => Some(ground_truth::PrecomputedTruth::load(
            ground_truth::Context {
                truth_file: file,
                manifest_file: manifest,
                plan_path: &args.plan,
                plan: &plan,
                truth_n,
                k: args.k,
                truth_depth,
                corpus_rows,
            },
        )?),
        _ => None,
    };
    let slot_truth = match args.slot_ground_truth_manifest.as_ref() {
        Some(manifest) if truth_n > 0 => Some(slot_truth::SlotTruth::load(slot_truth::Context {
            manifest_file: manifest,
            plan_path: &args.plan,
            plan: &plan,
            truth_n,
            truth_depth,
            corpus_rows,
        })?),
        Some(_) => {
            return Err(CliError::usage(
                "--slot-ground-truth-manifest requires --ground-truth > 0",
            ));
        }
        None => None,
    };
    let scale_truth = precomputed_truth
        .as_ref()
        .is_some_and(ground_truth::PrecomputedTruth::scale_suitable)
        || slot_truth
            .as_ref()
            .is_some_and(slot_truth::SlotTruth::scale_suitable);
    truth_gate::enforce(args.recall_floor.is_some(), truth_n, scale_truth)?;
    if n == 0 {
        return Err(CliError::usage(
            "partitioned-rrf has zero query rows across plan slots",
        ));
    }
    let mut fused_latencies_us = Vec::with_capacity(n);
    let mut fused_hits_for_truth = Vec::with_capacity(truth_n);
    let mut single_hits_for_truth: BTreeMap<SlotId, Vec<Vec<u64>>> = slots
        .iter()
        .map(|slot| (slot_id(slot.spec.slot), Vec::with_capacity(truth_n)))
        .collect();
    for query_idx in 0..n {
        let started = Instant::now();
        let mut per_slot = BTreeMap::new();
        let slot_hits = slots
            .par_iter()
            .map(|slot| {
                let query = row_for_metric(&slot.queries, query_idx as u64, slot.distance_metric);
                let raw_hits = slot
                    .search
                    .search(&query, truth_depth, args.n_probe, args.region_beam)
                    .map_err(CliError::Calyx)?;
                Ok((slot_id(slot.spec.slot), to_index_hits(raw_hits)))
            })
            .collect::<CliResult<Vec<_>>>()?;
        for (slot, hits) in slot_hits {
            if query_idx < truth_n {
                single_hits_for_truth
                    .get_mut(&slot)
                    .expect("slot seeded")
                    .push(hit_ids(&hits, truth_depth));
            }
            per_slot.insert(slot, hits);
        }
        let fused = fuse(&per_slot, args.k);
        fused_latencies_us
            .push((started.elapsed().as_micros().min(u128::from(u64::MAX)) as u64).max(1));
        if query_idx < truth_n {
            fused_hits_for_truth.push(fused_hit_ids(&fused, args.k));
        }
    }
    let recall = if truth_n > 0 {
        recall::readback(recall::Request {
            slots: &slots,
            truth_n,
            truth_depth,
            k: args.k,
            fused_hits: &fused_hits_for_truth,
            single_hits: &single_hits_for_truth,
            timeline: timeline.as_ref(),
            precomputed_truth: precomputed_truth.as_ref(),
            slot_truth: slot_truth.as_ref(),
        })
    } else {
        recall::RecallReadback::default()
    };
    let written_ground_truth = match (
        args.write_fused_ground_truth_file.as_ref(),
        args.write_fused_ground_truth_manifest.as_ref(),
    ) {
        (Some(file), Some(manifest)) if truth_n > 0 => Some(ground_truth::write(
            &recall.exact_fused_rows,
            ground_truth::Context {
                truth_file: file,
                manifest_file: manifest,
                plan_path: &args.plan,
                plan: &plan,
                truth_n,
                k: args.k,
                truth_depth,
                corpus_rows,
            },
        )?),
        _ => None,
    };
    enforce_recall_floor(args.recall_floor, truth_n, recall.fused_recall)?;
    let latency_us = percentiles(&fused_latencies_us);
    let tuner_status_path = if let Some(vault) = &args.anneal_vault {
        Some(tuner::write_status(tuner::StatusRequest {
            vault,
            latencies_us: &fused_latencies_us[..truth_n],
            per_query_recall: &recall.per_query_recall,
            region_beam: args.region_beam,
            n_probe: args.n_probe,
            tuner_slo_us: args.tuner_slo_us,
            recall_floor: args.recall_floor,
            report_path: args.out.as_deref(),
            report_latency_us: &latency_us,
            fused_recall: recall.fused_recall,
            lens_count: slots.len(),
            queries: n,
            k: args.k,
        })?)
    } else {
        None
    };
    let report = json!({
        "trigger": "calyx bench partitioned-rrf",
        "mode": "real_multi_slot_rrf",
        "metric_class": report::METRIC_CLASS,
        "metric_scope": report::METRIC_SCOPE,
        "ann_correctness_contract": report::ann_correctness_contract(),
        "grounded_phase_exit_contract": report::grounded_phase_exit_contract(),
        "plan": args.plan,
        "lens_roster": a35::lens_roster(&slots),
        "per_lens_bits": a35::per_lens_bits(&slots),
        "ensemble_decomposition": ensemble_readback,
        "temporal": timeline.as_ref().map(|timeline| timeline.report()),
        "slots": report::slot_report(&slots),
        "queries": n,
        "k": args.k,
        "n_probe": args.n_probe,
        "region_beam": args.region_beam,
        "per_slot_search_depth": truth_depth,
        "truth_depth": truth_depth,
        "ground_truth_queries": truth_n,
        "ground_truth_source": recall.ground_truth_source,
        "written_fused_ground_truth_source": written_ground_truth,
        "latency_us": latency_us.clone(),
        "fused_ground_truth_recall_at_k": recall.fused_recall,
        "fused_result": a35::fused_result(recall.fused_recall, &latency_us, &recall.sample_readback),
        "best_single_lens_recall_vs_fused_truth": recall.best_single,
        "fusion_matches_or_beats_best_single": recall.fused_recall.zip(recall.best_single).map(|(fused, single)| fused + f32::EPSILON >= single),
        "best_two_lens_rrf_control": recall.best_two_lens_rrf_control,
        "per_slot_recall_vs_fused_truth": recall.per_slot_recall,
        "sample_readback": recall.sample_readback,
        "recall_floor": args.recall_floor,
        "tuner_status_path": tuner_status_path,
    });
    let bytes = serde_json::to_vec_pretty(&report)?;
    if let Some(path) = &args.out {
        io::write_bytes_atomic(path, &bytes)?;
    }
    println!("{}", String::from_utf8(bytes).expect("json is utf8"));
    Ok(())
}

fn load_plan(path: &Path) -> CliResult<Plan> {
    let plan: Plan = serde_json::from_slice(&std::fs::read(path)?)?;
    let mut seen = std::collections::BTreeSet::new();
    for slot in &plan.slots {
        if !seen.insert(slot.slot) {
            return Err(CliError::usage(format!(
                "partitioned-rrf plan has duplicate slot {}",
                slot.slot
            )));
        }
    }
    Ok(plan)
}

fn open_slots(plan: &Plan) -> CliResult<Vec<OpenSlot>> {
    plan.slots
        .iter()
        .map(|slot| {
            let search = PartitionedSearch::open(&slot.vault).map_err(CliError::Calyx)?;
            let queries = DenseVectorFile::open(&slot.queries).map_err(CliError::Calyx)?;
            let corpus = DenseVectorFile::open(&slot.corpus).map_err(CliError::Calyx)?;
            if queries.dim() != search.dim() || corpus.dim() != search.dim() {
                return Err(CliError::usage(format!(
                    "slot {} dim mismatch: vault={} queries={} corpus={}",
                    slot.slot,
                    search.dim(),
                    queries.dim(),
                    corpus.dim()
                )));
            }
            Ok(OpenSlot {
                spec: slot.clone(),
                distance_metric: search.manifest().distance_metric,
                search,
                queries,
                corpus,
            })
        })
        .collect()
}

fn fuse(per_slot: &BTreeMap<SlotId, Vec<IndexSearchHit>>, k: usize) -> Vec<calyx_sextant::Hit> {
    let context = FusionContext {
        k,
        explain: false,
        strategy: FusionStrategy::Rrf,
        weights: BTreeMap::new(),
        stage1_slots: Vec::new(),
    };
    fusion::fuse(per_slot, &context)
}

fn parse<T: std::str::FromStr>(value: &str, flag: &str) -> CliResult<T> {
    value
        .parse::<T>()
        .map_err(|_| CliError::usage(format!("{flag} expects a valid value, got {value}")))
}

#[cfg(test)]
#[path = "multi_rrf/tests.rs"]
mod tests;
