use std::collections::{BTreeSet, VecDeque};

use calyx_core::{AnchorKind, CxId, Ts};
use calyx_paths::AssocGraph;
use serde::{Deserialize, Serialize};

use crate::{LodestarError, Result};

const MAX_SCOPE_DEPTH: usize = 5;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CollectionId(pub String);

impl From<&str> for CollectionId {
    fn from(value: &str) -> Self {
        Self(value.to_string())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TenantId(pub String);

impl From<&str> for TenantId {
    fn from(value: &str) -> Self {
        Self(value.to_string())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "op")]
pub enum FilterExpr {
    Named { name: String },
    MetadataEq { key: String, value: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Scope {
    AllAssociations,
    Collection { id: CollectionId },
    Domain { anchor_kind: AnchorKind },
    Subgraph { query: CxId, radius: usize },
    TimeWindow { t0: Ts, t1: Ts },
    Tenant { id: TenantId },
    Filter { expr: FilterExpr },
    Union { left: Box<Scope>, right: Box<Scope> },
    Intersect { left: Box<Scope>, right: Box<Scope> },
}

pub trait AssocStore {
    fn full_graph(&self) -> Result<AssocGraph>;
    fn collection_nodes(&self, id: &CollectionId) -> Result<Option<BTreeSet<CxId>>>;
    fn domain_anchors(&self, kind: &AnchorKind) -> Result<Vec<CxId>>;
    fn time_window_nodes(&self, t0: Ts, t1: Ts) -> Result<Option<BTreeSet<CxId>>>;
    fn tenant_nodes(&self, id: &TenantId) -> Result<Option<BTreeSet<CxId>>>;
    fn filter_nodes(&self, expr: &FilterExpr) -> Result<BTreeSet<CxId>>;
}

pub fn scope_hash(scope: &Scope) -> [u8; 32] {
    let bytes = serde_json::to_vec(scope).expect("scope serializes");
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"calyx-lodestar-scope-v1");
    hasher.update(&bytes);
    *hasher.finalize().as_bytes()
}

pub fn materialize_scope(scope: &Scope, store: &dyn AssocStore) -> Result<AssocGraph> {
    materialize_scope_at(scope, store, 1)
}

fn materialize_scope_at(scope: &Scope, store: &dyn AssocStore, depth: usize) -> Result<AssocGraph> {
    if depth > MAX_SCOPE_DEPTH {
        return Err(LodestarError::ScopeDepthExceeded {
            depth,
            max: MAX_SCOPE_DEPTH,
        });
    }
    match scope {
        Scope::AllAssociations => store.full_graph(),
        Scope::Collection { id } => {
            let nodes = store
                .collection_nodes(id)?
                .ok_or_else(|| LodestarError::CollectionNotFound { id: id.0.clone() })?;
            subgraph_from_nodes(&store.full_graph()?, &nodes)
        }
        Scope::Domain { anchor_kind } => {
            let graph = store.full_graph()?;
            let nodes = reachable_from(&graph, &store.domain_anchors(anchor_kind)?);
            subgraph_from_nodes(&graph, &nodes)
        }
        Scope::Subgraph { query, radius } => {
            let graph = store.full_graph()?;
            let nodes = nodes_within_radius(&graph, *query, *radius);
            subgraph_from_nodes(&graph, &nodes)
        }
        Scope::TimeWindow { t0, t1 } => {
            if t0 > t1 {
                return Err(LodestarError::KernelInvalidParams {
                    detail: format!("time window t0={t0} must be <= t1={t1}"),
                });
            }
            let nodes = store
                .time_window_nodes(*t0, *t1)?
                .ok_or(LodestarError::ScopeTemporalNotReady)?;
            subgraph_from_nodes(&store.full_graph()?, &nodes)
        }
        Scope::Tenant { id } => {
            let nodes = store
                .tenant_nodes(id)?
                .ok_or_else(|| LodestarError::ScopeTenantNotFound { id: id.0.clone() })?;
            subgraph_from_nodes(&store.full_graph()?, &nodes)
        }
        Scope::Filter { expr } => {
            let nodes = store.filter_nodes(expr)?;
            subgraph_from_nodes(&store.full_graph()?, &nodes)
        }
        Scope::Union { left, right } => {
            let left = materialize_scope_at(left, store, depth + 1)?;
            let right = materialize_scope_at(right, store, depth + 1)?;
            union_graphs(&left, &right)
        }
        Scope::Intersect { left, right } => {
            let left = materialize_scope_at(left, store, depth + 1)?;
            let right = materialize_scope_at(right, store, depth + 1)?;
            let nodes = graph_node_set(&left)
                .intersection(&graph_node_set(&right))
                .copied()
                .collect();
            subgraph_from_nodes(&store.full_graph()?, &nodes)
        }
    }
}

fn subgraph_from_nodes(source: &AssocGraph, nodes: &BTreeSet<CxId>) -> Result<AssocGraph> {
    let mut builder = AssocGraph::builder();
    for id in nodes {
        builder.add_node(*id, source.node_weight(*id)?)?;
    }
    for edge in source.edges() {
        let (src, dst) = source.edge_endpoints(*edge);
        if nodes.contains(&src) && nodes.contains(&dst) {
            builder.add_edge(src, dst, edge.weight)?;
        }
    }
    Ok(builder.build())
}

fn union_graphs(left: &AssocGraph, right: &AssocGraph) -> Result<AssocGraph> {
    let mut builder = AssocGraph::builder();
    let mut seen = BTreeSet::new();
    add_graph_nodes(&mut builder, &mut seen, left)?;
    add_graph_nodes(&mut builder, &mut seen, right)?;
    add_graph_edges(&mut builder, left)?;
    add_graph_edges(&mut builder, right)?;
    Ok(builder.build())
}

fn add_graph_nodes(
    builder: &mut calyx_paths::AssocGraphBuilder,
    seen: &mut BTreeSet<CxId>,
    graph: &AssocGraph,
) -> Result<()> {
    for node in graph.nodes() {
        if seen.insert(node.id) {
            builder.add_node(node.id, node.frequency_weight)?;
        }
    }
    Ok(())
}

fn add_graph_edges(builder: &mut calyx_paths::AssocGraphBuilder, graph: &AssocGraph) -> Result<()> {
    for edge in graph.edges() {
        let (src, dst) = graph.edge_endpoints(*edge);
        builder.add_edge(src, dst, edge.weight)?;
    }
    Ok(())
}

fn nodes_within_radius(graph: &AssocGraph, query: CxId, radius: usize) -> BTreeSet<CxId> {
    let Some(start) = graph.node_index(query) else {
        return BTreeSet::new();
    };
    let mut nodes = BTreeSet::from([query]);
    let mut seen = BTreeSet::from([start]);
    let mut queue = VecDeque::from([(start, 0_usize)]);
    while let Some((current, hops)) = queue.pop_front() {
        if hops == radius {
            continue;
        }
        for edge in graph.out_edges_by_index(current) {
            if seen.insert(edge.dst) {
                nodes.insert(graph.node_id(edge.dst).expect("scoped node id"));
                queue.push_back((edge.dst, hops + 1));
            }
        }
    }
    nodes
}

fn reachable_from(graph: &AssocGraph, roots: &[CxId]) -> BTreeSet<CxId> {
    let mut nodes = BTreeSet::new();
    let mut seen = BTreeSet::new();
    let mut queue = VecDeque::new();
    for root in roots {
        if let Some(index) = graph.node_index(*root)
            && seen.insert(index)
        {
            nodes.insert(*root);
            queue.push_back(index);
        }
    }
    while let Some(current) = queue.pop_front() {
        for edge in graph.out_edges_by_index(current) {
            if seen.insert(edge.dst) {
                nodes.insert(graph.node_id(edge.dst).expect("domain node id"));
                queue.push_back(edge.dst);
            }
        }
    }
    nodes
}

fn graph_node_set(graph: &AssocGraph) -> BTreeSet<CxId> {
    graph.node_ids().collect()
}
