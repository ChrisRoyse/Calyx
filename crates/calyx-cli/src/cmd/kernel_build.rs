//! `calyx kernel-build <vault>` — ground the MFVS kernel on the persisted
//! between-doc association graph (#871).
//!
//! Reads the `graph` CF that `weave-loom` (#870) wrote — topology via
//! `PlainGraph::assoc_graph`, per-node embedding + anchor kinds from the node
//! props — then runs the (now scalable, #948) `build_kernel_pipeline` and a
//! bounded `kernel_recall_test`. Emits kernel size, groundedness, recall ratio,
//! tau*, and the A10 recall-gate verdict. Fail-closed: a vault with no woven
//! graph, no embeddings, or no anchored nodes errors loudly.

use std::collections::BTreeMap;

use calyx_aster::plain_graph::PlainGraph;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::CxId;
use calyx_lodestar::{
    AsterAssocNodeProps, DEFAULT_ASTER_ASSOC_COLLECTION, InMemoryAnnIndex, InMemoryCorpus,
    KernelParams, RecallQuery, RecallTestParams, build_kernel_index, build_kernel_pipeline,
    kernel_recall_test,
};
use serde_json::json;

use super::vault::{home_dir, now_ms, resolve_vault_info, vault_salt};
use super::{Subcommand, value};
use crate::error::{CliError, CliResult};
use crate::output::print_json;

const DEFAULT_HELD_OUT_FRACTION: f32 = 0.005;
const DEFAULT_TOP_K: usize = 10;
const DEFAULT_MIN_RECALL: f32 = 0.95;
const RNG_SEED: u64 = 871;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct KernelBuildArgs {
    pub vault: String,
    pub held_out_fraction: f32,
    pub top_k: usize,
    pub min_recall: f32,
}

impl Default for KernelBuildArgs {
    fn default() -> Self {
        Self {
            vault: String::new(),
            held_out_fraction: DEFAULT_HELD_OUT_FRACTION,
            top_k: DEFAULT_TOP_K,
            min_recall: DEFAULT_MIN_RECALL,
        }
    }
}

pub(crate) fn run(command: Subcommand) -> CliResult {
    let Subcommand::KernelBuild(args) = command else {
        unreachable!("non-kernel-build command routed to kernel_build module");
    };
    run_kernel_build(args)
}

