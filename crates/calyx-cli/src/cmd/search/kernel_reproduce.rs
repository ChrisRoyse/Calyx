//! Re-derive a recorded kernel answer from its retained query and immutable generation.

use std::collections::BTreeSet;
use std::fs;
use std::net::SocketAddr;
use std::path::{Component, Path};

use calyx_aster::plain_graph::PhysicalPlainGraph;
use calyx_core::{CalyxError, CxId, SlotId};
use calyx_lodestar::{
    ASTER_ASSOC_METADATA_KEY, FsKernelStore, KernelAnswerRecordContext, KernelAnswerSourceSupport,
    KernelQueryResolutionEvidence, PANEL_ASTER_ASSOC_COLLECTION, RecallPassMode,
    TypedPanelFusionLane, derive_panel_kernel_answer, kernel_answer_derivation_hash, kernel_health,
    read_kernel_artifact, typed_panel_kernel_rank_with_nearest,
};
use calyx_registry::{load_vault_panel_state, require_vault_registry_contracts};

use super::super::ingest::parse_anchor_kind;
use super::super::kernel_generation::{
    decode_sha256, hex32, load_generation_by_sha256_reference,
    load_generation_by_sha256_with_index, physical_graph_contract, sha256_bytes,
};
use super::super::vault::ResolvedVault;
use super::kernel_answer::{
    grounded_kernel_targets, measure_kernel_query_vectors, nearest_graph_node, parse_graph_props,
    query_panel_vectors,
};
use super::kernel_source_support::evaluate_path_source_support_with_method;
use super::roster::SearchTextRoster;
use crate::error::{CliError, CliResult};
use crate::panel_commands::rank_kernel_at_resident;

