use std::collections::BTreeSet;

use calyx_aster::layers::relational::{RecordKey, RecordValue, Row};
use calyx_aster::plain_graph::{PlainGraph, PlainGraphDirection, TraverseOptions};
use calyx_aster::vault::AsterVault;
use calyx_core::{Clock, CxId, Result, Seq};

use crate::error::{
    CALYX_SEXTANT_ASSOC_GRAPH_MISSING, CALYX_SEXTANT_GRAPH_HOP_KIND_UNKNOWN,
    CALYX_SEXTANT_TRAVERSE_HOPS, sextant_error,
};
use crate::navigation::MAX_TRAVERSE_HOPS;
use crate::query::ProvenancedRow;

use super::ExecState;
use super::support::shape;

const DEFAULT_ASTER_ASSOC_COLLECTION: &str = "default";
const GRAPH_HOP_COST_CAP: usize = 100_000;

pub(super) fn execute_graph_hop<C>(
    vault: &AsterVault<C>,
    snapshot: Seq,
    state: &mut ExecState,
    from_cx_ids: &[CxId],
    hop_kind: &str,
    max_hops: u8,
) -> Result<()>
where
    C: Clock,
{
    validate_graph_hop_args(from_cx_ids, max_hops)?;
    let graph = PlainGraph::new(vault, DEFAULT_ASTER_ASSOC_COLLECTION)?;
    let projection = graph.csr_projection(snapshot)?;
    if projection.nodes.is_empty() {
        return Err(sextant_error(
            CALYX_SEXTANT_ASSOC_GRAPH_MISSING,
            format!(
                "GraphHop hop_kind={hop_kind} requested from {} source id(s), but graph collection {DEFAULT_ASTER_ASSOC_COLLECTION:?} has no persisted nodes",
                from_cx_ids.len()
            ),
        ));
    }

    let known_types = projection
        .edges
        .iter()
        .map(|edge| edge.edge_type.as_str())
        .collect::<BTreeSet<_>>();
    if !known_types.contains(hop_kind) {
        return Err(sextant_error(
            CALYX_SEXTANT_GRAPH_HOP_KIND_UNKNOWN,
            format!(
                "GraphHop hop_kind={hop_kind} is absent from graph collection {DEFAULT_ASTER_ASSOC_COLLECTION:?}; known hop kinds: {:?}",
                known_types
            ),
        ));
    }
    state.total_scanned += (projection.nodes.len() + projection.edges.len()) as u64;

    let opts = TraverseOptions {
        edge_type: Some(hop_kind),
        direction: PlainGraphDirection::Out,
        max_hops: max_hops as usize,
        cost_cap: GRAPH_HOP_COST_CAP,
    };
    let mut reached = BTreeSet::new();
    for source in from_cx_ids {
        reached.extend(graph.traverse(snapshot, *source, opts)?);
    }
    state.candidates = reached.clone();
    state.rows = reached
        .into_iter()
        .map(|cx_id| graph_hop_row(cx_id, hop_kind, max_hops))
        .collect::<Result<Vec<_>>>()?;
    Ok(())
}

fn validate_graph_hop_args(from_cx_ids: &[CxId], max_hops: u8) -> Result<()> {
    if from_cx_ids.is_empty() {
        return Err(shape("GraphHop requires at least one source cx_id"));
    }
    if !(1..=MAX_TRAVERSE_HOPS as u8).contains(&max_hops) {
        return Err(sextant_error(
            CALYX_SEXTANT_TRAVERSE_HOPS,
            format!("GraphHop max_hops {max_hops} outside 1..={MAX_TRAVERSE_HOPS}"),
        ));
    }
    Ok(())
}

fn graph_hop_row(cx_id: CxId, hop_kind: &str, max_hops: u8) -> Result<ProvenancedRow> {
    Ok(ProvenancedRow {
        key: RecordKey::from_bytes(cx_id.as_bytes().to_vec())?,
        value: Some(Row::new([
            ("cx_id", RecordValue::Text(cx_id.to_string())),
            ("hop_kind", RecordValue::Text(hop_kind.to_string())),
            ("max_hops", RecordValue::U64(u64::from(max_hops))),
        ])),
        score: None,
        ledger_ref: None,
    })
}
