//! `calyx materialize-graph-csr <vault>` — persist the Aster assoc-graph CSR
//! projection for a collection so physical graph readers (spectral-communities,
//! kernel-build, discovery-chain, ...) load the persisted CSR instead of
//! row-scanning every edge (#996).

use std::time::Instant;

use calyx_aster::plain_graph::{PhysicalPlainGraph, PlainGraph};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{CalyxError, VaultStore};
use calyx_lodestar::DEFAULT_ASTER_ASSOC_COLLECTION;
use serde_json::json;
use sha2::{Digest, Sha256};

use super::vault::{home_dir, resolve_vault_info, vault_salt};
use super::{Subcommand, value};
use crate::error::{CliError, CliResult};
use crate::output::print_json;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MaterializeGraphCsrArgs {
    pub vault: String,
    pub collection: String,
}

pub(crate) fn parse_materialize_graph_csr(rest: &[String]) -> CliResult<Subcommand> {
    let vault = rest
        .first()
        .ok_or_else(|| CliError::usage("materialize-graph-csr requires <vault>"))?
        .clone();
    let mut collection = DEFAULT_ASTER_ASSOC_COLLECTION.to_string();
    let mut idx = 1;
    while idx < rest.len() {
        match rest[idx].as_str() {
            "--collection" => {
                idx += 1;
                collection = value(rest, idx, "--collection")?.to_string();
            }
            other => {
                return Err(CliError::usage(format!(
                    "unexpected materialize-graph-csr flag {other}"
                )));
            }
        }
        idx += 1;
    }
    Ok(Subcommand::MaterializeGraphCsr(MaterializeGraphCsrArgs {
        vault,
        collection,
    }))
}

pub(crate) fn run(command: Subcommand) -> CliResult {
    let Subcommand::MaterializeGraphCsr(args) = command else {
        unreachable!("non-materialize-graph-csr command routed to graph_csr module");
    };
    run_materialize_graph_csr_with_home(&home_dir()?, args)
}

