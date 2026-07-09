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
        let adjacency = condensed_adjacency(self);
        let mut color = vec![0_u8; self.component_nodes.len()];
        (0..self.component_nodes.len()).all(|node| !has_cycle(node, &adjacency, &mut color))
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

/// Iterative Tarjan strongly-connected-components from `start`. An explicit work
/// stack of `(node, next-out-edge index)` replaces recursion — the recursive form
/// reaches DFS depth O(V) and overflows the thread stack on the corpus graph
/// (~2×10^5 nodes, long chains). A child's final lowlink is propagated to its
/// parent when the child frame is popped, exactly as the recursive return did.
fn strong_connect(graph: &AssocGraph, start: usize, state: &mut TarjanState) {
    enter_node(start, state);
    let mut work: Vec<(usize, usize)> = vec![(start, 0)];

    while let Some(&(node, edge_idx)) = work.last() {
        let edges = graph.out_edges_by_index(node);
        if edge_idx < edges.len() {
            work.last_mut().expect("tarjan work frame").1 += 1;
            let dst = edges[edge_idx].dst;
            if state.indices[dst].is_none() {
                enter_node(dst, state);
                work.push((dst, 0));
            } else if state.on_stack[dst] {
                state.lowlinks[node] = state.lowlinks[node].min(state.indices[dst].unwrap());
            }
        } else {
            if state.lowlinks[node] == state.indices[node].unwrap() {
                pop_component(graph, node, state);
            }
            work.pop();
            if let Some(&(parent, _)) = work.last() {
                state.lowlinks[parent] = state.lowlinks[parent].min(state.lowlinks[node]);
            }
        }
    }
}

fn enter_node(node: usize, state: &mut TarjanState) {
    state.indices[node] = Some(state.next_index);
    state.lowlinks[node] = state.next_index;
    state.next_index += 1;
    state.stack.push(node);
    state.on_stack[node] = true;
}

fn pop_component(graph: &AssocGraph, root: usize, state: &mut TarjanState) {
    let mut component = Vec::new();
    loop {
        let member = state.stack.pop().expect("tarjan stack member");
        state.on_stack[member] = false;
        component.push(graph.node_id(member).expect("component node id"));
        if member == root {
            break;
        }
    }
    component.sort();
    state.components.push(component);
}

fn condensed_adjacency(graph: &CondensedGraph) -> Vec<Vec<usize>> {
    let mut adjacency = vec![Vec::new(); graph.component_nodes.len()];
    for edge in &graph.edges {
        if edge.src_component < adjacency.len() && edge.dst_component < adjacency.len() {
            adjacency[edge.src_component].push(edge.dst_component);
        }
    }
    adjacency
}

