use std::collections::{BTreeMap, HashMap};
use std::ops::Range;

use calyx_core::CxId;
use serde::{Deserialize, Serialize};

use crate::{PathsError, Result};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct NodeEntry {
    pub id: CxId,
    pub frequency_weight: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Edge {
    pub src: usize,
    pub dst: usize,
    pub weight: f32,
}

#[derive(Clone, Debug)]
pub struct AssocGraph {
    nodes: Vec<NodeEntry>,
    edges: Vec<Edge>,
    adj: Vec<Range<usize>>,
    id_to_idx: HashMap<CxId, usize>,
}

#[derive(Clone, Debug, Default)]
pub struct AssocGraphBuilder {
    nodes: Vec<NodeEntry>,
    id_to_idx: HashMap<CxId, usize>,
    edges: Vec<Edge>,
}

impl AssocGraph {
    pub fn builder() -> AssocGraphBuilder {
        AssocGraphBuilder::default()
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    pub fn edge_count(&self) -> usize {
        self.edges.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    pub fn nodes(&self) -> &[NodeEntry] {
        &self.nodes
    }

    pub fn edges(&self) -> &[Edge] {
        &self.edges
    }

    pub fn node_ids(&self) -> impl Iterator<Item = CxId> + '_ {
        self.nodes.iter().map(|node| node.id)
    }

    pub fn node_index(&self, id: CxId) -> Option<usize> {
        self.id_to_idx.get(&id).copied()
    }

    pub fn require_node_index(&self, id: CxId) -> Result<usize> {
        self.node_index(id)
            .ok_or(PathsError::GraphUnknownNode { id })
    }

    pub fn node_id(&self, index: usize) -> Option<CxId> {
        self.nodes.get(index).map(|node| node.id)
    }

    pub fn edge_endpoints(&self, edge: Edge) -> (CxId, CxId) {
        (self.nodes[edge.src].id, self.nodes[edge.dst].id)
    }

    pub fn out_edges_by_index(&self, index: usize) -> &[Edge] {
        let range = self.adj[index].clone();
        &self.edges[range]
    }

    pub fn out_neighbors(&self, id: CxId) -> Result<&[Edge]> {
        let index = self.require_node_index(id)?;
        Ok(self.out_edges_by_index(index))
    }

    pub fn incoming_edges_by_index(&self, index: usize) -> impl Iterator<Item = Edge> + '_ {
        self.edges
            .iter()
            .copied()
            .filter(move |edge| edge.dst == index)
    }

    pub fn out_degree(&self, id: CxId) -> Result<usize> {
        Ok(self.out_neighbors(id)?.len())
    }

    pub fn in_degree(&self, id: CxId) -> Result<usize> {
        let index = self.require_node_index(id)?;
        Ok(self.edges.iter().filter(|edge| edge.dst == index).count())
    }

    pub fn node_weight(&self, id: CxId) -> Result<f32> {
        let index = self.require_node_index(id)?;
        Ok(self.nodes[index].frequency_weight)
    }
}

impl AssocGraphBuilder {
    pub fn add_node(&mut self, id: CxId, frequency_weight: f32) -> Result<&mut Self> {
        validate_frequency_weight(frequency_weight)?;
        if self.id_to_idx.contains_key(&id) {
            return Err(PathsError::GraphDuplicateNode { id });
        }
        let index = self.nodes.len();
        self.nodes.push(NodeEntry {
            id,
            frequency_weight,
        });
        self.id_to_idx.insert(id, index);
        Ok(self)
    }

    pub fn add_edge(&mut self, src: CxId, dst: CxId, weight: f32) -> Result<&mut Self> {
        validate_edge_weight(weight)?;
        let src = self
            .id_to_idx
            .get(&src)
            .copied()
            .ok_or(PathsError::GraphUnknownNode { id: src })?;
        let dst = self
            .id_to_idx
            .get(&dst)
            .copied()
            .ok_or(PathsError::GraphUnknownNode { id: dst })?;
        self.edges.push(Edge { src, dst, weight });
        Ok(self)
    }

    pub fn build(self) -> AssocGraph {
        let mut node_order: Vec<_> = self.nodes.iter().enumerate().collect();
        node_order.sort_by_key(|(_, node)| node.id);

        let mut old_to_new = vec![0; self.nodes.len()];
        let mut nodes = Vec::with_capacity(self.nodes.len());
        for (new_index, (old_index, node)) in node_order.into_iter().enumerate() {
            old_to_new[old_index] = new_index;
            nodes.push(*node);
        }

        let mut dedup = BTreeMap::<(usize, usize), f32>::new();
        for edge in self.edges {
            let key = (old_to_new[edge.src], old_to_new[edge.dst]);
            dedup
                .entry(key)
                .and_modify(|current| *current = current.max(edge.weight))
                .or_insert(edge.weight);
        }

        let edges: Vec<_> = dedup
            .into_iter()
            .map(|((src, dst), weight)| Edge { src, dst, weight })
            .collect();
        let adj = build_ranges(nodes.len(), &edges);
        let id_to_idx = nodes
            .iter()
            .enumerate()
            .map(|(index, node)| (node.id, index))
            .collect();

        AssocGraph {
            nodes,
            edges,
            adj,
            id_to_idx,
        }
    }
}

fn build_ranges(node_count: usize, edges: &[Edge]) -> Vec<Range<usize>> {
    let mut starts = vec![0; node_count + 1];
    for edge in edges {
        starts[edge.src + 1] += 1;
    }
    for index in 1..starts.len() {
        starts[index] += starts[index - 1];
    }
    starts
        .windows(2)
        .map(|window| window[0]..window[1])
        .collect()
}

fn validate_frequency_weight(value: f32) -> Result<()> {
    if value.is_finite() && value > 0.0 {
        Ok(())
    } else {
        Err(PathsError::GraphInvalidWeight {
            field: "frequency",
            value,
        })
    }
}

fn validate_edge_weight(value: f32) -> Result<()> {
    if value.is_finite() && (0.0..=1.0).contains(&value) {
        Ok(())
    } else {
        Err(PathsError::GraphInvalidWeight {
            field: "edge",
            value,
        })
    }
}
