use std::collections::{BTreeMap, BTreeSet};

use calyx_core::{CalyxError, CxId, Result, Seq};
use calyx_paths::AssocGraph;

use crate::cf::{CfRouter, ColumnFamily, KeyRange};
use crate::mvcc::is_tombstone_value;

use super::assoc_graph::{assoc_graph_from_csr, flatten_csr_edges};
use super::csr_store;
use super::key::{GraphKeyspace, graph_corrupt, path_error};
use super::lifecycle::{GraphCollectionGenerationStatus, PhysicalGraphCollectionLifecycle};
use super::types::{
    EdgeWeightPolicy, EdgeWeightSource, PlainGraphCsr, PlainGraphCsrEdge, PlainGraphEdge,
    PlainGraphEdgeWeightStats, plain_graph_edge_raw_weight,
    plain_graph_edge_raw_weight_with_policy, plain_graph_normalized_edge_weight,
};

pub struct PhysicalPlainGraph {
    router: CfRouter,
    keys: GraphKeyspace,
}

impl PhysicalPlainGraph {
    pub fn open_latest(vault_dir: impl AsRef<std::path::Path>, collection: &str) -> Result<Self> {
        let vault_dir = vault_dir.as_ref();
        ensure_collection_accepted(vault_dir, collection)?;
        Self::open_latest_unchecked(vault_dir, collection)
    }

    pub fn open_latest_unchecked(
        vault_dir: impl AsRef<std::path::Path>,
        collection: &str,
    ) -> Result<Self> {
        Ok(Self {
            router: CfRouter::open_selected_cfs(vault_dir, 0, [ColumnFamily::Graph])?,
            keys: GraphKeyspace::new(collection)?,
        })
    }

    pub fn open_latest_accepted(
        vault_dir: impl AsRef<std::path::Path>,
        collection: &str,
    ) -> Result<Self> {
        Self::open_latest(vault_dir, collection)
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

    pub fn edge_out_props(&self) -> Result<Vec<PlainGraphEdge>> {
        let range = self.keys.edge_out_range();
        let end = range
            .end
            .as_deref()
            .ok_or_else(|| graph_corrupt("graph edge-out range is unexpectedly unbounded"))?;
        self.router
            .range(ColumnFamily::Graph, &range.start, end)?
            .into_iter()
            .filter(|entry| !is_tombstone_value(&entry.value))
            .map(|entry| {
                let edge = self.keys.decode_edge_out_key(&entry.key)?;
                Ok(PlainGraphEdge {
                    src: edge.src,
                    dst: edge.dst,
                    edge_type: edge.edge_type,
                    value: entry.value,
                })
            })
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

    pub fn csr_projection_with_legacy_unit_weights(
        &self,
        source_snapshot: Seq,
    ) -> Result<(PlainGraphCsr, PlainGraphEdgeWeightStats)> {
        let nodes = self
            .node_props()?
            .into_iter()
            .map(|(id, _value)| id)
            .collect::<Vec<_>>();
        let node_index = nodes
            .iter()
            .enumerate()
            .map(|(index, id)| (*id, index))
            .collect::<BTreeMap<_, _>>();
        let mut builder = AssocGraph::builder();
        for node in &nodes {
            builder.add_node(*node, 1.0).map_err(path_error)?;
        }
        let mut stats = PlainGraphEdgeWeightStats::default();
        let mut drafts = Vec::new();
        let mut max_raw_weight = 0.0_f32;
        for edge in self.edge_out_props()? {
            let src_index = *node_index.get(&edge.src).ok_or_else(|| {
                graph_corrupt(format!("edge source {} has no node row", edge.src))
            })?;
            if !node_index.contains_key(&edge.dst) {
                return Err(graph_corrupt(format!(
                    "edge destination {} has no node row",
                    edge.dst
                )));
            }
            let parsed =
                plain_graph_edge_raw_weight_with_policy(&edge.value, EdgeWeightPolicy::LegacyUnit)?;
            match parsed.source {
                EdgeWeightSource::Explicit => stats.explicit_weight_edges += 1,
                EdgeWeightSource::LegacyUnit => stats.legacy_unit_weight_edges += 1,
            }
            max_raw_weight = max_raw_weight.max(parsed.raw);
            drafts.push((edge.src, src_index, edge.dst, edge.edge_type, parsed.raw));
        }
        let mut by_src = vec![Vec::<PlainGraphCsrEdge>::new(); nodes.len()];
        for (src, src_index, dst, edge_type, raw_weight) in drafts {
            let weight = plain_graph_normalized_edge_weight(raw_weight, max_raw_weight)?;
            builder.add_edge(src, dst, weight).map_err(path_error)?;
            by_src[src_index].push(PlainGraphCsrEdge {
                dst,
                edge_type,
                weight,
            });
        }
        let association_edge_count = builder.build().edge_count();
        let (offsets, edges) = flatten_csr_edges(by_src);
        let projection = PlainGraphCsr {
            collection: self.keys.collection_name(),
            source_snapshot,
            nodes,
            offsets,
            edges,
            association_edge_count,
        };
        Ok((projection, stats))
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
        let mut edges = Vec::new();
        let mut max_raw_weight = 0.0_f32;
        for edge in self.edge_out_props()? {
            if !node_set.contains(&edge.src) || !node_set.contains(&edge.dst) {
                return Err(graph_corrupt("graph edge endpoint has no node row"));
            }
            let raw_weight = plain_graph_edge_raw_weight(&edge.value)?;
            max_raw_weight = max_raw_weight.max(raw_weight);
            edges.push((edge.src, edge.dst, raw_weight));
        }
        for (src, dst, raw_weight) in edges {
            let weight = plain_graph_normalized_edge_weight(raw_weight, max_raw_weight)?;
            builder.add_edge(src, dst, weight).map_err(path_error)?;
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

fn ensure_collection_accepted(vault_dir: &std::path::Path, collection: &str) -> Result<()> {
    let lifecycle = PhysicalGraphCollectionLifecycle::open_latest(vault_dir)?;
    let collection_rows = lifecycle
        .list_states()?
        .into_iter()
        .filter(|row| row.state.collection == collection)
        .collect::<Vec<_>>();
    if collection_rows.is_empty()
        || collection_rows
            .iter()
            .any(|row| row.state.status == GraphCollectionGenerationStatus::Accepted)
    {
        return Ok(());
    }
    Err(CalyxError {
        code: "CALYX_GRAPH_COLLECTION_NOT_ACCEPTED",
        message: format!(
            "graph collection {collection} has lifecycle rows but no accepted generation"
        ),
        remediation: "mark an accepted generation after physical readback or use a different graph collection",
    })
}
