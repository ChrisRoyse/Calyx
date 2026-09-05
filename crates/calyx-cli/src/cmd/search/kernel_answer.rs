//! Issue #1480 grounded answer execution against a sealed kernel generation.

use std::collections::BTreeMap;
use std::fs;
use std::net::SocketAddr;
use std::path::Path;
use std::time::Instant;

use calyx_aster::ledger_view::AsterLedgerCfStore;
use calyx_aster::plain_graph::PhysicalPlainGraph;
use calyx_aster::resource::heap_rss_bytes;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{AnchorKind, CalyxError, CxId, SlotId, SlotVector};
use calyx_ledger::{EntryKind, LedgerCfStore, SubjectId, decode};
use calyx_lodestar::{
    ASTER_ASSOC_METADATA_KEY, AsterAssocMetadata, AsterAssocNodeProps,
    AsterPanelKernelAnswerRequest, FsKernelStore, KernelAnswerRecordContext,
    KernelQueryResolutionEvidence, PANEL_ASTER_ASSOC_COLLECTION, RecallPassMode,
    TypedPanelFusionHit, TypedPanelVectors, derive_panel_kernel_answer, kernel_health,
    panel_kernel_answer_with_aster_ledger, rank_typed_panel_candidate_refs, read_kernel_artifact,
    typed_panel_kernel_rank_with_nearest,
};
use calyx_registry::{VaultPanelState, load_vault_panel_state, require_vault_registry_contracts};

use super::super::ingest::parse_anchor_kind;
use super::super::kernel_generation::{
    decode_sha256, hex32, load_current_generation_reference, load_current_generation_with_index,
    sha256_bytes,
};
use super::super::vault::{ResolvedVault, vault_salt};
use super::engine::{
    measure_search_query_vectors_via_resident, require_resident_for_gpu_text_search,
    resolve_cli_vault,
};
use super::kernel_source_support::{CALYX_KERNEL_SOURCE_LOW_SUPPORT, evaluate_path_source_support};
use super::output;
use super::parse::KernelAnswerArgs;
use super::roster::{SearchTextRoster, measure_local_cpu_query_vectors};
use crate::durable_write::write_bytes_atomic;
use crate::error::{CliError, CliResult};
use crate::output::print_json;
use crate::panel_commands::rank_kernel_at_resident;

/// Always-on per-phase timing + peak-RSS profiler for the kernel-answer path
/// (issue #1977). Emits one structured `CALYX_KERNEL_ANSWER_RUNTIME` stderr line
/// per phase boundary — the same runtime-line convention `ingest`
/// (`CALYX_INGEST_RUNTIME`) and `kernel-build` use, including the resident rank
/// process id and its measured exact-ranking time.
///
/// `elapsed_ms` is the wall time spent *in that phase* (delta since the previous
/// boundary); `total_ms` is cumulative since the profiler was armed at the very
/// top of [`run`]. RSS is sampled from the operating system's physical source of
/// truth via [`heap_rss_bytes`] (Linux `/proc/self/status` `VmRSS`, Windows
/// `WorkingSetSize`); a probe failure is reported verbatim and never fails the
/// answer — this is observability, not a gate. The [`Drop`] guard emits the
/// `total` line on every exit path (success, fail-closed refusal, or `?` error
/// propagation) so the breakdown is never lost.
struct KernelAnswerRuntime {
    start: Instant,
    last: Instant,
    peak_bytes: u64,
}

impl KernelAnswerRuntime {
    fn arm() -> Self {
        let now = Instant::now();
        Self {
            start: now,
            last: now,
            peak_bytes: 0,
        }
    }

    /// Close out one phase: record its wall time (delta since the previous
    /// boundary), sample RSS, advance the boundary cursor, and emit the line.
    fn phase(&mut self, name: &str, detail: impl std::fmt::Display) {
        let now = Instant::now();
        let elapsed_ms = now.duration_since(self.last).as_millis();
        let total_ms = now.duration_since(self.start).as_millis();
        self.last = now;
        self.emit(name, elapsed_ms, total_ms, detail);
    }

