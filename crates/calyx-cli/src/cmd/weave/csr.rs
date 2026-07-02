//! #996: persist the assoc CSR projection with the woven graph so physical
//! readers (spectral-communities, kernel-build, ...) load the CSR path
//! instead of row-scanning millions of edge rows.

use calyx_aster::plain_graph::PlainGraph;
use calyx_aster::vault::AsterVault;
use calyx_core::VaultStore;
use calyx_lodestar::DEFAULT_ASTER_ASSOC_COLLECTION;
use serde_json::json;

use super::error_details;
use super::progress::WeaveLoomProgressWriter;
use crate::error::{CliError, CliResult};

pub(super) fn persist_assoc_csr<C: calyx_core::Clock>(
    vault: &AsterVault<C>,
    graph: &PlainGraph<'_, C>,
    vault_dir: &std::path::Path,
    progress: &WeaveLoomProgressWriter,
) -> CliResult {
    let commit = match graph.rebuild_csr(vault.snapshot()) {
        Ok(commit) => commit,
        Err(error) => {
            let inner: CliError = error.into();
            let _ = progress.write(
                "incomplete",
                "assoc_csr_error",
                json!({ "error": error_details(&inner) }),
            );
            return Err(CliError::from(calyx_core::CalyxError {
                code: "CALYX_GRAPH_CSR_MATERIALIZE_FAILED",
                message: format!(
                    "materialize assoc CSR failed for collection={DEFAULT_ASTER_ASSOC_COLLECTION} vault={}: {} ({})",
                    vault_dir.display(),
                    inner.message(),
                    inner.code()
                ),
                remediation: "fix the underlying graph rows and re-run weave-loom or `calyx materialize-graph-csr <vault>`",
            }));
        }
    };
    progress.write(
        "running",
        "assoc_csr_persisted",
        json!({
            "commit_seq": commit.seq,
            "nodes": commit.projection.nodes.len(),
            "csr_edges": commit.projection.edges.len(),
            "association_edge_count": commit.projection.association_edge_count,
        }),
    )
}