/// Iterative DFS cycle check (explicit stack of `(node, next-edge index)`). The
/// condensed graph can have O(V) vertices when the base graph is largely acyclic
/// (every node its own SCC), so the recursive form overflowed the stack at corpus
/// scale. `color`: 0 = unseen, 1 = on the current DFS path (grey), 2 = done (black).
fn has_cycle(node: usize, adjacency: &[Vec<usize>], color: &mut [u8]) -> bool {
    if color[node] != 0 {
        return color[node] == 1;
    }
    let mut work: Vec<(usize, usize)> = vec![(node, 0)];
    color[node] = 1;
    while let Some(&(current, edge_idx)) = work.last() {
        match adjacency[current].get(edge_idx).copied() {
            Some(dst) if dst < color.len() => {
                work.last_mut().expect("cycle work frame").1 += 1;
                match color[dst] {
                    1 => return true,
                    0 => {
                        color[dst] = 1;
                        work.push((dst, 0));
                    }
                    _ => {}
                }
            }
            Some(_) => {
                work.last_mut().expect("cycle work frame").1 += 1;
            }
            None => {
                color[current] = 2;
                work.pop();
            }
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn cx_n(index: usize) -> CxId {
        let mut bytes = [0_u8; 16];
        bytes[..8].copy_from_slice(&(index as u64).to_be_bytes());
        CxId::from_bytes(bytes)
    }

    #[test]
    fn deep_chain_does_not_overflow_and_is_all_singletons() {
        // A 50,000-node directed chain reaches DFS depth 50,000 — the recursive
        // Tarjan overflows the thread stack here; the iterative form must complete
        // and report every node as its own SCC.
        const N: usize = 50_000;
        let mut builder = AssocGraph::builder();
        for i in 0..N {
            builder.add_node(cx_n(i), 1.0).unwrap();
        }
        for i in 0..N - 1 {
            builder.add_edge(cx_n(i), cx_n(i + 1), 1.0).unwrap();
        }
        let graph = builder.build();
        let scc = tarjan_scc(&graph);
        assert_eq!(scc.components.len(), N);
        assert!(scc.components.iter().all(|component| component.len() == 1));
        assert_eq!(scc.component_of.len(), N);
    }

    #[test]
    fn long_cycle_is_one_component() {
        // A 10,000-node directed ring is a single SCC (also a deep DFS path).
        const N: usize = 10_000;
        let mut builder = AssocGraph::builder();
        for i in 0..N {
            builder.add_node(cx_n(i), 1.0).unwrap();
        }
        for i in 0..N {
            builder.add_edge(cx_n(i), cx_n((i + 1) % N), 1.0).unwrap();
        }
        let graph = builder.build();
        let scc = tarjan_scc(&graph);
        assert_eq!(scc.components.len(), 1);
        assert_eq!(scc.components[0].len(), N);
    }

    #[test]
    fn two_cycles_joined_by_a_bridge_split_into_two_components() {
        // a->b->a and c->d->c, with a bridge b->c: two SCCs {a,b} and {c,d}.
        let mut builder = AssocGraph::builder();
        for i in 0..4 {
            builder.add_node(cx_n(i), 1.0).unwrap();
        }
        builder.add_edge(cx_n(0), cx_n(1), 1.0).unwrap();
        builder.add_edge(cx_n(1), cx_n(0), 1.0).unwrap();
        builder.add_edge(cx_n(1), cx_n(2), 1.0).unwrap();
        builder.add_edge(cx_n(2), cx_n(3), 1.0).unwrap();
        builder.add_edge(cx_n(3), cx_n(2), 1.0).unwrap();
        let graph = builder.build();
        let scc = tarjan_scc(&graph);
        assert_eq!(scc.components.len(), 2);
        assert!(scc.components.iter().all(|component| component.len() == 2));
        // The two members of each component share a component id.
        assert_eq!(scc.component_of[&cx_n(0)], scc.component_of[&cx_n(1)]);
        assert_eq!(scc.component_of[&cx_n(2)], scc.component_of[&cx_n(3)]);
        assert_ne!(scc.component_of[&cx_n(0)], scc.component_of[&cx_n(2)]);
    }

    #[test]
    fn condensed_dag_check_handles_long_chain() {
        const N: usize = 20_000;
        let graph = CondensedGraph {
            component_nodes: (0..N).map(|index| vec![cx_n(index)]).collect(),
            edges: (0..N - 1)
                .map(|index| CondensedEdge {
                    src_component: index,
                    dst_component: index + 1,
                    weight: 1.0,
                })
                .collect(),
        };

        assert!(graph.is_dag());
    }

    #[test]
    fn condensed_dag_check_detects_component_cycle() {
        let graph = CondensedGraph {
            component_nodes: (0..3).map(|index| vec![cx_n(index)]).collect(),
            edges: vec![
                CondensedEdge {
                    src_component: 0,
                    dst_component: 1,
                    weight: 1.0,
                },
                CondensedEdge {
                    src_component: 1,
                    dst_component: 2,
                    weight: 1.0,
                },
                CondensedEdge {
                    src_component: 2,
                    dst_component: 0,
                    weight: 1.0,
                },
            ],
        };

        assert!(!graph.is_dag());
    }
}
