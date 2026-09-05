//! Ground the MFVS kernel on the persisted association graph (#871).

mod admission;
mod admission_select;
mod model;
mod parse;

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::Path;
use std::path::PathBuf;
use std::time::Instant;

use calyx_aster::ledger_view::read_ledger_seq;
use calyx_aster::plain_graph::PhysicalPlainGraph;
use calyx_aster::resource::heap_rss_bytes;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{CalyxError, CxId};
use calyx_ledger::{ActorId, EntryKind, SubjectId, decode};
use calyx_lodestar::{
    ASTER_ASSOC_METADATA_KEY, AsterAssocMetadata, AsterAssocNodeProps, FsKernelStore, KernelParams,
    LodestarError, PANEL_ASTER_ASSOC_COLLECTION, PANEL_RRF_K, RecallTestParams, TypedPanelVectors,
    build_kernel_pipeline, build_typed_panel_kernel_index_owned, kernel_health,
    kernel_members_hash, prepare_typed_panel_corpus, prepare_typed_panel_recall_reference,
    read_kernel_artifact, refine_kernel_with_recall_support, seal_completed_kernel_identity,
    typed_panel_kernel_recall_test_members_against_reference,
    typed_panel_rank_stabilization_support_from_reference,
    verify_typed_panel_kernel_index_physical, write_kernel_artifact,
    write_typed_panel_kernel_index,
};
use calyx_registry::load_vault_panel_state;
use rayon::prelude::*;
use serde_json::json;

use super::Subcommand;
use super::kernel_generation::{
    KernelAdmissionContract, KernelGenerationManifest, KernelGraphContract,
    KernelJurisdictionContract, acquire_build_lock, artifact_ref,
    artifact_ref_from_physical_readback, hex32, ledger_hash, load_current_generation_reference,
    physical_graph_contract, publish_current_generation, sha256_bytes,
};
use super::vault::{home_dir, now_ms, resolve_vault_info, vault_salt};
use crate::error::{CliError, CliResult};
use crate::output::print_json;

use self::model::{KernelBuildNodeProps, KernelBuildRefinement};

const DEFAULT_HELD_OUT_FRACTION: f32 = 0.005;
const DEFAULT_TOP_K: usize = 10;
const DEFAULT_MIN_RECALL: f32 = 0.95;
const RNG_SEED: u64 = 871;

/// Optional fail-closed peak-RSS budget for the whole kernel-build run, in
/// bytes. When set, kernel-build refuses (fail closed, `CALYX_BACKPRESSURE`)
/// before the corpus-resident index build and again before publication if the
/// projected peak resident set would exceed it — instead of being OOM-killed
/// mid-run at ~30 GB (issue #1863).
const KERNEL_BUILD_RSS_BUDGET_ENV: &str = "CALYX_KERNEL_BUILD_RSS_BUDGET_BYTES";

/// Phase-tagged peak-RSS + timing profiler for the kernel-build run. Samples the
/// kernel's `VmRSS` (via [`heap_rss_bytes`], the in-repo `/proc` probe) at each
/// milestone, tracks the running high-water mark, and emits the trail on stderr
/// and into the build JSON so a killed run still leaves a memory/timing record.
/// A probe failure is recorded verbatim (never silently dropped) and does not
/// fail the build; the memory *contract* is enforced separately by the
/// fail-closed budget guard ([`enforce_rss_budget`]).
struct RssTracker {
    start: Instant,
    peak_bytes: u64,
    samples: Vec<serde_json::Value>,
}

impl RssTracker {
    fn new() -> Self {
        Self {
            start: Instant::now(),
            peak_bytes: 0,
            samples: Vec::new(),
        }
    }

