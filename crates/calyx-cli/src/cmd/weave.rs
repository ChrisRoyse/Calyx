//! `calyx weave-loom <vault>` — corpus-scale Loom weave (#870).
//!
//! Populates the **XTerm CF** with within-doc cross-lens agreement cross-terms,
//! and the **graph CF** with the between-doc directed k-NN association graph
//! (nodes = constellations, edges = panel-measured nearest neighbours via the
//! persisted DiskANN index). Emits the acceptance report: XTerm rows persisted,
//! the corpus slot-pair agreement graph, and the association graph's
//! node/edge/groundedness counts. Fail-closed throughout — no fallbacks.

mod passes;

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
use serde_json::json;

use super::vault::{home_dir, resolve_vault_info, vault_salt};
use super::{Subcommand, value};
use crate::error::{CliError, CliResult};
use crate::output::print_json;

const DEFAULT_KNN: usize = 16;
const DEFAULT_EDGE_COS_THRESHOLD: f32 = 0.5;
const DEFAULT_MAX_GROUNDEDNESS_DISTANCE: usize = 3;
const DEFAULT_BATCH: usize = 512;

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
    let state = load_vault_panel_state(&resolved.path)?;
    let content_slots = content_lens_slots(&state.panel);
    let incompatible_content_slots = incompatible_content_lens_slots(&state.panel);
    if content_slots.len() < 2 {
        return Err(CliError::usage(format!(
            "weave-loom needs >=2 active dense content lenses (state=Active, not retrieval_only, shape=Dense); panel has {}; incompatible active content slots={:?}",
            content_slots.len(),
            incompatible_content_slots
        )));
    }
    let knn_slot = resolve_knn_slot(
        args.content_slot,
        &content_slots,
        &incompatible_content_slots,
    )?;
    let vault = AsterVault::open(
        &resolved.path,
        resolved.vault_id,
        vault_salt(resolved.vault_id, &resolved.name),
        VaultOptions::default(),
    )?;
    let indexes = super::PersistedSearchIndexes::open(&resolved.path)?;

    let snapshot = vault.latest_seq();
    let graph = PlainGraph::new(&vault, DEFAULT_ASTER_ASSOC_COLLECTION)?;
    let within = passes::weave_within_doc(
        &vault,
        &graph,
        snapshot,
        &content_slots,
        knn_slot,
        args.batch,
        args.limit,
    )?;
    let total_in_vault = within.constellations_in_vault;
    let (edges_persisted, assoc_graph) = passes::build_between_doc_graph(
        &vault,
        &graph,
        &indexes,
        knn_slot,
        args.knn,
        args.edge_cos_threshold,
        &within.knn_vectors,
    )?;
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

    let output = json!({
        "status": "ok",
        "vault": resolved.name,
        "vault_dir": resolved.path.display().to_string(),
        "content_slots": content_slots.iter().map(|s| s.get()).collect::<Vec<_>>(),
        "skipped_incompatible_content_slots": incompatible_content_slots,
        "knn_slot": knn_slot.get(),
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
    write_fsv_readback(&output)?;
    print_json(&output)
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

fn resolve_knn_slot(
    requested: Option<u16>,
    content_slots: &[SlotId],
    incompatible_content_slots: &[IncompatibleContentSlot],
) -> CliResult<SlotId> {
    match requested {
        None => Ok(content_slots[0]),
        Some(raw) => {
            let slot = SlotId::new(raw);
            if content_slots.contains(&slot) {
                Ok(slot)
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

pub(crate) fn parse_weave_loom(rest: &[String]) -> CliResult<Subcommand> {
    let vault = rest
        .first()
        .ok_or_else(|| CliError::usage("weave-loom requires <vault>"))?
        .clone();
    let mut args = WeaveLoomArgs {
        vault,
        ..WeaveLoomArgs::default()
    };
    let mut idx = 1;
    while idx < rest.len() {
        match rest[idx].as_str() {
            "--content-slot" => {
                idx += 1;
                args.content_slot = Some(parse_u16(value(rest, idx, "--content-slot")?)?);
            }
            "--knn" => {
                idx += 1;
                args.knn = parse_usize(value(rest, idx, "--knn")?, "--knn", 1)?;
            }
            "--edge-cos-threshold" => {
                idx += 1;
                args.edge_cos_threshold =
                    parse_threshold(value(rest, idx, "--edge-cos-threshold")?)?;
            }
            "--max-groundedness-distance" => {
                idx += 1;
                args.max_groundedness_distance = parse_usize(
                    value(rest, idx, "--max-groundedness-distance")?,
                    "--max-groundedness-distance",
                    1,
                )?;
            }
            "--batch" => {
                idx += 1;
                args.batch = parse_usize(value(rest, idx, "--batch")?, "--batch", 1)?;
            }
            "--limit" => {
                idx += 1;
                args.limit = parse_usize(value(rest, idx, "--limit")?, "--limit", 0)?;
            }
            other => {
                return Err(CliError::usage(format!(
                    "unexpected weave-loom flag {other}"
                )));
            }
        }
        idx += 1;
    }
    Ok(Subcommand::WeaveLoom(args))
}

fn parse_u16(raw: &str) -> CliResult<u16> {
    raw.parse::<u16>()
        .map_err(|err| CliError::usage(format!("parse u16 {raw}: {err}")))
}

fn parse_usize(raw: &str, flag: &str, min: usize) -> CliResult<usize> {
    let value = raw
        .parse::<usize>()
        .map_err(|err| CliError::usage(format!("parse {flag} {raw}: {err}")))?;
    if value < min {
        return Err(CliError::usage(format!("{flag} must be >= {min}")));
    }
    Ok(value)
}

fn parse_threshold(raw: &str) -> CliResult<f32> {
    let value = raw
        .parse::<f32>()
        .map_err(|err| CliError::usage(format!("parse --edge-cos-threshold {raw}: {err}")))?;
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        return Err(CliError::usage(
            "--edge-cos-threshold must be finite and in [0,1]",
        ));
    }
    Ok(value)
}

#[cfg(test)]
mod tests;
