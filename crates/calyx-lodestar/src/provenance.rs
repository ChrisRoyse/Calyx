//! Ledger-backed Lodestar provenance writers.

use calyx_core::{Clock, CxId, LedgerRef, SlotId};
use calyx_ledger::{
    ActorId, EntryKind, LedgerAppender, LedgerCfStore, PayloadBuilder, RedactionPolicy, SubjectId,
};
use calyx_paths::AssocGraph;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{Kernel, KernelParams, LodestarError, Result, build_kernel_pipeline};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct KernelBuildReceipt {
    pub kernel: Kernel,
    pub ledger_ref: LedgerRef,
}

/// Content-addressed inputs needed to independently re-derive a persisted
/// kernel answer. Raw query text is retained outside the ledger.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct KernelAnswerRecordContext {
    pub answer_id: Vec<u8>,
    pub query_input_sha256: [u8; 32],
    pub query_input_pointer: String,
    pub kernel_manifest_sha256: [u8; 32],
    pub embedding_slots: Vec<SlotId>,
    pub fusion: String,
    pub rrf_k: u32,
    pub nearest_score: f32,
    pub nearest_lanes: Vec<crate::TypedPanelFusionLane>,
    pub admission_threshold: f32,
    pub resident_addr: String,
    pub anchor: Option<String>,
    pub max_hops: usize,
    pub source_support: KernelAnswerSourceSupport,
    /// Present for the bounded full-corpus-entry → resident-kernel traversal
    /// introduced by kernel answer v4. Absent preserves byte-for-byte v3
    /// derivation and replay for already sealed answers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query_resolution: Option<KernelQueryResolutionEvidence>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct KernelQueryResolutionEvidence {
    pub method: String,
    pub search_manifest_sha256: String,
    pub search_base_seq: u64,
    pub corpus_rank: usize,
    pub corpus_score: f32,
    pub kernel_nearest_cx_id: CxId,
}

/// Deterministic proposition-support proof over the retained source bytes of
/// every constellation in an Answer path. Counts are integer/basis-point
/// values so the proof is byte-stable across CPU/GPU/platform reproductions.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KernelAnswerSourceSupport {
    pub schema_version: u32,
    pub method: String,
    pub verdict: String,
    pub query_terms: Vec<String>,
    pub matched_terms: Vec<String>,
    pub missing_terms: Vec<String>,
    pub matched_term_pairs: Vec<String>,
    pub matched_weight: u64,
    pub total_weight: u64,
    pub weighted_coverage_bps: u16,
    pub minimum_weighted_coverage_bps: u16,
    pub sources: Vec<KernelAnswerSourceEvidence>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KernelAnswerSourceEvidence {
    pub cx_id: CxId,
    pub input_blake3: String,
    pub retained_bytes_sha256: String,
    pub input_bytes: u64,
    pub base_ledger_seq: u64,
    pub base_ledger_hash: String,
    pub matched_terms: Vec<String>,
    pub matched_term_pairs: Vec<String>,
}