    fn sample(&mut self, phase: &str) {
        let elapsed_ms = self.start.elapsed().as_millis();
        let detail = match heap_rss_bytes() {
            Ok(bytes) => {
                self.peak_bytes = self.peak_bytes.max(bytes);
                eprintln!(
                    "kernel-build: rss phase={phase} rss_bytes={bytes} peak_rss_bytes={} elapsed_ms={elapsed_ms}",
                    self.peak_bytes
                );
                json!({
                    "phase": phase,
                    "rss_bytes": bytes,
                    "peak_rss_bytes": self.peak_bytes,
                    "elapsed_ms": elapsed_ms,
                })
            }
            Err(error) => {
                eprintln!(
                    "kernel-build: rss phase={phase} probe_error={} ({}) elapsed_ms={elapsed_ms}",
                    error.code, error.message
                );
                json!({
                    "phase": phase,
                    "rss_bytes": serde_json::Value::Null,
                    "peak_rss_bytes": self.peak_bytes,
                    "elapsed_ms": elapsed_ms,
                    "probe_error": error.code,
                    "probe_error_message": error.message,
                })
            }
        };
        self.samples.push(detail);
    }

    fn report(&self) -> serde_json::Value {
        json!({
            "peak_rss_bytes": self.peak_bytes,
            "elapsed_ms": self.start.elapsed().as_millis(),
            "budget_env": KERNEL_BUILD_RSS_BUDGET_ENV,
            "samples": &self.samples,
        })
    }
}

/// Reads the optional per-run peak-RSS budget from the environment. A malformed
/// or zero value fails closed as a usage error rather than being ignored.
fn configured_rss_budget() -> CliResult<Option<u64>> {
    match std::env::var(KERNEL_BUILD_RSS_BUDGET_ENV) {
        Ok(raw) => Ok(Some(parse_rss_budget(&raw)?)),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => Err(CliError::usage(format!(
            "{KERNEL_BUILD_RSS_BUDGET_ENV} value is not valid UTF-8"
        ))),
    }
}

/// Parses a raw budget string into a positive byte count, failing closed on a
/// non-numeric or zero value (pure, env-free, so the contract is unit-testable).
fn parse_rss_budget(raw: &str) -> CliResult<u64> {
    let bytes = raw.trim().parse::<u64>().map_err(|error| {
        CliError::usage(format!(
            "{KERNEL_BUILD_RSS_BUDGET_ENV}={raw:?} must be a positive byte count: {error}"
        ))
    })?;
    if bytes == 0 {
        return Err(CliError::usage(format!(
            "{KERNEL_BUILD_RSS_BUDGET_ENV} must be a positive byte budget; got 0"
        )));
    }
    Ok(bytes)
}

/// Pure budget decision: returns the projected peak RSS iff it exceeds `budget`.
/// `None` means "within budget" (or no budget). Uses saturating arithmetic so a
/// pathological additive estimate can never wrap the comparison.
fn projected_peak_over_budget(
    current_bytes: u64,
    additional_bytes: u64,
    budget: Option<u64>,
) -> Option<u64> {
    let budget = budget?;
    let projected = current_bytes.saturating_add(additional_bytes);
    (projected > budget).then_some(projected)
}

/// Fail-closed peak-RSS pre-flight. When a budget is configured, refuses (with
/// `CALYX_BACKPRESSURE`) before `phase` if the current resident set plus the
/// estimated additional working set would exceed the budget — instead of
/// proceeding toward an OOM kill. With no budget configured it is a no-op. A
/// probe failure fails closed (the budget cannot be proven honoured) rather than
/// silently skipping the guard.
fn enforce_rss_budget(
    phase: &str,
    budget: Option<u64>,
    projected_additional_bytes: u64,
) -> CliResult {
    let Some(budget) = budget else {
        return Ok(());
    };
    let current = heap_rss_bytes()?;
    if let Some(projected_peak) =
        projected_peak_over_budget(current, projected_additional_bytes, Some(budget))
    {
        return Err(CalyxError::backpressure(format!(
            "kernel-build refused before {phase}: projected peak RSS {projected_peak} bytes \
             (current {current} + estimated additional working set {projected_additional_bytes}) \
             exceeds {KERNEL_BUILD_RSS_BUDGET_ENV}={budget} bytes; raise the budget or shard the corpus"
        ))
        .into());
    }
    Ok(())
}

