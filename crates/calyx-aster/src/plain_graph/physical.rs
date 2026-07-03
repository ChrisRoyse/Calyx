use std::collections::BTreeSet;

use calyx_core::{CxId, Result};
use calyx_paths::AssocGraph;

use crate::cf::{CfRouter, ColumnFamily, KeyRange};
use crate::mvcc::is_tombstone_value;

use super::assoc_graph::assoc_graph_from_csr;
use super::csr_store;
use super::key::{GraphKeyspace, graph_corrupt, path_error};
use super::types::PlainGraphCsr;

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

    pub fn get_edge(&self, src: CxId, edge_type: &str, dst: CxId) -> Result<Option<Vec<u8>>> {
        let key = self.keys.edge_out_key(src, edge_type, dst)?;
        Ok(self
            .router
            .get(ColumnFamily::Graph, &key)?
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