    fn emit(
        &mut self,
        name: &str,
        elapsed_ms: u128,
        total_ms: u128,
        detail: impl std::fmt::Display,
    ) {
        match heap_rss_bytes() {
            Ok(bytes) => {
                self.peak_bytes = self.peak_bytes.max(bytes);
                eprintln!(
                    "CALYX_KERNEL_ANSWER_RUNTIME phase={name} elapsed_ms={elapsed_ms} total_ms={total_ms} rss_bytes={bytes} peak_rss_bytes={} detail={detail}",
                    self.peak_bytes
                );
            }
            Err(error) => {
                eprintln!(
                    "CALYX_KERNEL_ANSWER_RUNTIME phase={name} elapsed_ms={elapsed_ms} total_ms={total_ms} rss_bytes=probe_error peak_rss_bytes={} probe_error_code={} probe_error_message={:?} detail={detail}",
                    self.peak_bytes, error.code, error.message
                );
            }
        }
    }
}

impl Drop for KernelAnswerRuntime {
    fn drop(&mut self) {
        let total_ms = self.start.elapsed().as_millis();
        self.emit("total", total_ms, total_ms, "kernel_answer_path_complete");
    }
}

pub(super) fn run(args: KernelAnswerArgs) -> CliResult {
    let mut runtime = KernelAnswerRuntime::arm();
    let anchor = args.anchor.as_deref().map(parse_anchor_kind).transpose()?;
    let resolved = resolve_cli_vault(&args.vault)?;
    require_vault_registry_contracts(&resolved.path)?;
    let state = load_vault_panel_state(&resolved.path)?;
    let roster = SearchTextRoster::derive(&state);
    runtime.phase(
        "registry_resolve",
        format_args!(
            "vault={} panel_version={}",
            resolved.path.display(),
            state.panel.version
        ),
    );
    let (current, local_kernel_index) = if args.resident_addr.is_some() {
        (load_current_generation_reference(&resolved.path)?, None)
    } else {
        let (current, index) = load_current_generation_with_index(&resolved.path)?;
        (current, Some(index))
    };
    let graph_contract = &current.manifest.graph;
    let admission = &current.manifest.admission;
    if graph_contract.fusion != "typed_block_rrf" {
        return Err(CliError::runtime(format!(
            "CURRENT kernel fusion {} is not the required no-flatten typed_block_rrf contract; rebuild weave and kernel artifacts",
            graph_contract.fusion
        )));
    }
    if graph_contract.panel_version != u64::from(state.panel.version) {
        return Err(CliError::runtime(format!(
            "CURRENT kernel panel version {} differs from active panel {}",
            graph_contract.panel_version, state.panel.version
        )));
    }
    runtime.phase(
        "generation_contract",
        format_args!(
            "kernel_id={} fusion={} nodes={} edges={}",
            current.manifest.kernel_id,
            graph_contract.fusion,
            graph_contract.nodes,
            graph_contract.edges
        ),
    );
    let plain =
        PhysicalPlainGraph::open_latest_isolated(&resolved.path, PANEL_ASTER_ASSOC_COLLECTION)?;
    let metadata_bytes = plain
        .get_metadata(ASTER_ASSOC_METADATA_KEY)?
        .ok_or_else(|| CliError::runtime("physical graph metadata disappeared"))?;
    let metadata: AsterAssocMetadata = serde_json::from_slice(&metadata_bytes)
        .map_err(|error| CliError::runtime(format!("decode physical graph metadata: {error}")))?;
    let (graph, csr_bytes) = plain
        .assoc_graph_with_csr_bytes()?
        .ok_or_else(|| CliError::runtime("physical graph CSR disappeared"))?;
    if metadata.embedding_slot.is_some()
        || metadata.embedding_slots != graph_contract.embedding_slots
        || metadata.fusion.as_deref() != Some(graph_contract.fusion.as_str())
        || metadata.rrf_k != Some(graph_contract.rrf_k)
        || metadata.panel_version != Some(graph_contract.panel_version)
        || metadata.graph_source_seq != Some(graph_contract.source_seq)
        || metadata.knn != Some(graph_contract.knn)
        || metadata.edge_score_threshold != Some(graph_contract.edge_score_threshold)
        || graph.node_count() != graph_contract.nodes
        || graph.edge_count() != graph_contract.edges
        || sha256_bytes(&metadata_bytes) != graph_contract.metadata_sha256
        || sha256_bytes(&csr_bytes) != graph_contract.csr_sha256
    {
        return Err(CliError::runtime(
            "physical association graph differs from the CURRENT kernel generation contract",
        ));
    }
    runtime.phase(
        "physical_graph_load",
        format_args!(
            "nodes={} edges={} csr_bytes={} node_props_sha256={}",
            graph.node_count(),
            graph.edge_count(),
            csr_bytes.len(),
            graph_contract.node_props_sha256
        ),
    );
    let store = FsKernelStore::new(&resolved.path);
    let kernel = read_kernel_artifact(current.manifest.kernel_id, &store)?;
    let health = kernel_health(current.manifest.kernel_id, &store)?;
    if health.recall.pass_mode != RecallPassMode::Passed
        || health.recall.ratio < health.recall.min_recall_ratio
        || health.grounded_fraction != 1.0
    {
        return Err(CalyxError::kernel_ungrounded(format!(
            "CURRENT kernel failed answer gate: recall_mode={:?} ratio={} min={} grounded_fraction={}",
            health.recall.pass_mode,
            health.recall.ratio,
            health.recall.min_recall_ratio,
            health.grounded_fraction
        ))
        .into());
    }
    if hex32(&kernel.corpus_shard_hash) != graph_contract.physical_contract_sha256 {
        return Err(CliError::runtime(
            "sealed kernel corpus shard hash differs from the CURRENT physical graph contract",
        ));
    }
    if let Some(jurisdiction) = &current.manifest.jurisdiction
        && let Some(conflict) =
            super::super::kernel_scope::explicit_scope_conflict(&args.query, jurisdiction)
    {
        roster.emit_runtime_line();
        return print_json(&output::GroundedKernelScopeRefusalOut {
            status: "refused",
            code: "CALYX_KERNEL_QUERY_OUT_OF_SCOPE",
            reason: "explicit_query_jurisdiction_conflicts_with_kernel_scope",
            detected_kind: conflict.kind,
            detected_scope: conflict.detected,
            allowed_court_system: jurisdiction.court_system.clone(),
            allowed_state: jurisdiction.state.clone(),
            allowed_counties: super::super::kernel_scope::allowed_counties(jurisdiction),
            allowed_appellate_districts: super::super::kernel_scope::allowed_appellate_districts(
                jurisdiction,
            ),
            includes_statewide_scope: super::super::kernel_scope::includes_statewide_scope(
                jurisdiction,
            ),
            kernel_id: current.manifest.kernel_id.to_string(),
            kernel_manifest_sha256: current.manifest_sha256,
        });
    }
    let query_vectors =
        measure_kernel_query_vectors(&state, &roster, &resolved, &args.query, args.resident_addr)?;
    runtime.phase(
        "query_measure",
        format_args!(
            "resident_addr={} measured_slots={}",
            args.resident_addr
                .map(|addr| addr.to_string())
                .unwrap_or_else(|| "local-cpu".to_string()),
            query_vectors.len()
        ),
    );
    let query_panel = query_panel_vectors(&query_vectors, &graph_contract.embedding_slots)?;
    let ranked_kernel_nodes = if let Some(addr) = args.resident_addr {
        let ranked = rank_kernel_at_resident(
            addr,
            &current.manifest_sha256,
            current.manifest.kernel_id,
            &current.manifest.index_json.sha256,
            &query_panel,
        )?;
        if ranked.index_rows != kernel.members.len() {
            return Err(CliError::runtime(format!(
                "kernel rank resident returned {} rows for {} sealed kernel members",
                ranked.index_rows,
                kernel.members.len()
            )));
        }
        runtime.phase(
            "kernel_rank_resident",
            format_args!(
                "addr={} process_id={} kernel_id={} members={} index_rows={} resident_elapsed_ms={} recall_ratio={}",
                addr,
                ranked.process_id,
                kernel.kernel_id,
                kernel.members.len(),
                ranked.index_rows,
                ranked.elapsed_ms,
                health.recall.ratio
            ),
        );
        (ranked.ranked_ids, ranked.nearest)
    } else {
        let kernel_index = local_kernel_index
            .as_ref()
            .expect("local kernel index loaded without resident");
        runtime.phase(
            "kernel_index_pin",
            format_args!(
                "kernel_id={} members={} index_rows={} recall_ratio={}",
                kernel.kernel_id,
                kernel.members.len(),
                kernel_index.rows().len(),
                health.recall.ratio
            ),
        );
        typed_panel_kernel_rank_with_nearest(kernel_index, &query_panel)?
    };
    let (ranked_kernel_nodes, nearest) = ranked_kernel_nodes;
    if nearest.score < admission.threshold {
        roster.emit_runtime_line();
        return print_json(&output::GroundedKernelRefusalOut {
            status: "refused",
            code: "CALYX_KERNEL_QUERY_OUT_OF_SCOPE",
            reason: "nearest_constellation_panel_score_below_calibrated_in_domain_lower_tail",
            nearest_cx_id: nearest.cx_id.to_string(),
            nearest_score: nearest.score,
            nearest_lanes: nearest.lanes,
            admission_threshold: admission.threshold,
            kernel_id: kernel.kernel_id.to_string(),
            kernel_manifest_sha256: current.manifest_sha256,
            embedding_slots: graph_contract
                .embedding_slots
                .iter()
                .map(|slot| slot.get())
                .collect(),
            fusion: "typed_block_rrf",
            rrf_k: graph_contract.rrf_k,
        });
    }
    let query_resolution =
        super::engine::resolve_query_constellation(&resolved, &state, &query_vectors)?;
    let query_hit = &query_resolution.hit;
    let query_cx = query_hit.cx_id;
    let query_props = read_graph_node_props(&plain, query_cx)?;
    validate_graph_node_props(query_cx, &query_props, &graph_contract.embedding_slots)?;
    drop(query_props);
    runtime.phase(
        "query_entry_resolve",
        format_args!(
            "query_cx={} corpus_rank={} corpus_score={} kernel_nearest_cx={} kernel_nearest_score={} kernel_candidates={}",
            query_cx,
            query_hit.rank,
            query_hit.score,
            nearest.cx_id,
            nearest.score,
            ranked_kernel_nodes.len()
        ),
    );
    if args.citation_target.is_some() {
        return super::kernel_citation_answer::run(
            super::kernel_citation_answer::CitationAnswerRequest {
                args: &args,
                resolved: &resolved,
                roster: &roster,
                kernel_id: kernel.kernel_id,
                kernel_manifest_sha256: &current.manifest_sha256,
                embedding_slots: &graph_contract.embedding_slots,
                fusion: &graph_contract.fusion,
                rrf_k: graph_contract.rrf_k,
                nearest: &nearest,
                admission_threshold: admission.threshold,
            },
        );
    }
    let anchored_kernel_nodes = grounded_kernel_targets(
        &plain,
        &kernel.members,
        anchor.as_ref(),
        &graph_contract.embedding_slots,
    )?;
    if anchored_kernel_nodes.is_empty() {
        return Err(CalyxError::kernel_ungrounded(
            "CURRENT kernel has no grounded members matching the requested physical graph anchors",
        )
        .into());
    }
    let derivation = derive_panel_kernel_answer(
        kernel.kernel_id,
        &ranked_kernel_nodes,
        &graph,
        query_cx,
        &anchored_kernel_nodes,
        args.max_hops,
    )?;
    runtime.phase(
        "hop_chain",
        format_args!(
            "ranked_nodes={} anchored_nodes={} hops={} max_hops={}",
            ranked_kernel_nodes.len(),
            anchored_kernel_nodes.len(),
            derivation.hops.len(),
            args.max_hops
        ),
    );
    let source_support = evaluate_path_source_support(&resolved, &args.query, &derivation)?;
    // The per-source read loop reads one retained source file per path node
    // (get_base + input_from_ref) after a read-capable vault open + full Ledger
    // CF open; surface the source count and total retained bytes so the phase
    // cost divides by the loop cardinality — a cheap in-detail split without
    // restructuring the callee (issue #1977 item 1).
    let source_bytes: u64 = source_support
        .sources
        .iter()
        .map(|source| source.input_bytes)
        .sum();
    runtime.phase(
        "source_support",
        format_args!(
            "verdict={} sources={} source_bytes={} matched_terms={} coverage_bps={}",
            source_support.verdict,
            source_support.sources.len(),
            source_bytes,
            source_support.matched_terms.len(),
            source_support.weighted_coverage_bps
        ),
    );
    if source_support.verdict != "supported" {
        roster.emit_runtime_line();
        return print_json(&output::GroundedKernelSourceRefusalOut {
            status: "refused",
            code: CALYX_KERNEL_SOURCE_LOW_SUPPORT,
            reason: "retained_constellation_sources_do_not_support_query_proposition",
            nearest_cx_id: query_cx.to_string(),
            nearest_score: nearest.score,
            nearest_lanes: nearest.lanes,
            admission_threshold: admission.threshold,
            kernel_id: kernel.kernel_id.to_string(),
            kernel_manifest_sha256: current.manifest_sha256,
            embedding_slots: graph_contract
                .embedding_slots
                .iter()
                .map(|slot| slot.get())
                .collect(),
            fusion: "typed_block_rrf",
            rrf_k: graph_contract.rrf_k,
            source_support,
        });
    }
    let query_sha256 = sha256_bytes(args.query.as_bytes());
    let query_hash = decode_sha256(&query_sha256, "query_input_sha256")?;
    let manifest_hash = decode_sha256(&current.manifest_sha256, "kernel_manifest_sha256")?;
    let answer_id = kernel_answer_id(
        resolved.vault_id.to_string().as_bytes(),
        &query_hash,
        &manifest_hash,
        args.anchor.as_deref(),
        args.max_hops,
    );
    // answer_persist sub-phase 1/5 (issue #1977): pure in-heap hashing — query
    // sha256 + blake3 answer_id + manifest-hash decode. No I/O; establishes the
    // baseline so the vault-I/O sub-phases below stand on their own boundary.
    runtime.phase(
        "answer_id_derive",
        format_args!(
            "answer_id={} query_bytes={}",
            hex32(&answer_id),
            args.query.len()
        ),
    );
    let query_pointer = retain_kernel_query(&resolved.path, &query_sha256, args.query.as_bytes())?;
    // answer_persist sub-phase 2/5: retain the query bytes under inputs/queries/
    // (content-addressed write-if-absent + physical readback verify) — one small
    // file, isolated so it can never be conflated with the ledger/vault cost.
    runtime.phase("retain_query", format_args!("pointer={query_pointer}"));
    let resident_addr = args
        .resident_addr
        .map(|addr| addr.to_string())
        .unwrap_or_else(|| "local-cpu".to_string());
    let context = KernelAnswerRecordContext {
        answer_id: answer_id.to_vec(),
        query_input_sha256: query_hash,
        query_input_pointer: query_pointer.clone(),
        kernel_manifest_sha256: manifest_hash,
        embedding_slots: graph_contract.embedding_slots.clone(),
        fusion: graph_contract.fusion.clone(),
        rrf_k: graph_contract.rrf_k,
        nearest_score: nearest.score,
        nearest_lanes: nearest.lanes.clone(),
        admission_threshold: admission.threshold,
        resident_addr,
        anchor: args.anchor.clone(),
        max_hops: args.max_hops,
        source_support: source_support.clone(),
        query_resolution: Some(KernelQueryResolutionEvidence {
            method: "full_corpus_rrf_to_resident_kernel_v1".to_string(),
            search_manifest_sha256: query_resolution.search_manifest_sha256,
            search_base_seq: query_resolution.search_base_seq,
            corpus_rank: query_hit.rank,
            corpus_score: query_hit.score,
            kernel_nearest_cx_id: nearest.cx_id,
        }),
    };
    // Shared substrate fix (issue #1864): the answer path only appends one
    // Answer ledger row — it must not materialize every MVCC row of every CF
    // into the heap (the +8.5 GB / 30.6 GB spike). Open write-capable but
    // latest-readback so durable rows stay demand-paged through the mmap-backed
    // CF router; the ledger append, WAL/group-commit, and the fresh ledger
    // readback below are all preserved.
    let writable_vault = AsterVault::open(
        &resolved.path,
        resolved.vault_id,
        vault_salt(resolved.vault_id, &resolved.name),
        VaultOptions::write_latest_readback(),
    )?;
    // answer_persist sub-phase 3/5: the write-capable vault open. Even in
    // latest-readback mode (issue #1864, no MVCC materialization) this open runs
    // private_fs::harden_tree over the whole vault tree, WAL recovery, the
    // CfRouter SST open, and retained-input reconciliation. `last_recovered_seq`
    // and `torn_tail` here reveal whether WAL replay actually did work; a
    // last_recovered_seq that equals latest_seq with torn_tail=false means the
    // open cost is tree-walk + SST/router open, not WAL replay.
    let recovery = writable_vault.recovery_report();
    runtime.phase(
        "writable_vault_open",
        format_args!(
            "latest_seq={} last_recovered_seq={} torn_tail={}",
            writable_vault.latest_seq(),
            recovery.last_recovered_seq,
            recovery.torn_tail.is_some()
        ),
    );
    let answer = panel_kernel_answer_with_aster_ledger(AsterPanelKernelAnswerRequest {
        kernel_id: kernel.kernel_id,
        ranked_kernel_nodes: &ranked_kernel_nodes,
        graph: &graph,
        query_cx,
        anchored_kernel_nodes: &anchored_kernel_nodes,
        max_hops: args.max_hops,
        context: &context,
        vault: &writable_vault,
        vault_dir: &resolved.path,
    })?;
    // answer_persist sub-phase 4/5: the ledger append + group-commit inside
    // panel_kernel_answer_with_aster_ledger. This call re-derives the hop chain,
    // appends the per-hop + complete Answer rows through the WAL/group-commit
    // path, then reopens the Ledger CF (AsterLedgerCfStore::open, a full
    // ledger-CF materialization) to verify the freshly written refs.
    runtime.phase(
        "ledger_append_commit",
        format_args!(
            "provenance_rows={} hops={}",
            answer.provenance.len(),
            answer.hops.len()
        ),
    );
    let complete_ref = answer
        .provenance
        .last()
        .ok_or_else(|| CliError::runtime("kernel answer returned no complete ledger ref"))?;
    verify_complete_answer_readback(&resolved.path, complete_ref, &context, query_cx)?;
    // answer_persist sub-phase 5/5: independent physical readback of the complete
    // Answer row. verify_complete_answer_readback opens the Ledger CF afresh
    // (another full ledger-CF materialization) purely to read back one seq.
    runtime.phase(
        "answer_readback_verify",
        format_args!(
            "answer_id={} complete_seq={} provenance_rows={}",
            hex32(&answer_id),
            complete_ref.seq,
            answer.provenance.len()
        ),
    );
    let ledger_refs = answer
        .provenance
        .iter()
        .map(ledger_ref_out)
        .collect::<Vec<_>>();
    let hops = answer
        .hops
        .iter()
        .map(|hop| output::GroundedKernelAnswerHopOut {
            edge_kind: "association",
            from_cx_id: hop.from.to_string(),
            to_cx_id: hop.to.to_string(),
            edge_weight: hop.edge_weight,
            hop_index: hop.hop_index,
            hop_score: hop.hop_score,
            ledger_ref: ledger_ref_out(&hop.ledger_ref),
        })
        .collect();
    roster.emit_runtime_line();
    print_json(&output::GroundedKernelAnswerOut {
        status: "grounded",
        traversal_mode: "association",
        answer_id: hex32(&answer_id),
        query_cx_id: query_cx.to_string(),
        kernel_id: kernel.kernel_id.to_string(),
        kernel_manifest_sha256: current.manifest_sha256,
        embedding_slots: graph_contract
            .embedding_slots
            .iter()
            .map(|slot| slot.get())
            .collect(),
        fusion: "typed_block_rrf",
        rrf_k: graph_contract.rrf_k,
        nearest_score: nearest.score,
        nearest_lanes: nearest.lanes,
        admission_threshold: admission.threshold,
        anchor_kernel_node_id: answer.anchor_kernel_node.to_string(),
        hops,
        total_score: answer.total_score,
        ledger_refs,
        retained_query_pointer: query_pointer,
        source_support,
        physical_readback: "verified",
    })
}