/// Physical dense payload the given constellations hold resident: the summed
/// byte size of their per-slot dense vectors, read straight from the
/// materialized panel map (the source of truth), not an a-priori guess.
fn panel_payload_bytes<'a>(rows: impl IntoIterator<Item = &'a TypedPanelVectors>) -> u64 {
    rows.into_iter()
        .flat_map(|vectors| vectors.values())
        .map(|vector| {
            let values = match vector {
                calyx_core::SlotVector::Dense { data, .. } => data.len(),
                calyx_core::SlotVector::Sparse { entries, .. } => entries.len() * 2,
                calyx_core::SlotVector::Multi { tokens, .. } => tokens.iter().map(Vec::len).sum(),
                calyx_core::SlotVector::SegmentedMulti { segments, .. } => {
                    segments.iter().map(|segment| segment.data.len()).sum()
                }
                calyx_core::SlotVector::Absent { .. } => 0,
            };
            (values * std::mem::size_of::<f32>()) as u64
        })
        .sum()
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct KernelBuildArgs {
    pub vault: String,
    pub held_out_fraction: f32,
    pub top_k: usize,
    pub min_recall: f32,
    pub admission_queries: Option<PathBuf>,
    pub resident_addr: Option<SocketAddr>,
}

pub(crate) use parse::parse_kernel_build;

impl Default for KernelBuildArgs {
    fn default() -> Self {
        Self {
            vault: String::new(),
            held_out_fraction: DEFAULT_HELD_OUT_FRACTION,
            top_k: DEFAULT_TOP_K,
            min_recall: DEFAULT_MIN_RECALL,
            admission_queries: None,
            resident_addr: None,
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
    // Fail fast on a malformed budget before any work is done.
    let rss_budget = configured_rss_budget()?;
    let mut rss = RssTracker::new();
    rss.sample("run_start");
    let resolved = resolve_vault_info(home, &args.vault)?;
    let _generation_lock = acquire_build_lock(&resolved.path)?;
    eprintln!(
        "kernel-build: opening physical graph name={} id={} path={}",
        resolved.name,
        resolved.vault_id,
        resolved.path.display()
    );
    let plain =
        PhysicalPlainGraph::open_latest_isolated(&resolved.path, PANEL_ASTER_ASSOC_COLLECTION)?;
    let metadata_bytes = plain
        .get_metadata(ASTER_ASSOC_METADATA_KEY)?
        .ok_or_else(|| {
            CliError::usage(
                "kernel-build requires a sealed weave graph contract; re-run `calyx weave-loom`",
            )
        })?;
    let graph_metadata: AsterAssocMetadata = serde_json::from_slice(&metadata_bytes)
        .map_err(|error| CliError::runtime(format!("decode weave graph metadata: {error}")))?;
    if graph_metadata.embedding_slot.is_some() {
        return Err(CliError::usage(
            "woven graph uses the forbidden legacy single-vector contract; re-run `calyx weave-loom`",
        ));
    }
    let embedding_slots = graph_metadata.embedding_slots.clone();
    if embedding_slots.len() < 2
        || !embedding_slots.windows(2).all(|pair| pair[0] < pair[1])
        || graph_metadata.fusion.as_deref() != Some("typed_block_rrf")
        || graph_metadata.rrf_k != Some(PANEL_RRF_K)
    {
        return Err(CliError::usage(
            "woven graph has no valid ordered no-flatten typed-block/RRF contract; re-run `calyx weave-loom`",
        ));
    }
    let panel_version = graph_metadata.panel_version.ok_or_else(|| {
        CliError::usage("woven graph metadata has no panel_version; re-run `calyx weave-loom`")
    })?;
    let graph_source_seq = graph_metadata.graph_source_seq.ok_or_else(|| {
        CliError::usage("woven graph metadata has no graph_source_seq; re-run `calyx weave-loom`")
    })?;
    let knn = graph_metadata
        .knn
        .filter(|value| *value > 0)
        .ok_or_else(|| CliError::usage("woven graph metadata has no valid knn"))?;
    let edge_score_threshold = graph_metadata
        .edge_score_threshold
        .filter(|value| value.is_finite() && (0.0..=1.0).contains(value))
        .ok_or_else(|| CliError::usage("woven graph metadata has no valid edge_score_threshold"))?;
    let panel_state = load_vault_panel_state(&resolved.path)?;
    if u64::from(panel_state.panel.version) != panel_version {
        return Err(CliError::usage(format!(
            "woven graph panel version {panel_version} differs from active vault panel {}; re-run `calyx weave-loom`",
            panel_state.panel.version
        )));
    }
    let mut expected_slots = panel_state
        .panel
        .slots
        .iter()
        .filter(|slot| slot.state == calyx_core::SlotState::Active && !slot.retrieval_only)
        .map(|slot| slot.slot_id)
        .collect::<Vec<_>>();
    expected_slots.sort();
    expected_slots.dedup();
    if embedding_slots != expected_slots {
        return Err(CliError::usage(format!(
            "woven graph slots {:?} differ from every active typed content slot {:?}; re-run `calyx weave-loom`",
            embedding_slots
                .iter()
                .map(|slot| slot.get())
                .collect::<Vec<_>>(),
            expected_slots
                .iter()
                .map(|slot| slot.get())
                .collect::<Vec<_>>()
        )));
    }
    if let Some(resident_addr) = args.resident_addr {
        preflight_kernel_resident(resident_addr, &resolved.path, &embedding_slots)?;
    }
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

    // Complete per-node constellation panel + anchors from physical graph props.
    let mut embeddings: BTreeMap<CxId, TypedPanelVectors> = BTreeMap::new();
    let mut anchors: Vec<CxId> = Vec::new();
    let stage = Instant::now();
    let node_props = plain.node_props()?;
    let csr_bytes = plain.read_csr_bytes()?.ok_or_else(|| {
        CliError::usage(
            "kernel-build requires a persisted association CSR; re-run `calyx weave-loom`",
        )
    })?;
    let physical_node_count = plain.node_key_count()?;
    let physical_edge_count = plain.edge_out_key_count()?;
    if physical_node_count != graph.node_count() || physical_edge_count != graph.edge_count() {
        return Err(CliError::runtime(format!(
            "physical graph rows differ from CSR: node_rows={physical_node_count} csr_nodes={} edge_rows={physical_edge_count} csr_edges={}",
            graph.node_count(),
            graph.edge_count()
        )));
    }
    let (physical_contract_hash, node_props_sha256) =
        physical_graph_contract(&metadata_bytes, &node_props, &csr_bytes);
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
            if props.embedding.is_some() || !props.embeddings.is_empty() {
                return Err(CliError::usage(format!(
                    "graph node {id} contains forbidden legacy dense embedding state; re-run weave-loom"
                )));
            }
            if props
                .typed_embeddings
                .keys()
                .copied()
                .ne(embedding_slots.iter().copied())
            {
                return Err(CliError::usage(format!(
                    "graph node {id} slots {:?} differ from graph contract {:?}; re-run weave-loom",
                    props.typed_embeddings.keys().map(|slot| slot.get()).collect::<Vec<_>>(),
                    embedding_slots.iter().map(|slot| slot.get()).collect::<Vec<_>>()
                )));
            }
            Ok(KernelBuildNodeProps {
                id,
                embeddings: props.typed_embeddings,
                anchored: !props.anchors.is_empty(),
                metadata: props.metadata,
            })
        })
        .collect::<CliResult<Vec<_>>>()?;
    let jurisdiction = admission::jurisdiction(&parsed_props)?;
    for parsed in parsed_props {
        if parsed.anchored {
            anchors.push(parsed.id);
        }
        embeddings.insert(parsed.id, parsed.embeddings);
    }
    if embeddings.len() != graph.node_count() {
        return Err(CliError::usage(format!(
            "graph topology has {} nodes but parsed {} complete panel constellations; rebuild the graph CF",
            graph.node_count(),
            embeddings.len()
        )));
    }
    eprintln!(
        "kernel-build: loaded node props rows={} anchors={} panel_constellations={} lanes={} elapsed_ms={}",
        embeddings.len(),
        anchors.len(),
        embeddings.len(),
        embedding_slots.len(),
        stage.elapsed().as_millis()
    );
    if anchors.is_empty() {
        return Err(CliError::usage(
            "kernel-build found no anchored nodes in the graph; anchor the corpus before grounding a kernel",
        ));
    }

    let kernel_params = KernelParams {
        panel_version,
        anchor_kind: Some("any".to_string()),
        corpus_shard_hash: physical_contract_hash,
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
    rss.sample("kernel_built");

    let recall_params = RecallTestParams {
        held_out_fraction: args.held_out_fraction,
        top_k: args.top_k,
        rng_seed: RNG_SEED,
        min_recall_ratio: args.min_recall,
    };
    let admission = admission_select::select(
        &resolved,
        &panel_state,
        &embedding_slots,
        &embeddings,
        jurisdiction.as_ref(),
        &args,
    )?;
    admission::emit(&admission);
    let corpus_name = format!("kernel-build:{}:typed-block-rrf", resolved.name);
    // Conservative fail-closed pre-flight for the prepared recall workspace.
    // Its borrowed views/norms are bounded well below one selected-member
    // payload copy, so that payload size is a safe upper bound.
    let member_payload_bytes = panel_payload_bytes(
        kernel
            .members
            .iter()
            .filter_map(|member| embeddings.get(member)),
    );
    enforce_rss_budget("panel_index_build", rss_budget, member_payload_bytes)?;
    let prepared_corpus = prepare_typed_panel_corpus(&embeddings, &embedding_slots)?;
    rss.sample("recall_corpus_prepared");
    let stage = Instant::now();
    let mut recall_reference = prepare_typed_panel_recall_reference(
        &prepared_corpus,
        &recall_params,
        recall_params.top_k,
    )?;
    let initial_recall = typed_panel_kernel_recall_test_members_against_reference(
        &kernel,
        &prepared_corpus,
        &recall_reference,
        &corpus_name,
    )?;
    eprintln!(
        "kernel-build: sealed exact full-corpus recall reference queries={} lane_depth={}",
        initial_recall.n_queries_tested, recall_params.top_k
    );
    let mut final_recall = initial_recall.clone();
    let mut refinement = None;
    if initial_recall.ratio < args.min_recall {
        eprintln!(
            "kernel-build: recall below gate before refinement ratio={:.6} min={:.6}; extracting exact full top-k support",
            initial_recall.ratio, args.min_recall
        );
        let initial_members = kernel.members.len();
        let initial_kernel_graph = kernel.kernel_graph.len();
        let mut lane_depth = recall_params.top_k.min(embeddings.len());
        let (support, refined_recall, final_lane_depth) = loop {
            if lane_depth != recall_params.top_k {
                recall_reference = prepare_typed_panel_recall_reference(
                    &prepared_corpus,
                    &recall_params,
                    lane_depth,
                )?;
            }
            let support = typed_panel_rank_stabilization_support_from_reference(&recall_reference)?;
            kernel = refine_kernel_with_recall_support(
                kernel,
                &support.members,
                &graph,
                &anchors,
                &kernel_params,
                &format!("exact_panel_rank_support_depth_{lane_depth}"),
            )?;
            let refined = typed_panel_kernel_recall_test_members_against_reference(
                &kernel,
                &prepared_corpus,
                &recall_reference,
                &corpus_name,
            )?;
            eprintln!(
                "kernel-build: panel recall refinement lane_depth={} support_members={} candidate_hits={} ratio={:.6}",
                lane_depth,
                support.members.len(),
                support.candidate_hits,
                refined.ratio
            );
            if refined.ratio >= args.min_recall || lane_depth == embeddings.len() {
                break (support, refined, lane_depth);
            }
            lane_depth = lane_depth.saturating_mul(2).min(embeddings.len());
        };
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
            "kernel-build: recall refinement support_members={} candidate_hits={} queries={} lane_depth={} ratio={:.6} members {}->{} kernel_graph {}->{}",
            support.members.len(),
            support.candidate_hits,
            support.n_queries_tested,
            final_lane_depth,
            refined_recall.ratio,
            initial_members,
            kernel.members.len(),
            initial_kernel_graph,
            kernel.kernel_graph.len()
        );
        final_recall = refined_recall;
    }
    if final_recall.ratio < args.min_recall {
        return Err(LodestarError::RecallBelowGate {
            ratio: final_recall.ratio,
            min: args.min_recall,
        }
        .into());
    }
    let mut recall = final_recall;
    recall.approx_factor = kernel.recall.approx_factor;
    recall.tau_star_estimate = kernel.recall.tau_star_estimate;
    recall.tau_star_exact = kernel.recall.tau_star_exact;
    kernel.recall = recall.clone();
    // Wall-clock execution time belongs to the ledger and FSV log, not the
    // immutable semantic artifact. Keeping it here would make an exact retry
    // target the same semantic generation with different kernel.json bytes.
    kernel.built_at_millis = 0;
    seal_completed_kernel_identity(&mut kernel, &physical_contract_hash)?;
    drop(prepared_corpus);
    let kernel_index = build_typed_panel_kernel_index_owned(&kernel, &embedding_slots, embeddings)?;
    eprintln!(
        "kernel-build: recall ratio={:.6} kernel_only={:.6} full={:.6} queries={} elapsed_ms={}",
        recall.ratio,
        recall.kernel_only,
        recall.full,
        recall.n_queries_tested,
        stage.elapsed().as_millis()
    );
    rss.sample("recall_gated");
    // The terminal index took ownership of the selected member vectors and
    // dropped all non-members, so the full corpus is not co-resident with the
    // write/readback path.
    rss.sample("corpus_freed");

    let store = FsKernelStore::new(&resolved.path);
    let stage = Instant::now();
    write_typed_panel_kernel_index(&kernel_index, &store)?;
    write_kernel_artifact(&kernel, &store)?;
    let expected_index_contracts = kernel_index.contracts().clone();

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
    // Streamed readback verification: parse and validate the on-disk index
    // row-by-row (contract header + per-row slot/dim/finite checks) and count
    // members, without materializing the whole multi-hundred-MB index a second
    // time during publication (issue #1863).
    let (persisted_index, persisted_index_bytes, persisted_index_sha256) =
        verify_typed_panel_kernel_index_physical(kernel.kernel_id, &store)?;
    if persisted_index.row_count != kernel.members.len()
        || persisted_index.slots != embedding_slots
        || persisted_index.contracts != expected_index_contracts
    {
        return Err(LodestarError::KernelIndexCodec {
            detail: format!(
                "readback mismatch for typed kernel {}; index rows {} did not match members {} or its sealed contracts",
                kernel.kernel_id,
                persisted_index.row_count,
                kernel.members.len()
            ),
        }
        .into());
    }
    let persisted_index_rows = persisted_index.row_count;
    drop(kernel_index);
    rss.sample("persisted_index_verified");
    let health = kernel_health(kernel.kernel_id, &store)?;
    let admission_ledger = kernel_admission_ledger_payload(&admission);
    let build_payload = serde_json::to_vec_pretty(&json!({
        "type": "kernel_build_v3",
        "kernel_id": kernel.kernel_id.to_string(),
        "members_hash": hex32(&kernel_members_hash(&kernel)),
        "graph_source_seq": graph_source_seq,
        "physical_graph_contract_sha256": hex32(&physical_contract_hash),
        "embedding_slots": embedding_slots.iter().map(|slot| slot.get()).collect::<Vec<_>>(),
        "fusion": "typed_block_rrf",
        "rrf_k": PANEL_RRF_K,
        "kernel_json_sha256": super::kernel_generation::sha256_file(&store.kernel_file_path(kernel.kernel_id))?,
        "index_json_sha256": persisted_index_sha256,
        "recall_ratio": recall.ratio,
        "groundedness_fraction": kernel.groundedness.reached_anchor,
        "admission": admission_ledger,
        "jurisdiction": jurisdiction
            .as_ref()
            .map(kernel_jurisdiction_ledger_payload),
    }))
    .map_err(|error| CliError::runtime(format!("encode kernel build ledger payload: {error}")))?;
    let graph_contract = KernelGraphContract {
        collection: PANEL_ASTER_ASSOC_COLLECTION.to_string(),
        nodes: graph.node_count(),
        edges: graph.edge_count(),
        source_seq: graph_source_seq,
        embedding_slots: embedding_slots.clone(),
        fusion: "typed_block_rrf".to_string(),
        rrf_k: PANEL_RRF_K,
        panel_version,
        knn,
        edge_score_threshold,
        metadata_sha256: sha256_bytes(&metadata_bytes),
        node_props_sha256,
        csr_sha256: sha256_bytes(&csr_bytes),
        physical_contract_sha256: hex32(&physical_contract_hash),
    };
    let kernel_json_ref = artifact_ref(&resolved.path, &store.kernel_file_path(kernel.kernel_id))?;
    let index_json_ref = artifact_ref_from_physical_readback(
        &resolved.path,
        &store.index_file_path(kernel.kernel_id),
        persisted_index_bytes,
        persisted_index_sha256,
    )?;
    let current_path = resolved.path.join("idx/kernel/CURRENT");
    let exact_current = if current_path.exists() {
        // The just-built bytes were compared against the immutable artifact,
        // SHA-256 hashed, and row-stream validated above. Read only the
        // content-addressed CURRENT contract here instead of cold-parsing the
        // same 8+ GB index a second time.
        let current = load_current_generation_reference(&resolved.path)?;
        let same_contract = current.manifest.kernel_id == kernel.kernel_id
            && current.manifest.kernel_json == kernel_json_ref
            && current.manifest.index_json == index_json_ref
            && current.manifest.graph == graph_contract
            && current.manifest.admission == admission
            && current.manifest.jurisdiction == jurisdiction;
        if same_contract {
            let ledger_row = read_ledger_seq(&resolved.path, current.manifest.build_ledger_seq)?
                .ok_or_else(|| CliError::runtime("current kernel build ledger row is missing"))?;
            let ledger_entry = decode(&ledger_row.bytes)?;
            if ledger_entry.seq != current.manifest.build_ledger_seq
                || ledger_entry.kind != EntryKind::Kernel
                || ledger_entry.subject != SubjectId::Kernel(kernel.kernel_id.as_bytes().to_vec())
                || ledger_entry.actor != ActorId::Service("calyx-kernel-build".to_string())
                || ledger_entry.payload != build_payload
                || hex32(&ledger_entry.entry_hash) != current.manifest.build_ledger_hash
            {
                return Err(CliError::runtime(
                    "current kernel generation ledger readback differs from the exact retry",
                ));
            }
            Some(current)
        } else {
            None
        }
    } else {
        None
    };
    let (published, build_ledger_seq, build_ledger_hash) = if let Some(current) = exact_current {
        eprintln!(
            "kernel-build: exact generation already current; verified artifacts and ledger, publication is an idempotent no-op"
        );
        let seq = current.manifest.build_ledger_seq;
        let hash = current.manifest.build_ledger_hash.clone();
        (current, seq, hash)
    } else {
        // Fail closed before opening write state and appending toward an OOM.
        enforce_rss_budget("publication", rss_budget, 0)?;
        let vault = AsterVault::open(
            &resolved.path,
            resolved.vault_id,
            vault_salt(resolved.vault_id, &resolved.name),
            VaultOptions::write_latest_readback(),
        )?;
        rss.sample("vault_opened");
        let build_ledger_ref = vault.append_ledger_entry(
            EntryKind::Kernel,
            SubjectId::Kernel(kernel.kernel_id.as_bytes().to_vec()),
            build_payload.clone(),
            ActorId::Service("calyx-kernel-build".to_string()),
        )?;
        let ledger_row = read_ledger_seq(&resolved.path, build_ledger_ref.seq)?
            .ok_or_else(|| CliError::runtime("kernel build ledger row missing after append"))?;
        let ledger_entry = decode(&ledger_row.bytes)?;
        if ledger_entry.entry_hash != build_ledger_ref.hash
            || ledger_entry.kind != EntryKind::Kernel
            || ledger_entry.payload != build_payload
        {
            return Err(CliError::runtime(
                "kernel build ledger physical readback differs from the appended row",
            ));
        }
        let hash = ledger_hash(&build_ledger_ref);
        let seq = build_ledger_ref.seq;
        let published = publish_current_generation(
            &resolved.path,
            KernelGenerationManifest {
                schema_version: 3,
                kernel_id: kernel.kernel_id,
                kernel_json: kernel_json_ref,
                index_json: index_json_ref,
                graph: graph_contract,
                admission: admission.clone(),
                jurisdiction: jurisdiction.clone(),
                build_ledger_seq: seq,
                build_ledger_hash: hash.clone(),
            },
        )?;
        (published, seq, hash)
    };
    rss.sample("published");
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
        "admission": admission,
        "jurisdiction": jurisdiction,
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
                "index_rows": persisted_index_rows,
                "health_recall_pass_mode": format!("{:?}", health.recall.pass_mode).to_ascii_lowercase(),
                "health_grounded_fraction": health.grounded_fraction,
            },
            "current_manifest": published.manifest_path,
            "current_manifest_sha256": published.manifest_sha256,
            "build_ledger_seq": build_ledger_seq,
            "build_ledger_hash": build_ledger_hash,
        },
        "rss": rss.report(),
    });
    print_json(&output)
}

