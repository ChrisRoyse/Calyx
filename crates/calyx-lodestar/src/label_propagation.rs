//! Grounded label propagation by harmonic extension over the association graph.

use std::collections::{BTreeMap, VecDeque};

use calyx_core::{Clock, CxId, LedgerRef};
use calyx_ledger::{
    ActorId, EntryKind, LedgerAppender, LedgerCfStore, PayloadBuilder, RedactionPolicy, SubjectId,
};
use calyx_paths::AssocGraph;
use serde::{Deserialize, Serialize};
use serde_json::json;
use thiserror::Error;

use crate::Result;

pub type NodeId = CxId;
pub type SparseGraph = AssocGraph;
pub type PropagationResult<T> = std::result::Result<T, PropagationError>;

pub const CALYX_PROP_GRAPH_EMPTY: &str = "CALYX_PROP_GRAPH_EMPTY";
pub const CALYX_PROP_NO_KERNEL_NODES: &str = "CALYX_PROP_NO_KERNEL_NODES";
pub const CALYX_PROP_NOT_CONVERGED: &str = "CALYX_PROP_NOT_CONVERGED";
pub const CALYX_PROP_INVALID_INPUT: &str = "CALYX_PROP_INVALID_INPUT";
pub const DEFAULT_PROPAGATION_DECAY_LAMBDA: f32 = 0.5;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PropagatedLabel {
    pub node_id: NodeId,
    pub label: f32,
    pub confidence: f32,
    pub hop_distance: u32,
    pub provisional: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LabelPropagationReceipt {
    pub labels: Vec<PropagatedLabel>,
    pub ledger_ref: LedgerRef,
    pub kernel_hash: String,
    pub graph_version: u64,
}

#[derive(Clone, Debug, PartialEq, Error)]
pub enum PropagationError {
    #[error("CALYX_PROP_GRAPH_EMPTY: label propagation graph has no nodes")]
    GraphEmpty,
    #[error("CALYX_PROP_NO_KERNEL_NODES: label propagation requires at least one kernel node")]
    NoKernelNodes,
    #[error("CALYX_PROP_NOT_CONVERGED: label propagation did not converge after {iter} iterations")]
    NotConverged { iter: usize },
    #[error("CALYX_PROP_INVALID_INPUT: {detail}")]
    InvalidInput { detail: String },
}

impl PropagationError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::GraphEmpty => CALYX_PROP_GRAPH_EMPTY,
            Self::NoKernelNodes => CALYX_PROP_NO_KERNEL_NODES,
            Self::NotConverged { .. } => CALYX_PROP_NOT_CONVERGED,
            Self::InvalidInput { .. } => CALYX_PROP_INVALID_INPUT,
        }
    }
}

pub fn propagate_labels(
    graph: &SparseGraph,
    kernel_labels: &[(NodeId, f32)],
    max_iter: usize,
    tol: f32,
) -> PropagationResult<Vec<PropagatedLabel>> {
    propagate_labels_with_decay(
        graph,
        kernel_labels,
        max_iter,
        tol,
        DEFAULT_PROPAGATION_DECAY_LAMBDA,
    )
}

pub fn propagate_labels_with_decay(
    graph: &SparseGraph,
    kernel_labels: &[(NodeId, f32)],
    max_iter: usize,
    tol: f32,
    decay_lambda: f32,
) -> PropagationResult<Vec<PropagatedLabel>> {
    validate_inputs(graph, kernel_labels, max_iter, tol, decay_lambda)?;
    let kernel = kernel_map(graph, kernel_labels)?;
    let neighbors = sym_neighbors(graph);
    let hops = hop_distances(graph, &neighbors, &kernel);
    let mut values = initial_values(graph, &kernel);
    let mut converged = false;

    for iter in 1..=max_iter {
        let mut next = values.clone();
        let mut max_delta = 0.0_f32;
        for index in 0..graph.node_count() {
            if kernel.contains_key(&index) {
                continue;
            }
            if neighbors[index].is_empty() {
                next[index] = 0.0;
                continue;
            }
            let (weighted_sum, degree) = neighbors[index].iter().fold(
                (0.0_f32, 0.0_f32),
                |(sum, degree), (neighbor, weight)| {
                    (sum + values[*neighbor] * *weight, degree + *weight)
                },
            );
            next[index] = if degree > 0.0 {
                weighted_sum / degree
            } else {
                0.0
            };
            max_delta = max_delta.max((next[index] - values[index]).abs());
        }
        values = next;
        if max_delta <= tol {
            converged = true;
            break;
        }
        if iter == max_iter {
            return Err(PropagationError::NotConverged { iter });
        }
    }
    if !converged && max_iter > 0 {
        return Err(PropagationError::NotConverged { iter: max_iter });
    }
    Ok(label_rows(graph, &kernel, &hops, &values, decay_lambda))
}

