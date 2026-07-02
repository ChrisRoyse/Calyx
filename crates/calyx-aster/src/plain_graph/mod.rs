//! Plain graph key-encoding layer for 0-lens collections.

mod assoc_graph;
mod csr_store;
mod key;
mod types;

use std::collections::{BTreeMap, BTreeSet};

use calyx_core::{Clock, CxId, Result, Seq};
use calyx_paths::AssocGraph;

use crate::cf::{CfRouter, ColumnFamily, KeyRange};
use crate::mvcc::is_tombstone_value;
use crate::vault::AsterVault;
use assoc_graph::{assoc_graph_from_csr, flatten_csr_edges};
use key::{
    GraphKeyspace, MAX_TRAVERSE_COST, MAX_TRAVERSE_HOPS, graph_corrupt, graph_limit, graph_missing,
    path_error, validate_edge_type, validate_value,
};

pub use types::{
    CsrCommit, GraphEdgeCommit, PlainGraphCsr, PlainGraphCsrEdge, PlainGraphDirection,
    PlainGraphEdge, TraverseOptions,
};

pub struct PlainGraph<'a, C: Clock> {
    vault: &'a AsterVault<C>,
    keys: GraphKeyspace,
}

pub struct PhysicalPlainGraph {
    router: CfRouter,
    keys: GraphKeyspace,
}

impl PhysicalPlainGraph {
    pub fn open_latest(vault_dir: impl AsRef<std::path::Path>, collection: &str) -> Result<Self> {
        Ok(Self {
            router: CfRouter::open_selected_cfs(vault_dir, 0, [ColumnFamily::Graph])?,
            keys: GraphKeyspace::new(collection)?,
        })
    }

    pub fn get_node(&self, node: CxId) -> Result<Option<Vec<u8>>> {
        Ok(self
            .router
            .get(ColumnFamily::Graph, &self.keys.node_key(node))?
            .filter(|value| !is_tombstone_value(value)))
    }

    pub fn node_props(&self) -> Result<Vec<(CxId, Vec<u8>)>> {
        let range = self.keys.node_range();
        let end = range
            .end
            .as_deref()
            .ok_or_else(|| graph_corrupt("graph node range is unexpectedly unbounded"))?;
        self.router
            .range(ColumnFamily::Graph, &range.start, end)?
            .into_iter()
            .filter(|entry| !is_tombstone_value(&entry.value))
            .map(|entry| Ok((self.keys.decode_node_key(&entry.key)?, entry.value)))
            .collect()
    }

    /// Reassembled persisted CSR stream bytes, for byte-size/hash evidence in
    /// materialization readback (#996).
    pub fn read_csr_bytes(&self) -> Result<Option<Vec<u8>>> {
        csr_store::load_csr_bytes(&self.keys, |key| self.router.get(ColumnFamily::Graph, key))
    }

    /// Physical node-row key count, independent of any persisted CSR. Used to
    /// cross-check CSR materialization against the row-level source of truth.
    pub fn node_key_count(&self) -> Result<usize> {
        Ok(self.scan_keys_at(&self.keys.node_range())?.len())
    }

    /// Physical outgoing-edge key count, independent of any persisted CSR.
    pub fn edge_out_key_count(&self) -> Result<usize> {
        Ok(self.scan_keys_at(&self.keys.edge_out_range())?.len())
    }

    pub fn read_csr(&self) -> Result<Option<PlainGraphCsr>> {
        csr_store::load_csr(&self.keys, |key| self.router.get(ColumnFamily::Graph, key))
    }

    pub fn assoc_graph(&self) -> Result<AssocGraph> {
        if let Some(csr) = self.read_csr()? {
            eprintln!(
                "plain-graph: loading persisted CSR collection={} nodes={} edges={}",
                csr.collection,
                csr.nodes.len(),
                csr.edges.len()
            );
            return assoc_graph_from_csr(&csr);
        }
        eprintln!(
            "plain-graph: persisted CSR missing for collection={}, scanning graph edge rows",
            self.keys.collection_name()
        );
        let nodes = self.node_ids()?;
        let node_set = nodes.iter().copied().collect::<BTreeSet<_>>();
        let mut builder = AssocGraph::builder();
        for node in &nodes {
            builder.add_node(*node, 1.0).map_err(path_error)?;
        }
        for key in self.scan_keys_at(&self.keys.edge_out_range())? {
            let edge = self.keys.decode_edge_out_key(&key)?;
            if !node_set.contains(&edge.src) || !node_set.contains(&edge.dst) {
                return Err(graph_corrupt("graph edge endpoint has no node row"));
            }
            builder
                .add_edge(edge.src, edge.dst, 1.0)
                .map_err(path_error)?;
        }
        Ok(builder.build())
    }

