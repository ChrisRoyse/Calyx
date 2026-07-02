//! `calyx chain-walks <vault>` -- run grounded chain-walk reports (#880).

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use calyx_core::CxId;
use calyx_lodestar::{
    AssocStore, ChainWalkParams, ChainWalkReport, ChainWalkSeed, DEFAULT_ASTER_ASSOC_COLLECTION,
    DiscoveryChainParams, LodestarError, PhysicalAsterAssocSnapshot, run_grounded_chain_walks,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};

use super::vault::{home_dir, resolve_vault_info};
use super::{Subcommand, value};
use crate::error::{CliError, CliResult};
use crate::output::print_json;

const CHAIN_WALKS_ARTIFACT_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ChainWalksArgs {
    pub vault: String,
    pub seed_file: PathBuf,
    pub anchors: Vec<CxId>,
    pub anchor_files: Vec<PathBuf>,
    pub max_hops: usize,
    pub branch_width: usize,
    pub probe_width: usize,
    pub max_groundedness_distance: usize,
    pub min_gate_confidence: f32,
    pub novelty_weight: f32,
    pub max_hypotheses_per_seed: usize,
    pub min_terminal_confidence: f32,
    pub out: Option<PathBuf>,
}

impl Default for ChainWalksArgs {
    fn default() -> Self {
        let chain = DiscoveryChainParams::default();
        let params = ChainWalkParams::default();
        Self {
            vault: String::new(),
            seed_file: PathBuf::new(),
            anchors: Vec::new(),
            anchor_files: Vec::new(),
            max_hops: chain.max_hops,
            branch_width: chain.branch_width,
            probe_width: chain.probe_width,
            max_groundedness_distance: chain.max_groundedness_distance,
            min_gate_confidence: chain.min_gate_confidence,
            novelty_weight: chain.novelty_weight,
            max_hypotheses_per_seed: params.max_hypotheses_per_seed,
            min_terminal_confidence: params.min_terminal_confidence,
            out: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct ChainWalksArtifact {
    schema_version: u32,
    graph_node_count: usize,
    graph_edge_count: usize,
    node_metadata: BTreeMap<CxId, BTreeMap<String, String>>,
    report: ChainWalkReport,
}

struct PersistedReport {
    path: PathBuf,
    bytes: u64,
    sha256: String,
    readback_seed_count: usize,
    readback_completed_chain_count: usize,
    readback_hypothesis_count: usize,
}

pub(crate) fn run(command: Subcommand) -> CliResult {
    let Subcommand::ChainWalks(args) = command else {
        unreachable!("non-chain-walks command routed to chain_walks module");
    };
    run_chain_walks_with_home(&home_dir()?, args)
}

pub(crate) fn run_chain_walks_with_home(home: &Path, args: ChainWalksArgs) -> CliResult {
    let started = Instant::now();
    let resolved = resolve_vault_info(home, &args.vault)?;
    eprintln!(
        "chain-walks: opening physical graph name={} id={} path={}",
        resolved.name,
        resolved.vault_id,
        resolved.path.display()
    );
    let store = PhysicalAsterAssocSnapshot::latest(&resolved.path, DEFAULT_ASTER_ASSOC_COLLECTION)?;
    let graph = store.full_graph()?;
    let seeds = load_seed_file(&args.seed_file)?;
    let anchors = load_effective_anchors(&args)?;
    let params = ChainWalkParams {
        chain: DiscoveryChainParams {
            max_hops: args.max_hops,
            branch_width: args.branch_width,
            probe_width: args.probe_width,
            max_groundedness_distance: args.max_groundedness_distance,
            min_gate_confidence: args.min_gate_confidence,
            novelty_weight: args.novelty_weight,
        },
        max_hypotheses_per_seed: args.max_hypotheses_per_seed,
        min_terminal_confidence: args.min_terminal_confidence,
    };
    eprintln!(
        "chain-walks: running nodes={} edges={} seeds={} anchors={} max_hops={} branch_width={} probe_width={} rayon_threads={}",
        graph.node_count(),
        graph.edge_count(),
        seeds.len(),
        anchors.len(),
        params.chain.max_hops,
        params.chain.branch_width,
        params.chain.probe_width,
        rayon::current_num_threads()
    );
    let report = run_grounded_chain_walks(&graph, &seeds, &anchors, &params)?;
    ensure_useful_report(&report)?;
    let artifact = ChainWalksArtifact {
        schema_version: CHAIN_WALKS_ARTIFACT_SCHEMA_VERSION,
        graph_node_count: graph.node_count(),
        graph_edge_count: graph.edge_count(),
        node_metadata: collect_node_metadata(&store, &report)?,
        report,
    };
    let persisted = persist_report(&resolved.path, args.out.as_deref(), &artifact)?;
    eprintln!(
        "chain-walks: persisted report={} bytes={} sha256={} elapsed_ms={}",
        persisted.path.display(),
        persisted.bytes,
        persisted.sha256,
        started.elapsed().as_millis()
    );
    print_json(&json!({
        "status": "ok",
        "vault": resolved.name,
        "vault_dir": resolved.path.display().to_string(),
        "seed_file": args.seed_file,
        "anchor_files": args.anchor_files,
        "params": params,
        "chain_walks": artifact,
        "artifacts": {
            "report_json": persisted.path,
            "report_json_bytes": persisted.bytes,
            "report_json_sha256": persisted.sha256,
            "readback": {
                "seed_count": persisted.readback_seed_count,
                "completed_chain_count": persisted.readback_completed_chain_count,
                "hypothesis_count": persisted.readback_hypothesis_count,
            }
        }
    }))
}

pub(crate) fn parse_chain_walks(rest: &[String]) -> CliResult<Subcommand> {
    let vault = rest
        .first()
        .ok_or_else(|| CliError::usage("chain-walks requires <vault>"))?
        .clone();
    let mut args = ChainWalksArgs {
        vault,
        ..ChainWalksArgs::default()
    };
    let mut idx = 1;
    while idx < rest.len() {
        match rest[idx].as_str() {
            "--seed-file" => {
                idx += 1;
                args.seed_file = PathBuf::from(value(rest, idx, "--seed-file")?);
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
            "--max-hypotheses-per-seed" => {
                idx += 1;
                args.max_hypotheses_per_seed = parse_usize(
                    value(rest, idx, "--max-hypotheses-per-seed")?,
                    "--max-hypotheses-per-seed",
                    1,
                )?;
            }
            "--min-terminal-confidence" => {
                idx += 1;
                args.min_terminal_confidence = parse_unit(
                    value(rest, idx, "--min-terminal-confidence")?,
                    "--min-terminal-confidence",
                )?;
            }
            "--out" => {
                idx += 1;
                args.out = Some(value(rest, idx, "--out")?.into());
            }
            other => {
                return Err(CliError::usage(format!(
                    "unexpected chain-walks flag {other}"
                )));
            }
        }
        idx += 1;
    }
    if args.seed_file.as_os_str().is_empty() {
        return Err(CliError::usage("chain-walks requires --seed-file <json>"));
    }
    if args.anchors.is_empty() && args.anchor_files.is_empty() {
        return Err(CliError::usage(
            "chain-walks requires at least one --anchor <cxid> or --anchor-file <path>",
        ));
    }
    Ok(Subcommand::ChainWalks(args))
}

fn load_seed_file(path: &Path) -> CliResult<Vec<ChainWalkSeed>> {
    let text = fs::read_to_string(path)
        .map_err(|error| CliError::io(format!("read --seed-file {}: {error}", path.display())))?;
    let seeds: Vec<ChainWalkSeed> = serde_json::from_str(&text)?;
    if seeds.is_empty() {
        return Err(CliError::usage(format!(
            "--seed-file {} did not contain any seeds",
            path.display()
        )));
    }
    Ok(seeds)
}

fn load_effective_anchors(args: &ChainWalksArgs) -> CliResult<Vec<CxId>> {
    let mut ids = args.anchors.clone();
    for path in &args.anchor_files {
        ids.extend(load_anchor_file(path)?);
    }
    let unique = ids.into_iter().collect::<BTreeSet<_>>();
    if unique.is_empty() {
        return Err(CliError::usage(
            "chain-walks resolved zero anchors from --anchor/--anchor-file",
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

fn ensure_useful_report(report: &ChainWalkReport) -> CliResult {
    if report.completed_chain_count == 0 || report.hypothesis_count == 0 {
        return Err(LodestarError::KernelInvalidParams {
            detail: format!(
                "chain-walk report produced no terminal A-B-C hypotheses; completed_chains={} seeds={}",
                report.completed_chain_count, report.seed_count
            ),
        }
        .into());
    }
    Ok(())
}

fn collect_node_metadata(
    store: &PhysicalAsterAssocSnapshot,
    report: &ChainWalkReport,
) -> CliResult<BTreeMap<CxId, BTreeMap<String, String>>> {
    let mut ids = BTreeSet::new();
    for result in &report.results {
        ids.insert(result.seed.start);
        for row in &result.log.candidates {
            ids.insert(row.candidate.from);
            ids.insert(row.candidate.to);
        }
        for hop in &result.log.accepted_hops {
            ids.insert(hop.from);
            ids.insert(hop.to);
            ids.extend(hop.path.iter().copied());
        }
        for hypothesis in &result.hypotheses {
            ids.insert(hypothesis.a);
            ids.insert(hypothesis.b);
            ids.insert(hypothesis.c);
            ids.extend(hypothesis.terminal_path.iter().copied());
        }
    }
    let mut out = BTreeMap::new();
    for id in ids {
        out.insert(id, store.node_props(id)?.metadata.clone());
    }
    Ok(out)
}

fn persist_report(
    vault_dir: &Path,
    explicit: Option<&Path>,
    artifact: &ChainWalksArtifact,
) -> CliResult<PersistedReport> {
    let bytes = serde_json::to_vec_pretty(artifact)?;
    let report_id = blake3::hash(&bytes).to_hex().to_string();
    let path = explicit.map(Path::to_path_buf).unwrap_or_else(|| {
        vault_dir
            .join("idx")
            .join("chain_walks")
            .join(report_id)
            .join("chain_walks.json")
    });
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    if path.exists() {
        let existing = fs::read(&path)?;
        if existing != bytes {
            return Err(CliError::usage(format!(
                "refusing to overwrite existing different chain-walk report {}",
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
            "chain-walk report readback mismatch at {}",
            path.display()
        )));
    }
    let decoded: ChainWalksArtifact = serde_json::from_slice(&readback)?;
    Ok(PersistedReport {
        path,
        bytes: readback.len() as u64,
        sha256: sha256_hex(&readback),
        readback_seed_count: decoded.report.seed_count,
        readback_completed_chain_count: decoded.report.completed_chain_count,
        readback_hypothesis_count: decoded.report.hypothesis_count,
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