pub fn propagate_labels_with_ledger<S, C>(
    graph: &SparseGraph,
    kernel_labels: &[(NodeId, f32)],
    max_iter: usize,
    tol: f32,
    graph_version: u64,
    ledger: &mut LedgerAppender<S, C>,
) -> Result<LabelPropagationReceipt>
where
    S: LedgerCfStore,
    C: Clock,
{
    let labels = propagate_labels(graph, kernel_labels, max_iter, tol).map_err(|error| {
        crate::LodestarError::Graph {
            code: error.code(),
            message: error.to_string(),
        }
    })?;
    let kernel_hash = hex(&kernel_labels_hash(kernel_labels));
    let ledger_ref =
        append_label_propagation_entry(ledger, graph, &labels, graph_version, kernel_hash.clone())?;
    Ok(LabelPropagationReceipt {
        labels,
        ledger_ref,
        kernel_hash,
        graph_version,
    })
}

pub fn append_label_propagation_entry<S, C>(
    ledger: &mut LedgerAppender<S, C>,
    graph: &SparseGraph,
    labels: &[PropagatedLabel],
    graph_version: u64,
    kernel_hash: String,
) -> Result<LedgerRef>
where
    S: LedgerCfStore,
    C: Clock,
{
    let subject = SubjectId::Kernel(kernel_hash.as_bytes().to_vec());
    ledger
        .append(
            EntryKind::Kernel,
            subject,
            propagation_payload(graph, labels, graph_version, &kernel_hash)?,
            ActorId::Service("calyx-lodestar".to_string()),
        )
        .map_err(Into::into)
}

pub fn kernel_labels_hash(kernel_labels: &[(NodeId, f32)]) -> [u8; 32] {
    let mut sorted = kernel_labels.to_vec();
    sorted.sort_by_key(|(node_id, _)| *node_id);
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"calyx-lodestar-label-propagation-kernel-v1");
    for (node_id, label) in sorted {
        hasher.update(node_id.as_bytes());
        hasher.update(&label.to_bits().to_be_bytes());
    }
    *hasher.finalize().as_bytes()
}

fn validate_inputs(
    graph: &SparseGraph,
    kernel_labels: &[(NodeId, f32)],
    max_iter: usize,
    tol: f32,
    decay_lambda: f32,
) -> PropagationResult<()> {
    if graph.is_empty() {
        return Err(PropagationError::GraphEmpty);
    }
    if kernel_labels.is_empty() {
        return Err(PropagationError::NoKernelNodes);
    }
    if max_iter == 0 {
        return Err(PropagationError::NotConverged { iter: 0 });
    }
    if !tol.is_finite() || tol <= 0.0 {
        return invalid_input(format!("tol must be finite and positive, got {tol}"));
    }
    if !decay_lambda.is_finite() || decay_lambda < 0.0 {
        return invalid_input(format!(
            "decay lambda must be finite and non-negative, got {decay_lambda}"
        ));
    }
    Ok(())
}

fn kernel_map(
    graph: &SparseGraph,
    kernel_labels: &[(NodeId, f32)],
) -> PropagationResult<BTreeMap<usize, f32>> {
    let mut kernel = BTreeMap::new();
    for (node_id, label) in kernel_labels {
        if !label.is_finite() || !(0.0..=1.0).contains(label) {
            return invalid_input(format!(
                "kernel label must be finite in [0, 1], got {label}"
            ));
        }
        let index = graph
            .node_index(*node_id)
            .ok_or_else(|| PropagationError::InvalidInput {
                detail: format!("kernel node {node_id} is absent from graph"),
            })?;
        if kernel.insert(index, *label).is_some() {
            return invalid_input(format!("kernel node {node_id} is duplicated"));
        }
    }
    Ok(kernel)
}

