//! `calyx kernel-build <vault>` — ground the MFVS kernel on the persisted
//! between-doc association graph (#871).
//!
//! Reads the `graph` CF that `weave-loom` (#870) wrote — topology and node
//! props through the physical SST graph reader — then runs the (now scalable,
//! #948) `build_kernel_pipeline` and a bounded `kernel_recall_test`. Emits
//! kernel size, groundedness, recall ratio, tau*, and the A10 recall-gate
//! verdict. Fail-closed: a vault with no woven graph, no embeddings, or no
//! anchored nodes errors loudly.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Instant;

use calyx_aster::plain_graph::PhysicalPlainGraph;
use calyx_core::CxId;
use calyx_lodestar::{
    AsterAssocNodeProps, DEFAULT_ASTER_ASSOC_COLLECTION, FsKernelStore, InMemoryAnnIndex,
    InMemoryCorpus, KernelParams, LodestarError, RecallQuery, RecallTestParams, build_kernel_index,
    build_kernel_pipeline, full_topk_support_set, kernel_health, kernel_recall_gate,
    kernel_recall_test, load_kernel_index, read_kernel_artifact, refine_kernel_with_recall_support,
    write_kernel_artifact, write_kernel_index,
};
use rayon::prelude::*;
use serde_json::json;

#[cfg(test)]
use super::vault::vault_salt;
use super::vault::{home_dir, now_ms, resolve_vault_info};
use super::{Subcommand, value};
use crate::error::{CliError, CliResult};
use crate::output::print_json;

const DEFAULT_HELD_OUT_FRACTION: f32 = 0.005;
const DEFAULT_TOP_K: usize = 10;
const DEFAULT_MIN_RECALL: f32 = 0.95;
const RNG_SEED: u64 = 871;

struct KernelBuildNodeProps {
    id: CxId,
    embedding: Vec<f32>,
    anchored: bool,
}

#[derive(Clone, Debug)]
struct KernelBuildRefinement {
    initial_ratio: f32,
    initial_kernel_only: f32,
    initial_members: usize,
    initial_kernel_graph: usize,
    support_members: usize,
    support_candidate_hits: usize,
    support_queries: usize,
    final_members: usize,
    final_kernel_graph: usize,
}

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
    run_kernel_build_with_home(&home_dir()?, args)
}

