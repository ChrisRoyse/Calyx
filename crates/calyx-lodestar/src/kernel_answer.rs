use std::collections::BTreeSet;

use calyx_core::{Clock, CxId, LedgerRef};
use calyx_ledger::{LedgerAppender, LedgerCfStore};
use calyx_paths::{AssocGraph, attenuate, reach};
use serde::{Deserialize, Serialize};

use crate::provenance::{AnswerHopEvidence, append_answer_hop_entry};
use crate::{KernelIndex, LodestarError, Result, kernel_search};

const CANDIDATE_K: usize = 10;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AnswerPath {
    pub query_cx: CxId,
    pub anchor_kernel_node: CxId,
    pub hops: Vec<AnswerHop>,
    pub total_score: f32,
    pub provenance: Vec<LedgerRef>,
}

impl AnswerPath {
    pub fn checked(
        query_cx: CxId,
        anchor_kernel_node: CxId,
        hops: Vec<AnswerHop>,
        total_score: f32,
    ) -> Result<Self> {
        validate_score(total_score, "total_score")?;
        let provenance = hops.iter().map(|hop| hop.ledger_ref.clone()).collect();
        Ok(Self {
            query_cx,
            anchor_kernel_node,
            hops,
            total_score,
            provenance,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AnswerHop {
    pub from: CxId,
    pub to: CxId,
    pub edge_weight: f32,
    pub hop_index: u32,
    pub hop_score: f32,
    pub ledger_ref: LedgerRef,
}

pub fn kernel_answer(
    kernel_index: &KernelIndex,
    graph: &AssocGraph,
    query_cx: CxId,
    query_vec: &[f32],
    anchored_kernel_nodes: &[CxId],
    max_hops: usize,
) -> Result<AnswerPath> {
    let anchor = nearest_anchored_kernel_node(kernel_index, query_vec, anchored_kernel_nodes)?;
    graph.require_node_index(anchor)?;
    if query_cx == anchor {
        return AnswerPath::checked(query_cx, anchor, Vec::new(), 1.0);
    }

    let path =
        reach(graph, anchor, query_cx, max_hops)?.ok_or(LodestarError::KernelAnswerNoPath {
            from: anchor,
            to: query_cx,
        })?;
    let hops = answer_hops_with(graph, &path, |from, to, hop_index, _, _| {
        Ok(stub_ledger_ref(from, to, hop_index))
    })?;
    let total_score = hops.iter().map(|hop| hop.hop_score).sum();
    AnswerPath::checked(query_cx, anchor, hops, total_score)
}

pub fn kernel_answer_with_ledger<S, C>(
    kernel_index: &KernelIndex,
    graph: &AssocGraph,
    query_cx: CxId,
    query_vec: &[f32],
    anchored_kernel_nodes: &[CxId],
    max_hops: usize,
    ledger: &mut LedgerAppender<S, C>,
) -> Result<AnswerPath>
where
    S: LedgerCfStore,
    C: Clock,
{
    let anchor = nearest_anchored_kernel_node(kernel_index, query_vec, anchored_kernel_nodes)?;
    graph.require_node_index(anchor)?;
    if query_cx == anchor {
        return AnswerPath::checked(query_cx, anchor, Vec::new(), 1.0);
    }

    let path =
        reach(graph, anchor, query_cx, max_hops)?.ok_or(LodestarError::KernelAnswerNoPath {
            from: anchor,
            to: query_cx,
        })?;
    let hops = answer_hops_with(
        graph,
        &path,
        |from, to, hop_index, edge_weight, hop_score| {
            append_answer_hop_entry(
                ledger,
                query_cx,
                anchor,
                AnswerHopEvidence {
                    from,
                    to,
                    edge_weight,
                    hop_index,
                    hop_score,
                },
            )
        },
    )?;
    let total_score = hops.iter().map(|hop| hop.hop_score).sum();
    AnswerPath::checked(query_cx, anchor, hops, total_score)
}

fn nearest_anchored_kernel_node(
    index: &KernelIndex,
    query_vec: &[f32],
    anchored_nodes: &[CxId],
) -> Result<CxId> {
    if anchored_nodes.is_empty() {
        return Err(LodestarError::KernelNoAnchoredNode);
    }
    let anchored: BTreeSet<_> = anchored_nodes.iter().copied().collect();
    let candidates = kernel_search(index, query_vec, CANDIDATE_K)?;
    candidates
        .into_iter()
        .map(|(cx_id, _)| cx_id)
        .find(|cx_id| anchored.contains(cx_id))
        .ok_or(LodestarError::KernelNoAnchoredNode)
}

fn answer_hops_with<F>(
    graph: &AssocGraph,
    path: &[CxId],
    mut ledger_ref: F,
) -> Result<Vec<AnswerHop>>
where
    F: FnMut(CxId, CxId, u32, f32, f32) -> Result<LedgerRef>,
{
    path.windows(2)
        .enumerate()
        .map(|(idx, pair)| {
            let from = pair[0];
            let to = pair[1];
            let edge_weight = edge_weight(graph, from, to)?;
            let hop_index = idx as u32;
            let hop_score = attenuate(edge_weight, hop_index);
            validate_score(hop_score, "hop_score")?;
            let ledger_ref = ledger_ref(from, to, hop_index, edge_weight, hop_score)?;
            Ok(AnswerHop {
                from,
                to,
                edge_weight,
                hop_index,
                hop_score,
                ledger_ref,
            })
        })
        .collect()
}

fn edge_weight(graph: &AssocGraph, from: CxId, to: CxId) -> Result<f32> {
    let from_idx = graph.require_node_index(from)?;
    let to_idx = graph.require_node_index(to)?;
    graph
        .out_edges_by_index(from_idx)
        .iter()
        .find_map(|edge| (edge.dst == to_idx).then_some(edge.weight))
        .ok_or(LodestarError::KernelAnswerNoPath { from, to })
}

fn validate_score(score: f32, field: &str) -> Result<()> {
    if score.is_finite() && score >= 0.0 {
        Ok(())
    } else {
        Err(LodestarError::KernelScoreInvalid {
            detail: format!("{field}={score} must be finite and non-negative"),
        })
    }
}

fn stub_ledger_ref(from: CxId, to: CxId, hop_index: u32) -> LedgerRef {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"ph33-kernel-answer-ledger-stub");
    hasher.update(from.as_bytes());
    hasher.update(to.as_bytes());
    hasher.update(&hop_index.to_be_bytes());
    let mut hash = [0_u8; 32];
    hash.copy_from_slice(hasher.finalize().as_bytes());
    LedgerRef {
        seq: hop_index as u64 + 1,
        hash,
    }
}