pub(super) fn measure_kernel_query_vectors(
    state: &VaultPanelState,
    roster: &SearchTextRoster<'_>,
    resolved: &ResolvedVault,
    query: &str,
    resident_addr: Option<SocketAddr>,
) -> CliResult<Vec<(SlotId, SlotVector)>> {
    match resident_addr {
        Some(addr) => {
            measure_search_query_vectors_via_resident(state, roster, resolved, query, addr)
        }
        None => {
            require_resident_for_gpu_text_search(roster)?;
            measure_local_cpu_query_vectors(state, &roster.local_cpu, query)
        }
    }
}

fn read_graph_node_props(graph: &PhysicalPlainGraph, id: CxId) -> CliResult<AsterAssocNodeProps> {
    let bytes = graph.get_node(id)?.ok_or_else(|| {
        CliError::runtime(format!(
            "accepted association graph has no node row for resolved query constellation {id}"
        ))
    })?;
    serde_json::from_slice(&bytes)
        .map_err(|error| CliError::runtime(format!("decode physical graph node {id}: {error}")))
}

fn validate_graph_node_props(
    id: CxId,
    props: &AsterAssocNodeProps,
    embedding_slots: &[SlotId],
) -> CliResult {
    if props.embedding.is_some() || !props.embeddings.is_empty() {
        return Err(CliError::runtime(format!(
            "physical graph node {id} contains forbidden legacy dense embedding state"
        )));
    }
    if props
        .typed_embeddings
        .keys()
        .copied()
        .ne(embedding_slots.iter().copied())
    {
        return Err(CliError::runtime(format!(
            "physical graph node {id} slots {:?} differ from contract {:?}",
            props
                .typed_embeddings
                .keys()
                .map(|slot| slot.get())
                .collect::<Vec<_>>(),
            embedding_slots
                .iter()
                .map(|slot| slot.get())
                .collect::<Vec<_>>()
        )));
    }
    Ok(())
}