    fn node_ids(&self) -> Result<Vec<CxId>> {
        self.scan_keys_at(&self.keys.node_range())?
            .into_iter()
            .map(|key| self.keys.decode_node_key(&key))
            .collect()
    }

    fn scan_keys_at(&self, range: &KeyRange) -> Result<Vec<Vec<u8>>> {
        self.router
            .range_keys_until(ColumnFamily::Graph, &range.start, range.end.as_deref())
    }
}

impl<'a, C: Clock> PlainGraph<'a, C> {
    pub fn new(vault: &'a AsterVault<C>, collection: &str) -> Result<Self> {
        Ok(Self {
            vault,
            keys: GraphKeyspace::new(collection)?,
        })
    }

    pub fn put_node(&self, node: CxId, props: &[u8]) -> Result<Seq> {
        validate_value("node props", props)?;
        self.vault
            .write_cf(ColumnFamily::Graph, self.node_key(node), props.to_vec())
    }

    pub fn put_edge(
        &self,
        src: CxId,
        edge_type: &str,
        dst: CxId,
        value: &[u8],
    ) -> Result<GraphEdgeCommit> {
        validate_edge_type(edge_type)?;
        validate_value("edge value", value)?;
        let snapshot = self.vault.latest_seq();
        self.require_node(snapshot, src)?;
        self.require_node(snapshot, dst)?;
        let edge_key = self.edge_out_key(src, edge_type, dst)?;
        let reverse_key = self.edge_in_key(dst, edge_type, src)?;
        let seq = self.vault.write_cf_batch([
            (ColumnFamily::Graph, edge_key.clone(), value.to_vec()),
            (ColumnFamily::Graph, reverse_key.clone(), edge_key.clone()),
        ])?;
        Ok(GraphEdgeCommit {
            seq,
            edge_key,
            reverse_key,
        })
    }

    pub fn get_node(&self, snapshot: Seq, node: CxId) -> Result<Option<Vec<u8>>> {
        self.vault
            .read_cf_at(snapshot, ColumnFamily::Graph, &self.node_key(node))
    }

    pub fn get_edge(
        &self,
        snapshot: Seq,
        src: CxId,
        edge_type: &str,
        dst: CxId,
    ) -> Result<Option<Vec<u8>>> {
        let key = self.edge_out_key(src, edge_type, dst)?;
        self.vault.read_cf_at(snapshot, ColumnFamily::Graph, &key)
    }

    pub fn out_neighbors(
        &self,
        snapshot: Seq,
        src: CxId,
        edge_type: Option<&str>,
        limit: usize,
    ) -> Result<Vec<PlainGraphEdge>> {
        self.neighbors(snapshot, src, edge_type, PlainGraphDirection::Out, limit)
    }

    pub fn in_neighbors(
        &self,
        snapshot: Seq,
        dst: CxId,
        edge_type: Option<&str>,
        limit: usize,
    ) -> Result<Vec<PlainGraphEdge>> {
        self.neighbors(snapshot, dst, edge_type, PlainGraphDirection::In, limit)
    }

    pub fn traverse(&self, snapshot: Seq, start: CxId, opts: TraverseOptions) -> Result<Vec<CxId>> {
        if opts.max_hops > MAX_TRAVERSE_HOPS || opts.cost_cap == 0 {
            return Err(graph_limit(format!(
                "max_hops={} cost_cap={} exceeds graph traversal bounds",
                opts.max_hops, opts.cost_cap
            )));
        }
        if let Some(edge_type) = opts.edge_type {
            validate_edge_type(edge_type)?;
        }
        self.require_node(snapshot, start)?;
        let mut visited = BTreeSet::from([start]);
        let mut reached = BTreeSet::new();
        let mut frontier = BTreeSet::from([start]);
        let mut cost = 0usize;
        let cap = opts.cost_cap.min(MAX_TRAVERSE_COST);
        for _ in 0..opts.max_hops {
            if frontier.is_empty() {
                break;
            }
            let mut next = BTreeSet::new();
            for node in &frontier {
                let neighbors =
                    self.neighbor_ids(snapshot, *node, opts.edge_type, opts.direction)?;
                for neighbor in neighbors {
                    cost += 1;
                    if cost > cap {
                        return Err(graph_limit(format!(
                            "graph traversal scanned more than {cap} edge rows"
                        )));
                    }
                    if visited.insert(neighbor) {
                        reached.insert(neighbor);
                        next.insert(neighbor);
                    }
                }
            }
            frontier = next;
        }
        Ok(reached.into_iter().collect())
    }