fn run_kernel_build(args: KernelBuildArgs) -> CliResult {
    let resolved = resolve_vault_info(&home_dir()?, &args.vault)?;
    let vault = AsterVault::open(
        &resolved.path,
        resolved.vault_id,
        vault_salt(resolved.vault_id, &resolved.name),
        VaultOptions::default(),
    )?;
    let snapshot = vault.latest_seq();
    let plain = PlainGraph::new(&vault, DEFAULT_ASTER_ASSOC_COLLECTION)?;
    let graph = plain.assoc_graph(snapshot)?;
    if graph.node_count() < 2 {
        return Err(CliError::usage(format!(
            "kernel-build needs a woven association graph (>=2 nodes); graph CF has {} — run `calyx weave-loom` first",
            graph.node_count()
        )));
    }

    // Per-node embedding + anchor from the props weave-loom stored.
    let mut embeddings: BTreeMap<CxId, Vec<f32>> = BTreeMap::new();
    let mut anchors: Vec<CxId> = Vec::new();
    let mut rows: Vec<RecallQuery> = Vec::with_capacity(graph.node_count());
    for id in graph.node_ids() {
        let bytes = plain.get_node(snapshot, id)?.ok_or_else(|| {
            CliError::usage(format!("graph node {id} has no props row"))
        })?;
        let props: AsterAssocNodeProps = serde_json::from_slice(&bytes)?;
        let embedding = props.embedding.ok_or_else(|| {
            CliError::usage(format!(
                "graph node {id} has no embedding in its props; re-run weave-loom"
            ))
        })?;
        if !props.anchors.is_empty() {
            anchors.push(id);
        }
        embeddings.insert(id, embedding.clone());
        rows.push(RecallQuery {
            cx_id: id,
            vector: embedding,
        });
    }
    if anchors.is_empty() {
        return Err(CliError::usage(
            "kernel-build found no anchored nodes in the graph; anchor the corpus before grounding a kernel",
        ));
    }

    let kernel_params = KernelParams {
        panel_version: 871,
        anchor_kind: Some("any".to_string()),
        built_at_millis: now_ms(),
        ..KernelParams::default()
    };
    let mut kernel = build_kernel_pipeline(&graph, &anchors, &kernel_params)?;
    // An acyclic selected subgraph yields no MFVS members; fall back to the
    // selected kernel graph (or the full corpus) so recall reflects real members,
    // never an empty index. Mirrors `vault_kernel::build_vault_kernel_inputs`.
    if kernel.members.is_empty() {
        kernel.members = if kernel.kernel_graph.is_empty() {
            graph.node_ids().collect()
        } else {
            kernel.kernel_graph.clone()
        };
    }

    let recall_params = RecallTestParams {
        held_out_fraction: args.held_out_fraction,
        top_k: args.top_k,
        rng_seed: RNG_SEED,
        min_recall_ratio: args.min_recall,
    };
    let kernel_index = build_kernel_index(&kernel, &embeddings)?;
    let full_index = InMemoryAnnIndex::new(rows.clone())?;
    let corpus = InMemoryCorpus::new(format!("kernel-build:{}", resolved.name), rows);
    let recall = kernel_recall_test(&kernel_index, &full_index, &corpus, &recall_params)?;

    let groundedness_fraction = kernel.groundedness.reached_anchor;
    let gate_passed = recall.ratio >= args.min_recall;
    let output = json!({
        "status": "ok",
        "vault": resolved.name,
        "graph": { "nodes": graph.node_count(), "edges": graph.edge_count() },
        "kernel": {
            "kernel_id": kernel.kernel_id.to_string(),
            "members": kernel.members.len(),
            "kernel_graph": kernel.kernel_graph.len(),
            "groundedness_fraction": groundedness_fraction,
        },
        "recall": {
            "kernel_only": recall.kernel_only,
            "full": recall.full,
            "ratio": recall.ratio,
            "tau_star_estimate": recall.tau_star_estimate,
            "tau_star_exact": recall.tau_star_exact,
            "n_queries_tested": recall.n_queries_tested,
            "held_out_fraction": args.held_out_fraction,
            "min_recall_ratio": args.min_recall,
            "gate_passed": gate_passed,
        },
    });
    print_json(&output)
}

pub(crate) fn parse_kernel_build(rest: &[String]) -> CliResult<Subcommand> {
    let vault = rest
        .first()
        .ok_or_else(|| CliError::usage("kernel-build requires <vault>"))?
        .clone();
    let mut args = KernelBuildArgs {
        vault,
        ..KernelBuildArgs::default()
    };
    let mut idx = 1;
    while idx < rest.len() {
        match rest[idx].as_str() {
            "--held-out-fraction" => {
                idx += 1;
                args.held_out_fraction = parse_unit(value(rest, idx, "--held-out-fraction")?, "--held-out-fraction")?;
            }
            "--top-k" => {
                idx += 1;
                let raw = value(rest, idx, "--top-k")?;
                args.top_k = raw
                    .parse::<usize>()
                    .map_err(|err| CliError::usage(format!("parse --top-k {raw}: {err}")))?;
                if args.top_k == 0 {
                    return Err(CliError::usage("--top-k must be >= 1"));
                }
            }
            "--min-recall" => {
                idx += 1;
                args.min_recall = parse_unit(value(rest, idx, "--min-recall")?, "--min-recall")?;
            }
            other => {
                return Err(CliError::usage(format!("unexpected kernel-build flag {other}")));
            }
        }
        idx += 1;
    }
    Ok(Subcommand::KernelBuild(args))
}

fn parse_unit(raw: &str, flag: &str) -> CliResult<f32> {
    let value = raw
        .parse::<f32>()
        .map_err(|err| CliError::usage(format!("parse {flag} {raw}: {err}")))?;
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        return Err(CliError::usage(format!("{flag} must be finite and in [0,1]")));
    }
    Ok(value)
}

#[cfg(test)]
mod tests;