/// The sealed kernel health gate proves every default member reaches physical
/// grounding. Therefore ordinary answers can traverse to the ranked kernel
/// members directly instead of re-reading every member's large vector payload
/// merely to rediscover its anchor flag. An explicit anchor-kind request keeps
/// its narrower semantics and performs bounded point reads over kernel members.
pub(super) fn grounded_kernel_targets(
    graph: &PhysicalPlainGraph,
    kernel_members: &[CxId],
    requested_anchor: Option<&AnchorKind>,
    embedding_slots: &[SlotId],
) -> CliResult<Vec<CxId>> {
    let Some(requested_anchor) = requested_anchor else {
        return Ok(kernel_members.to_vec());
    };
    let mut matched = Vec::new();
    for &id in kernel_members {
        let props = read_graph_node_props(graph, id)?;
        validate_graph_node_props(id, &props, embedding_slots)?;
        if props
            .anchors
            .iter()
            .any(|stored| stored == requested_anchor)
        {
            matched.push(id);
        }
    }
    Ok(matched)
}

pub(super) fn parse_graph_props(
    rows: &[(CxId, Vec<u8>)],
    embedding_slots: &[SlotId],
) -> CliResult<Vec<(CxId, AsterAssocNodeProps)>> {
    let mut parsed = Vec::with_capacity(rows.len());
    for (id, bytes) in rows {
        let props: AsterAssocNodeProps = serde_json::from_slice(bytes).map_err(|error| {
            CliError::runtime(format!("decode physical graph node {id} props: {error}"))
        })?;
        if props.embedding.is_some() || !props.embeddings.is_empty() {
            return Err(CliError::runtime(format!(
                "physical graph node {id} contains forbidden legacy dense embedding state"
            )));
        }
        if props
            .typed_embeddings
            .keys()
            .copied()
            .ne(embedding_slots.iter().copied())
        {
            return Err(CliError::runtime(format!(
                "physical graph node {id} slots {:?} differ from contract {:?}",
                props
                    .typed_embeddings
                    .keys()
                    .map(|slot| slot.get())
                    .collect::<Vec<_>>(),
                embedding_slots
                    .iter()
                    .map(|slot| slot.get())
                    .collect::<Vec<_>>()
            )));
        }
        parsed.push((*id, props));
    }
    if parsed.is_empty() {
        return Err(CliError::runtime("physical graph has no node props"));
    }
    Ok(parsed)
}

