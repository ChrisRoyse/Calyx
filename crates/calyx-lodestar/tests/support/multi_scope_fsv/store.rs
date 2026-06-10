use std::collections::{BTreeMap, BTreeSet};

use calyx_core::{AnchorKind, CxId, Ts};
use calyx_lodestar::{AssocStore, CollectionId, FilterExpr, RecallQuery, TenantId};
use calyx_paths::AssocGraph;

const RING_CHUNK: usize = 60;

#[derive(Clone)]
pub(super) struct RealScopeStore {
    graph: AssocGraph,
    collections: BTreeMap<CollectionId, BTreeSet<CxId>>,
    timestamps: BTreeMap<CxId, Ts>,
    anchors: BTreeMap<AnchorKind, Vec<CxId>>,
}

impl AssocStore for RealScopeStore {
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
        Ok(Some(
            self.timestamps
                .iter()
                .filter_map(|(id, ts)| ((*ts >= t0) && (*ts <= t1)).then_some(*id))
                .collect(),
        ))
    }

    fn tenant_nodes(&self, _id: &TenantId) -> calyx_lodestar::Result<Option<BTreeSet<CxId>>> {
        Ok(None)
    }

    fn filter_nodes(&self, _expr: &FilterExpr) -> calyx_lodestar::Result<BTreeSet<CxId>> {
        Ok(BTreeSet::new())
    }
}

pub(super) fn real_scope_store(rows: &[RecallQuery]) -> RealScopeStore {
    let diagnostic = diagnostic_nodes(rows);
    let mut builder = AssocGraph::builder();
    for (idx, row) in rows.iter().enumerate() {
        builder
            .add_node(row.cx_id, 1.0 + (idx % 7) as f32)
            .expect("node");
    }
    for chunk in rows[5..].chunks(RING_CHUNK) {
        for pair in chunk.windows(2) {
            builder
                .add_edge(pair[0].cx_id, pair[1].cx_id, 1.0)
                .expect("ring edge");
        }
        if chunk.len() > 2 {
            builder
                .add_edge(chunk[chunk.len() - 1].cx_id, chunk[0].cx_id, 1.0)
                .expect("ring close");
        }
    }
    add_diagnostic_cycles(&mut builder, &diagnostic);
    RealScopeStore {
        graph: builder.build(),
        collections: BTreeMap::from([
            (CollectionId::from("collection_a"), ids(&rows[0..125])),
            (CollectionId::from("collection_b"), ids(&rows[65..180])),
            (CollectionId::from("mfvs_a"), id_set(&diagnostic[0..3])),
            (CollectionId::from("mfvs_b"), id_set(&diagnostic[2..5])),
        ]),
        timestamps: rows
            .iter()
            .enumerate()
            .map(|(idx, row)| (row.cx_id, 1_700_000_000_u64 + idx as u64))
            .collect(),
        anchors: BTreeMap::from([(domain_anchor(), vec![rows[120].cx_id])]),
    }
}

pub(super) fn domain_anchor() -> AnchorKind {
    AnchorKind::Label("ph34-real-scope".to_string())
}

fn diagnostic_nodes(rows: &[RecallQuery]) -> Vec<CxId> {
    let mut ids: Vec<_> = rows[..5].iter().map(|row| row.cx_id).collect();
    ids.sort();
    ids
}

fn add_diagnostic_cycles(builder: &mut calyx_paths::AssocGraphBuilder, ids: &[CxId]) {
    for (src, dst) in [(0, 1), (1, 2), (2, 0), (2, 3), (3, 4), (4, 2)] {
        builder
            .add_edge(ids[src], ids[dst], 1.0)
            .expect("diag edge");
    }
}

fn ids(rows: &[RecallQuery]) -> BTreeSet<CxId> {
    rows.iter().map(|row| row.cx_id).collect()
}

fn id_set(ids: &[CxId]) -> BTreeSet<CxId> {
    ids.iter().copied().collect()
}
