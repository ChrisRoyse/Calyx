use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use calyx_core::SlotId;
use calyx_sextant::fusion;
use calyx_sextant::index::{DenseVectorFile, PartitionedSearch};
use calyx_sextant::{FusionContext, FusionStrategy, IndexSearchHit};
use rayon::prelude::*;
use serde_json::json;

use super::{enforce_recall_floor, percentiles, row_for_metric};
use crate::error::{CliError, CliResult};
use crate::partitioned_bench::rrf_plan::{self, LoadedPlan};
pub(super) use crate::partitioned_bench::rrf_plan::{Plan, PlanSlot};
use crate::partitioned_rrf_report_store;
#[cfg(test)]
use ids::low_u64;
use ids::{fused_hit_ids, hit_ids, slot_id, to_index_hits};

#[path = "multi_rrf/a35.rs"]
mod a35;
#[path = "multi_rrf/a37_admission.rs"]
mod a37_admission;
#[path = "multi_rrf/args.rs"]
mod args;
#[path = "multi_rrf/ensemble.rs"]
mod ensemble;
#[path = "multi_rrf/fused_truth_db.rs"]
mod fused_truth_db;
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
#[path = "multi_rrf/slot_truth_db.rs"]
mod slot_truth_db;
#[path = "multi_rrf/timeline.rs"]
mod timeline;
#[path = "multi_rrf/truth_gate.rs"]
mod truth_gate;
#[path = "multi_rrf/tuner.rs"]
mod tuner;

const DEFAULT_TRUTH_DEPTH: usize = 64;

struct OpenSlot {
    spec: PlanSlot,
    search: PartitionedSearch,
    queries: DenseVectorFile,
    corpus: DenseVectorFile,
    distance_metric: calyx_sextant::index::PartitionDistanceMetric,
}

