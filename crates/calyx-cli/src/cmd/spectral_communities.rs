//! `calyx spectral-communities <vault>` -- run physical spectral community mining (#877).

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use calyx_core::CalyxError;
use calyx_lodestar::{
    AssocStore, LodestarError, PANEL_ASTER_ASSOC_COLLECTION, PhysicalAsterAssocSnapshot,
    SpectralCommunityParams, SpectralCommunityReport, spectral_community_report,
};
use serde_json::json;
use sha2::{Digest, Sha256};

use super::vault::{home_dir, resolve_vault_info};
use super::{Subcommand, value};
use crate::durable_write::write_bytes_atomic_new;
use crate::error::{CliError, CliResult};
use crate::output::print_json;

const DEFAULT_CLI_COMMUNITIES: usize = 15;
const DEFAULT_CLI_EIGEN_K: usize = 16;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SpectralCommunitiesArgs {
    pub vault: String,
    /// Physical graph collection to read. Defaults to the panel-native assoc
    /// collection (`kernel-panel-v3`) that `weave-loom` persists into as of
    /// #1857; `--collection default` reads pre-#1857 single-lane vaults (#1971).
    pub collection: String,
    pub eigen_k: usize,
    pub eigen_max_iter: usize,
    pub community_count: usize,
    pub cluster_max_iter: usize,
    pub centrality_max_iter: usize,
    pub centrality_tol: f32,
    pub max_bridge_candidates: usize,
    pub max_centrality_candidates: usize,
    pub out: Option<PathBuf>,
}

impl Default for SpectralCommunitiesArgs {
    fn default() -> Self {
        let params = SpectralCommunityParams::default();
        Self {
            vault: String::new(),
            collection: PANEL_ASTER_ASSOC_COLLECTION.to_string(),
            eigen_k: DEFAULT_CLI_EIGEN_K,
            eigen_max_iter: params.eigen_max_iter,
            community_count: DEFAULT_CLI_COMMUNITIES,
            cluster_max_iter: params.cluster_max_iter,
            centrality_max_iter: params.centrality_max_iter,
            centrality_tol: params.centrality_tol,
            max_bridge_candidates: params.max_bridge_candidates,
            max_centrality_candidates: params.max_centrality_candidates,
            out: None,
        }
    }
}

struct PersistedReport {
    path: PathBuf,
    bytes: u64,
    sha256: String,
    readback_node_count: usize,
    readback_member_count: usize,
    readback_degenerate_node_count: usize,
    readback_community_count: usize,
    readback_bridge_candidate_count: usize,
    readback_centrality_candidate_count: usize,
}

pub(crate) fn run(command: Subcommand) -> CliResult {
    let Subcommand::SpectralCommunities(args) = command else {
        unreachable!("non-spectral-communities command routed to spectral_communities module");
    };
    run_spectral_communities_with_home(&home_dir()?, args)
}

pub(crate) fn run_spectral_communities_with_home(
    home: &Path,
    args: SpectralCommunitiesArgs,
) -> CliResult {
    preflight_explicit_report(args.out.as_deref())?;
    let started = Instant::now();
    let resolved = resolve_vault_info(home, &args.vault)?;
    eprintln!(
        "spectral-communities: opening physical graph name={} id={} collection={} path={}",
        resolved.name,
        resolved.vault_id,
        args.collection,
        resolved.path.display()
    );
    let store = PhysicalAsterAssocSnapshot::latest(&resolved.path, &args.collection)?;
    let graph = store.full_graph()?;
    if graph.node_count() == 0 {
        return Err(graph_not_persisted_error(&args.collection));
    }
    eprintln!(
        "spectral-communities: computing report nodes={} edges={} communities={} eigen_k={} eigen_max_iter={} cluster_max_iter={} rayon_threads={}",
        graph.node_count(),
        graph.edge_count(),
        args.community_count,
        args.eigen_k,
        args.eigen_max_iter,
        args.cluster_max_iter,
        rayon::current_num_threads()
    );
    let report = spectral_community_report(
        &graph,
        &SpectralCommunityParams {
            eigen_k: args.eigen_k,
            eigen_max_iter: args.eigen_max_iter,
            community_count: args.community_count,
            cluster_max_iter: args.cluster_max_iter,
            centrality_max_iter: args.centrality_max_iter,
            centrality_tol: args.centrality_tol,
            max_bridge_candidates: args.max_bridge_candidates,
            max_centrality_candidates: args.max_centrality_candidates,
        },
    )?;
    ensure_useful_report(&report)?;
    let persisted = persist_report(&resolved.path, args.out.as_deref(), &report)?;
    eprintln!(
        "spectral-communities: persisted report={} bytes={} sha256={} node_count={} members={} degenerate_nodes={} communities={} elapsed_ms={}",
        persisted.path.display(),
        persisted.bytes,
        persisted.sha256,
        persisted.readback_node_count,
        persisted.readback_member_count,
        persisted.readback_degenerate_node_count,
        persisted.readback_community_count,
        started.elapsed().as_millis()
    );
    print_json(&json!({
        "status": "ok",
        "vault": resolved.name,
        "vault_dir": resolved.path.display().to_string(),
        "params": {
            "eigen_k": args.eigen_k,
            "eigen_max_iter": args.eigen_max_iter,
            "community_count": args.community_count,
            "cluster_max_iter": args.cluster_max_iter,
            "centrality_max_iter": args.centrality_max_iter,
            "centrality_tol": args.centrality_tol,
            "max_bridge_candidates": args.max_bridge_candidates,
            "max_centrality_candidates": args.max_centrality_candidates,
        },
        "report": report,
        "artifacts": {
            "report_json": persisted.path,
            "report_json_bytes": persisted.bytes,
            "report_json_sha256": persisted.sha256,
            "readback": {
                "node_count": persisted.readback_node_count,
                "member_count": persisted.readback_member_count,
                "degenerate_node_count": persisted.readback_degenerate_node_count,
                "community_count": persisted.readback_community_count,
                "bridge_candidate_count": persisted.readback_bridge_candidate_count,
                "centrality_candidate_count": persisted.readback_centrality_candidate_count,
            }
        }
    }))
}

