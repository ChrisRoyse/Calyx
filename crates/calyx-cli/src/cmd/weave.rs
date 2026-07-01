//! `calyx weave-loom <vault>` — corpus-scale Loom weave (#870).
//!
//! Populates the **XTerm CF** with within-doc cross-lens agreement cross-terms,
//! and the **graph CF** with the between-doc directed k-NN association graph
//! (nodes = constellations, edges = panel-measured nearest neighbours via the
//! persisted DiskANN index). Emits the acceptance report: XTerm rows persisted,
//! the corpus slot-pair agreement graph, and the association graph's
//! node/edge/groundedness counts. Fail-closed throughout — no fallbacks.

mod coverage;
mod parse;
mod passes;
mod progress;

use std::fs;
use std::path::PathBuf;

use calyx_aster::plain_graph::PlainGraph;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{SlotId, SlotShape, SlotState};
use calyx_lodestar::{
    AsterAssocMetadata, CorpusWeaveReportParams, DEFAULT_ASTER_ASSOC_COLLECTION,
    corpus_weave_report, write_assoc_metadata,
};
use calyx_registry::load_vault_panel_state;
use serde::Serialize;
use serde_json::{Value, json};

use super::Subcommand;
use super::vault::{home_dir, resolve_vault_info, vault_salt};
use crate::bounded_progress::Deadline;
use crate::error::{CliError, CliResult};
use crate::output::print_json;

const DEFAULT_KNN: usize = 16;
const DEFAULT_EDGE_COS_THRESHOLD: f32 = 0.5;
const DEFAULT_MAX_GROUNDEDNESS_DISTANCE: usize = 3;
const DEFAULT_BATCH: usize = 512;

pub(crate) use parse::parse_weave_loom;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct WeaveLoomArgs {
    pub vault: String,
    pub content_slot: Option<u16>,
    pub knn: usize,
    pub edge_cos_threshold: f32,
    pub max_groundedness_distance: usize,
    pub batch: usize,
    /// Cap the number of constellations processed (0 = all). For bounded FSV
    /// runs; the report records the cap so partial runs are never read as full.
    pub limit: usize,
    /// Internal wall-clock budget. If exceeded, persist an incomplete progress
    /// artifact and return CALYX_CLI_TIMEOUT before an outer supervisor kill.
    pub time_budget_ms: Option<u64>,
}

impl Default for WeaveLoomArgs {
    fn default() -> Self {
        Self {
            vault: String::new(),
            content_slot: None,
            knn: DEFAULT_KNN,
            edge_cos_threshold: DEFAULT_EDGE_COS_THRESHOLD,
            max_groundedness_distance: DEFAULT_MAX_GROUNDEDNESS_DISTANCE,
            batch: DEFAULT_BATCH,
            limit: 0,
            time_budget_ms: None,
        }
    }
}

pub(crate) fn run(command: Subcommand) -> CliResult {
    let Subcommand::WeaveLoom(args) = command else {
        unreachable!("non-weave command routed to weave module");
    };
    run_weave_loom(args)
}

