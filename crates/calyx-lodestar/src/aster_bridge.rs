use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

use calyx_aster::plain_graph::PlainGraph;
use calyx_aster::timetravel::TimeTravelSnapshot;
use calyx_aster::vault::AsterVault;
use calyx_core::{AnchorKind, CalyxError, Clock, CxId, LedgerRef, Seq, SlotId, SlotVector, Ts};
use calyx_ledger::{ActorId, EntryKind, SubjectId};
use calyx_paths::AssocGraph;
use serde::{Deserialize, Serialize};

use crate::recall_test::RecallTestParams;
use crate::scope::{AssocStore, CollectionId, FilterExpr, Scope, TenantId};
use crate::summarize::{
    CALYX_TIMETRAVEL_BEFORE_HORIZON, SUMMARIZE_INVOKED_MARKER, SummarizeParams, SummarizeResult,
    summarize_with_ledger,
};
use crate::{LodestarError, Result, ScopeCache};

pub const DEFAULT_ASTER_ASSOC_COLLECTION: &str = "default";
pub const PANEL_ASTER_ASSOC_COLLECTION: &str = "kernel-panel-v3";
pub const ASTER_ASSOC_METADATA_KEY: &str = "lodestar_assoc_v1";

mod physical;
mod recall;
pub use recall::AsterRecallInputs;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AsterAssocMetadata {
    pub retention_horizon: Option<Ts>,
    /// Legacy single-lane graph marker. New sealed graphs must leave this empty
    /// and populate `embedding_slots` instead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embedding_slot: Option<SlotId>,
    /// Ordered dense content lanes used by every graph edge and query gate.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub embedding_slots: Vec<SlotId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fusion: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rrf_k: Option<u32>,
    /// Frozen panel version used when the graph embeddings were measured.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub panel_version: Option<u64>,
    /// Last vault sequence incorporated before the graph contract was sealed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graph_source_seq: Option<Seq>,
    /// k used for the persisted between-document nearest-neighbour graph.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub knn: Option<usize>,
    /// Corpus-native admission boundary used to refuse unsupported queries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edge_cos_threshold: Option<f32>,
    /// Fused graph score boundary for panel-native graphs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edge_score_threshold: Option<f32>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AsterAssocNodeProps {
    /// Legacy single-lane embedding. New panel-native graphs leave this empty.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embedding: Option<Vec<f32>>,
    /// Complete no-flatten panel measurement keyed by stable slot id.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub embeddings: BTreeMap<SlotId, Vec<f32>>,
    /// Complete no-flatten typed panel. `fusion=typed_block_rrf` requires this
    /// map and forbids both legacy vector fields.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub typed_embeddings: BTreeMap<SlotId, SlotVector>,
    pub ts: Option<Ts>,
    #[serde(default)]
    pub anchors: Vec<AnchorKind>,
    pub tenant: Option<TenantId>,
    #[serde(default)]
    pub named_filters: Vec<String>,
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
}

pub struct AsterAssocSnapshot<'a, C: Clock> {
    vault: &'a AsterVault<C>,
    collection: String,
    snapshot: Seq,
    metadata: AsterAssocMetadata,
    graph_cache: OnceLock<AssocGraph>,
    _lease: Option<TimeTravelSnapshot<'a, C>>,
}

pub struct PhysicalAsterAssocSnapshot {
    graph: AssocGraph,
    props: BTreeMap<CxId, AsterAssocNodeProps>,
    metadata: AsterAssocMetadata,
    collection: String,
}

#[derive(Clone, Debug)]
pub struct AsterSummarizeRequest<'a> {
    pub collection: &'a str,
    pub scope: Scope,
    pub params: Option<SummarizeParams>,
    pub recall_params: RecallTestParams,
}

impl<'a, C: Clock> AsterAssocSnapshot<'a, C> {
    pub fn latest(vault: &'a AsterVault<C>, collection: impl Into<String>) -> Result<Self> {
        Self::from_seq(vault, collection.into(), vault.latest_seq(), None)
    }

