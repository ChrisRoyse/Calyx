use std::collections::{BTreeMap, BTreeSet};

use calyx_core::CxId;
use calyx_paths::AssocGraph;
use serde::{Deserialize, Serialize};

use crate::{MincutError, Result};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SccResult {
    pub components: Vec<Vec<CxId>>,
    pub component_of: BTreeMap<CxId, usize>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CondensedEdge {
    pub src_component: usize,
    pub dst_component: usize,
    pub weight: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CondensedGraph {
    pub component_nodes: Vec<Vec<CxId>>,
    pub edges: Vec<CondensedEdge>,
}

impl CondensedGraph {
    pub fn is_dag(&self) -> bool {
        let mut color = vec![0_u8; self.component_nodes.len()];
        (0..self.component_nodes.len()).all(|node| !has_cycle(node, self, &mut color))
    }
}

pub fn tarjan_scc(graph: &AssocGraph) -> SccResult {
    let mut state = TarjanState::new(graph.node_count());
    for node in 0..graph.node_count() {
        if state.indices[node].is_none() {
            strong_connect(graph, node, &mut state);
        }
    }
    let component_of = state
        .components
        .iter()
        .enumerate()
        .flat_map(|(component, nodes)| nodes.iter().map(move |node| (*node, component)))
        .collect();
    SccResult {
        components: state.components,
        component_of,
    }
}

pub fn condensate(graph: &AssocGraph, scc: &SccResult) -> Result<CondensedGraph> {
    validate_scc(graph, scc)?;
    let mut edge_weights = BTreeMap::<(usize, usize), f32>::new();
    for edge in graph.edges() {
        let src = graph.node_id(edge.src).expect("edge src id");
        let dst = graph.node_id(edge.dst).expect("edge dst id");
        let src_component = scc.component_of[&src];
        let dst_component = scc.component_of[&dst];
        if src_component == dst_component {
            continue;
        }
        edge_weights
            .entry((src_component, dst_component))
            .and_modify(|current| *current = current.max(edge.weight))
            .or_insert(edge.weight);
    }
    let edges = edge_weights
        .into_iter()
        .map(|((src_component, dst_component), weight)| CondensedEdge {
            src_component,
            dst_component,
            weight,
        })
        .collect();
    Ok(CondensedGraph {
        component_nodes: scc.components.clone(),
        edges,
    })
}

fn validate_scc(graph: &AssocGraph, scc: &SccResult) -> Result<()> {
    if scc.component_of.len() != graph.node_count() {
        return Err(MincutError::SccGraphMismatch {
            detail: format!(
                "component map has {} nodes for graph with {}",
                scc.component_of.len(),
                graph.node_count()
            ),
        });
    }
    let graph_nodes: BTreeSet<_> = graph.node_ids().collect();
    let scc_nodes: BTreeSet<_> = scc.component_of.keys().copied().collect();
    if graph_nodes != scc_nodes {
        return Err(MincutError::SccGraphMismatch {
            detail: "SCC node set differs from graph node set".to_string(),
        });
    }
    Ok(())
}

fn strong_connect(graph: &AssocGraph, node: usize, state: &mut TarjanState) {
    state.indices[node] = Some(state.next_index);
    state.lowlinks[node] = state.next_index;
    state.next_index += 1;
    state.stack.push(node);
    state.on_stack[node] = true;

    for edge in graph.out_edges_by_index(node) {
        if state.indices[edge.dst].is_none() {
            strong_connect(graph, edge.dst, state);
            state.lowlinks[node] = state.lowlinks[node].min(state.lowlinks[edge.dst]);
        } else if state.on_stack[edge.dst] {
            state.lowlinks[node] = state.lowlinks[node].min(state.indices[edge.dst].unwrap());
        }
    }

    if state.lowlinks[node] == state.indices[node].unwrap() {
        let mut component = Vec::new();
        loop {
            let member = state.stack.pop().expect("tarjan stack member");
            state.on_stack[member] = false;
            component.push(graph.node_id(member).expect("component node id"));
            if member == node {
                break;
            }
        }
        component.sort();
        state.components.push(component);
    }
}

fn has_cycle(node: usize, graph: &CondensedGraph, color: &mut [u8]) -> bool {
    if color[node] == 1 {
        return true;
    }
    if color[node] == 2 {
        return false;
    }
    color[node] = 1;
    for edge in graph.edges.iter().filter(|edge| edge.src_component == node) {
        if has_cycle(edge.dst_component, graph, color) {
            return true;
        }
    }
    color[node] = 2;
    false
}

#[derive(Clone, Debug)]
struct TarjanState {
    next_index: usize,
    indices: Vec<Option<usize>>,
    lowlinks: Vec<usize>,
    stack: Vec<usize>,
    on_stack: Vec<bool>,
    components: Vec<Vec<CxId>>,
}

impl TarjanState {
    fn new(node_count: usize) -> Self {
        Self {
            next_index: 0,
            indices: vec![None; node_count],
            lowlinks: vec![0; node_count],
            stack: Vec::new(),
            on_stack: vec![false; node_count],
            components: Vec::new(),
        }
    }
}
