//! Deterministic engine specification for `calyx navigate` (issue #599).
//!
//! A nav spec is the CLI's portable, byte-stable description of a vault's
//! navigable state: per-slot dense index vectors, the constellation doc set,
//! and the association graph. `build_engine` rehydrates it into the exact
//! `SearchEngine` the calyx-sextant navigation primitives operate on, so the
//! same seeded spec that drives the engine-level ph63 FSV yields byte-identical
//! navigation results through the CLI. No silent defaults: a vector whose
//! length disagrees with its slot `dim`, a duplicate node, or an edge whose
//! endpoint is not a declared node fails closed with the upstream `CALYX_*`
//! code.

use std::collections::{BTreeMap, BTreeSet};

use calyx_core::{
    Constellation, CxFlags, CxId, InputRef, LedgerRef, Modality, SlotId, SlotVector, VaultId,
};
use calyx_paths::AssocGraphBuilder;
use calyx_sextant::{HnswIndex, SearchEngine, SextantIndex, SlotIndexMap};
use serde::Deserialize;

/// Stable synthetic vault id for CLI-synthesized constellation docs. The
/// navigation primitives key only on `cx_id`, so the vault id is a constant.
const NAV_SPEC_VAULT_ULID: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAV";

/// The full navigable state of a vault snapshot, as authored on disk.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NavSpec {
    /// One dense HNSW index per lens/slot.
    pub indexes: Vec<IndexSpec>,
    /// Association-graph edges (for `traverse`). Empty ⇒ no graph is set.
    #[serde(default)]
    pub edges: Vec<EdgeSpec>,
    /// Explicit association-graph node weights. Endpoints referenced by an
    /// edge but absent here default to weight `1.0`.
    #[serde(default)]
    pub nodes: Vec<NodeSpec>,
}

/// One dense lens index.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IndexSpec {
    /// Stable slot id (panel index).
    pub slot: SlotId,
    /// Dense vector dimensionality every entry must match.
    pub dim: u32,
    /// Deterministic HNSW construction seed.
    pub seed: u64,
    /// The constellation vectors held in this lens.
    pub entries: Vec<EntrySpec>,
}

/// One constellation's dense vector inside a lens.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EntrySpec {
    /// Constellation id (32 lowercase hex chars).
    pub cx: CxId,
    /// Dense coordinates; length must equal the owning index `dim`.
    pub vector: Vec<f32>,
    /// Stable insertion sequence (freshness/provenance ordering).
    pub seq: u64,
}

/// One association-graph edge `src -> dst` with a fusion weight.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EdgeSpec {
    pub src: CxId,
    pub dst: CxId,
    pub weight: f32,
}

/// One association-graph node frequency weight.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeSpec {
    pub cx: CxId,
    pub weight: f32,
}

/// Rehydrates the spec into a `SearchEngine`.
///
/// Order matters and is fail-closed at every step: index registration validates
/// vector shape, every constellation that appears in any lens becomes a doc so
/// `constellation_ids()` is complete, and the association graph is only attached
/// when edges/nodes are present.
pub fn build_engine(spec: &NavSpec) -> Result<SearchEngine, String> {
    let indexes = SlotIndexMap::new();
    let mut doc_ids: BTreeSet<CxId> = BTreeSet::new();
    for index_spec in &spec.indexes {
        let mut index = HnswIndex::new(index_spec.slot, index_spec.dim, index_spec.seed);
        for entry in &index_spec.entries {
            let vector = SlotVector::Dense {
                dim: index_spec.dim,
                data: entry.vector.clone(),
            };
            index
                .insert(entry.cx, vector, entry.seq)
                .map_err(|error| format!("{}: {}", error.code, error.message))?;
            doc_ids.insert(entry.cx);
        }
        indexes
            .register(index)
            .map_err(|error| format!("{}: {}", error.code, error.message))?;
    }

    let mut engine = SearchEngine::new(indexes);
    let vault_id = NAV_SPEC_VAULT_ULID
        .parse::<VaultId>()
        .map_err(|error| format!("internal nav-spec vault id is invalid: {error}"))?;
    for (seq, cx_id) in doc_ids.iter().enumerate() {
        engine.put_constellation(synthetic_doc(*cx_id, vault_id, seq as u64 + 1));
    }

    if !spec.edges.is_empty() || !spec.nodes.is_empty() {
        engine.set_assoc_graph(build_graph(spec)?);
    }
    Ok(engine)
}

/// Builds the association graph, declaring every referenced node before any edge.
fn build_graph(spec: &NavSpec) -> Result<calyx_paths::AssocGraph, String> {
    let mut weights: BTreeMap<CxId, f32> = BTreeMap::new();
    for node in &spec.nodes {
        if weights.insert(node.cx, node.weight).is_some() {
            return Err(format!(
                "CALYX_NAVIGATE_DUPLICATE_NODE: node {} declared twice",
                node.cx
            ));
        }
    }
    for edge in &spec.edges {
        weights.entry(edge.src).or_insert(1.0);
        weights.entry(edge.dst).or_insert(1.0);
    }
    let mut builder = AssocGraphBuilder::default();
    for (cx_id, weight) in &weights {
        builder
            .add_node(*cx_id, *weight)
            .map_err(|error| error.to_string())?;
    }
    for edge in &spec.edges {
        builder
            .add_edge(edge.src, edge.dst, edge.weight)
            .map_err(|error| error.to_string())?;
    }
    Ok(builder.build())
}

/// A minimal stored constellation doc keyed on `cx_id`. The navigation
/// primitives read vectors from the indexes; the doc only supplies identity and
/// provenance, so slots/scalars/metadata are intentionally empty.
fn synthetic_doc(cx_id: CxId, vault_id: VaultId, seq: u64) -> Constellation {
    // Widen the 16-byte content id into the 32-byte hash slots; the id stays
    // recoverable from the leading bytes, which is enough for identity here.
    let mut hash = [0u8; 32];
    hash[..16].copy_from_slice(cx_id.as_bytes());
    Constellation {
        cx_id,
        vault_id,
        panel_version: 1,
        created_at: seq,
        input_ref: InputRef {
            hash,
            pointer: None,
            redacted: false,
        },
        modality: Modality::Text,
        slots: BTreeMap::new(),
        scalars: BTreeMap::new(),
        metadata: BTreeMap::new(),
        anchors: Vec::new(),
        provenance: LedgerRef { seq, hash },
        flags: CxFlags::default(),
    }
}