    pub fn as_of(
        vault: &'a AsterVault<C>,
        collection: impl Into<String>,
        t_millis: Ts,
    ) -> std::result::Result<Self, CalyxError> {
        let collection = collection.into();
        let latest = Self::latest(vault, collection.clone()).map_err(to_calyx)?;
        if let Some(horizon) = latest.retention_horizon()
            && t_millis < horizon
        {
            return Err(before_horizon(t_millis, horizon));
        }
        let lease = vault.as_of(t_millis)?;
        let snapshot = lease.seqno();
        Self::from_seq(vault, collection, snapshot, Some(lease)).map_err(to_calyx)
    }

    pub fn retention_horizon(&self) -> Option<Ts> {
        self.metadata.retention_horizon
    }

    pub fn snapshot_seq(&self) -> Seq {
        self.snapshot
    }

    pub fn recall_inputs(
        &self,
        params: RecallTestParams,
    ) -> std::result::Result<Option<AsterRecallInputs>, CalyxError> {
        let graph = self.full_graph().map_err(to_calyx)?;
        if graph.is_empty() {
            return Ok(None);
        }
        // Open the graph handle exactly once and read every node's props from it
        // rather than reopening `PlainGraph` per node — the panel corpus already
        // materializes every constellation's full multi-lane panel, so we must not
        // amplify the vault-open surface on top of that.
        let handle = self.graph().map_err(to_calyx)?;
        let mut nodes = Vec::new();
        for cx_id in graph.node_ids() {
            nodes.push((
                cx_id,
                self.read_node_props(&handle, cx_id).map_err(to_calyx)?,
            ));
        }
        recall::build_recall_inputs(
            &self.metadata,
            nodes,
            format!("aster:{}@{}", self.collection, self.snapshot),
            params,
        )
    }

    /// Panel-native ledger provenance contract, if this snapshot is panel-native.
    /// Returns the sealed lane ids, fusion, and rrf_k so the `SUMMARIZE_INVOKED`
    /// payload can carry byte-auditable per-lane provenance.
    fn ledger_panel_contract(&self) -> Option<PanelLedgerContract> {
        if self.metadata.embedding_slots.len() >= 2 && self.metadata.embedding_slot.is_none() {
            Some(PanelLedgerContract {
                embedding_slots: self
                    .metadata
                    .embedding_slots
                    .iter()
                    .map(|slot| slot.get())
                    .collect(),
                fusion: self.metadata.fusion.clone().unwrap_or_default(),
                rrf_k: self.metadata.rrf_k.unwrap_or(0),
            })
        } else {
            None
        }
    }

    fn from_seq(
        vault: &'a AsterVault<C>,
        collection: String,
        snapshot: Seq,
        lease: Option<TimeTravelSnapshot<'a, C>>,
    ) -> Result<Self> {
        let graph = PlainGraph::new(vault, &collection)?;
        let metadata = graph
            .get_metadata(snapshot, ASTER_ASSOC_METADATA_KEY)?
            .map(|bytes| {
                serde_json::from_slice(&bytes).map_err(|error| LodestarError::KernelIndexCodec {
                    detail: format!("decode Aster assoc metadata: {error}"),
                })
            })
            .transpose()?
            .unwrap_or_default();
        Ok(Self {
            vault,
            collection,
            snapshot,
            metadata,
            graph_cache: OnceLock::new(),
            _lease: lease,
        })
    }

    fn graph(&self) -> Result<PlainGraph<'_, C>> {
        PlainGraph::new(self.vault, &self.collection).map_err(LodestarError::from)
    }

    fn cached_full_graph(&self) -> Result<AssocGraph> {
        if let Some(graph) = self.graph_cache.get() {
            return Ok(graph.clone());
        }
        let graph = self
            .graph()?
            .assoc_graph(self.snapshot)
            .map_err(LodestarError::from)?;
        let _ = self.graph_cache.set(graph);
        self.graph_cache
            .get()
            .cloned()
            .ok_or_else(|| LodestarError::KernelIndexCodec {
                detail: "cache Aster assoc graph snapshot".to_string(),
            })
    }

    fn node_props(&self, cx_id: CxId) -> Result<AsterAssocNodeProps> {
        let graph = self.graph()?;
        self.read_node_props(&graph, cx_id)
    }

    /// Decodes one node's props from an already-open graph handle, so callers
    /// that touch every node (recall corpus construction) open `PlainGraph` once.
    fn read_node_props(
        &self,
        graph: &PlainGraph<'_, C>,
        cx_id: CxId,
    ) -> Result<AsterAssocNodeProps> {
        let Some(bytes) = graph.get_node(self.snapshot, cx_id)? else {
            return Err(LodestarError::KernelInvalidParams {
                detail: format!("Aster graph node {cx_id} disappeared at snapshot"),
            });
        };
        serde_json::from_slice(&bytes).map_err(|error| LodestarError::KernelIndexCodec {
            detail: format!("decode Aster graph node props for {cx_id}: {error}"),
        })
    }
}