fn run_weave_loom(args: WeaveLoomArgs) -> CliResult {
    let resolved = resolve_vault_info(&home_dir()?, &args.vault)?;
    let progress =
        progress::WeaveLoomProgressWriter::create(&resolved.path, &resolved.name, &args)?;
    progress.write("running", "panel_load_start", json!({}))?;
    let state = load_vault_panel_state(&resolved.path)?;
    let content_slots = content_lens_slots(&state.panel);
    let incompatible_content_slots = incompatible_content_lens_slots(&state.panel);
    progress.write(
        "running",
        "panel_load_complete",
        json!({
            "content_slots": content_slots.iter().map(|s| s.get()).collect::<Vec<_>>(),
            "skipped_incompatible_content_slots": incompatible_content_slots,
        }),
    )?;
    if content_slots.len() < 2 {
        let detail = format!(
            "weave-loom needs >=2 active dense content lenses (state=Active, not retrieval_only, shape=Dense); panel has {}; incompatible active content slots={:?}",
            content_slots.len(),
            incompatible_content_slots
        );
        progress.write(
            "coverage_failed",
            "panel_content_slots",
            json!({ "error": detail }),
        )?;
        return Err(CliError::usage(detail));
    }
    let requested_slot = match resolve_requested_slot(
        args.content_slot,
        &content_slots,
        &incompatible_content_slots,
    ) {
        Ok(slot) => slot,
        Err(error) => {
            let _ = progress.write(
                "coverage_failed",
                "content_slot_validation",
                json!({ "error": error_details(&error) }),
            );
            return Err(error);
        }
    };

    let deadline = Deadline::new(args.time_budget_ms);
    progress.write("running", "coverage_preflight_start", json!({}))?;
    let preflight =
        match coverage::dense_slot_preflight(&resolved.path, &content_slots, args.limit, &deadline)
        {
            Ok(preflight) => preflight,
            Err(error) => {
                let _ = progress.write(
                    "incomplete",
                    "coverage_preflight_error",
                    json!({ "error": error_details(&error) }),
                );
                return Err(error);
            }
        };
    progress.write(
        "running",
        "coverage_preflight_complete",
        json!({
            "constellations_in_vault": preflight.constellations_in_vault,
            "candidate_rows": preflight.candidates.len(),
            "base_page_index_live_entries": preflight.base_page_index_live_entries,
            "candidate_order": "base_page_index_key_order",
            "coverage": &preflight.coverage,
        }),
    )?;

    let selection = match coverage::select_slot_from_coverage(requested_slot, &preflight.coverage) {
        Ok(selection) => selection,
        Err(detail) => {
            progress.write(
                "coverage_failed",
                "dense_slot_coverage",
                json!({
                    "error": detail,
                    "coverage": &preflight.coverage,
                    "progress_artifact": progress.path().display().to_string(),
                }),
            )?;
            return Err(coverage::invalid_params(format!(
                "{detail}; coverage artifact persisted at {}",
                progress.path().display()
            )));
        }
    };
    let knn_slot = selection.slot;
    progress.write(
        "running",
        "vault_open_start",
        json!({
            "selected_content_slot": knn_slot.get(),
            "selection_reason": selection.reason,
            "candidate_rows": preflight.candidates.len(),
        }),
    )?;
    let vault = match AsterVault::open(
        &resolved.path,
        resolved.vault_id,
        vault_salt(resolved.vault_id, &resolved.name),
        VaultOptions::default(),
    ) {
        Ok(vault) => vault,
        Err(error) => {
            let error: CliError = error.into();
            let _ = progress.write(
                "incomplete",
                "vault_open_error",
                json!({ "error": error_details(&error) }),
            );
            return Err(error);
        }
    };
    progress.write(
        "running",
        "within_doc_start",
        json!({
            "selected_content_slot": knn_slot.get(),
            "selection_reason": selection.reason,
            "candidate_rows": preflight.candidates.len(),
        }),
    )?;

    let graph = PlainGraph::new(&vault, DEFAULT_ASTER_ASSOC_COLLECTION)?;
    let within = match passes::weave_within_doc(&vault, &graph, &preflight, knn_slot, args.batch) {
        Ok(within) => within,
        Err(error) => {
            let _ = progress.write(
                "incomplete",
                "within_doc_error",
                json!({ "error": error_details(&error) }),
            );
            return Err(error);
        }
    };
    if let Err(error) = deadline.check(
        "weave-loom",
        "within_doc_complete",
        within.constellations_processed as u64,
    ) {
        let _ = progress.write(
            "incomplete",
            "within_doc_timeout",
            json!({ "error": error_details(&error) }),
        );
        return Err(error);
    }
    progress.write(
        "running",
        "within_doc_complete",
        json!({
            "constellations_processed": within.constellations_processed,
            "xterm_rows_persisted": within.xterm_rows_persisted,
            "knn_vectors": within.knn_vectors.len(),
        }),
    )?;

    let indexes = super::PersistedSearchIndexes::open(&resolved.path)?;
    let mut graph_progress = |update: passes::BetweenDocProgress| -> CliResult {
        if let Err(error) = deadline.check(
            "weave-loom",
            "between_doc_graph",
            update.nodes_processed as u64,
        ) {
            let _ = progress.write(
                "incomplete",
                "between_doc_timeout",
                json!({
                    "error": error_details(&error),
                    "graph_progress": update,
                }),
            );
            return Err(error);
        }
        progress.write(
            "running",
            "between_doc_graph",
            json!({ "graph_progress": update }),
        )
    };
    let graph_request = passes::BetweenDocGraphRequest {
        indexes: &indexes,
        knn_slot,
        knn: args.knn,
        edge_cos_threshold: args.edge_cos_threshold,
        knn_vectors: &within.knn_vectors,
    };
    let (edges_persisted, assoc_graph) = match passes::build_between_doc_graph(
        &vault,
        &graph,
        graph_request,
        Some(&mut graph_progress),
    ) {
        Ok(result) => result,
        Err(error) => {
            let _ = progress.write(
                "incomplete",
                "between_doc_error",
                json!({ "error": error_details(&error) }),
            );
            return Err(error);
        }
    };
    write_assoc_metadata(
        &vault,
        DEFAULT_ASTER_ASSOC_COLLECTION,
        &AsterAssocMetadata::default(),
    )?;

    let report_params = CorpusWeaveReportParams {
        max_groundedness_distance: args.max_groundedness_distance,
        ..CorpusWeaveReportParams::default()
    };
    let report = corpus_weave_report(&assoc_graph, &within.anchors, &report_params)?;

    let total_in_vault = within.constellations_in_vault;
    let output = json!({
        "status": "ok",
        "vault": resolved.name,
        "vault_dir": resolved.path.display().to_string(),
        "progress_artifact": progress.path().display().to_string(),
        "content_slots": content_slots.iter().map(|s| s.get()).collect::<Vec<_>>(),
        "skipped_incompatible_content_slots": incompatible_content_slots,
        "knn_slot": knn_slot.get(),
        "knn_slot_selection_reason": selection.reason,
        "dense_slot_coverage": &preflight.coverage,
        "candidate_order": "base_page_index_key_order",
        "knn": args.knn,
        "edge_cos_threshold": args.edge_cos_threshold,
        "constellations_in_vault": total_in_vault,
        "constellations_processed": within.constellations_processed,
        "limited": args.limit > 0 && args.limit < total_in_vault,
        "xterm": {
            "rows_persisted": within.xterm_rows_persisted,
            "slot_pair_count": within.agreement_pairs.len(),
            "slot_pairs": within.agreement_pairs,
        },
        "assoc_graph": {
            "edges_persisted": edges_persisted,
            "report": report,
        },
    });
    progress.write("ok", "complete", json!({ "output": &output }))?;
    write_fsv_readback(&output)?;
    print_json(&output)
}