pub(crate) fn validate_kernel_answer_record_context(
    context: &KernelAnswerRecordContext,
) -> Result<()> {
    if context.answer_id.is_empty()
        || context.query_input_pointer.is_empty()
        || context.embedding_slots.len() < 2
        || context.fusion != "typed_block_rrf"
        || context.rrf_k != crate::PANEL_RRF_K
        || context.nearest_lanes.len() != context.embedding_slots.len()
        || context.max_hops == 0
        || context.max_hops > 32
        || context.source_support.schema_version != 1
        || !matches!(
            context.source_support.method.as_str(),
            "retained_constellation_lexical_v1" | "retained_constellation_lexical_v2"
        )
        || context.source_support.verdict != "supported"
        || context.source_support.query_terms.is_empty()
        || context.source_support.total_weight == 0
        || context.source_support.matched_weight > context.source_support.total_weight
        || context.source_support.minimum_weighted_coverage_bps > 10_000
        || context.source_support.weighted_coverage_bps
            < context.source_support.minimum_weighted_coverage_bps
        || context.source_support.sources.is_empty()
        || context.query_resolution.as_ref().is_some_and(|resolution| {
            resolution.method != "full_corpus_rrf_to_resident_kernel_v1"
                || resolution.search_manifest_sha256.len() != 64
                || !resolution
                    .search_manifest_sha256
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit())
                || resolution.search_base_seq == 0
                || resolution.corpus_rank == 0
                || !resolution.corpus_score.is_finite()
                || resolution.corpus_score < 0.0
        })
    {
        return Err(LodestarError::KernelInvalidParams {
            detail: "kernel answer record context needs a nonempty answer id/query pointer and max_hops in 1..=32"
                .to_string(),
        });
    }
    for (field, value) in [
        ("nearest_score", context.nearest_score),
        ("admission_threshold", context.admission_threshold),
    ] {
        if !value.is_finite() || !(0.0..=1.0).contains(&value) {
            return Err(LodestarError::KernelInvalidParams {
                detail: format!("{field}={value} must be finite and in [0,1]"),
            });
        }
    }
    if context
        .nearest_lanes
        .iter()
        .zip(&context.embedding_slots)
        .any(|(lane, slot)| {
            lane.slot != *slot
                || lane.similarity.validate().is_err()
                || lane.rank == 0
                || !lane.rrf_contribution.is_finite()
                || lane.rrf_contribution < 0.0
        })
    {
        return Err(LodestarError::KernelInvalidParams {
            detail: "kernel answer lane evidence differs from the sealed panel contract"
                .to_string(),
        });
    }
    Ok(())
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub fn build_kernel_pipeline_with_ledger<S, C>(
    graph: &AssocGraph,
    anchors: &[CxId],
    params: &KernelParams,
    graph_seq: u64,
    ledger: &mut LedgerAppender<S, C>,
) -> Result<KernelBuildReceipt>
where
    S: LedgerCfStore,
    C: Clock,
{
    let kernel = build_kernel_pipeline(graph, anchors, params)?;
    let ledger_ref = append_kernel_build_entry(ledger, &kernel, graph_seq)?;
    Ok(KernelBuildReceipt { kernel, ledger_ref })
}

pub fn append_kernel_build_entry<S, C>(
    ledger: &mut LedgerAppender<S, C>,
    kernel: &Kernel,
    graph_seq: u64,
) -> Result<LedgerRef>
where
    S: LedgerCfStore,
    C: Clock,
{
    ledger
        .append(
            EntryKind::Kernel,
            SubjectId::Kernel(kernel.kernel_id.as_bytes().to_vec()),
            kernel_build_payload(kernel, graph_seq)?,
            ActorId::Service("calyx-lodestar".to_string()),
        )
        .map_err(Into::into)
}

pub fn append_answer_hop_entry<S, C>(
    ledger: &mut LedgerAppender<S, C>,
    query_cx: CxId,
    anchor_kernel_node: CxId,
    hop: AnswerHopEvidence,
) -> Result<LedgerRef>
where
    S: LedgerCfStore,
    C: Clock,
{
    ledger
        .append(
            EntryKind::Answer,
            SubjectId::Query(query_cx.as_bytes().to_vec()),
            answer_hop_payload(query_cx, anchor_kernel_node, hop)?,
            ActorId::Service("calyx-lodestar".to_string()),
        )
        .map_err(Into::into)
}

