use std::collections::{BTreeMap, BTreeSet};

use calyx_aster::plain_graph::PhysicalPlainGraph;
use calyx_core::{AnchorKind, CxId, Ts};
use calyx_paths::AssocGraph;

use crate::scope::{AssocStore, CollectionId, FilterExpr, TenantId};
use crate::{LodestarError, Result};

use super::{AsterAssocNodeProps, DEFAULT_ASTER_ASSOC_COLLECTION, PhysicalAsterAssocSnapshot};

impl PhysicalAsterAssocSnapshot {
    pub fn latest(vault_dir: impl AsRef<std::path::Path>, collection: &str) -> Result<Self> {
        let plain = PhysicalPlainGraph::open_latest(vault_dir, collection)?;
        let graph = plain.assoc_graph()?;
        let raw_props = plain.node_props()?;
        let mut props = BTreeMap::new();
        for (id, bytes) in raw_props {
            let decoded: AsterAssocNodeProps = serde_json::from_slice(&bytes).map_err(|error| {
                LodestarError::KernelIndexCodec {
                    detail: format!("decode physical Aster graph node props for {id}: {error}"),
                }
            })?;
            props.insert(id, decoded);
        }
        if props.len() != graph.node_count() {
            return Err(LodestarError::KernelInvalidParams {
                detail: format!(
                    "physical graph topology has {} nodes but node props scan returned {}; rebuild the graph CF",
                    graph.node_count(),
                    props.len()
                ),
            });
        }
        Ok(Self { graph, props })
    }

    pub fn node_props(&self, id: CxId) -> Result<&AsterAssocNodeProps> {
        self.props
            .get(&id)
            .ok_or_else(|| LodestarError::KernelInvalidParams {
                detail: format!("physical Aster graph node {id} is missing node props"),
            })
    }
}

impl AssocStore for PhysicalAsterAssocSnapshot {
    fn full_graph(&self) -> Result<AssocGraph> {
        Ok(self.graph.clone())
    }

    fn collection_nodes(&self, id: &CollectionId) -> Result<Option<BTreeSet<CxId>>> {
        if id.0 != DEFAULT_ASTER_ASSOC_COLLECTION {
            return Ok(None);
        }
        Ok(Some(self.graph.node_ids().collect()))
    }

    fn domain_anchors(&self, kind: &AnchorKind) -> Result<Vec<CxId>> {
        Ok(self
            .props
            .iter()
            .filter(|(_, props)| props.anchors.iter().any(|stored| stored == kind))
            .map(|(id, _)| *id)
            .collect())
    }

    fn time_window_nodes(&self, t0: Ts, t1: Ts) -> Result<Option<BTreeSet<CxId>>> {
        let mut saw_ts = false;
        let mut ids = BTreeSet::new();
        for (id, props) in &self.props {
            if let Some(ts) = props.ts {
                saw_ts = true;
                if (t0..=t1).contains(&ts) {
                    ids.insert(*id);
                }
            }
        }
        Ok(saw_ts.then_some(ids))
    }

    fn tenant_nodes(&self, id: &TenantId) -> Result<Option<BTreeSet<CxId>>> {
        let found = self
            .props
            .iter()
            .filter(|(_, props)| props.tenant.as_ref() == Some(id))
            .map(|(node, _)| *node)
            .collect::<BTreeSet<_>>();
        Ok((!found.is_empty()).then_some(found))
    }

    fn filter_nodes(&self, expr: &FilterExpr) -> Result<BTreeSet<CxId>> {
        let mut found = BTreeSet::new();
        for (node, props) in &self.props {
            let matches = match expr {
                FilterExpr::Named { name } => {
                    props.named_filters.iter().any(|stored| stored == name)
                }
                FilterExpr::MetadataEq { key, value } => props.metadata.get(key) == Some(value),
            };
            if matches {
                found.insert(*node);
            }
        }
        Ok(found)
    }

    fn node_metadata(&self, id: CxId) -> Result<Option<BTreeMap<String, String>>> {
        Ok(Some(self.node_props(id)?.metadata.clone()))
    }
}