    pub fn csr_projection(&self, snapshot: Seq) -> Result<PlainGraphCsr> {
        let nodes = self.node_ids(snapshot)?;
        let node_index = nodes
            .iter()
            .enumerate()
            .map(|(index, id)| (*id, index))
            .collect::<BTreeMap<_, _>>();
        let mut builder = AssocGraph::builder();
        for node in &nodes {
            builder.add_node(*node, 1.0).map_err(path_error)?;
        }
        let mut by_src = vec![Vec::<PlainGraphCsrEdge>::new(); nodes.len()];
        for key in self.scan_keys_at(snapshot, &self.keys.edge_out_range())? {
            let edge = self.keys.decode_edge_out_key(&key)?;
            let src = *node_index.get(&edge.src).ok_or_else(|| {
                graph_corrupt(format!("edge source {} has no node row", edge.src))
            })?;
            if !node_index.contains_key(&edge.dst) {
                return Err(graph_corrupt(format!(
                    "edge destination {} has no node row",
                    edge.dst
                )));
            }
            builder
                .add_edge(edge.src, edge.dst, 1.0)
                .map_err(path_error)?;
            by_src[src].push(PlainGraphCsrEdge {
                dst: edge.dst,
                edge_type: edge.edge_type,
            });
        }
        let association_edge_count = builder.build().edge_count();
        let (offsets, edges) = flatten_csr_edges(by_src);
        Ok(PlainGraphCsr {
            collection: self.keys.collection_name(),
            source_snapshot: snapshot,
            nodes,
            offsets,
            edges,
            association_edge_count,
        })
    }

    /// Persist the CSR projection as ordered byte segments plus a manifest.
    /// The manifest row is written last so a reader never sees a manifest
    /// whose segments have not been written yet; the manifest's stream hash
    /// fails any torn/stale segment state closed on read.
    pub fn rebuild_csr(&self, snapshot: Seq) -> Result<CsrCommit> {
        let projection = self.csr_projection(snapshot)?;
        let (manifest_bytes, segments) = csr_store::encode_csr_segments(&self.keys, &projection)?;
        for (ordinal, segment) in segments.into_iter().enumerate() {
            self.vault.write_cf(
                ColumnFamily::Graph,
                self.keys.csr_segment_key(ordinal as u32),
                segment,
            )?;
        }
        let key = self.keys.csr_key();
        let seq = self
            .vault
            .write_cf(ColumnFamily::Graph, key.clone(), manifest_bytes)?;
        Ok(CsrCommit {
            seq,
            key,
            projection,
        })
    }

    pub fn read_csr(&self, snapshot: Seq) -> Result<Option<PlainGraphCsr>> {
        csr_store::load_csr(&self.keys, |key| {
            self.vault.read_cf_at(snapshot, ColumnFamily::Graph, key)
        })
    }

    pub fn assoc_graph(&self, snapshot: Seq) -> Result<AssocGraph> {
        if let Some(csr) = self.read_csr(snapshot)? {
            return assoc_graph_from_csr(&csr);
        }
        let nodes = self.node_ids(snapshot)?;
        let node_set = nodes.iter().copied().collect::<BTreeSet<_>>();
        let mut builder = AssocGraph::builder();
        for node in &nodes {
            builder.add_node(*node, 1.0).map_err(path_error)?;
        }
        for key in self.scan_keys_at(snapshot, &self.keys.edge_out_range())? {
            let edge = self.keys.decode_edge_out_key(&key)?;
            if !node_set.contains(&edge.src) || !node_set.contains(&edge.dst) {
                return Err(graph_corrupt("graph edge endpoint has no node row"));
            }
            builder
                .add_edge(edge.src, edge.dst, 1.0)
                .map_err(path_error)?;
        }
        Ok(builder.build())
    }

    pub fn put_metadata(&self, name: &str, value: &[u8]) -> Result<Seq> {
        validate_value("metadata value", value)?;
        self.vault.write_cf(
            ColumnFamily::Graph,
            self.keys.metadata_key(name)?,
            value.to_vec(),
        )
    }

    pub fn get_metadata(&self, snapshot: Seq, name: &str) -> Result<Option<Vec<u8>>> {
        self.vault.read_cf_at(
            snapshot,
            ColumnFamily::Graph,
            &self.keys.metadata_key(name)?,
        )
    }