fn run_kernel_build_with_home(home: &Path, args: KernelBuildArgs) -> CliResult {
    let started = Instant::now();
    let resolved = resolve_vault_info(home, &args.vault)?;
    eprintln!(
        "kernel-build: opening physical graph name={} id={} path={}",
        resolved.name,
        resolved.vault_id,
        resolved.path.display()
    );
    let plain = PhysicalPlainGraph::open_latest(&resolved.path, DEFAULT_ASTER_ASSOC_COLLECTION)?;
    let stage = Instant::now();
    let graph = plain.assoc_graph()?;
    eprintln!(
        "kernel-build: loaded graph nodes={} edges={} elapsed_ms={}",
        graph.node_count(),
        graph.edge_count(),
        stage.elapsed().as_millis()
    );
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
    let stage = Instant::now();
    let node_props = plain.node_props()?;
    let graph_node_ids = graph.node_ids().collect::<Vec<_>>();
    let prop_node_ids = node_props.iter().map(|(id, _)| *id).collect::<Vec<_>>();
    if graph_node_ids != prop_node_ids {
        return Err(CliError::usage(format!(
            "graph topology has {} node keys but node props scan returned {}; rebuild the graph CF",
            graph_node_ids.len(),
            prop_node_ids.len()
        )));
    }
    let parsed_props = node_props
        .into_par_iter()
        .map(|(id, bytes)| -> CliResult<KernelBuildNodeProps> {
            let props: AsterAssocNodeProps = serde_json::from_slice(&bytes).map_err(|error| {
                CliError::runtime(format!("parse graph node {id} props: {error}"))
            })?;
            let embedding = props.embedding.ok_or_else(|| {
                CliError::usage(format!(
                    "graph node {id} has no embedding in its props; re-run weave-loom"
                ))
            })?;
            Ok(KernelBuildNodeProps {
                id,
                embedding,
                anchored: !props.anchors.is_empty(),
            })
        })
        .collect::<CliResult<Vec<_>>>()?;
    for parsed in parsed_props {
        if parsed.anchored {
            anchors.push(parsed.id);
        }
        embeddings.insert(parsed.id, parsed.embedding.clone());
        rows.push(RecallQuery {
            cx_id: parsed.id,
            vector: parsed.embedding,
        });
    }
    if rows.len() != graph.node_count() || embeddings.len() != graph.node_count() {
        return Err(CliError::usage(format!(
            "graph topology has {} nodes but parsed {} recall rows and {} embeddings; rebuild the graph CF",
            graph.node_count(),
            rows.len(),
            embeddings.len()
        )));
    }
    eprintln!(
        "kernel-build: loaded node props rows={} anchors={} embeddings={} elapsed_ms={}",
        rows.len(),
        anchors.len(),
        embeddings.len(),
        stage.elapsed().as_millis()
    );
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
    let stage = Instant::now();
    let mut kernel = build_kernel_pipeline(&graph, &anchors, &kernel_params)?;
    eprintln!(
        "kernel-build: built kernel id={} members={} kernel_graph={} groundedness={:.6} elapsed_ms={}",
        kernel.kernel_id,
        kernel.members.len(),
        kernel.kernel_graph.len(),
        kernel.groundedness.reached_anchor,
        stage.elapsed().as_millis()
    );

    let recall_params = RecallTestParams {
        held_out_fraction: args.held_out_fraction,
        top_k: args.top_k,
        rng_seed: RNG_SEED,
        min_recall_ratio: args.min_recall,
    };
    let full_index = InMemoryAnnIndex::new(rows.clone())?;
    let corpus = InMemoryCorpus::new(format!("kernel-build:{}", resolved.name), rows);
    let mut kernel_index = build_kernel_index(&kernel, &embeddings)?;
    let stage = Instant::now();
    let initial_recall = kernel_recall_test(&kernel_index, &full_index, &corpus, &recall_params)?;
    let mut refinement = None;
    if initial_recall.ratio < args.min_recall {
        eprintln!(
            "kernel-build: recall below gate before refinement ratio={:.6} min={:.6}; extracting exact full top-k support",
            initial_recall.ratio, args.min_recall
        );
        let support = full_topk_support_set(&full_index, &corpus, &recall_params)?;
        let initial_members = kernel.members.len();
        let initial_kernel_graph = kernel.kernel_graph.len();
        kernel = refine_kernel_with_recall_support(
            kernel,
            &support.members,
            &graph,
            &anchors,
            &kernel_params,
            "exact_full_topk_support",
        )?;
        kernel_index = build_kernel_index(&kernel, &embeddings)?;
        refinement = Some(KernelBuildRefinement {
            initial_ratio: initial_recall.ratio,
            initial_kernel_only: initial_recall.kernel_only,
            initial_members,
            initial_kernel_graph,
            support_members: support.members.len(),
            support_candidate_hits: support.candidate_hits,
            support_queries: support.n_queries_tested,
            final_members: kernel.members.len(),
            final_kernel_graph: kernel.kernel_graph.len(),
        });
        eprintln!(
            "kernel-build: recall refinement support_members={} candidate_hits={} queries={} members {}->{} kernel_graph {}->{}",
            support.members.len(),
            support.candidate_hits,
            support.n_queries_tested,
            initial_members,
            kernel.members.len(),
            initial_kernel_graph,
            kernel.kernel_graph.len()
        );
    }
    let mut recall = kernel_recall_gate(&kernel_index, &full_index, &corpus, &recall_params)?;
    recall.approx_factor = kernel.recall.approx_factor;
    recall.tau_star_estimate = kernel.recall.tau_star_estimate;
    recall.tau_star_exact = kernel.recall.tau_star_exact;
    kernel.recall = recall.clone();
    eprintln!(
        "kernel-build: recall ratio={:.6} kernel_only={:.6} full={:.6} queries={} elapsed_ms={}",
        recall.ratio,
        recall.kernel_only,
        recall.full,
        recall.n_queries_tested,
        stage.elapsed().as_millis()
    );

    let store = FsKernelStore::new(&resolved.path);
    let stage = Instant::now();
    write_kernel_index(&kernel_index, &store)?;
    write_kernel_artifact(&kernel, &store)?;

    let persisted_kernel = read_kernel_artifact(kernel.kernel_id, &store)?;
    if persisted_kernel != kernel {
        return Err(LodestarError::KernelArtifactCodec {
            detail: format!(
                "readback mismatch for kernel {}; persisted kernel.json did not match built kernel",
                kernel.kernel_id
            ),
        }
        .into());
    }
    let persisted_index = load_kernel_index(kernel.kernel_id, &store)?;
    if persisted_index.rows().len() != kernel.members.len() {
        return Err(LodestarError::KernelIndexCodec {
            detail: format!(
                "readback mismatch for kernel {}; index rows {} did not match members {}",
                kernel.kernel_id,
                persisted_index.rows().len(),
                kernel.members.len()
            ),
        }
        .into());
    }
    let health = kernel_health(kernel.kernel_id, &store)?;
    eprintln!(
        "kernel-build: persisted artifacts kernel_json={} index_json={} elapsed_ms={} total_elapsed_ms={}",
        store.kernel_file_path(kernel.kernel_id).display(),
        store.index_file_path(kernel.kernel_id).display(),
        stage.elapsed().as_millis(),
        started.elapsed().as_millis()
    );

    let groundedness_fraction = kernel.groundedness.reached_anchor;
    let kernel_file = store.kernel_file_path(kernel.kernel_id);
    let index_file = store.index_file_path(kernel.kernel_id);
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
            "gate_passed": true,
        },
        "refinement": refinement.as_ref().map(|refinement| json!({
            "initial_ratio": refinement.initial_ratio,
            "initial_kernel_only": refinement.initial_kernel_only,
            "initial_members": refinement.initial_members,
            "initial_kernel_graph": refinement.initial_kernel_graph,
            "support_members": refinement.support_members,
            "support_candidate_hits": refinement.support_candidate_hits,
            "support_queries": refinement.support_queries,
            "final_members": refinement.final_members,
            "final_kernel_graph": refinement.final_kernel_graph,
        })),
        "artifacts": {
            "store_root": resolved.path,
            "kernel_json": kernel_file,
            "kernel_json_bytes": std::fs::metadata(&kernel_file)?.len(),
            "index_json": index_file,
            "index_json_bytes": std::fs::metadata(&index_file)?.len(),
            "readback": {
                "kernel_id": persisted_kernel.kernel_id.to_string(),
                "kernel_members": persisted_kernel.members.len(),
                "index_rows": persisted_index.rows().len(),
                "health_recall_pass_mode": format!("{:?}", health.recall.pass_mode).to_ascii_lowercase(),
                "health_grounded_fraction": health.grounded_fraction,
            },
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
                args.held_out_fraction = parse_unit(
                    value(rest, idx, "--held-out-fraction")?,
                    "--held-out-fraction",
                )?;
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
                return Err(CliError::usage(format!(
                    "unexpected kernel-build flag {other}"
                )));
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
        return Err(CliError::usage(format!(
            "{flag} must be finite and in [0,1]"
        )));
    }
    Ok(value)
}

#[cfg(test)]
mod tests;