impl<C: Clock> AssocStore for AsterAssocSnapshot<'_, C> {
    fn full_graph(&self) -> Result<AssocGraph> {
        self.cached_full_graph()
    }

    fn collection_nodes(&self, id: &CollectionId) -> Result<Option<BTreeSet<CxId>>> {
        if id.0 != self.collection {
            return Ok(None);
        }
        Ok(Some(self.full_graph()?.node_ids().collect()))
    }

    fn domain_anchors(&self, kind: &AnchorKind) -> Result<Vec<CxId>> {
        let mut anchors = Vec::new();
        for id in self.full_graph()?.node_ids() {
            if self
                .node_props(id)?
                .anchors
                .iter()
                .any(|stored| stored == kind)
            {
                anchors.push(id);
            }
        }
        Ok(anchors)
    }

    fn time_window_nodes(&self, t0: Ts, t1: Ts) -> Result<Option<BTreeSet<CxId>>> {
        let mut saw_ts = false;
        let mut ids = BTreeSet::new();
        for id in self.full_graph()?.node_ids() {
            if let Some(ts) = self.node_props(id)?.ts {
                saw_ts = true;
                if (t0..=t1).contains(&ts) {
                    ids.insert(id);
                }
            }
        }
        Ok(saw_ts.then_some(ids))
    }

    fn tenant_nodes(&self, id: &TenantId) -> Result<Option<BTreeSet<CxId>>> {
        let mut found = BTreeSet::new();
        for node in self.full_graph()?.node_ids() {
            if self.node_props(node)?.tenant.as_ref() == Some(id) {
                found.insert(node);
            }
        }
        Ok((!found.is_empty()).then_some(found))
    }

    fn filter_nodes(&self, expr: &FilterExpr) -> Result<BTreeSet<CxId>> {
        let mut found = BTreeSet::new();
        for node in self.full_graph()?.node_ids() {
            let props = self.node_props(node)?;
            let matches = match expr {
                FilterExpr::Named { name } => {
                    props.named_filters.iter().any(|stored| stored == name)
                }
                FilterExpr::MetadataEq { key, value } => props.metadata.get(key) == Some(value),
            };
            if matches {
                found.insert(node);
            }
        }
        Ok(found)
    }

    fn node_metadata(&self, id: CxId) -> Result<Option<BTreeMap<String, String>>> {
        Ok(Some(self.node_props(id)?.metadata))
    }
}

/// Byte-auditable panel provenance recorded in the `SUMMARIZE_INVOKED` payload.
struct PanelLedgerContract {
    embedding_slots: Vec<u16>,
    fusion: String,
    rrf_k: u32,
}

pub fn write_assoc_metadata<C: Clock>(
    vault: &AsterVault<C>,
    collection: &str,
    metadata: &AsterAssocMetadata,
) -> std::result::Result<Seq, CalyxError> {
    let graph = PlainGraph::new(vault, collection)?;
    let bytes = serde_json::to_vec(metadata).map_err(|error| {
        bridge_error(
            "CALYX_ASSOC_BRIDGE_CODEC",
            format!("encode Aster assoc metadata: {error}"),
        )
    })?;
    graph.put_metadata(ASTER_ASSOC_METADATA_KEY, &bytes)
}

pub fn encode_assoc_node_props(
    props: &AsterAssocNodeProps,
) -> std::result::Result<Vec<u8>, CalyxError> {
    serde_json::to_vec(props).map_err(|error| {
        bridge_error(
            "CALYX_ASSOC_BRIDGE_CODEC",
            format!("encode Aster assoc node props: {error}"),
        )
    })
}

