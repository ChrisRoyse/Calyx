use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

use calyx_core::{CxId, SlotId};
use calyx_sextant::fusion;
use calyx_sextant::index::partitioned::cx;
use calyx_sextant::index::{DenseVectorFile, PartitionedSearch};
use calyx_sextant::{FusionContext, FusionStrategy, IndexSearchHit};
use rayon::prelude::*;
use serde::Deserialize;
use serde_json::json;

use super::brute_force::brute_force_topk_vecfile_ranked;
use super::{enforce_recall_floor, percentiles, row_for_metric};
use crate::error::{CliError, CliResult};

#[path = "multi_rrf/a35.rs"]
mod a35;
#[path = "multi_rrf/timeline.rs"]
mod timeline;
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
    lens_id: Option<String>,
    weights_sha256: Option<String>,
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
        Ok(Self {
            plan,
            n,
            k,
            n_probe,
            region_beam,
            ground_truth,
            recall_floor,
            truth_depth,
            out,
            anneal_vault,
            tuner_slo_us,
        })
    }
}

pub(crate) fn run(raw: &[String]) -> CliResult {
    let args = Args::parse(raw)?;
    let plan = load_plan(&args.plan)?;
    a35::validate_plan(&plan)?;
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
    let truth_depth = args
        .truth_depth
        .unwrap_or_else(|| DEFAULT_TRUTH_DEPTH.max(args.k * 8))
        .max(args.k);
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
                    .push(hit_ids(&hits, args.k));
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
        recall_readback(
            &slots,
            truth_n,
            truth_depth,
            args.k,
            &fused_hits_for_truth,
            &single_hits_for_truth,
            timeline.as_ref(),
        )
    } else {
        RecallReadback::default()
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
        "plan": args.plan,
        "lens_roster": a35::lens_roster(&slots),
        "per_lens_bits": a35::per_lens_bits(&slots),
        "temporal": timeline.as_ref().map(|timeline| timeline.report()),
        "slots": slot_report(&slots),
        "queries": n,
        "k": args.k,
        "n_probe": args.n_probe,
        "region_beam": args.region_beam,
        "per_slot_search_depth": truth_depth,
        "truth_depth": truth_depth,
        "ground_truth_queries": truth_n,
        "latency_us": latency_us.clone(),
        "fused_ground_truth_recall_at_k": recall.fused_recall,
        "fused_result": a35::fused_result(recall.fused_recall, &latency_us, &recall.sample_readback),
        "best_single_lens_recall_vs_fused_truth": recall.best_single,
        "fusion_matches_or_beats_best_single": recall.fused_recall.zip(recall.best_single).map(|(fused, single)| fused + f32::EPSILON >= single),
        "per_slot_recall_vs_fused_truth": recall.per_slot_recall,
        "sample_readback": recall.sample_readback,
        "recall_floor": args.recall_floor,
        "tuner_status_path": tuner_status_path,
    });
    let bytes = serde_json::to_vec_pretty(&report)?;
    if let Some(path) = &args.out {
        write_bytes_atomic(path, &bytes)?;
    }
    println!("{}", String::from_utf8(bytes).expect("json is utf8"));
    Ok(())
}

#[derive(Default)]
struct RecallReadback {
    fused_recall: Option<f32>,
    per_slot_recall: Vec<serde_json::Value>,
    best_single: Option<f32>,
    sample_readback: Vec<serde_json::Value>,
    per_query_recall: Vec<f32>,
}

