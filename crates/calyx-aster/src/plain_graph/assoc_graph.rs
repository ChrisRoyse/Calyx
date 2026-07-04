use std::collections::BTreeSet;

use calyx_core::Result;
use calyx_paths::AssocGraph;

use super::key::{graph_corrupt, path_error};
use super::types::validate_plain_graph_csr_weight;
use super::{PlainGraphCsr, PlainGraphCsrEdge};

pub(super) fn assoc_graph_from_csr(csr: &PlainGraphCsr) -> Result<AssocGraph> {
    if csr.offsets.len() != csr.nodes.len() + 1 {
        return Err(graph_corrupt(format!(
            "CSR offsets length {} does not match node count {}",
            csr.offsets.len(),
            csr.nodes.len()
        )));
    }
    if csr.offsets.first().copied() != Some(0) {
        return Err(graph_corrupt("CSR offsets must start at 0"));
    }
    if csr.offsets.last().copied() != Some(csr.edges.len()) {
        return Err(graph_corrupt(format!(
            "CSR final offset {:?} does not match edge count {}",
            csr.offsets.last(),
            csr.edges.len()
        )));
    }
    for pair in csr.offsets.windows(2) {
        if pair[0] > pair[1] {
            return Err(graph_corrupt(
                "CSR offsets must be monotonically increasing",
            ));
        }
    }
    let node_set = csr.nodes.iter().copied().collect::<BTreeSet<_>>();
    let mut builder = AssocGraph::builder();
    for node in &csr.nodes {
        builder.add_node(*node, 1.0).map_err(path_error)?;
    }
    for (src_index, window) in csr.offsets.windows(2).enumerate() {
        let src = csr
            .nodes
            .get(src_index)
            .copied()
            .ok_or_else(|| graph_corrupt("CSR source index has no node"))?;
        for edge in &csr.edges[window[0]..window[1]] {
            if !node_set.contains(&edge.dst) {
                return Err(graph_corrupt(format!(
                    "CSR edge destination {} has no node row",
                    edge.dst
                )));
            }
            let weight = validate_plain_graph_csr_weight(edge.weight)?;
            builder
                .add_edge(src, edge.dst, weight)
                .map_err(path_error)?;
        }
    }
    let graph = builder.build();
    if graph.edge_count() != csr.association_edge_count {
        return Err(graph_corrupt(format!(
            "CSR association_edge_count={} but decoded graph has {}; rebuild the graph CSR projection",
            csr.association_edge_count,
            graph.edge_count()
        )));
    }
    Ok(graph)
}

pub(super) fn flatten_csr_edges(
    mut by_src: Vec<Vec<PlainGraphCsrEdge>>,
) -> (Vec<usize>, Vec<PlainGraphCsrEdge>) {
    let mut offsets = Vec::with_capacity(by_src.len() + 1);
    let mut edges = Vec::new();
    offsets.push(0);
    for src_edges in &mut by_src {
        src_edges.sort_by(|left, right| {
            left.edge_type
                .cmp(&right.edge_type)
                .then_with(|| left.dst.cmp(&right.dst))
        });
        edges.append(src_edges);
        offsets.push(edges.len());
    }
    (offsets, edges)
}