pub(crate) fn rederive_kernel_answer_hash(
    resolved: &ResolvedVault,
    answer_id: &[u8],
    payload: &serde_json::Value,
    resident_override: Option<SocketAddr>,
) -> CliResult<[u8; 32]> {
    let payload_type = payload.get("type").and_then(|value| value.as_str());
    let bounded_v4 = payload_type == Some("kernel_answer_v4");
    if !bounded_v4 && payload_type != Some("kernel_answer_v3") {
        return Err(CliError::runtime(
            "kernel reproduce requires a kernel_answer_v3 or kernel_answer_v4 payload",
        ));
    }
    let recorded_answer_id =
        decode_sha256(required_string(payload, "answer_id")?, "kernel answer_id")?;
    if recorded_answer_id.as_slice() != answer_id {
        return Err(CliError::runtime(
            "kernel Answer payload id differs from its ledger subject",
        ));
    }
    let query_hash = decode_sha256(
        required_string(payload, "query_input_sha256")?,
        "query_input_sha256",
    )?;
    let manifest_sha256 = required_string(payload, "kernel_manifest_sha256")?;
    let manifest_hash = decode_sha256(manifest_sha256, "kernel_manifest_sha256")?;
    let saved_resident = required_string(payload, "resident_addr")?;
    let resident_addr = match resident_override {
        Some(addr) => Some(addr),
        None if saved_resident == "local-cpu" => None,
        None => Some(super::parse::parse_resident_addr(saved_resident)?),
    };
    let (loaded, local_kernel_index) = if resident_addr.is_some() {
        (
            load_generation_by_sha256_reference(&resolved.path, manifest_sha256)?,
            None,
        )
    } else {
        let (loaded, index) =
            load_generation_by_sha256_with_index(&resolved.path, manifest_sha256)?;
        (loaded, Some(index))
    };
    let graph_contract = &loaded.manifest.graph;
    let admission = &loaded.manifest.admission;
    if graph_contract.fusion != "typed_block_rrf" {
        return Err(CalyxError::stale_derived(format!(
            "kernel Answer references legacy fusion {}; rebuild the typed graph/kernel and issue a new answer",
            graph_contract.fusion
        ))
        .into());
    }
    let embedding_slots = required_slots(payload, "embedding_slots")?;
    if embedding_slots != graph_contract.embedding_slots
        || required_string(payload, "fusion")? != graph_contract.fusion
        || required_u64(payload, "rrf_k")? != u64::from(graph_contract.rrf_k)
    {
        return Err(CliError::runtime(
            "kernel Answer panel fusion contract differs from its immutable manifest",
        ));
    }
    let query_pointer = format!("calyx-vault://inputs/queries/{}.txt", hex32(&query_hash));
    let query_path = retained_kernel_query_path(&resolved.path, &query_pointer)?;
    let query = fs::read(&query_path).map_err(|error| {
        CliError::io(format!(
            "read retained kernel query {}: {error}",
            query_path.display()
        ))
    })?;
    if sha256_bytes(&query) != hex32(&query_hash) {
        return Err(CliError::runtime(format!(
            "retained kernel query {} fails its recorded SHA-256",
            query_path.display()
        )));
    }
    let query_text = std::str::from_utf8(&query).map_err(|error| {
        CliError::runtime(format!("retained kernel query is not UTF-8: {error}"))
    })?;
    if let Some(jurisdiction) = &loaded.manifest.jurisdiction
        && let Some(conflict) =
            super::super::kernel_scope::explicit_scope_conflict(query_text, jurisdiction)
    {
        return Err(CalyxError::reproduce_drift_exceeded(format!(
            "retained kernel query now conflicts with immutable scope: {}={}",
            conflict.kind, conflict.detected
        ))
        .into());
    }
    require_vault_registry_contracts(&resolved.path)?;
    let state = load_vault_panel_state(&resolved.path)?;
    let roster = SearchTextRoster::derive(&state);
    let vectors =
        measure_kernel_query_vectors(&state, &roster, resolved, query_text, resident_addr)?;
    let query_panel = query_panel_vectors(&vectors, &embedding_slots)?;
    let plain =
        PhysicalPlainGraph::open_latest_isolated(&resolved.path, PANEL_ASTER_ASSOC_COLLECTION)?;
    let metadata_bytes = plain
        .get_metadata(ASTER_ASSOC_METADATA_KEY)?
        .ok_or_else(|| CliError::runtime("physical graph metadata disappeared"))?;
    let node_rows = if bounded_v4 {
        None
    } else {
        Some(plain.node_props()?)
    };
    let (graph, csr_bytes) = plain
        .assoc_graph_with_csr_bytes()?
        .ok_or_else(|| CliError::runtime("physical graph CSR disappeared"))?;
    let legacy_contract_mismatch = if let Some(node_rows) = node_rows.as_deref() {
        let (physical_hash, node_props_hash) =
            physical_graph_contract(&metadata_bytes, node_rows, &csr_bytes);
        node_props_hash != graph_contract.node_props_sha256
            || hex32(&physical_hash) != graph_contract.physical_contract_sha256
    } else {
        false
    };
    if graph.node_count() != graph_contract.nodes
        || graph.edge_count() != graph_contract.edges
        || sha256_bytes(&metadata_bytes) != graph_contract.metadata_sha256
        || sha256_bytes(&csr_bytes) != graph_contract.csr_sha256
        || legacy_contract_mismatch
    {
        return Err(CliError::runtime(
            "kernel reproduce physical graph differs from the immutable answer manifest",
        ));
    }
    let store = FsKernelStore::new(&resolved.path);
    let kernel = read_kernel_artifact(loaded.manifest.kernel_id, &store)?;
    let health = kernel_health(loaded.manifest.kernel_id, &store)?;
    if health.recall.pass_mode != RecallPassMode::Passed || health.grounded_fraction != 1.0 {
        return Err(CalyxError::kernel_ungrounded(
            "kernel reproduce refuses an ungrounded or below-gate immutable kernel",
        )
        .into());
    }
    if bounded_v4 && hex32(&kernel.corpus_shard_hash) != graph_contract.physical_contract_sha256 {
        return Err(CalyxError::reproduce_drift_exceeded(
            "bounded kernel answer corpus shard differs from its physical graph contract",
        )
        .into());
    }
    let (ranked_kernel_nodes, ranked_nearest) = if let Some(addr) = resident_addr {
        let ranked = rank_kernel_at_resident(
            addr,
            &loaded.manifest_sha256,
            loaded.manifest.kernel_id,
            &loaded.manifest.index_json.sha256,
            &query_panel,
        )?;
        if ranked.index_rows != kernel.members.len() {
            return Err(CliError::runtime(format!(
                "kernel reproduce resident returned {} rows for {} sealed members",
                ranked.index_rows,
                kernel.members.len()
            )));
        }
        (ranked.ranked_ids, ranked.nearest)
    } else {
        let index = local_kernel_index
            .as_ref()
            .expect("local historical kernel index loaded without resident");
        typed_panel_kernel_rank_with_nearest(index, &query_panel)?
    };
    let parsed_props = node_rows
        .as_deref()
        .map(|rows| parse_graph_props(rows, &embedding_slots))
        .transpose()?;
    let (query_cx, nearest, query_resolution) =
        if bounded_v4 {
            let resolved_query =
                super::engine::resolve_query_constellation(resolved, &state, &vectors)?;
            let recorded_resolution: KernelQueryResolutionEvidence =
                serde_json::from_value(payload.get("query_resolution").cloned().ok_or_else(
                    || CliError::runtime("kernel Answer query_resolution is missing"),
                )?)
                .map_err(|error| {
                    CliError::runtime(format!("decode kernel Answer query_resolution: {error}"))
                })?;
            let observed_resolution = KernelQueryResolutionEvidence {
                method: "full_corpus_rrf_to_resident_kernel_v1".to_string(),
                search_manifest_sha256: resolved_query.search_manifest_sha256,
                search_base_seq: resolved_query.search_base_seq,
                corpus_rank: resolved_query.hit.rank,
                corpus_score: resolved_query.hit.score,
                kernel_nearest_cx_id: ranked_nearest.cx_id,
            };
            if observed_resolution != recorded_resolution {
                return Err(CalyxError::reproduce_drift_exceeded(
                    "bounded kernel query resolution differs from its ledger evidence",
                )
                .into());
            }
            if plain.get_node(resolved_query.hit.cx_id)?.is_none() {
                return Err(CalyxError::reproduce_drift_exceeded(
                "resolved full-corpus query entry is absent from the accepted association graph",
            )
            .into());
            }
            (
                resolved_query.hit.cx_id,
                ranked_nearest,
                Some(recorded_resolution),
            )
        } else {
            let nearest = nearest_graph_node(
                parsed_props
                    .as_deref()
                    .expect("legacy kernel answer loaded graph props"),
                &query_panel,
                &embedding_slots,
            )?;
            (nearest.cx_id, nearest, None)
        };
    let recorded_query_cx = required_string(payload, "query_id")?
        .parse::<CxId>()
        .map_err(|error| CliError::runtime(format!("parse kernel Answer query_id: {error}")))?;
    let recorded_score = required_f32(payload, "nearest_score")?;
    let recorded_lanes: Vec<TypedPanelFusionLane> = serde_json::from_value(
        payload
            .get("nearest_lanes")
            .cloned()
            .ok_or_else(|| CliError::runtime("kernel Answer nearest_lanes is missing"))?,
    )
    .map_err(|error| CliError::runtime(format!("decode kernel Answer nearest_lanes: {error}")))?;
    let admission_threshold = required_f32(payload, "admission_threshold")?;
    if query_cx != recorded_query_cx
        || nearest.score.to_bits() != recorded_score.to_bits()
        || nearest.lanes != recorded_lanes
        || admission_threshold.to_bits() != admission.threshold.to_bits()
        || nearest.score < admission_threshold
    {
        return Err(CalyxError::reproduce_drift_exceeded(format!(
            "kernel query admission drift: target {}->{query_cx} score {}->{} threshold {}->{}",
            recorded_query_cx,
            recorded_score,
            nearest.score,
            admission_threshold,
            admission.threshold
        ))
        .into());
    }
    let anchor_raw = payload
        .get("anchor")
        .and_then(|value| value.as_str())
        .map(str::to_string);
    let anchor = anchor_raw.as_deref().map(parse_anchor_kind).transpose()?;
    let max_hops: usize = required_u64(payload, "max_hops")?
        .try_into()
        .map_err(|_| CliError::runtime("kernel Answer max_hops exceeds usize"))?;
    let anchored = if bounded_v4 {
        grounded_kernel_targets(&plain, &kernel.members, anchor.as_ref(), &embedding_slots)?
    } else {
        let kernel_members = kernel.members.iter().copied().collect::<BTreeSet<_>>();
        parsed_props
            .as_deref()
            .expect("legacy kernel answer loaded graph props")
            .iter()
            .filter(|(id, props)| {
                kernel_members.contains(id)
                    && props
                        .anchors
                        .iter()
                        .any(|stored| anchor.as_ref().is_none_or(|requested| stored == requested))
            })
            .map(|(id, _)| *id)
            .collect::<Vec<_>>()
    };
    let derivation = derive_panel_kernel_answer(
        kernel.kernel_id,
        &ranked_kernel_nodes,
        &graph,
        query_cx,
        &anchored,
        max_hops,
    )?;
    let recorded: KernelAnswerSourceSupport = serde_json::from_value(
        payload
            .get("source_support")
            .cloned()
            .ok_or_else(|| CliError::runtime("kernel Answer source_support is missing"))?,
    )
    .map_err(|error| CliError::runtime(format!("decode kernel Answer source_support: {error}")))?;
    let source_support = evaluate_path_source_support_with_method(
        resolved,
        query_text,
        &derivation,
        &recorded.method,
    )?;
    if source_support != recorded || source_support.verdict != "supported" {
        return Err(CalyxError::reproduce_drift_exceeded(
            "kernel Answer retained source-support proof differs from its ledger payload",
        )
        .into());
    }
    let context = KernelAnswerRecordContext {
        answer_id: answer_id.to_vec(),
        query_input_sha256: query_hash,
        query_input_pointer: query_pointer,
        kernel_manifest_sha256: manifest_hash,
        embedding_slots: embedding_slots.clone(),
        fusion: graph_contract.fusion.clone(),
        rrf_k: graph_contract.rrf_k,
        nearest_score: nearest.score,
        nearest_lanes: nearest.lanes,
        admission_threshold,
        resident_addr: saved_resident.to_string(),
        anchor: anchor_raw,
        max_hops,
        source_support,
        query_resolution,
    };
    kernel_answer_derivation_hash(&derivation, &context).map_err(Into::into)
}