pub(super) fn nearest_graph_node(
    props: &[(CxId, AsterAssocNodeProps)],
    query: &TypedPanelVectors,
    embedding_slots: &[SlotId],
) -> CliResult<TypedPanelFusionHit> {
    let candidates = props
        .iter()
        .map(|(id, props)| (*id, &props.typed_embeddings))
        .collect::<BTreeMap<_, _>>();
    rank_typed_panel_candidate_refs(query, &candidates, embedding_slots)?
        .into_iter()
        .next()
        .ok_or_else(|| CliError::runtime("physical graph has no nearest constellation"))
}

pub(super) fn query_panel_vectors(
    vectors: &[(SlotId, SlotVector)],
    embedding_slots: &[SlotId],
) -> CliResult<TypedPanelVectors> {
    let panel = vectors
        .iter()
        .filter(|(slot, _)| embedding_slots.contains(slot))
        .map(|(slot, vector)| (*slot, vector.clone()))
        .collect::<TypedPanelVectors>();
    if panel.keys().copied().ne(embedding_slots.iter().copied()) {
        return Err(CliError::runtime(format!(
            "kernel query measured slots {:?}, expected {:?}",
            panel.keys().map(|slot| slot.get()).collect::<Vec<_>>(),
            embedding_slots
                .iter()
                .map(|slot| slot.get())
                .collect::<Vec<_>>()
        )));
    }
    Ok(panel)
}