fn preflight_kernel_resident(
    addr: SocketAddr,
    vault: &Path,
    embedding_slots: &[calyx_core::SlotId],
) -> CliResult {
    let ready = crate::panel_commands::resident_ready_value_at(addr)?;
    let expected_source = format!("vault:{}", vault.display());
    let observed_source = ready
        .get("template_source")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    let count = |field| {
        ready
            .get(field)
            .and_then(serde_json::Value::as_u64)
            .and_then(|value| usize::try_from(value).ok())
            .unwrap_or(0)
    };
    let observed_content = count("content_lens_count");
    let observed_registry = count("registry_lens_count");
    let observed_warmed = count("warmed_lens_count");
    let observed_gpu = count("gpu_content_lens_count");
    let observed_cpu = count("cpu_content_lens_count");
    let expected_content = embedding_slots.len();
    let is_ready = ready
        .get("ready")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    if !is_ready
        || observed_source != expected_source
        || observed_content != expected_content
        || observed_registry != expected_content
        || observed_warmed != expected_content
        || observed_gpu != expected_content
        || observed_cpu != 0
    {
        return Err(CliError::from(CalyxError {
            code: "CALYX_PANEL_RESIDENT_IDENTITY_MISMATCH",
            message: format!(
                "kernel-build resident {addr} differs from the sealed graph contract: ready={is_ready} source={observed_source:?} expected_source={expected_source:?} content={observed_content} registry={observed_registry} warmed={observed_warmed} gpu={observed_gpu} cpu={observed_cpu} expected_content={expected_content}"
            ),
            remediation: "restart `calyx panel resident serve --vault <vault>` on the exact target vault and verify its full slot readiness",
        }));
    }
    eprintln!(
        "kernel-build: resident preflight ok addr={} process_id={} source={} slots={}",
        addr,
        ready
            .get("process_id")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0),
        observed_source,
        observed_content
    );
    Ok(())
}

