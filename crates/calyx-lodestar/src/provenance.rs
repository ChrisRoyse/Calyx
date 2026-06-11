//! Ledger-backed Lodestar provenance writers.

use calyx_core::{Clock, CxId, LedgerRef};
use calyx_ledger::{
    ActorId, EntryKind, LedgerAppender, LedgerCfStore, PayloadBuilder, RedactionPolicy, SubjectId,
};
use calyx_paths::AssocGraph;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{Kernel, KernelParams, Result, build_kernel_pipeline};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct KernelBuildReceipt {
    pub kernel: Kernel,
    pub ledger_ref: LedgerRef,
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
    let bytes = serde_json::to_vec(payload.value()).expect("payload serializes");
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
        .insert_str("from_id", hop.from.to_string())
        .insert_str("to_id", hop.to.to_string())
        .insert_u64("hop_index", u64::from(hop.hop_index))
        .insert_value("edge_weight", json!(hop.edge_weight))
        .insert_value("hop_score", json!(hop.hop_score));
    let bytes = serde_json::to_vec(payload.value()).expect("payload serializes");
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
    let bytes = serde_json::to_vec(payload.value()).expect("payload serializes");
    RedactionPolicy::check_payload(&bytes)?;
    Ok(bytes)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