fn sym_neighbors(graph: &SparseGraph) -> Vec<Vec<(usize, f32)>> {
    let mut by_pair = BTreeMap::<(usize, usize), f32>::new();
    for edge in graph.edges() {
        if edge.src == edge.dst {
            continue;
        }
        let (left, right) = if edge.src < edge.dst {
            (edge.src, edge.dst)
        } else {
            (edge.dst, edge.src)
        };
        by_pair
            .entry((left, right))
            .and_modify(|weight| *weight = weight.max(edge.weight))
            .or_insert(edge.weight);
    }
    let mut neighbors = vec![Vec::new(); graph.node_count()];
    for ((left, right), weight) in by_pair {
        neighbors[left].push((right, weight));
        neighbors[right].push((left, weight));
    }
    neighbors
}

fn hop_distances(
    graph: &SparseGraph,
    neighbors: &[Vec<(usize, f32)>],
    kernel: &BTreeMap<usize, f32>,
) -> Vec<u32> {
    let mut hops = vec![u32::MAX; graph.node_count()];
    let mut queue = VecDeque::new();
    for index in kernel.keys().copied() {
        hops[index] = 0;
        queue.push_back(index);
    }
    while let Some(index) = queue.pop_front() {
        for (neighbor, _) in &neighbors[index] {
            if hops[*neighbor] == u32::MAX {
                hops[*neighbor] = hops[index].saturating_add(1);
                queue.push_back(*neighbor);
            }
        }
    }
    hops
}

fn initial_values(graph: &SparseGraph, kernel: &BTreeMap<usize, f32>) -> Vec<f32> {
    let default = kernel.values().sum::<f32>() / kernel.len() as f32;
    (0..graph.node_count())
        .map(|index| kernel.get(&index).copied().unwrap_or(default))
        .collect()
}

fn label_rows(
    graph: &SparseGraph,
    kernel: &BTreeMap<usize, f32>,
    hops: &[u32],
    values: &[f32],
    decay_lambda: f32,
) -> Vec<PropagatedLabel> {
    (0..graph.node_count())
        .map(|index| {
            let kernel_label = kernel.get(&index).copied();
            let hop = hops[index];
            let confidence = if let Some(label) = kernel_label {
                label
            } else if hop == u32::MAX {
                0.0
            } else {
                values[index].clamp(0.0, 1.0) * (-decay_lambda * hop as f32).exp()
            };
            PropagatedLabel {
                node_id: graph.node_id(index).expect("graph node id"),
                label: values[index].clamp(0.0, 1.0),
                confidence: confidence.clamp(0.0, 1.0),
                hop_distance: hop,
                provisional: kernel_label.is_none(),
            }
        })
        .collect()
}

fn propagation_payload(
    graph: &SparseGraph,
    labels: &[PropagatedLabel],
    graph_version: u64,
    kernel_hash: &str,
) -> Result<Vec<u8>> {
    let mut payload = PayloadBuilder::default();
    payload
        .insert_str("propagation_id", kernel_hash)
        .insert_str("kernel_hash", kernel_hash)
        .insert_u64("graph_version", graph_version)
        .insert_u64("node_count", graph.node_count() as u64)
        .insert_u64(
            "n_propagated",
            labels.iter().filter(|row| row.provisional).count() as u64,
        )
        .insert_value(
            "max_hop_distance",
            json!(
                labels
                    .iter()
                    .filter(|row| row.hop_distance != u32::MAX)
                    .map(|row| row.hop_distance)
                    .max()
                    .unwrap_or(0)
            ),
        );
    let bytes = serde_json::to_vec(payload.value()).expect("payload serializes");
    RedactionPolicy::check_payload(&bytes)?;
    Ok(bytes)
}

fn invalid_input<T>(detail: String) -> PropagationResult<T> {
    Err(PropagationError::InvalidInput { detail })
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