fn kernel_answer_id(
    vault_id: &[u8],
    query_sha256: &[u8; 32],
    manifest_sha256: &[u8; 32],
    anchor: Option<&str>,
    max_hops: usize,
) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"calyx-kernel-answer-v5-bounded-graph-local-source-support-v2");
    hasher.update(&(vault_id.len() as u64).to_be_bytes());
    hasher.update(vault_id);
    hasher.update(query_sha256);
    hasher.update(manifest_sha256);
    hasher.update(anchor.unwrap_or("any").as_bytes());
    hasher.update(&(max_hops as u64).to_be_bytes());
    *hasher.finalize().as_bytes()
}

pub(super) fn retain_kernel_query(vault: &Path, sha256: &str, bytes: &[u8]) -> CliResult<String> {
    let relative = format!("queries/{sha256}.txt");
    let path = vault.join("inputs").join(&relative);
    if path.exists() {
        let existing = fs::read(&path).map_err(|error| {
            CliError::io(format!(
                "read retained kernel query {}: {error}",
                path.display()
            ))
        })?;
        if existing != bytes {
            return Err(CliError::runtime(format!(
                "retained query path {} contains bytes that do not match its digest",
                path.display()
            )));
        }
    } else {
        write_bytes_atomic(&path, bytes, "retained kernel query")?;
    }
    let readback = fs::read(&path).map_err(|error| {
        CliError::io(format!(
            "read back retained kernel query {}: {error}",
            path.display()
        ))
    })?;
    if readback != bytes || sha256_bytes(&readback) != sha256 {
        return Err(CliError::runtime(format!(
            "retained kernel query physical readback mismatch at {}",
            path.display()
        )));
    }
    Ok(format!("calyx-vault://inputs/{relative}"))
}