fn retained_kernel_query_path(vault: &Path, pointer: &str) -> CliResult<std::path::PathBuf> {
    let relative = pointer
        .strip_prefix("calyx-vault://inputs/")
        .ok_or_else(|| {
            CliError::runtime("kernel Answer query pointer has an unsupported scheme")
        })?;
    let path = Path::new(relative);
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(CliError::runtime(
            "kernel Answer query pointer is not a strict relative input path",
        ));
    }
    Ok(vault.join("inputs").join(path))
}

fn required_string<'a>(payload: &'a serde_json::Value, field: &str) -> CliResult<&'a str> {
    payload
        .get(field)
        .and_then(|value| value.as_str())
        .ok_or_else(|| CliError::runtime(format!("kernel Answer payload field {field} is missing")))
}

fn required_u64(payload: &serde_json::Value, field: &str) -> CliResult<u64> {
    payload
        .get(field)
        .and_then(|value| value.as_u64())
        .ok_or_else(|| CliError::runtime(format!("kernel Answer payload field {field} is not u64")))
}

fn required_slots(payload: &serde_json::Value, field: &str) -> CliResult<Vec<SlotId>> {
    let values = payload
        .get(field)
        .and_then(|value| value.as_array())
        .ok_or_else(|| {
            CliError::runtime(format!(
                "kernel Answer payload field {field} is not an array"
            ))
        })?;
    let slots = values
        .iter()
        .map(|value| {
            let raw = value.as_u64().ok_or_else(|| {
                CliError::runtime(format!(
                    "kernel Answer payload field {field} contains a non-u64"
                ))
            })?;
            let raw = u16::try_from(raw).map_err(|_| {
                CliError::runtime(format!("kernel Answer payload field {field} exceeds u16"))
            })?;
            Ok(SlotId::new(raw))
        })
        .collect::<CliResult<Vec<_>>>()?;
    if slots.len() < 2 || !slots.windows(2).all(|pair| pair[0] < pair[1]) {
        return Err(CliError::runtime(format!(
            "kernel Answer payload field {field} is not a strict ordered panel"
        )));
    }
    Ok(slots)
}

fn required_f32(payload: &serde_json::Value, field: &str) -> CliResult<f32> {
    let value = payload
        .get(field)
        .and_then(|value| value.as_f64())
        .ok_or_else(|| {
            CliError::runtime(format!(
                "kernel Answer payload field {field} is not numeric"
            ))
        })? as f32;
    if !value.is_finite() {
        return Err(CliError::runtime(format!(
            "kernel Answer payload field {field} is non-finite"
        )));
    }
    Ok(value)
}