pub fn append_answer_complete_entry<S, C>(
    ledger: &mut LedgerAppender<S, C>,
    query_cx: CxId,
    anchor_kernel_node: CxId,
    kernel_id: CxId,
    hops: &[AnswerCompleteHopEvidence],
    total_score: f32,
) -> Result<LedgerRef>
where
    S: LedgerCfStore,
    C: Clock,
{
    ledger
        .append(
            EntryKind::Answer,
            SubjectId::Query(query_cx.as_bytes().to_vec()),
            complete_answer_payload(query_cx, anchor_kernel_node, kernel_id, hops, total_score)?,
            ActorId::Service("calyx-lodestar".to_string()),
        )
        .map_err(Into::into)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AnswerHopEvidence {
    pub from: CxId,
    pub to: CxId,
    pub edge_weight: f32,
    pub hop_index: u32,
    pub hop_score: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AnswerCompleteHopEvidence {
    pub from: CxId,
    pub to: CxId,
    pub edge_weight: f32,
    pub hop_index: u32,
    pub hop_score: f32,
    pub ledger_ref: LedgerRef,
}

pub fn kernel_members_hash(kernel: &Kernel) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"calyx-lodestar-kernel-members-v1");
    for member in &kernel.members {
        hasher.update(member.as_bytes());
    }
    *hasher.finalize().as_bytes()
}

fn kernel_build_payload(kernel: &Kernel, graph_seq: u64) -> Result<Vec<u8>> {
    let mut payload = PayloadBuilder::default();
    payload
        .insert_str("kernel_id", kernel.kernel_id.to_string())
        .insert_str("members_hash", hex(&kernel_members_hash(kernel)))
        .insert_u64("graph_seq", graph_seq)
        .insert_value("mfvs_approx_factor", json!(kernel.recall.approx_factor))
        .insert_value(
            "mfvs_tau_star_estimate",
            json!(kernel.recall.tau_star_estimate),
        )
        .insert_value("mfvs_tau_star_exact", json!(kernel.recall.tau_star_exact))
        .insert_value("recall_ratio", json!(kernel.recall.ratio));
    let bytes = encode_payload(payload.value(), "kernel build")?;
    RedactionPolicy::check_payload(&bytes)?;
    Ok(bytes)
}

fn answer_hop_payload(
    query_cx: CxId,
    anchor_kernel_node: CxId,
    hop: AnswerHopEvidence,
) -> Result<Vec<u8>> {
    let mut payload = PayloadBuilder::default();
    payload
        .insert_str("query_id", query_cx.to_string())
        .insert_str("anchor_kernel_node_id", anchor_kernel_node.to_string())
        .insert_str("edge_kind", "association")
        .insert_str("from_id", hop.from.to_string())
        .insert_str("to_id", hop.to.to_string())
        .insert_u64("hop_index", u64::from(hop.hop_index))
        .insert_value("edge_weight", json!(hop.edge_weight))
        .insert_value("hop_score", json!(hop.hop_score));
    let bytes = encode_payload(payload.value(), "answer hop")?;
    RedactionPolicy::check_payload(&bytes)?;
    Ok(bytes)
}

pub(crate) fn kernel_answer_hop_payload(
    context: &KernelAnswerRecordContext,
    query_cx: CxId,
    anchor_kernel_node: CxId,
    hop: AnswerHopEvidence,
) -> Result<Vec<u8>> {
    let mut payload = PayloadBuilder::default();
    payload
        .insert_str("type", "kernel_answer_hop_v1")
        .insert_str("answer_id", hex(&context.answer_id))
        .insert_str("query_id", query_cx.to_string())
        .insert_str("anchor_kernel_node_id", anchor_kernel_node.to_string())
        .insert_str("edge_kind", "association")
        .insert_str("from_id", hop.from.to_string())
        .insert_str("to_id", hop.to.to_string())
        .insert_u64("hop_index", u64::from(hop.hop_index))
        .insert_value("edge_weight", json!(hop.edge_weight))
        .insert_value("hop_score", json!(hop.hop_score));
    let bytes = encode_payload(payload.value(), "kernel answer hop")?;
    RedactionPolicy::check_payload(&bytes)?;
    Ok(bytes)
}