fn verify_complete_answer_readback(
    vault: &Path,
    reference: &calyx_core::LedgerRef,
    context: &KernelAnswerRecordContext,
    query_cx: CxId,
) -> CliResult {
    let store = AsterLedgerCfStore::open(vault)?;
    let row = store
        .read_seq(reference.seq)?
        .ok_or_else(|| CliError::runtime("complete kernel Answer row is physically absent"))?;
    let entry = decode(&row.bytes)?;
    let payload: serde_json::Value = serde_json::from_slice(&entry.payload).map_err(|error| {
        CliError::runtime(format!("decode complete kernel Answer payload: {error}"))
    })?;
    let answer_id: &[u8; 32] = context
        .answer_id
        .as_slice()
        .try_into()
        .map_err(|_| CliError::runtime("kernel answer id is not exactly 32 bytes"))?;
    let expected_answer_id = hex32(answer_id);
    let expected_query_id = query_cx.to_string();
    if entry.kind != EntryKind::Answer
        || entry.entry_hash != reference.hash
        || !matches!(&entry.subject, SubjectId::Query(id) if id == &context.answer_id)
        || payload.get("type").and_then(|value| value.as_str()) != Some("kernel_answer_v4")
        || payload.get("answer_id").and_then(|value| value.as_str())
            != Some(expected_answer_id.as_str())
        || payload.get("query_id").and_then(|value| value.as_str())
            != Some(expected_query_id.as_str())
        || payload.get("source_support")
            != Some(
                &serde_json::to_value(&context.source_support).map_err(|error| {
                    CliError::runtime(format!("encode expected source support: {error}"))
                })?,
            )
    {
        return Err(CliError::runtime(
            "complete kernel Answer physical readback differs from the issued answer",
        ));
    }
    Ok(())
}

fn ledger_ref_out(reference: &calyx_core::LedgerRef) -> output::GroundedKernelLedgerRefOut {
    output::GroundedKernelLedgerRefOut {
        seq: reference.seq,
        hash: hex32(&reference.hash),
    }
}