pub(crate) fn run_materialize_graph_csr_with_home(
    home: &std::path::Path,
    args: MaterializeGraphCsrArgs,
) -> CliResult {
    let started = Instant::now();
    let resolved = resolve_vault_info(home, &args.vault)?;
    eprintln!(
        "materialize-graph-csr: opening vault name={} id={} path={} collection={}",
        resolved.name,
        resolved.vault_id,
        resolved.path.display(),
        args.collection
    );
    // Before-state readback (physical, read-only) for FSV evidence.
    let before_bytes = PhysicalPlainGraph::open_latest(&resolved.path, &args.collection)?
        .read_csr_bytes()?
        .map(|bytes| bytes.len());
    eprintln!(
        "materialize-graph-csr: before csr_present={} csr_bytes={:?}",
        before_bytes.is_some(),
        before_bytes
    );
    // Latest-only open (like ingest/search since #1029): with the default
    // full-MVCC restore, rows checkpointed through router-flush SSTs are NOT
    // restored into the MVCC table and a snapshot scan silently misses them —
    // observed as a 198,993-node / 0-edge projection on the real #877 vault.
    let vault = AsterVault::open(
        &resolved.path,
        resolved.vault_id,
        vault_salt(resolved.vault_id, &resolved.name),
        VaultOptions {
            restore_mvcc_rows: false,
            ..VaultOptions::default()
        },
    )?;
    let graph = PlainGraph::new(&vault, &args.collection)?;
    let snapshot = vault.snapshot();
    let commit = graph
        .rebuild_csr(snapshot)
        .map_err(|error| csr_materialize_failed(&args, &resolved.path, &error))?;
    vault.flush()?;
    eprintln!(
        "materialize-graph-csr: committed seq={} nodes={} edges={} association_edges={} elapsed_ms={}",
        commit.seq,
        commit.projection.nodes.len(),
        commit.projection.edges.len(),
        commit.projection.association_edge_count,
        started.elapsed().as_millis()
    );
    // Source-of-truth readback: reopen the Graph CF physically (latest SST
    // state, independent of the writing vault handle) and prove the persisted
    // CSR is present, decodable, and drives the CSR load path.
    drop(graph);
    drop(vault);
    let physical = PhysicalPlainGraph::open_latest(&resolved.path, &args.collection)?;
    let raw = physical
        .read_csr_bytes()?
        .ok_or_else(|| csr_readback_missing(&args, &resolved.path))?;
    let csr = physical
        .read_csr()?
        .ok_or_else(|| csr_readback_missing(&args, &resolved.path))?;
    let assoc = physical.assoc_graph()?;
    // Independent row-level cross-check: enumerate the physical node/edge keys
    // (never the CSR row itself) so a partial or empty projection can never be
    // silently accepted just because it round-trips consistently.
    let physical_nodes = physical.node_key_count()?;
    let physical_edges = physical.edge_out_key_count()?;
    if csr.nodes.len() != commit.projection.nodes.len()
        || csr.edges.len() != commit.projection.edges.len()
        || assoc.node_count() != commit.projection.nodes.len()
        || assoc.edge_count() != commit.projection.association_edge_count
        || physical_nodes != csr.nodes.len()
        || physical_edges != csr.edges.len()
    {
        return Err(CliError::from(CalyxError {
            code: "CALYX_GRAPH_CSR_READBACK_MISMATCH",
            message: format!(
                "persisted CSR readback disagrees with the physical Graph CF for collection={} vault={}: committed nodes={} edges={} assoc_edges={}, readback nodes={} edges={} graph nodes={} graph_edges={}, physical node_keys={physical_nodes} edge_out_keys={physical_edges}",
                args.collection,
                resolved.path.display(),
                commit.projection.nodes.len(),
                commit.projection.edges.len(),
                commit.projection.association_edge_count,
                csr.nodes.len(),
                csr.edges.len(),
                assoc.node_count(),
                assoc.edge_count()
            ),
            remediation: "the persisted CSR does not match the row-level source of truth; re-run materialize-graph-csr and do not trust CSR readers for this collection until it reports ok",
        }));
    }
    let elapsed_ms = started.elapsed().as_millis();
    eprintln!(
        "materialize-graph-csr: physical readback ok csr_bytes={} nodes={} graph_edges={} elapsed_ms={elapsed_ms}",
        raw.len(),
        assoc.node_count(),
        assoc.edge_count()
    );
    print_json(&json!({
        "status": "ok",
        "vault": resolved.name,
        "vault_dir": resolved.path.display().to_string(),
        "collection": args.collection,
        "commit_seq": commit.seq,
        "csr_present_before": before_bytes.is_some(),
        "csr_bytes_before": before_bytes,
        "source_snapshot": csr.source_snapshot,
        "csr_bytes": raw.len(),
        "csr_sha256": sha256_hex(&raw),
        "csr_blake3": blake3::hash(&raw).to_hex().to_string(),
        "nodes": csr.nodes.len(),
        "csr_edges": csr.edges.len(),
        "association_edge_count": csr.association_edge_count,
        "readback": {
            "source_of_truth": "physical Graph CF latest readback via PhysicalPlainGraph::read_csr + assoc_graph + independent node/edge key enumeration",
            "assoc_graph_nodes": assoc.node_count(),
            "assoc_graph_edges": assoc.edge_count(),
            "physical_node_keys": physical_nodes,
            "physical_edge_out_keys": physical_edges,
        },
        "elapsed_ms": elapsed_ms,
    }))
}

fn csr_materialize_failed(
    args: &MaterializeGraphCsrArgs,
    vault_dir: &std::path::Path,
    error: &CalyxError,
) -> CliError {
    CliError::from(CalyxError {
        code: "CALYX_GRAPH_CSR_MATERIALIZE_FAILED",
        message: format!(
            "materialize assoc CSR failed for collection={} vault={}: {} ({})",
            args.collection,
            vault_dir.display(),
            error.message,
            error.code
        ),
        remediation: "fix the underlying graph rows (see inner error), then re-run materialize-graph-csr; do not fall back to row-scan readers for physical graph workloads",
    })
}

fn csr_readback_missing(args: &MaterializeGraphCsrArgs, vault_dir: &std::path::Path) -> CliError {
    CliError::from(CalyxError {
        code: "CALYX_GRAPH_CSR_READBACK_MISSING",
        message: format!(
            "persisted CSR row is missing from the physical Graph CF after commit+flush for collection={} vault={}",
            args.collection,
            vault_dir.display()
        ),
        remediation: "the write did not become durable; check vault flush errors and disk state, then re-run materialize-graph-csr",
    })
}

fn sha256_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests;
