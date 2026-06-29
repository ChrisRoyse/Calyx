//! `calyx discovery-chain <vault>` -- run the physical gated discovery harness (#878).

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use calyx_core::CxId;
use calyx_lodestar::{
    AssocStore, DEFAULT_ASTER_ASSOC_COLLECTION, DiscoveryChainLog, DiscoveryChainParams,
    LodestarError, PhysicalAsterAssocSnapshot, run_grounded_discovery_chain,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};

use super::vault::{home_dir, resolve_vault_info};
use super::{Subcommand, value};
use crate::error::{CliError, CliResult};
use crate::output::print_json;

const DISCOVERY_CHAIN_ARTIFACT_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct DiscoveryChainArgs {
    pub vault: String,
    pub starts: Vec<CxId>,
    pub anchors: Vec<CxId>,
    pub anchor_files: Vec<PathBuf>,
    pub max_hops: usize,
    pub branch_width: usize,
    pub probe_width: usize,
    pub max_groundedness_distance: usize,
    pub min_gate_confidence: f32,
    pub novelty_weight: f32,
    pub out: Option<PathBuf>,
}

impl Default for DiscoveryChainArgs {
    fn default() -> Self {
        let params = DiscoveryChainParams::default();
        Self {
            vault: String::new(),
            starts: Vec::new(),
            anchors: Vec::new(),
            anchor_files: Vec::new(),
            max_hops: params.max_hops,
            branch_width: params.branch_width,
            probe_width: params.probe_width,
            max_groundedness_distance: params.max_groundedness_distance,
            min_gate_confidence: params.min_gate_confidence,
            novelty_weight: params.novelty_weight,
            out: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct DiscoveryChainArtifact {
    schema_version: u32,
    graph_node_count: usize,
    graph_edge_count: usize,
    node_metadata: BTreeMap<CxId, BTreeMap<String, String>>,
    log: DiscoveryChainLog,
}

struct PersistedChain {
    path: PathBuf,
    bytes: u64,
    sha256: String,
    readback_accepted_hop_count: usize,
    readback_candidate_count: usize,
    readback_gate_pass_count: usize,
    readback_refused_count: usize,
}

pub(crate) fn run(command: Subcommand) -> CliResult {
    let Subcommand::DiscoveryChain(args) = command else {
        unreachable!("non-discovery-chain command routed to discovery_chain module");
    };
    run_discovery_chain_with_home(&home_dir()?, args)
}

pub(crate) fn run_discovery_chain_with_home(home: &Path, args: DiscoveryChainArgs) -> CliResult {
    let started = Instant::now();
    let resolved = resolve_vault_info(home, &args.vault)?;
    eprintln!(
        "discovery-chain: opening physical graph name={} id={} path={}",
        resolved.name,
        resolved.vault_id,
        resolved.path.display()
    );
    let store = PhysicalAsterAssocSnapshot::latest(&resolved.path, DEFAULT_ASTER_ASSOC_COLLECTION)?;
    let graph = store.full_graph()?;
    let anchors = load_effective_anchors(&args)?;
    let params = DiscoveryChainParams {
        max_hops: args.max_hops,
        branch_width: args.branch_width,
        probe_width: args.probe_width,
        max_groundedness_distance: args.max_groundedness_distance,
        min_gate_confidence: args.min_gate_confidence,
        novelty_weight: args.novelty_weight,
    };
    eprintln!(
        "discovery-chain: running nodes={} edges={} starts={} anchors={} max_hops={} branch_width={} probe_width={} rayon_threads={}",
        graph.node_count(),
        graph.edge_count(),
        args.starts.len(),
        anchors.len(),
        params.max_hops,
        params.branch_width,
        params.probe_width,
        rayon::current_num_threads()
    );
    let log = run_grounded_discovery_chain(&graph, &args.starts, &anchors, &params)?;
    ensure_useful_chain(&log)?;
    let artifact = DiscoveryChainArtifact {
        schema_version: DISCOVERY_CHAIN_ARTIFACT_SCHEMA_VERSION,
        graph_node_count: graph.node_count(),
        graph_edge_count: graph.edge_count(),
        node_metadata: collect_node_metadata(&store, &log)?,
        log,
    };
    let persisted = persist_chain(&resolved.path, args.out.as_deref(), &artifact)?;
    eprintln!(
        "discovery-chain: persisted chain={} bytes={} sha256={} elapsed_ms={}",
        persisted.path.display(),
        persisted.bytes,
        persisted.sha256,
        started.elapsed().as_millis()
    );
    print_json(&json!({
        "status": "ok",
        "vault": resolved.name,
        "vault_dir": resolved.path.display().to_string(),
        "params": params,
        "anchor_files": args.anchor_files,
        "chain": artifact,
        "artifacts": {
            "chain_json": persisted.path,
            "chain_json_bytes": persisted.bytes,
            "chain_json_sha256": persisted.sha256,
            "readback": {
                "accepted_hop_count": persisted.readback_accepted_hop_count,
                "candidate_count": persisted.readback_candidate_count,
                "gate_pass_count": persisted.readback_gate_pass_count,
                "refused_count": persisted.readback_refused_count,
            }
        }
    }))
}

pub(crate) fn parse_discovery_chain(rest: &[String]) -> CliResult<Subcommand> {
    let vault = rest
        .first()
        .ok_or_else(|| CliError::usage("discovery-chain requires <vault>"))?
        .clone();
    let mut args = DiscoveryChainArgs {
        vault,
        ..DiscoveryChainArgs::default()
    };
    let mut idx = 1;
    while idx < rest.len() {
        match rest[idx].as_str() {
            "--start" => {
                idx += 1;
                args.starts
                    .push(parse_cx_id(value(rest, idx, "--start")?, "--start")?);
            }
            "--anchor" => {
                idx += 1;
                args.anchors
                    .push(parse_cx_id(value(rest, idx, "--anchor")?, "--anchor")?);
            }
            "--anchor-file" => {
                idx += 1;
                args.anchor_files
                    .push(PathBuf::from(value(rest, idx, "--anchor-file")?));
            }
            "--max-hops" => {
                idx += 1;
                args.max_hops = parse_usize(value(rest, idx, "--max-hops")?, "--max-hops", 1)?;
            }
            "--branch-width" => {
                idx += 1;
                args.branch_width =
                    parse_usize(value(rest, idx, "--branch-width")?, "--branch-width", 1)?;
            }
            "--probe-width" => {
                idx += 1;
                args.probe_width =
                    parse_usize(value(rest, idx, "--probe-width")?, "--probe-width", 1)?;
            }
            "--max-groundedness-distance" => {
                idx += 1;
                args.max_groundedness_distance = value(rest, idx, "--max-groundedness-distance")?
                    .parse::<usize>()
                    .map_err(|err| {
                        CliError::usage(format!(
                            "parse --max-groundedness-distance {}: {err}",
                            rest[idx]
                        ))
                    })?;
            }
            "--min-gate-confidence" => {
                idx += 1;
                args.min_gate_confidence = parse_unit(
                    value(rest, idx, "--min-gate-confidence")?,
                    "--min-gate-confidence",
                )?;
            }
            "--novelty-weight" => {
                idx += 1;
                args.novelty_weight =
                    parse_unit(value(rest, idx, "--novelty-weight")?, "--novelty-weight")?;
            }
            "--out" => {
                idx += 1;
                args.out = Some(value(rest, idx, "--out")?.into());
            }
            other => {
                return Err(CliError::usage(format!(
                    "unexpected discovery-chain flag {other}"
                )));
            }
        }
        idx += 1;
    }
    if args.starts.is_empty() {
        return Err(CliError::usage(
            "discovery-chain requires at least one --start <cxid>",
        ));
    }
    if args.anchors.is_empty() && args.anchor_files.is_empty() {
        return Err(CliError::usage(
            "discovery-chain requires at least one --anchor <cxid> or --anchor-file <path>",
        ));
    }
    Ok(Subcommand::DiscoveryChain(args))
}

fn load_effective_anchors(args: &DiscoveryChainArgs) -> CliResult<Vec<CxId>> {
    let mut ids = args.anchors.clone();
    for path in &args.anchor_files {
        ids.extend(load_anchor_file(path)?);
    }
    let unique = ids.into_iter().collect::<BTreeSet<_>>();
    if unique.is_empty() {
        return Err(CliError::usage(
            "discovery-chain resolved zero anchors from --anchor/--anchor-file",
        ));
    }
    Ok(unique.into_iter().collect())
}

fn load_anchor_file(path: &Path) -> CliResult<Vec<CxId>> {
    let text = fs::read_to_string(path)
        .map_err(|error| CliError::io(format!("read --anchor-file {}: {error}", path.display())))?;
    let mut ids = Vec::new();
    for (line_no, line) in text.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let label = format!("--anchor-file {}:{}", path.display(), line_no + 1);
        ids.push(parse_cx_id(trimmed, &label)?);
    }
    if ids.is_empty() {
        return Err(CliError::usage(format!(
            "--anchor-file {} did not contain any CxId rows",
            path.display()
        )));
    }
    Ok(ids)
}

fn ensure_useful_chain(log: &DiscoveryChainLog) -> CliResult {
    if log.candidates.is_empty() {
        return Err(LodestarError::KernelInvalidParams {
            detail: "discovery chain produced no evaluated candidates".to_string(),
        }
        .into());
    }
    if log.gate_pass_count == 0 || log.accepted_hops.is_empty() {
        return Err(LodestarError::KernelInvalidParams {
            detail: format!(
                "discovery chain had no accepted gate-PASS hops; refused_count={} termination={:?}",
                log.refused_count, log.terminated
            ),
        }
        .into());
    }
    Ok(())
}

fn collect_node_metadata(
    store: &PhysicalAsterAssocSnapshot,
    log: &DiscoveryChainLog,
) -> CliResult<BTreeMap<CxId, BTreeMap<String, String>>> {
    let mut ids = BTreeSet::new();
    ids.extend(log.starts.iter().copied());
    for row in &log.candidates {
        ids.insert(row.candidate.from);
        ids.insert(row.candidate.to);
    }
    for hop in &log.accepted_hops {
        ids.insert(hop.from);
        ids.insert(hop.to);
        ids.extend(hop.path.iter().copied());
    }
    let mut out = BTreeMap::new();
    for id in ids {
        out.insert(id, store.node_props(id)?.metadata.clone());
    }
    Ok(out)
}

fn persist_chain(
    vault_dir: &Path,
    explicit: Option<&Path>,
    artifact: &DiscoveryChainArtifact,
) -> CliResult<PersistedChain> {
    let bytes = serde_json::to_vec_pretty(artifact)?;
    let chain_id = blake3::hash(&bytes).to_hex().to_string();
    let path = explicit.map(Path::to_path_buf).unwrap_or_else(|| {
        vault_dir
            .join("idx")
            .join("discovery_chains")
            .join(chain_id)
            .join("chain.json")
    });
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    if path.exists() {
        let existing = fs::read(&path)?;
        if existing != bytes {
            return Err(CliError::usage(format!(
                "refusing to overwrite existing different discovery chain {}",
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
            "discovery chain readback mismatch at {}",
            path.display()
        )));
    }
    let decoded: DiscoveryChainArtifact = serde_json::from_slice(&readback)?;
    Ok(PersistedChain {
        path,
        bytes: readback.len() as u64,
        sha256: sha256_hex(&readback),
        readback_accepted_hop_count: decoded.log.accepted_hops.len(),
        readback_candidate_count: decoded.log.candidates.len(),
        readback_gate_pass_count: decoded.log.gate_pass_count,
        readback_refused_count: decoded.log.refused_count,
    })
}

fn parse_cx_id(raw: &str, flag: &str) -> CliResult<CxId> {
    raw.parse::<CxId>()
        .map_err(|error| CliError::usage(format!("parse {flag} {raw}: {error}")))
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

fn parse_unit(raw: &str, flag: &str) -> CliResult<f32> {
    let value = raw
        .parse::<f32>()
        .map_err(|err| CliError::usage(format!("parse {flag} {raw}: {err}")))?;
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        return Err(CliError::usage(format!(
            "{flag} must be finite and in [0,1]"
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
