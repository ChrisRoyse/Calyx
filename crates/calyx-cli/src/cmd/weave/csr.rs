//! #996: persist the assoc CSR projection with the woven graph so physical
//! readers (spectral-communities, kernel-build, ...) load the CSR path
//! instead of row-scanning millions of edge rows.

use calyx_aster::plain_graph::{PhysicalPlainGraph, PlainGraph};
use calyx_aster::vault::AsterVault;
use calyx_core::{CalyxError, VaultStore};
use calyx_lodestar::PANEL_ASTER_ASSOC_COLLECTION;
use serde_json::json;

use super::error_details;
use super::progress::WeaveLoomProgressWriter;
use crate::error::{CliError, CliResult};

pub(super) fn persist_assoc_csr<C: calyx_core::Clock>(
    vault: &AsterVault<C>,
    graph: &PlainGraph<'_, C>,
    vault_dir: &std::path::Path,
    generation: &str,
    progress: &WeaveLoomProgressWriter,
) -> CliResult {
    let fail = |phase: &str, detail: String| {
        let error = CliError::from(CalyxError {
            code: "CALYX_GRAPH_CSR_MATERIALIZE_FAILED",
            message: format!(
                "persist assoc CSR failed for collection={PANEL_ASTER_ASSOC_COLLECTION} vault={}: {detail}",
                vault_dir.display()
            ),
            remediation: "fix the underlying graph rows and re-run weave-loom or `calyx materialize-graph-csr <vault>`",
        });
        let _ = progress.write(
            "incomplete",
            phase,
            json!({ "error": error_details(&error) }),
        );
        error
    };
    let commit = match graph.rebuild_csr(vault.snapshot()) {
        Ok(commit) => commit,
        Err(error) => {
            let inner: CliError = error.into();
            return Err(fail(
                "assoc_csr_error",
                format!("{} ({})", inner.message(), inner.code()),
            ));
        }
    };
    vault.flush().map_err(|error| {
        fail(
            "assoc_csr_flush_error",
            format!("durable vault flush failed: {error}"),
        )
    })?;

    let physical = PhysicalPlainGraph::open_generation_unchecked(
        vault_dir,
        PANEL_ASTER_ASSOC_COLLECTION,
        generation,
    )
    .map_err(|error| {
        fail(
            "assoc_csr_readback_error",
            format!("physical Graph CF reopen failed: {error}"),
        )
    })?;
    let raw = physical
        .read_csr_bytes()
        .map_err(|error| {
            fail(
                "assoc_csr_readback_error",
                format!("physical CSR byte readback failed: {error}"),
            )
        })?
        .ok_or_else(|| {
            fail(
                "assoc_csr_readback_error",
                "physical CSR is absent after flush".to_string(),
            )
        })?;
    let csr = physical
        .read_csr()
        .map_err(|error| {
            fail(
                "assoc_csr_readback_error",
                format!("physical CSR decode failed: {error}"),
            )
        })?
        .ok_or_else(|| {
            fail(
                "assoc_csr_readback_error",
                "physical CSR decoded as absent after flush".to_string(),
            )
        })?;
    let assoc = physical.assoc_graph().map_err(|error| {
        fail(
            "assoc_csr_readback_error",
            format!("physical CSR graph load failed: {error}"),
        )
    })?;
    let physical_nodes = physical.node_key_count().map_err(|error| {
        fail(
            "assoc_csr_readback_error",
            format!("physical node row count failed: {error}"),
        )
    })?;
    let physical_edges = physical.edge_out_key_count().map_err(|error| {
        fail(
            "assoc_csr_readback_error",
            format!("physical edge row count failed: {error}"),
        )
    })?;
    if csr.nodes.len() != commit.projection.nodes.len()
        || csr.edges.len() != commit.projection.edges.len()
        || assoc.node_count() != commit.projection.nodes.len()
        || assoc.edge_count() != commit.projection.association_edge_count
        || physical_nodes != csr.nodes.len()
        || physical_edges != csr.edges.len()
    {
        return Err(fail(
            "assoc_csr_readback_mismatch",
            format!(
                "committed nodes={} edges={} association_edges={}, readback nodes={} edges={} graph_nodes={} graph_edges={}, physical node_rows={physical_nodes} edge_rows={physical_edges}",
                commit.projection.nodes.len(),
                commit.projection.edges.len(),
                commit.projection.association_edge_count,
                csr.nodes.len(),
                csr.edges.len(),
                assoc.node_count(),
                assoc.edge_count(),
            ),
        ));
    }
    progress.write(
        "running",
        "assoc_csr_persisted",
        json!({
            "commit_seq": commit.seq,
            "csr_bytes": raw.len(),
            "nodes": csr.nodes.len(),
            "csr_edges": csr.edges.len(),
            "association_edge_count": assoc.edge_count(),
            "physical_node_rows": physical_nodes,
            "physical_edge_rows": physical_edges,
            "readback": "fresh PhysicalPlainGraph after vault.flush",
        }),
    )
}
