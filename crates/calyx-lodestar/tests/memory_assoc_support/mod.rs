#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};

use calyx_core::{AnchorKind, CxId, Ts};
use calyx_lodestar::{AssocStore, CollectionId, FilterExpr, TenantId};
use calyx_paths::AssocGraph;

#[derive(Clone)]
pub struct MemoryAssocStore {
    graph: AssocGraph,
    collections: BTreeMap<CollectionId, BTreeSet<CxId>>,
    anchors: BTreeMap<AnchorKind, Vec<CxId>>,
    timestamps: Option<BTreeMap<CxId, Ts>>,
    tenants: BTreeMap<TenantId, BTreeSet<CxId>>,
    filters: BTreeMap<FilterExpr, BTreeSet<CxId>>,
}

impl MemoryAssocStore {
    pub fn with_scope_data(
        graph: AssocGraph,
        collections: BTreeMap<CollectionId, BTreeSet<CxId>>,
        anchors: BTreeMap<AnchorKind, Vec<CxId>>,
    ) -> Self {
        Self::with_indexes(
            graph,
            collections,
            anchors,
            None,
            BTreeMap::new(),
            BTreeMap::new(),
        )
    }

    pub fn with_indexes(
        graph: AssocGraph,
        collections: BTreeMap<CollectionId, BTreeSet<CxId>>,
        anchors: BTreeMap<AnchorKind, Vec<CxId>>,
        timestamps: Option<BTreeMap<CxId, Ts>>,
        tenants: BTreeMap<TenantId, BTreeSet<CxId>>,
        filters: BTreeMap<FilterExpr, BTreeSet<CxId>>,
    ) -> Self {
        Self {
            graph,
            collections,
            anchors,
            timestamps,
            tenants,
            filters,
        }
    }
}

impl AssocStore for MemoryAssocStore {
    fn full_graph(&self) -> calyx_lodestar::Result<AssocGraph> {
        Ok(self.graph.clone())
    }

    fn collection_nodes(
        &self,
        id: &CollectionId,
    ) -> calyx_lodestar::Result<Option<BTreeSet<CxId>>> {
        Ok(self.collections.get(id).cloned())
    }

    fn domain_anchors(&self, kind: &AnchorKind) -> calyx_lodestar::Result<Vec<CxId>> {
        Ok(self.anchors.get(kind).cloned().unwrap_or_default())
    }

    fn time_window_nodes(&self, t0: Ts, t1: Ts) -> calyx_lodestar::Result<Option<BTreeSet<CxId>>> {
        let Some(timestamps) = &self.timestamps else {
            return Ok(None);
        };
        Ok(Some(
            timestamps
                .iter()
                .filter_map(|(cx_id, ts)| ((*ts >= t0) && (*ts <= t1)).then_some(*cx_id))
                .collect(),
        ))
    }

    fn tenant_nodes(&self, id: &TenantId) -> calyx_lodestar::Result<Option<BTreeSet<CxId>>> {
        Ok(self.tenants.get(id).cloned())
    }

    fn filter_nodes(&self, expr: &FilterExpr) -> calyx_lodestar::Result<BTreeSet<CxId>> {
        Ok(self.filters.get(expr).cloned().unwrap_or_default())
    }
}

pub fn cx(seed: u8) -> CxId {
    CxId::from_bytes([seed; 16])
}

pub fn ids<const N: usize>(values: [u8; N]) -> BTreeSet<CxId> {
    values.into_iter().map(cx).collect()
}
