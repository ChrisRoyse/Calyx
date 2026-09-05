use std::collections::BTreeMap;
use std::path::Path;

use calyx_aster::plain_graph::PhysicalPlainGraph;
use calyx_core::CxId;
use calyx_lodestar::AsterAssocNodeProps;

use super::{
    DEFAULT_COLLECTION, EDGE_TYPE, SOURCE_NODE_TYPE, TARGET_NODE_TYPE, alias_node_id,
    contract_error,
};
use crate::cmd::vault::resolve_vault_info;
use crate::error::CliResult;

/// Proves that an import map is identical to the accepted, DB-native alias
/// relation before another collection is allowed to consume it.
pub(crate) fn verify_idmap_physical(
    home: &Path,
    vault_name: &str,
    idmap: &BTreeMap<String, CxId>,
) -> CliResult<usize> {
    let resolved = resolve_vault_info(home, vault_name)?;
    let physical = PhysicalPlainGraph::open_latest_isolated(&resolved.path, DEFAULT_COLLECTION)?;
    let nodes = physical
        .node_props()?
        .into_iter()
        .collect::<BTreeMap<_, _>>();
    let edges = physical
        .edge_out_props()?
        .into_iter()
        .filter(|edge| edge.edge_type == EDGE_TYPE)
        .map(|edge| ((edge.src, edge.dst), edge.value))
        .collect::<BTreeMap<_, _>>();
    for (opinion_id, expected_cx_id) in idmap {
        verify_one(&nodes, &edges, opinion_id, *expected_cx_id)?;
    }
    Ok(idmap.len())
}

fn verify_one(
    nodes: &BTreeMap<CxId, Vec<u8>>,
    edges: &BTreeMap<(CxId, CxId), Vec<u8>>,
    opinion_id: &str,
    expected_cx_id: CxId,
) -> CliResult {
    let alias_id = alias_node_id(opinion_id);
    let alias_bytes = nodes.get(&alias_id).ok_or_else(|| {
        contract_error(
            "CALYX_CITATION_ALIAS_NODE_MISSING",
            format!("citation idmap opinion {opinion_id} has no physical alias node"),
            "materialize the complete accepted opinion-alias relation before citation ingestion",
        )
    })?;
    let alias: AsterAssocNodeProps = serde_json::from_slice(alias_bytes).map_err(|error| {
        contract_error(
            "CALYX_CITATION_ALIAS_NODE_CORRUPT",
            format!("citation idmap opinion {opinion_id} alias node does not decode: {error}"),
            "quarantine and rebuild the opinion-alias collection",
        )
    })?;
    let metadata = &alias.metadata;
    let expected_cx_text = expected_cx_id.to_string();
    if metadata.get("node_type").map(String::as_str) != Some(SOURCE_NODE_TYPE)
        || metadata.get("opinion_id").map(String::as_str) != Some(opinion_id)
        || metadata.get("canonical_cx_id").map(String::as_str) != Some(&expected_cx_text)
    {
        return Err(contract_error(
            "CALYX_CITATION_ALIAS_IDMAP_MISMATCH",
            format!(
                "citation idmap opinion {opinion_id} -> {expected_cx_id} differs from its physical alias node"
            ),
            "rebuild the citation idmap from the accepted physical opinion-alias relation",
        ));
    }
    let target_bytes = nodes.get(&expected_cx_id).ok_or_else(|| {
        contract_error(
            "CALYX_CITATION_ALIAS_TARGET_MISSING",
            format!(
                "citation idmap opinion {opinion_id} targets absent physical node {expected_cx_id}"
            ),
            "quarantine and rebuild the opinion-alias collection from Base bytes",
        )
    })?;
    let target: AsterAssocNodeProps = serde_json::from_slice(target_bytes).map_err(|error| {
        contract_error(
            "CALYX_CITATION_ALIAS_TARGET_CORRUPT",
            format!(
                "citation idmap opinion {opinion_id} target {expected_cx_id} does not decode: {error}"
            ),
            "quarantine and rebuild the opinion-alias collection from Base bytes",
        )
    })?;
    if target.metadata.get("node_type").map(String::as_str) != Some(TARGET_NODE_TYPE)
        || target.metadata.get("canonical_cx_id").map(String::as_str) != Some(&expected_cx_text)
        || target.metadata.get("content_sha256") != metadata.get("content_sha256")
    {
        return Err(contract_error(
            "CALYX_CITATION_ALIAS_TARGET_MISMATCH",
            format!(
                "citation idmap opinion {opinion_id} target {expected_cx_id} differs from its alias"
            ),
            "quarantine and rebuild the opinion-alias collection from Base bytes",
        ));
    }
    let edge_bytes = edges.get(&(alias_id, expected_cx_id)).ok_or_else(|| {
        contract_error(
            "CALYX_CITATION_ALIAS_EDGE_MISSING",
            format!(
                "citation idmap opinion {opinion_id} lacks aliases_to edge to {expected_cx_id}"
            ),
            "quarantine and rebuild the complete opinion-alias collection",
        )
    })?;
    let edge: serde_json::Value = serde_json::from_slice(edge_bytes).map_err(|error| {
        contract_error(
            "CALYX_CITATION_ALIAS_EDGE_CORRUPT",
            format!("citation idmap opinion {opinion_id} edge does not decode: {error}"),
            "quarantine and rebuild the complete opinion-alias collection",
        )
    })?;
    if edge.get("opinion_id").and_then(serde_json::Value::as_str) != Some(opinion_id)
        || edge
            .get("canonical_cx_id")
            .and_then(serde_json::Value::as_str)
            != Some(&expected_cx_text)
    {
        return Err(contract_error(
            "CALYX_CITATION_ALIAS_EDGE_MISMATCH",
            format!(
                "citation idmap opinion {opinion_id} aliases_to edge differs from {expected_cx_id}"
            ),
            "quarantine and rebuild the complete opinion-alias collection",
        ));
    }
    Ok(())
}