fn load_plan(path: &Path) -> CliResult<Plan> {
    let plan: Plan = serde_json::from_slice(&std::fs::read(path)?)?;
    let mut seen = BTreeSet::new();
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

fn recall_readback(
    slots: &[OpenSlot],
    truth_n: usize,
    truth_depth: usize,
    k: usize,
    fused_hits: &[Vec<u64>],
    single_hits: &BTreeMap<SlotId, Vec<Vec<u64>>>,
    timeline: Option<&timeline::Timeline>,
) -> RecallReadback {
    let mut single_found: BTreeMap<SlotId, usize> = slots
        .iter()
        .map(|slot| (slot_id(slot.spec.slot), 0))
        .collect();
    let mut fused_found = 0usize;
    let mut total = 0usize;
    let mut sample_readback = Vec::new();
    let mut per_query_recall = Vec::with_capacity(truth_n);
    for query_idx in 0..truth_n {
        let mut exact_per_slot = BTreeMap::new();
        let mut exact_slot_rows = Vec::new();
        for slot in slots {
            let query = row_for_metric(&slot.queries, query_idx as u64, slot.distance_metric);
            let exact = brute_force_topk_vecfile_ranked(
                &slot.corpus,
                &[query],
                truth_depth,
                slot.distance_metric,
            )
            .pop()
            .expect("one query");
            exact_slot_rows.push(json!({
                "slot": slot.spec.slot,
                "exact_top_k": exact.iter().take(k).map(|(id, _)| *id).collect::<Vec<_>>(),
            }));
            exact_per_slot.insert(slot_id(slot.spec.slot), to_index_hits(exact));
        }
        let exact_fused = fuse(&exact_per_slot, k);
        let exact_ids = fused_hit_ids(&exact_fused, k);
        let truth = exact_ids.iter().copied().collect::<BTreeSet<_>>();
        if sample_readback.len() < 3 {
            let mut row = json!({
                "query_idx": query_idx,
                "partitioned_fused_top_k": fused_hits[query_idx],
                "exact_fused_top_k": exact_ids,
                "per_slot_exact_top_k": exact_slot_rows,
            });
            if let Some(timeline) = timeline {
                row["query_timeline"] = timeline.row_value(query_idx);
                row["partitioned_fused_timeline"] = timeline.rows_value(&fused_hits[query_idx]);
                row["exact_fused_timeline"] = timeline.rows_value(&exact_ids);
                row["time_walk"] = timeline.time_walk(query_idx);
            }
            sample_readback.push(row);
        }
        let truth_len = truth.len();
        let query_found = fused_hits[query_idx]
            .iter()
            .filter(|id| truth.contains(id))
            .count();
        total += truth_len;
        fused_found += query_found;
        per_query_recall.push(query_found as f32 / truth_len.max(1) as f32);
        for (slot, rows) in single_hits {
            let found = rows[query_idx]
                .iter()
                .filter(|id| truth.contains(id))
                .count();
            *single_found.get_mut(slot).expect("slot seeded") += found;
        }
    }
    let denom = total.max(1) as f32;
    let per_slot = single_found
        .into_iter()
        .map(|(slot, found)| {
            json!({
                "slot": slot.get(),
                "recall_at_k": found as f32 / denom,
            })
        })
        .collect::<Vec<_>>();
    let best = per_slot
        .iter()
        .filter_map(|row| row["recall_at_k"].as_f64().map(|value| value as f32))
        .max_by(f32::total_cmp);
    RecallReadback {
        fused_recall: Some(fused_found as f32 / denom),
        per_slot_recall: per_slot,
        best_single: best,
        sample_readback,
        per_query_recall,
    }
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

fn to_index_hits(rows: Vec<(u64, f32)>) -> Vec<IndexSearchHit> {
    rows.into_iter()
        .enumerate()
        .map(|(idx, (id, score))| IndexSearchHit {
            cx_id: cx(id),
            score,
            rank: idx + 1,
        })
        .collect()
}

fn hit_ids(hits: &[IndexSearchHit], k: usize) -> Vec<u64> {
    hits.iter().take(k).map(|hit| low_u64(hit.cx_id)).collect()
}

fn fused_hit_ids(hits: &[calyx_sextant::Hit], k: usize) -> Vec<u64> {
    hits.iter().take(k).map(|hit| low_u64(hit.cx_id)).collect()
}

fn write_bytes_atomic(path: &Path, bytes: &[u8]) -> CliResult {
    if path.exists() {
        return Err(CliError::usage(format!(
            "--out {} already exists; remove it before re-running",
            path.display()
        )));
    }
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    {
        let mut file = File::create(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    fs::rename(&tmp, path).inspect_err(|_| {
        let _ = fs::remove_file(&tmp);
    })?;
    Ok(())
}

fn slot_report(slots: &[OpenSlot]) -> Vec<serde_json::Value> {
    slots
        .iter()
        .map(|slot| {
            json!({
                "slot": slot.spec.slot,
                "lens_id": slot.spec.lens_id.as_deref().expect("A35 validated"),
                "weights_sha256": slot.spec.weights_sha256.as_deref().expect("A35 validated"),
                "bits_about": slot.spec.bits_about.expect("A35 validated"),
                "vault": slot.spec.vault,
                "queries": slot.spec.queries,
                "corpus": slot.spec.corpus,
                "n_cx": slot.search.manifest().n_cx,
                "dim": slot.search.dim(),
                "n_regions": slot.search.manifest().n_regions,
            })
        })
        .collect()
}

fn parse<T: std::str::FromStr>(value: &str, flag: &str) -> CliResult<T> {
    value
        .parse::<T>()
        .map_err(|_| CliError::usage(format!("{flag} expects a valid value, got {value}")))
}

fn slot_id(value: u16) -> SlotId {
    SlotId::new(value)
}

fn low_u64(cx_id: CxId) -> u64 {
    let bytes = cx_id.as_bytes();
    u64::from_be_bytes(bytes[8..16].try_into().expect("CxId is 16 bytes"))
}

#[cfg(test)]
#[path = "multi_rrf/tests.rs"]
mod tests;