/// Fail-closed error for a vault whose selected graph collection has no
/// persisted assoc graph (0 nodes). Distinct from `CALYX_SPECTRAL_GRAPH_TOO_SMALL`
/// (a real graph with fewer than two nodes): here the collection is absent or
/// empty, which almost always means weave-loom has not run or wrote a different
/// collection. Returning a specific code prevents the silent-empty
/// (0 nodes, rc=0 / misleading "too small") behaviour from masking the miss
/// (#1971).
fn graph_not_persisted_error(collection: &str) -> CliError {
    CliError::from(CalyxError {
        code: "CALYX_SPECTRAL_GRAPH_NOT_PERSISTED",
        message: format!(
            "graph collection {collection:?} has no persisted assoc graph (0 nodes); \
             the weave-loom association graph is not readable in this collection"
        ),
        remediation: "run `calyx weave-loom <vault>` to persist the assoc graph, or pass `--collection <name>` to read the collection weave-loom wrote (pre-#1857 vaults used `default`, current vaults use `kernel-panel-v3`)",
    })
}

fn preflight_explicit_report(path: Option<&Path>) -> CliResult {
    let Some(path) = path else { return Ok(()) };
    match fs::symlink_metadata(path) {
        Ok(_) => Err(CliError::usage(format!(
            "spectral report destination {} already exists; evidence is immutable",
            path.display()
        ))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(CliError::io(format!(
            "inspect spectral report destination {} before graph computation failed: {error}",
            path.display()
        ))),
    }
}

pub(crate) fn parse_spectral_communities(rest: &[String]) -> CliResult<Subcommand> {
    let vault = rest
        .first()
        .ok_or_else(|| CliError::usage("spectral-communities requires <vault>"))?
        .clone();
    let mut args = SpectralCommunitiesArgs {
        vault,
        ..SpectralCommunitiesArgs::default()
    };
    let mut idx = 1;
    while idx < rest.len() {
        match rest[idx].as_str() {
            "--eigen-k" => {
                idx += 1;
                args.eigen_k = parse_usize(value(rest, idx, "--eigen-k")?, "--eigen-k", 2)?;
            }
            "--eigen-max-iter" => {
                idx += 1;
                args.eigen_max_iter =
                    parse_usize(value(rest, idx, "--eigen-max-iter")?, "--eigen-max-iter", 1)?;
            }
            "--communities" => {
                idx += 1;
                args.community_count =
                    parse_usize(value(rest, idx, "--communities")?, "--communities", 2)?;
            }
            "--cluster-max-iter" => {
                idx += 1;
                args.cluster_max_iter = parse_usize(
                    value(rest, idx, "--cluster-max-iter")?,
                    "--cluster-max-iter",
                    1,
                )?;
            }
            "--centrality-max-iter" => {
                idx += 1;
                args.centrality_max_iter = parse_usize(
                    value(rest, idx, "--centrality-max-iter")?,
                    "--centrality-max-iter",
                    1,
                )?;
            }
            "--centrality-tol" => {
                idx += 1;
                args.centrality_tol =
                    parse_positive_f32(value(rest, idx, "--centrality-tol")?, "--centrality-tol")?;
            }
            "--max-bridge-candidates" => {
                idx += 1;
                args.max_bridge_candidates = parse_usize(
                    value(rest, idx, "--max-bridge-candidates")?,
                    "--max-bridge-candidates",
                    1,
                )?;
            }
            "--max-centrality-candidates" => {
                idx += 1;
                args.max_centrality_candidates = parse_usize(
                    value(rest, idx, "--max-centrality-candidates")?,
                    "--max-centrality-candidates",
                    1,
                )?;
            }
            "--collection" => {
                idx += 1;
                let collection = value(rest, idx, "--collection")?;
                if collection.is_empty() {
                    return Err(CliError::usage("--collection must not be empty"));
                }
                args.collection = collection.to_string();
            }
            "--out" => {
                idx += 1;
                args.out = Some(value(rest, idx, "--out")?.into());
            }
            other => {
                return Err(CliError::usage(format!(
                    "unexpected spectral-communities flag {other}"
                )));
            }
        }
        idx += 1;
    }
    if args.community_count > usize::from(u8::MAX) + 1 {
        return Err(CliError::usage("--communities must be at most 256"));
    }
    if args.eigen_k < args.community_count {
        return Err(CliError::usage(
            "--eigen-k must be greater than or equal to --communities",
        ));
    }
    Ok(Subcommand::SpectralCommunities(args))
}

fn ensure_useful_report(report: &SpectralCommunityReport) -> CliResult {
    // The main spectral k-means always yields exactly `requested_communities`
    // non-empty communities. Disconnected/isolated nodes (#1973) add further
    // deterministic per-component communities on top, so the total is
    // `requested_communities + <disconnected component count>` and can exceed
    // the request. Fewer than requested means the well-embedded partition
    // itself collapsed, which must still fail closed.
    if report.communities.len() < report.requested_communities {
        return Err(LodestarError::KernelInvalidParams {
            detail: format!(
                "spectral report produced {} communities but at least {} were requested",
                report.communities.len(),
                report.requested_communities
            ),
        }
        .into());
    }
    if report.bridge_candidates.is_empty() {
        return Err(LodestarError::KernelInvalidParams {
            detail: "spectral report produced no inter-community bridge candidates".to_string(),
        }
        .into());
    }
    if report.centrality_candidates.is_empty() {
        return Err(LodestarError::KernelInvalidParams {
            detail: "spectral report produced no eigenvector-centrality candidates".to_string(),
        }
        .into());
    }
    Ok(())
}

fn persist_report(
    vault_dir: &Path,
    explicit: Option<&Path>,
    report: &SpectralCommunityReport,
) -> CliResult<PersistedReport> {
    let bytes = serde_json::to_vec_pretty(report)
        .map_err(|error| CliError::runtime(format!("serialize spectral report: {error}")))?;
    let report_id = blake3::hash(&bytes).to_hex().to_string();
    let path = explicit.map(Path::to_path_buf).unwrap_or_else(|| {
        vault_dir
            .join("idx")
            .join("spectral_communities")
            .join(report_id)
            .join("report.json")
    });
    match fs::symlink_metadata(&path) {
        Ok(_) if explicit.is_some() => {
            return Err(CliError::usage(format!(
                "spectral report destination {} appeared during graph computation; refusing to replace immutable evidence",
                path.display()
            )));
        }
        Ok(metadata) => {
            if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
                return Err(CliError::usage(format!(
                    "content-addressed spectral report is not a plain file: {}",
                    path.display()
                )));
            }
            let existing = fs::read(&path).map_err(|error| {
                CliError::io(format!(
                    "read existing content-addressed spectral report {}: {error}",
                    path.display()
                ))
            })?;
            if existing != bytes {
                return Err(CliError::runtime(format!(
                    "content-addressed spectral report digest collision at {}",
                    path.display()
                )));
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            write_bytes_atomic_new(&path, &bytes, "spectral community report")?;
        }
        Err(error) => {
            return Err(CliError::io(format!(
                "inspect spectral report destination {} failed: {error}",
                path.display()
            )));
        }
    }
    let readback = fs::read(&path).map_err(|error| {
        CliError::io(format!(
            "read back spectral community report {}: {error}",
            path.display()
        ))
    })?;
    if readback != bytes || sha256_hex(&readback) != sha256_hex(&bytes) {
        return Err(CliError::runtime(format!(
            "spectral community report physical readback mismatch at {}",
            path.display()
        )));
    }
    let decoded: SpectralCommunityReport = serde_json::from_slice(&readback).map_err(|error| {
        CliError::runtime(format!(
            "parse spectral community report {}: {error}",
            path.display()
        ))
    })?;
    Ok(PersistedReport {
        path,
        bytes: readback.len() as u64,
        sha256: sha256_hex(&readback),
        readback_node_count: decoded.node_count,
        readback_member_count: decoded.members.len(),
        readback_degenerate_node_count: decoded.degenerate_node_count,
        readback_community_count: decoded.communities.len(),
        readback_bridge_candidate_count: decoded.bridge_candidates.len(),
        readback_centrality_candidate_count: decoded.centrality_candidates.len(),
    })
}

fn parse_usize(raw: &str, flag: &str, min: usize) -> CliResult<usize> {
    let value = raw
        .parse::<usize>()
        .map_err(|err| CliError::usage(format!("parse {flag} {raw}: {err}")))?;
    if value < min {
        return Err(CliError::usage(format!("{flag} must be >= {min}")));
    }
    Ok(value)
}

fn parse_positive_f32(raw: &str, flag: &str) -> CliResult<f32> {
    let value = raw
        .parse::<f32>()
        .map_err(|err| CliError::usage(format!("parse {flag} {raw}: {err}")))?;
    if !value.is_finite() || value <= 0.0 {
        return Err(CliError::usage(format!(
            "{flag} must be finite and greater than 0"
        )));
    }
    Ok(value)
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex_lower(&Sha256::digest(bytes))
}

fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests;