pub fn summarize_vault_latest<C: Clock>(
    vault: &AsterVault<C>,
    request: AsterSummarizeRequest<'_>,
    cache: &mut ScopeCache,
    clock: &dyn Clock,
) -> std::result::Result<SummarizeResult, CalyxError> {
    let snapshot = AsterAssocSnapshot::latest(vault, request.collection).map_err(to_calyx)?;
    summarize_snapshot(
        vault,
        &snapshot,
        request.scope,
        request.params,
        request.recall_params,
        cache,
        clock,
    )
}

pub fn summarize_vault_as_of<C: Clock>(
    vault: &AsterVault<C>,
    request: AsterSummarizeRequest<'_>,
    t_millis: Ts,
    cache: &mut ScopeCache,
    clock: &dyn Clock,
) -> std::result::Result<SummarizeResult, CalyxError> {
    let snapshot = AsterAssocSnapshot::as_of(vault, request.collection, t_millis)?;
    summarize_snapshot(
        vault,
        &snapshot,
        request.scope,
        request.params,
        request.recall_params,
        cache,
        clock,
    )
}

fn summarize_snapshot<C: Clock>(
    vault: &AsterVault<C>,
    snapshot: &AsterAssocSnapshot<'_, C>,
    scope: Scope,
    params: Option<SummarizeParams>,
    recall_params: RecallTestParams,
    cache: &mut ScopeCache,
    clock: &dyn Clock,
) -> std::result::Result<SummarizeResult, CalyxError> {
    let recall_inputs = snapshot.recall_inputs(recall_params)?;
    let recall = recall_inputs.as_ref().map(AsterRecallInputs::measurement);
    let panel = snapshot.ledger_panel_contract();
    summarize_with_ledger(
        snapshot,
        scope,
        params,
        recall,
        cache,
        clock,
        |scope_h, kernel_size, kernel_only_recall, grounded_fraction| {
            append_invoked_to_vault(
                vault,
                scope_h,
                kernel_size,
                kernel_only_recall,
                grounded_fraction,
                panel.as_ref(),
            )
        },
    )
}

fn append_invoked_to_vault<C: Clock>(
    vault: &AsterVault<C>,
    scope_h: &[u8; 32],
    kernel_size: usize,
    kernel_only_recall: f32,
    grounded_fraction: f32,
    panel: Option<&PanelLedgerContract>,
) -> std::result::Result<LedgerRef, CalyxError> {
    let mut payload_json = serde_json::json!({
        "marker": SUMMARIZE_INVOKED_MARKER,
        "scope_hash": hex32(scope_h),
        "kernel_size": kernel_size,
        "kernel_only_recall": kernel_only_recall,
        "grounded_fraction": grounded_fraction,
    });
    if let Some(panel) = panel {
        // Additive per-lane provenance for panel-native recall; legacy graphs keep
        // the historic payload shape untouched.
        payload_json["embedding_slots"] = serde_json::json!(panel.embedding_slots);
        payload_json["fusion"] = serde_json::json!(panel.fusion);
        payload_json["rrf_k"] = serde_json::json!(panel.rrf_k);
        payload_json["n_lanes"] = serde_json::json!(panel.embedding_slots.len());
    }
    let payload = serde_json::to_vec(&payload_json).map_err(|error| {
        bridge_error(
            "CALYX_SUMMARIZE_LEDGER_ENCODE",
            format!("failed to encode SUMMARIZE_INVOKED payload: {error}"),
        )
    })?;
    vault.append_ledger_entry(
        EntryKind::Kernel,
        SubjectId::Kernel(scope_h.to_vec()),
        payload,
        ActorId::Service("calyx-lodestar-summarize".to_string()),
    )
}

fn before_horizon(t: Ts, horizon: Ts) -> CalyxError {
    CalyxError {
        code: CALYX_TIMETRAVEL_BEFORE_HORIZON,
        message: format!("as-of time {t} is before retention horizon {horizon}"),
        remediation: "summarize at or after the retention horizon",
    }
}

fn to_calyx(error: LodestarError) -> CalyxError {
    CalyxError {
        code: error.code(),
        message: error.to_string(),
        remediation: "inspect the Aster graph bridge source rows",
    }
}

fn bridge_error(code: &'static str, message: String) -> CalyxError {
    CalyxError {
        code,
        message,
        remediation: "repair the Aster graph bridge metadata or node props",
    }
}

fn hex32(bytes: &[u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(64);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}
