//! `calyx spectral-communities <vault>` -- run physical spectral community mining (#877).

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use calyx_lodestar::{
    AssocStore, DEFAULT_ASTER_ASSOC_COLLECTION, LodestarError, PhysicalAsterAssocSnapshot,
    SpectralCommunityParams, SpectralCommunityReport, spectral_community_report,
};
use serde_json::json;
use sha2::{Digest, Sha256};

use super::vault::{home_dir, resolve_vault_info};
use super::{Subcommand, value};
use crate::error::{CliError, CliResult};
use crate::output::print_json;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SpectralCommunitiesArgs {
    pub vault: String,
    pub eigen_k: usize,
    pub eigen_max_iter: usize,
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
            eigen_k: params.eigen_k,
            eigen_max_iter: params.eigen_max_iter,
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
    let started = Instant::now();
    let resolved = resolve_vault_info(home, &args.vault)?;
    eprintln!(
        "spectral-communities: opening physical graph name={} id={} path={}",
        resolved.name,
        resolved.vault_id,
        resolved.path.display()
    );
    let store = PhysicalAsterAssocSnapshot::latest(&resolved.path, DEFAULT_ASTER_ASSOC_COLLECTION)?;
    let graph = store.full_graph()?;
    eprintln!(
        "spectral-communities: computing report nodes={} edges={} eigen_k={} eigen_max_iter={} rayon_threads={}",
        graph.node_count(),
        graph.edge_count(),
        args.eigen_k,
        args.eigen_max_iter,
        rayon::current_num_threads()
    );
    let report = spectral_community_report(
        &graph,
        &SpectralCommunityParams {
            eigen_k: args.eigen_k,
            eigen_max_iter: args.eigen_max_iter,
            centrality_max_iter: args.centrality_max_iter,
            centrality_tol: args.centrality_tol,
            max_bridge_candidates: args.max_bridge_candidates,
            max_centrality_candidates: args.max_centrality_candidates,
        },
    )?;
    ensure_useful_report(&report)?;
    let persisted = persist_report(&resolved.path, args.out.as_deref(), &report)?;
    eprintln!(
        "spectral-communities: persisted report={} bytes={} sha256={} elapsed_ms={}",
        persisted.path.display(),
        persisted.bytes,
        persisted.sha256,
        started.elapsed().as_millis()
    );
    print_json(&json!({
        "status": "ok",
        "vault": resolved.name,
        "vault_dir": resolved.path.display().to_string(),
        "params": {
            "eigen_k": args.eigen_k,
            "eigen_max_iter": args.eigen_max_iter,
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
                "community_count": persisted.readback_community_count,
                "bridge_candidate_count": persisted.readback_bridge_candidate_count,
                "centrality_candidate_count": persisted.readback_centrality_candidate_count,
            }
        }
    }))
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
    Ok(Subcommand::SpectralCommunities(args))
}

fn ensure_useful_report(report: &SpectralCommunityReport) -> CliResult {
    if report.communities.len() < 2 {
        return Err(LodestarError::KernelInvalidParams {
            detail: "spectral report produced fewer than two communities".to_string(),
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
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    if path.exists() {
        let existing = fs::read(&path)?;
        if existing != bytes {
            return Err(CliError::usage(format!(
                "refusing to overwrite existing different spectral community report {}",
                path.display()
            )));
        }
    } else {
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, &bytes)?;
        fs::rename(&tmp, &path)?;
    }
    let readback = fs::read(&path)?;
    if readback != bytes {
        return Err(CliError::usage(format!(
            "spectral community report readback mismatch at {}",
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