fn error_details(error: &CliError) -> Value {
    json!({
        "code": error.code(),
        "message": error.message(),
    })
}

fn content_lens_slots(panel: &calyx_core::Panel) -> Vec<SlotId> {
    let mut slots: Vec<SlotId> = panel
        .slots
        .iter()
        .filter(|slot| {
            slot.state == SlotState::Active
                && !slot.retrieval_only
                && matches!(slot.shape, SlotShape::Dense(_))
        })
        .map(|slot| slot.slot_id)
        .collect();
    slots.sort();
    slots.dedup();
    slots
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
struct IncompatibleContentSlot {
    slot_id: u16,
    shape: String,
    reason: &'static str,
}

fn incompatible_content_lens_slots(panel: &calyx_core::Panel) -> Vec<IncompatibleContentSlot> {
    let mut slots: Vec<IncompatibleContentSlot> = panel
        .slots
        .iter()
        .filter(|slot| {
            slot.state == SlotState::Active
                && !slot.retrieval_only
                && !matches!(slot.shape, SlotShape::Dense(_))
        })
        .map(|slot| IncompatibleContentSlot {
            slot_id: slot.slot_id.get(),
            shape: slot_shape_label(slot.shape),
            reason: "active_content_slot_shape_is_not_dense",
        })
        .collect();
    slots.sort_by_key(|slot| slot.slot_id);
    slots.dedup();
    slots
}

fn slot_shape_label(shape: SlotShape) -> String {
    match shape {
        SlotShape::Dense(dim) => format!("dense:{dim}"),
        SlotShape::Sparse(dim) => format!("sparse:{dim}"),
        SlotShape::Multi { token_dim } => format!("multi:{token_dim}"),
    }
}

fn resolve_requested_slot(
    requested: Option<u16>,
    content_slots: &[SlotId],
    incompatible_content_slots: &[IncompatibleContentSlot],
) -> CliResult<Option<SlotId>> {
    match requested {
        None => Ok(None),
        Some(raw) => {
            let slot = SlotId::new(raw);
            if content_slots.contains(&slot) {
                Ok(Some(slot))
            } else {
                Err(CliError::usage(format!(
                    "--content-slot {raw} is not an active dense content lens; choose one of {:?}; incompatible active content slots={:?}",
                    content_slots.iter().map(|s| s.get()).collect::<Vec<_>>(),
                    incompatible_content_slots
                )))
            }
        }
    }
}

fn write_fsv_readback(output: &serde_json::Value) -> CliResult {
    let Some(root) = std::env::var_os("CALYX_FSV_ROOT") else {
        return Ok(());
    };
    let dir = PathBuf::from(root).join("weave-loom");
    fs::create_dir_all(&dir)?;
    let path = dir.join("weave_loom_report.json");
    fs::write(&path, serde_json::to_vec_pretty(output)?)?;
    eprintln!("WEAVE_LOOM_READBACK={}", path.display());
    Ok(())
}

#[cfg(test)]
mod tests;
