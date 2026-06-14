use std::collections::{BTreeMap, BTreeSet};

use calyx_aster::plain_graph::PlainGraph;
use calyx_aster::timetravel::TimeTravelSnapshot;
use calyx_aster::vault::AsterVault;
use calyx_core::{AnchorKind, CalyxError, Clock, CxId, LedgerRef, Seq, Ts};
use calyx_ledger::{ActorId, EntryKind, SubjectId};
use calyx_paths::AssocGraph;
use serde::{Deserialize, Serialize};

use crate::recall_test::{InMemoryAnnIndex, InMemoryCorpus, RecallQuery, RecallTestParams};
use crate::scope::{AssocStore, CollectionId, FilterExpr, Scope, TenantId};
use crate::summarize::{
    CALYX_TIMETRAVEL_BEFORE_HORIZON, SUMMARIZE_INVOKED_MARKER, SummarizeParams, SummarizeRecall,
    SummarizeResult, summarize_with_ledger,
};
use crate::{LodestarError, Result, ScopeCache};

pub const DEFAULT_ASTER_ASSOC_COLLECTION: &str = "default";
pub const ASTER_ASSOC_METADATA_KEY: &str = "lodestar_assoc_v1";

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AsterAssocMetadata {
    pub retention_horizon: Option<Ts>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AsterAssocNodeProps {
    pub embedding: Option<Vec<f32>>,
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
    _lease: Option<TimeTravelSnapshot<'a, C>>,
}

pub struct AsterRecallInputs {
    embeddings: BTreeMap<CxId, Vec<f32>>,
    corpus: InMemoryCorpus,
    full_index: InMemoryAnnIndex,
    params: RecallTestParams,
}

#[derive(Clone, Debug)]
pub struct AsterSummarizeRequest<'a> {
    pub collection: &'a str,
    pub scope: Scope,
    pub params: Option<SummarizeParams>,
    pub recall_params: RecallTestParams,
}

impl AsterRecallInputs {
    pub fn measurement(&self) -> SummarizeRecall<'_> {
        SummarizeRecall {
            embeddings: &self.embeddings,
            full_index: &self.full_index,
            corpus: &self.corpus,
            params: self.params.clone(),
        }
    }
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
        let mut rows = Vec::new();
        let mut embeddings = BTreeMap::new();
        for cx_id in graph.node_ids() {
            let props = self.node_props(cx_id).map_err(to_calyx)?;
            let vector = props.embedding.ok_or_else(|| {
                bridge_error(
                    "CALYX_SUMMARIZE_RECALL_MISSING_EMBEDDING",
                    format!("node {cx_id} has no embedding in Aster graph props"),
                )
            })?;
            embeddings.insert(cx_id, vector.clone());
            rows.push(RecallQuery { cx_id, vector });
        }
        let full_index = InMemoryAnnIndex::new(rows.clone()).map_err(to_calyx)?;
        Ok(Some(AsterRecallInputs {
            embeddings,
            corpus: InMemoryCorpus::new(
                format!("aster:{}@{}", self.collection, self.snapshot),
                rows,
            ),
            full_index,
            params,
        }))
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
            _lease: lease,
        })
    }

    fn graph(&self) -> Result<PlainGraph<'_, C>> {
        PlainGraph::new(self.vault, &self.collection).map_err(LodestarError::from)
    }

    fn node_props(&self, cx_id: CxId) -> Result<AsterAssocNodeProps> {
        let graph = self.graph()?;
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
        self.graph()?
            .assoc_graph(self.snapshot)
            .map_err(LodestarError::from)
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
) -> std::result::Result<LedgerRef, CalyxError> {
    let payload = serde_json::to_vec(&serde_json::json!({
        "marker": SUMMARIZE_INVOKED_MARKER,
        "scope_hash": hex32(scope_h),
        "kernel_size": kernel_size,
        "kernel_only_recall": kernel_only_recall,
        "grounded_fraction": grounded_fraction,
    }))
    .map_err(|error| {
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
