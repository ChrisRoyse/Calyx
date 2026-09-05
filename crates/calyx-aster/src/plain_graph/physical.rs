use std::collections::{BTreeMap, BTreeSet};

use calyx_core::{CalyxError, CxId, Result, Seq};
use calyx_paths::AssocGraph;

use crate::cf::{CfRouter, ColumnFamily, KeyRange};
use crate::mvcc::is_tombstone_value;

use super::assoc_graph::{assoc_graph_from_csr, flatten_csr_edges};
use super::csr_store;
use super::key::{GraphKeyspace, graph_corrupt, path_error};
use super::lifecycle::{
    GRAPH_GENERATION_STORAGE_COLLECTION_DETAIL, GRAPH_GENERATION_STORAGE_SCHEMA,
    GRAPH_GENERATION_STORAGE_SCHEMA_DETAIL, GraphCollectionGenerationStatus,
    PhysicalGraphCollectionLifecycle, graph_generation_storage_collection,
};
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
        let storage_collection = accepted_storage_collection(vault_dir, collection, true)?;
        Self::open_latest_unchecked(vault_dir, &storage_collection)
    }

    /// Opens only an accepted, physically isolated generation.
    ///
    /// Unlike [`Self::open_latest`], this refuses pre-isolation logical
    /// collections and accepted lifecycle rows without generation storage
    /// metadata. Production readers of reconstructible graph generations must
    /// use this entry point so an absent collection can never fall through to
    /// a loose logical keyspace.
    pub fn open_latest_isolated(
        vault_dir: impl AsRef<std::path::Path>,
        collection: &str,
    ) -> Result<Self> {
        let vault_dir = vault_dir.as_ref();
        let storage_collection = accepted_storage_collection(vault_dir, collection, false)?;
        Self::open_latest_unchecked(vault_dir, &storage_collection)
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

    /// Opens one exact generation for physical write/readback before it is
    /// published. This never consults lifecycle acceptance.
    pub fn open_generation_unchecked(
        vault_dir: impl AsRef<std::path::Path>,
        collection: &str,
        generation: &str,
    ) -> Result<Self> {
        let storage = graph_generation_storage_collection(collection, generation)?;
        Self::open_latest_unchecked(vault_dir, &storage)
    }

    pub fn storage_collection(&self) -> String {
        self.keys.collection_name()
    }

    pub fn get_node(&self, node: CxId) -> Result<Option<Vec<u8>>> {
        Ok(self
            .router
            .get(ColumnFamily::Graph, &self.keys.node_key(node))?
            .filter(|value| !is_tombstone_value(value)))
    }

    /// Reads a graph metadata row from the latest physical SST/WAL view.
    pub fn get_metadata(&self, name: &str) -> Result<Option<Vec<u8>>> {
        Ok(self
            .router
            .get(ColumnFamily::Graph, &self.keys.metadata_key(name)?)?
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
        self.count_keys_at(&self.keys.node_range())
    }

    /// Physical outgoing-edge key count, independent of any persisted CSR.
    pub fn edge_out_key_count(&self) -> Result<usize> {
        self.count_keys_at(&self.keys.edge_out_range())
    }

    pub fn read_csr(&self) -> Result<Option<PlainGraphCsr>> {
        csr_store::load_csr(&self.keys, |key| self.router.get(ColumnFamily::Graph, key))
    }

    /// One physical CSR segment pass yielding both the traversable graph and
    /// the exact verified bytes used for immutable-contract hashing.
    pub fn assoc_graph_with_csr_bytes(&self) -> Result<Option<(AssocGraph, Vec<u8>)>> {
        let Some((csr, bytes)) = csr_store::load_csr_with_bytes(&self.keys, |key| {
            self.router.get(ColumnFamily::Graph, key)
        })?
        else {
            return Ok(None);
        };
        eprintln!(
            "plain-graph: loading persisted CSR collection={} nodes={} edges={}",
            csr.collection,
            csr.nodes.len(),
            csr.edges.len()
        );
        Ok(Some((assoc_graph_from_csr(&csr)?, bytes)))
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

    /// Counts the latest live rows without retaining every key or populating
    /// the point-reader cache with every overlapping SST. Full-corpus graph
    /// verification is a sequential scan; routing it through
    /// `range_keys_until` mmap-retains each visited SST in the global reader
    /// cache and can inflate RSS by the size of the Graph CF.
    fn count_keys_at(&self, range: &KeyRange) -> Result<usize> {
        const COUNT_PAGE_ROWS: usize = 2_048;

        let mut count = 0_usize;
        self.router.range_pages_until(
            ColumnFamily::Graph,
            &range.start,
            range.end.as_deref(),
            COUNT_PAGE_ROWS,
            Vec::new(),
            |page| {
                count = count.checked_add(page.len()).ok_or_else(|| {
                    graph_corrupt("physical graph row count exceeds the platform usize range")
                })?;
                Ok(())
            },
        )?;
        Ok(count)
    }
}

/// A `writing` generation whose row has not been touched within this window, with no
/// accepted generation, is treated as a WEDGED/stale rebuild — a distinct, non-retryable
/// error class — rather than a transient in-progress one. A live materialize re-writes
/// its lifecycle row far more often than this, so a `writing` row older than the window
/// means no rebuild process is actually running.
const REBUILD_STALE_AFTER_MS: u128 = 15 * 60 * 1000;

fn now_unix_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default()
}

fn accepted_storage_collection(
    vault_dir: &std::path::Path,
    collection: &str,
    allow_legacy_logical_collection: bool,
) -> Result<String> {
    let lifecycle = PhysicalGraphCollectionLifecycle::open_latest(vault_dir)?;
    let collection_rows = lifecycle
        .list_states()?
        .into_iter()
        .filter(|row| row.state.collection == collection)
        .collect::<Vec<_>>();
    if collection_rows.is_empty() {
        if allow_legacy_logical_collection {
            // Explicit legacy schema: collections written before generation
            // isolation have no lifecycle rows and use their logical keyspace.
            return Ok(collection.to_string());
        }
        return Err(CalyxError {
            code: "CALYX_GRAPH_COLLECTION_NOT_ACCEPTED",
            message: format!(
                "graph collection {collection} has no lifecycle rows and no accepted isolated generation"
            ),
            remediation: "materialize a physically isolated generation, verify its graph and CSR bytes, and publish its accepted lifecycle row",
        });
    }
    if let Some(accepted) = collection_rows
        .iter()
        .filter(|row| row.state.status == GraphCollectionGenerationStatus::Accepted)
        .max_by(|left, right| {
            (left.state.updated_unix_ms, &left.state.generation)
                .cmp(&(right.state.updated_unix_ms, &right.state.generation))
        })
    {
        let schema = accepted
            .state
            .detail
            .get(GRAPH_GENERATION_STORAGE_SCHEMA_DETAIL);
        let storage = accepted
            .state
            .detail
            .get(GRAPH_GENERATION_STORAGE_COLLECTION_DETAIL);
        return match (schema, storage) {
            (None, None) if allow_legacy_logical_collection => Ok(collection.to_string()),
            (None, None) => Err(CalyxError {
                code: "CALYX_GRAPH_GENERATION_ISOLATION_REQUIRED",
                message: format!(
                    "accepted graph generation {}/{} has no isolated storage metadata",
                    collection, accepted.state.generation
                ),
                remediation: "rebuild into a generation-addressed Graph/XTerm keyspace, physically verify it, and publish an accepted lifecycle row with storage metadata",
            }),
            (Some(schema), Some(storage)) if schema == GRAPH_GENERATION_STORAGE_SCHEMA => {
                let expected =
                    graph_generation_storage_collection(collection, &accepted.state.generation)?;
                if storage != &expected {
                    return Err(graph_corrupt(format!(
                        "accepted graph generation {}/{} names storage collection {}, expected {}",
                        collection, accepted.state.generation, storage, expected
                    )));
                }
                Ok(storage.clone())
            }
            _ => Err(graph_corrupt(format!(
                "accepted graph generation {}/{} has incomplete or unsupported storage metadata",
                collection, accepted.state.generation
            ))),
        };
    }
    // No accepted generation exists. Make the failure HONEST: a genuinely in-progress
    // rebuild (a fresh `writing` row) is transient and retry is meaningful; a `writing`
    // row with no recent activity is WEDGED (no rebuild process is running) and retry
    // will never succeed — it needs an operator to accept/tombstone it. These MUST be
    // distinct codes so a stale flag is never mistaken for a transient one. A collection
    // with only failed/tombstoned generations (no `writing` at all) is a structural
    // not-accepted state (never a rebuild-in-flight).
    let latest_writing = collection_rows
        .iter()
        .filter(|row| row.state.status == GraphCollectionGenerationStatus::Writing)
        .max_by_key(|row| row.state.updated_unix_ms);
    if let Some(writing) = latest_writing {
        let age_ms = now_unix_ms().saturating_sub(writing.state.updated_unix_ms);
        if age_ms <= REBUILD_STALE_AFTER_MS {
            return Err(CalyxError {
                code: "CALYX_GRAPH_COLLECTION_REBUILD_IN_PROGRESS",
                message: format!(
                    "graph collection {collection} generation {gen} is mid-rebuild \
                     (writing, last touched {age_ms}ms ago); no accepted generation yet",
                    gen = writing.state.generation
                ),
                remediation: "transient: the citation overlay rebuild is in progress — retry after it completes",
            });
        }
        return Err(CalyxError {
            code: "CALYX_GRAPH_COLLECTION_REBUILD_STALE",
            message: format!(
                "graph collection {collection} generation {gen} is flagged writing but has not \
                 advanced in {age_ms}ms and no rebuild process is active (wedged/stale)",
                gen = writing.state.generation
            ),
            remediation: "NOT transient: no rebuild is running — accept or tombstone the stale generation via graph-collection-state, or re-materialize the overlay",
        });
    }
    Err(CalyxError {
        code: "CALYX_GRAPH_COLLECTION_NOT_ACCEPTED",
        message: format!(
            "graph collection {collection} has lifecycle rows but no accepted generation"
        ),
        remediation: "mark an accepted generation after physical readback or use a different graph collection",
    })
}