pub(crate) fn run(raw: &[String]) -> CliResult {
    let args = args::Args::parse(raw)?;
    let loaded_plan = load_plan(&args)?;
    let plan = &loaded_plan.plan;
    let plan_ref = args
        .plan
        .clone()
        .unwrap_or_else(|| PathBuf::from(format!("aster-graph-cf:{}", args.plan_key)));
    a35::validate_plan(plan)?;
    let a37_admission_readback = a37_admission::load_from_cf(
        args.a37_admission_cf_root.as_deref(),
        &args.a37_admission_key,
        plan,
    )?
    .or(a37_admission::load(
        args.a37_admission_card.as_deref(),
        plan,
    )?);
    let ensemble_readback = ensemble::load(
        args.ensemble_card.as_deref(),
        plan,
        args.recall_floor.is_some() && a37_admission_readback.is_none(),
    )?;
    let slots = open_slots(plan, &loaded_plan.base_dir)?;
    let n = slots
        .iter()
        .fold(args.n, |acc, slot| acc.min(slot.queries.count() as usize));
    let corpus_rows = slots
        .iter()
        .fold(u64::MAX, |acc, slot| acc.min(slot.corpus.count())) as usize;
    let timeline = match args.timeline_cf_root.as_ref() {
        Some(cf_root) => Some(timeline::Timeline::load_from_db(
            cf_root,
            &args.timeline_key,
            corpus_rows,
        )?),
        None => plan
            .timeline
            .as_ref()
            .map(|path| {
                timeline::Timeline::load(
                    &rrf_plan::resolve(&loaded_plan.base_dir, path),
                    corpus_rows,
                )
            })
            .transpose()?,
    };
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
                plan_path: &plan_ref,
                plan_sha256: &loaded_plan.plan_sha256,
                plan,
                truth_n,
                k: args.k,
                truth_depth,
                corpus_rows,
            },
        )?),
        _ => None,
    };
    let db_fused_truth = match args.fused_ground_truth_cf_root.as_ref() {
        Some(cf_root) if truth_n > 0 => Some(fused_truth_db::DbFusedTruth::load(
            fused_truth_db::Context {
                cf_root,
                association_key: &args.fused_ground_truth_key,
                plan_path: &plan_ref,
                plan_sha256: &loaded_plan.plan_sha256,
                plan,
                truth_n,
                k: args.k,
                truth_depth,
                corpus_rows,
            },
        )?),
        Some(_) => {
            return Err(CliError::usage(
                "--fused-ground-truth-cf-root requires --ground-truth > 0",
            ));
        }
        None => None,
    };
    let slot_truth = match args.slot_ground_truth_manifest.as_ref() {
        Some(manifest) if truth_n > 0 => Some(slot_truth::SlotTruth::load(slot_truth::Context {
            manifest_file: manifest,
            plan_path: &plan_ref,
            plan_sha256: &loaded_plan.plan_sha256,
            plan,
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
    let db_slot_truth = match args.slot_ground_truth_cf_root.as_ref() {
        Some(cf_root) if truth_n > 0 => {
            Some(slot_truth_db::DbSlotTruth::load(slot_truth_db::Context {
                cf_root,
                association_key: &args.slot_ground_truth_key,
                plan_path: &plan_ref,
                plan_sha256: &loaded_plan.plan_sha256,
                plan,
                truth_n,
                truth_depth,
                corpus_rows,
            })?)
        }
        Some(_) => {
            return Err(CliError::usage(
                "--slot-ground-truth-cf-root requires --ground-truth > 0",
            ));
        }
        None => None,
    };
    let scale_truth = precomputed_truth
        .as_ref()
        .is_some_and(ground_truth::PrecomputedTruth::scale_suitable)
        || db_fused_truth
            .as_ref()
            .is_some_and(fused_truth_db::DbFusedTruth::scale_suitable)
        || slot_truth
            .as_ref()
            .is_some_and(slot_truth::SlotTruth::scale_suitable)
        || db_slot_truth
            .as_ref()
            .is_some_and(slot_truth_db::DbSlotTruth::scale_suitable);
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
    let search_opts = calyx_sextant::index::PartitionedSearchOptions {
        n_probe: args.n_probe,
        region_beam: args.region_beam,
        pruning_epsilon: args.pruning_epsilon,
    };
    for query_idx in 0..n {
        let started = Instant::now();
        let mut per_slot = BTreeMap::new();
        let slot_hits = slots
            .par_iter()
            .map(|slot| {
                let query = row_for_metric(&slot.queries, query_idx as u64, slot.distance_metric);
                let raw_hits = slot
                    .search
                    .search_with_readback_opts(&query, truth_depth, search_opts)
                    .map_err(CliError::Calyx)?
                    .hits;
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
            db_fused_truth: db_fused_truth.as_ref(),
            slot_truth: slot_truth.as_ref(),
            db_slot_truth: db_slot_truth.as_ref(),
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
                plan_path: &plan_ref,
                plan_sha256: &loaded_plan.plan_sha256,
                plan,
                truth_n,
                k: args.k,
                truth_depth,
                corpus_rows,
            },
        )?),
        _ => match args.write_fused_ground_truth_cf_root.as_ref() {
            Some(cf_root) if truth_n > 0 => Some(fused_truth_db::write(
                &recall.exact_fused_rows,
                fused_truth_db::Context {
                    cf_root,
                    association_key: &args.write_fused_ground_truth_key,
                    plan_path: &plan_ref,
                    plan_sha256: &loaded_plan.plan_sha256,
                    plan,
                    truth_n,
                    k: args.k,
                    truth_depth,
                    corpus_rows,
                },
                scale_truth,
            )?),
            Some(_) => {
                return Err(CliError::usage(
                    "--write-fused-ground-truth-cf-root requires --ground-truth > 0",
                ));
            }
            None => None,
        },
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
    let mut report = json!({
        "trigger": "calyx bench partitioned-rrf",
        "mode": "real_multi_slot_rrf",
        "metric_class": report::METRIC_CLASS,
        "metric_scope": report::METRIC_SCOPE,
        "ann_correctness_contract": report::ann_correctness_contract(),
        "grounded_phase_exit_contract": report::grounded_phase_exit_contract(),
        "plan": args.plan,
        "plan_source": plan_source_report(&args, &loaded_plan),
        "plan_sha256": loaded_plan.plan_sha256,
        "lens_roster": a35::lens_roster(&slots),
        "per_lens_bits": a35::per_lens_bits(&slots),
        "ensemble_decomposition": ensemble_readback,
        "a37_admission": a37_admission_readback,
        "temporal": timeline.as_ref().map(|timeline| timeline.report()),
        "slots": report::slot_report(&slots),
        "queries": n,
        "k": args.k,
        "n_probe": args.n_probe,
        "region_beam": args.region_beam,
        "pruning_epsilon": args.pruning_epsilon,
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
    let report_db_readback = match args.report_cf_root.as_ref() {
        Some(cf_root) => Some(
            partitioned_rrf_report_store::write(cf_root, &args.report_key, &report)
                .map_err(CliError::from)?,
        ),
        None => None,
    };
    if let Some(readback) = &report_db_readback
        && let Some(object) = report.as_object_mut()
    {
        object.insert("report_db_readback".to_string(), json!(readback));
    }
    let bytes = serde_json::to_vec_pretty(&report)
        .map_err(|error| CliError::runtime(format!("serialize partitioned-rrf report: {error}")))?;
    if let Some(path) = &args.out {
        io::write_bytes_atomic(path, &bytes)?;
    }
    if !args.report_db_only {
        println!("{}", String::from_utf8(bytes).expect("json is utf8"));
    }
    Ok(())
}

fn load_plan(args: &args::Args) -> CliResult<LoadedPlan> {
    if let Some(path) = &args.plan {
        return rrf_plan::load_from_file(path);
    }
    let cf_root = args.plan_cf_root.as_ref().expect("validated");
    rrf_plan::load_from_db(cf_root, &args.plan_key)
}

fn open_slots(plan: &Plan, base_dir: &Path) -> CliResult<Vec<OpenSlot>> {
    plan.slots
        .iter()
        .map(|slot| {
            let vault_path = rrf_plan::resolve(base_dir, &slot.vault);
            let queries_path = rrf_plan::resolve(base_dir, &slot.queries);
            let corpus_path = rrf_plan::resolve(base_dir, &slot.corpus);
            let search = PartitionedSearch::open(&vault_path).map_err(CliError::Calyx)?;
            let queries = DenseVectorFile::open(&queries_path).map_err(CliError::Calyx)?;
            let corpus = DenseVectorFile::open(&corpus_path).map_err(CliError::Calyx)?;
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

fn plan_source_report(args: &args::Args, loaded: &LoadedPlan) -> serde_json::Value {
    match &loaded.db_readback {
        Some(readback) => json!({
            "mode": "aster_graph_cf",
            "cf_root": args.plan_cf_root,
            "association_key": args.plan_key,
            "base_dir": loaded.base_dir,
            "db_readback": readback,
        }),
        None => json!({
            "mode": "legacy_json_import",
            "path": args.plan,
            "base_dir": loaded.base_dir,
        }),
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

#[cfg(test)]
#[path = "multi_rrf/tests.rs"]
mod tests;