fn kernel_admission_ledger_payload(admission: &KernelAdmissionContract) -> serde_json::Value {
    json!({
        "schema_version": admission.schema_version,
        "method": admission.method,
        "corpus_count": admission.corpus_count,
        "sample_count": admission.sample_count,
        "sample_limit": admission.sample_limit,
        "sample_seed": admission.sample_seed,
        "lower_tail_quantile": admission.lower_tail_quantile,
        "threshold": admission.threshold,
        "min_score": admission.min_score,
        "median_score": admission.median_score,
        "max_score": admission.max_score,
        "sample_ids_sha256": admission.sample_ids_sha256,
        "observations_sha256": admission.observations_sha256,
        "calibration_queries_bytes": admission.calibration_queries.as_ref().map(|artifact| artifact.bytes),
        "calibration_queries_sha256": admission.calibration_queries.as_ref().map(|artifact| &artifact.sha256),
    })
}

/// Keep the append-only Kernel row hash/id-only. The hash-addressed generation
/// manifest retains the complete jurisdiction contract (including canonical
/// dataset names and court IDs); repeating those operator-controlled strings in
/// the Ledger both adds no provenance and crosses the Ledger's secret-token
/// boundary. The sealed metadata digest commits those exact bytes.
fn kernel_jurisdiction_ledger_payload(
    jurisdiction: &KernelJurisdictionContract,
) -> serde_json::Value {
    json!({
        "schema_version": jurisdiction.schema_version,
        "country": jurisdiction.country,
        "court_system": jurisdiction.court_system,
        "state": jurisdiction.state,
        "source_rows": jurisdiction.source_rows,
        "scope_count": jurisdiction.scopes.len(),
        "metadata_contract_sha256": jurisdiction.metadata_contract_sha256,
    })
}

#[cfg(test)]
mod tests;