    pub fn node_key(&self, node: CxId) -> Vec<u8> {
        self.keys.node_key(node)
    }

    pub fn edge_out_key(&self, src: CxId, edge_type: &str, dst: CxId) -> Result<Vec<u8>> {
        self.keys.edge_out_key(src, edge_type, dst)
    }

    pub fn edge_in_key(&self, dst: CxId, edge_type: &str, src: CxId) -> Result<Vec<u8>> {
        self.keys.edge_in_key(dst, edge_type, src)
    }

    fn neighbors(
        &self,
        snapshot: Seq,
        node: CxId,
        edge_type: Option<&str>,
        direction: PlainGraphDirection,
        limit: usize,
    ) -> Result<Vec<PlainGraphEdge>> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        if let Some(edge_type) = edge_type {
            validate_edge_type(edge_type)?;
        }
        let mut out = Vec::new();
        if matches!(
            direction,
            PlainGraphDirection::Out | PlainGraphDirection::Both
        ) {
            self.push_out_neighbors(snapshot, node, edge_type, limit, &mut out)?;
        }
        if matches!(
            direction,
            PlainGraphDirection::In | PlainGraphDirection::Both
        ) {
            self.push_in_neighbors(snapshot, node, edge_type, limit, &mut out)?;
        }
        Ok(out)
    }

    fn push_out_neighbors(
        &self,
        snapshot: Seq,
        node: CxId,
        edge_type: Option<&str>,
        limit: usize,
        out: &mut Vec<PlainGraphEdge>,
    ) -> Result<()> {
        for (key, value) in
            self.scan_at(snapshot, &self.keys.edge_prefix(true, node, edge_type)?)?
        {
            if out.len() >= limit {
                return Err(graph_limit(format!("neighbor scan exceeded limit {limit}")));
            }
            let edge = self.keys.decode_edge_out_key(&key)?;
            out.push(PlainGraphEdge {
                src: edge.src,
                dst: edge.dst,
                edge_type: edge.edge_type,
                value,
            });
        }
        Ok(())
    }

    fn push_in_neighbors(
        &self,
        snapshot: Seq,
        node: CxId,
        edge_type: Option<&str>,
        limit: usize,
        out: &mut Vec<PlainGraphEdge>,
    ) -> Result<()> {
        for (key, forward_key) in
            self.scan_at(snapshot, &self.keys.edge_prefix(false, node, edge_type)?)?
        {
            if out.len() >= limit {
                return Err(graph_limit(format!("neighbor scan exceeded limit {limit}")));
            }
            let edge = self.keys.decode_edge_in_key(&key)?;
            let value = self
                .vault
                .read_cf_at(snapshot, ColumnFamily::Graph, &forward_key)?
                .ok_or_else(|| graph_corrupt("reverse graph edge points at missing row"))?;
            out.push(PlainGraphEdge {
                src: edge.src,
                dst: edge.dst,
                edge_type: edge.edge_type,
                value,
            });
        }
        Ok(())
    }

    fn neighbor_ids(
        &self,
        snapshot: Seq,
        node: CxId,
        edge_type: Option<&str>,
        direction: PlainGraphDirection,
    ) -> Result<Vec<CxId>> {
        Ok(self
            .neighbors(snapshot, node, edge_type, direction, MAX_TRAVERSE_COST)?
            .into_iter()
            .map(|edge| if edge.src == node { edge.dst } else { edge.src })
            .collect())
    }

    fn node_ids(&self, snapshot: Seq) -> Result<Vec<CxId>> {
        self.scan_keys_at(snapshot, &self.keys.node_range())?
            .into_iter()
            .map(|key| self.keys.decode_node_key(&key))
            .collect()
    }

    fn require_node(&self, snapshot: Seq, node: CxId) -> Result<()> {
        self.get_node(snapshot, node)?.map_or_else(
            || Err(graph_missing(format!("graph node {node} is absent"))),
            |_| Ok(()),
        )
    }

    fn scan_at(&self, snapshot: Seq, range: &KeyRange) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        self.vault
            .scan_cf_range_at(snapshot, ColumnFamily::Graph, range)
    }

    fn scan_keys_at(&self, snapshot: Seq, range: &KeyRange) -> Result<Vec<Vec<u8>>> {
        self.vault
            .scan_cf_range_keys_at(snapshot, ColumnFamily::Graph, range)
    }
}

#[cfg(test)]
mod tests;