fn complete_answer_payload(
    query_cx: CxId,
    anchor_kernel_node: CxId,
    kernel_id: CxId,
    hops: &[AnswerCompleteHopEvidence],
    total_score: f32,
) -> Result<Vec<u8>> {
    let path = hops
        .iter()
        .map(|hop| {
            json!({
                "from_id": hop.from.to_string(),
                "cx_id": hop.to.to_string(),
                "to_id": hop.to.to_string(),
                "hop": hop.hop_index,
                "hop_index": hop.hop_index,
                "score": hop.hop_score,
                "hop_score": hop.hop_score,
                "edge_weight": hop.edge_weight,
                "edge_kind": "association",
                "ledger_ref": {
                    "seq": hop.ledger_ref.seq,
                    "hash": hex(&hop.ledger_ref.hash),
                },
            })
        })
        .collect::<Vec<_>>();
    let mut payload = PayloadBuilder::default();
    payload
        .insert_value("complete", json!(true))
        .insert_u64("expected_hops", hops.len() as u64)
        .insert_str("query_id", query_cx.to_string())
        .insert_str("anchor_kernel_node_id", anchor_kernel_node.to_string())
        .insert_str("kernel_id", kernel_id.to_string())
        .insert_value("total_score", json!(total_score))
        .insert_value("path", json!(path));
    let bytes = encode_payload(payload.value(), "complete answer")?;
    RedactionPolicy::check_payload(&bytes)?;
    Ok(bytes)
}

pub(crate) fn kernel_answer_complete_payload(
    context: &KernelAnswerRecordContext,
    query_cx: CxId,
    anchor_kernel_node: CxId,
    kernel_id: CxId,
    hops: &[AnswerCompleteHopEvidence],
    total_score: f32,
    derivation_hash: [u8; 32],
) -> Result<Vec<u8>> {
    let mut value: serde_json::Value = serde_json::from_slice(&complete_answer_payload(
        query_cx,
        anchor_kernel_node,
        kernel_id,
        hops,
        total_score,
    )?)
    .map_err(|error| LodestarError::KernelProvenancePayloadCodec {
        detail: format!("decode generated complete answer payload: {error}"),
    })?;
    let object =
        value
            .as_object_mut()
            .ok_or_else(|| LodestarError::KernelProvenancePayloadCodec {
                detail: "generated complete answer payload is not a JSON object".to_string(),
            })?;
    object.insert(
        "type".to_string(),
        json!(if context.query_resolution.is_some() {
            "kernel_answer_v4"
        } else {
            "kernel_answer_v3"
        }),
    );
    object.insert("traversal_mode".to_string(), json!("association"));
    object.insert("answer_id".to_string(), json!(hex(&context.answer_id)));
    object.insert(
        "query_input_sha256".to_string(),
        json!(hex(&context.query_input_sha256)),
    );
    object.insert(
        "kernel_manifest_sha256".to_string(),
        json!(hex(&context.kernel_manifest_sha256)),
    );
    object.insert(
        "embedding_slots".to_string(),
        json!(
            context
                .embedding_slots
                .iter()
                .map(|slot| slot.get())
                .collect::<Vec<_>>()
        ),
    );
    object.insert("fusion".to_string(), json!(context.fusion));
    object.insert("rrf_k".to_string(), json!(context.rrf_k));
    object.insert("nearest_score".to_string(), json!(context.nearest_score));
    object.insert("nearest_lanes".to_string(), json!(context.nearest_lanes));
    object.insert(
        "admission_threshold".to_string(),
        json!(context.admission_threshold),
    );
    object.insert("resident_addr".to_string(), json!(context.resident_addr));
    object.insert("anchor".to_string(), json!(context.anchor));
    object.insert("max_hops".to_string(), json!(context.max_hops));
    object.insert("source_support".to_string(), json!(context.source_support));
    if let Some(query_resolution) = &context.query_resolution {
        object.insert("query_resolution".to_string(), json!(query_resolution));
    }
    object.insert("derivation_hash".to_string(), json!(hex(&derivation_hash)));
    let bytes = encode_payload(&value, "kernel complete answer")?;
    RedactionPolicy::check_payload(&bytes)?;
    Ok(bytes)
}

fn encode_payload(value: &serde_json::Value, label: &str) -> Result<Vec<u8>> {
    serde_json::to_vec(value).map_err(|error| LodestarError::KernelProvenancePayloadCodec {
        detail: format!("encode {label} payload: {error}"),
    })
}
