use std::collections::BTreeSet;

use calyx_core::CxId;
use calyx_mincut::tarjan_scc;
use calyx_paths::AssocGraph;
use serde::{Deserialize, Serialize};

use crate::{Kernel, KernelParams, Result, build_kernel_pipeline};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeAddEdge {
    Out { dst: CxId, weight: f32 },
    In { src: CxId, weight: f32 },
}

#[must_use]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IncrementalResult {
    Dirty { affected_sccs: BTreeSet<usize> },
    FullRebuildRequired { reason: String },
    KernelMemberRemoved { id: CxId },
    Unchanged,
}

#[derive(Clone, Debug)]
pub struct IncrementalKernelEval {
    pub kernel: Kernel,
    pub graph: AssocGraph,
    pub anchors: Vec<CxId>,
    pub dirty_sccs: BTreeSet<usize>,
    pub params: KernelParams,
    pub stale: bool,
}

impl IncrementalKernelEval {
    pub fn new(
        kernel: Kernel,
        graph: AssocGraph,
        anchors: Vec<CxId>,
        params: KernelParams,
    ) -> Self {
        Self {
            kernel,
            graph,
            anchors,
            dirty_sccs: BTreeSet::new(),
            params,
            stale: false,
        }
    }

    pub fn apply_edge_weight_change(
        &mut self,
        src: CxId,
        dst: CxId,
        new_weight: f32,
    ) -> Result<IncrementalResult> {
        self.graph.require_node_index(src)?;
        self.graph.require_node_index(dst)?;
        let mut changed = false;
        self.graph = rebuild_graph(&self.graph, |edge_src, edge_dst, weight| {
            if edge_src == src && edge_dst == dst {
                changed = true;
                Some(new_weight)
            } else {
                Some(weight)
            }
        })?;
        if !changed {
            return Ok(IncrementalResult::Unchanged);
        }
        let affected = self.components_for(&[src, dst]);
        self.dirty_sccs.extend(affected.iter().copied());
        Ok(IncrementalResult::Dirty {
            affected_sccs: affected,
        })
    }

    pub fn apply_node_add(
        &mut self,
        id: CxId,
        frequency: f32,
        edges: Vec<NodeAddEdge>,
    ) -> Result<IncrementalResult> {
        let mut builder = copy_nodes(&self.graph)?;
        builder.add_node(id, frequency)?;
        copy_edges(&self.graph, &mut builder)?;
        for edge in edges {
            match edge {
                NodeAddEdge::Out { dst, weight } => builder.add_edge(id, dst, weight)?,
                NodeAddEdge::In { src, weight } => builder.add_edge(src, id, weight)?,
            };
        }
        let candidate = builder.build();
        let scc = tarjan_scc(&candidate);
        let node_component = scc.component_of[&id];
        let component_size = scc.components[node_component].len();
        if component_size > 1 {
            self.graph = candidate;
            self.stale = true;
            return Ok(IncrementalResult::FullRebuildRequired {
                reason: "node addition merged an SCC".to_string(),
            });
        }
        self.graph = candidate;
        self.dirty_sccs.insert(node_component);
        Ok(IncrementalResult::Dirty {
            affected_sccs: BTreeSet::from([node_component]),
        })
    }

    pub fn apply_node_remove(&mut self, id: CxId) -> Result<IncrementalResult> {
        self.graph.require_node_index(id)?;
        let was_member = self.kernel.members.contains(&id);
        let mut builder = AssocGraph::builder();
        for node in self.graph.nodes() {
            if node.id != id {
                builder.add_node(node.id, node.frequency_weight)?;
            }
        }
        for edge in self.graph.edges() {
            let (src, dst) = self.graph.edge_endpoints(*edge);
            if src != id && dst != id {
                builder.add_edge(src, dst, edge.weight)?;
            }
        }
        self.graph = builder.build();
        self.stale = true;
        if was_member {
            Ok(IncrementalResult::KernelMemberRemoved { id })
        } else {
            Ok(IncrementalResult::FullRebuildRequired {
                reason: "node removal can split or reindex SCCs".to_string(),
            })
        }
    }

    pub fn rebuild_dirty(&mut self) -> Result<()> {
        if self.dirty_sccs.is_empty() && !self.stale {
            return Ok(());
        }
        self.kernel = build_kernel_pipeline(&self.graph, &self.anchors, &self.params)?;
        self.dirty_sccs.clear();
        self.stale = false;
        Ok(())
    }

    fn components_for(&self, ids: &[CxId]) -> BTreeSet<usize> {
        let scc = tarjan_scc(&self.graph);
        ids.iter()
            .filter_map(|id| scc.component_of.get(id).copied())
            .collect()
    }
}

fn rebuild_graph(
    graph: &AssocGraph,
    mut edge_weight: impl FnMut(CxId, CxId, f32) -> Option<f32>,
) -> calyx_paths::Result<AssocGraph> {
    let mut builder = copy_nodes(graph)?;
    for edge in graph.edges() {
        let (src, dst) = graph.edge_endpoints(*edge);
        if let Some(weight) = edge_weight(src, dst, edge.weight) {
            builder.add_edge(src, dst, weight)?;
        }
    }
    Ok(builder.build())
}

fn copy_nodes(graph: &AssocGraph) -> calyx_paths::Result<calyx_paths::AssocGraphBuilder> {
    let mut builder = AssocGraph::builder();
    for node in graph.nodes() {
        builder.add_node(node.id, node.frequency_weight)?;
    }
    Ok(builder)
}

fn copy_edges(
    graph: &AssocGraph,
    builder: &mut calyx_paths::AssocGraphBuilder,
) -> calyx_paths::Result<()> {
    for edge in graph.edges() {
        let (src, dst) = graph.edge_endpoints(*edge);
        builder.add_edge(src, dst, edge.weight)?;
    }
    Ok(())
}
