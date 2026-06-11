use calyx_core::{Clock, CxId, LedgerRef};
use calyx_ledger::{LedgerAppender, LedgerCfStore};
use calyx_paths::{AssocGraph, attenuate, reach};
use serde::{Deserialize, Serialize};

use crate::provenance::{
    AnswerCompleteHopEvidence, AnswerHopEvidence, append_answer_complete_entry,
    append_answer_hop_entry,
};
use crate::{KernelIndex, LodestarError, Result, kernel_search};

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
    let (anchor, path) = nearest_answerable_anchored_path(
        kernel_index,
        graph,
        query_cx,
        query_vec,
        anchored_kernel_nodes,
        max_hops,
    )?;
    if path.len() == 1 {
        return AnswerPath::checked(query_cx, anchor, Vec::new(), 1.0);
    }
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
    let (anchor, path) = nearest_answerable_anchored_path(
        kernel_index,
        graph,
        query_cx,
        query_vec,
        anchored_kernel_nodes,
        max_hops,
    )?;
    if path.len() == 1 {
        append_answer_complete_entry(ledger, query_cx, anchor, kernel_index.kernel_id, &[], 1.0)?;
        return AnswerPath::checked(query_cx, anchor, Vec::new(), 1.0);
    }
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
    let complete_hops = hops
        .iter()
        .map(|hop| AnswerCompleteHopEvidence {
            from: hop.from,
            to: hop.to,
            edge_weight: hop.edge_weight,
            hop_index: hop.hop_index,
            hop_score: hop.hop_score,
            ledger_ref: hop.ledger_ref.clone(),
        })
        .collect::<Vec<_>>();
    append_answer_complete_entry(
        ledger,
        query_cx,
        anchor,
        kernel_index.kernel_id,
        &complete_hops,
        total_score,
    )?;
    AnswerPath::checked(query_cx, anchor, hops, total_score)
}

fn nearest_answerable_anchored_path(
    index: &KernelIndex,
    graph: &AssocGraph,
    query_cx: CxId,
    query_vec: &[f32],
    anchored_nodes: &[CxId],
    max_hops: usize,
) -> Result<(CxId, Vec<CxId>)> {
    if anchored_nodes.is_empty() {
        return Err(LodestarError::KernelNoAnchoredNode);
    }
    let candidates = kernel_search(index, query_vec, index.rows().len())?;
    let mut saw_anchored_candidate = false;
    let mut first_path_error = None;
    for anchor in candidates
        .into_iter()
        .map(|(cx_id, _)| cx_id)
        .filter(|cx_id| anchored_nodes.contains(cx_id))
    {
        saw_anchored_candidate = true;
        if graph.node_index(anchor).is_none() {
            continue;
        }
        if query_cx == anchor {
            return Ok((anchor, vec![anchor]));
        }
        match reach(graph, anchor, query_cx, max_hops) {
            Ok(Some(path)) => return Ok((anchor, path)),
            Ok(None) => {
                first_path_error.get_or_insert(LodestarError::KernelAnswerNoPath {
                    from: anchor,
                    to: query_cx,
                });
            }
            Err(err) => {
                let error = LodestarError::from(err);
                if error.code() != "CALYX_PATHS_MAX_HOPS" {
                    return Err(error);
                }
                first_path_error.get_or_insert(error);
            }
        }
    }
    if !saw_anchored_candidate {
        return Err(LodestarError::KernelNoAnchoredNode);
    }
    Err(first_path_error.unwrap_or(LodestarError::KernelNoAnchoredNode))
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
