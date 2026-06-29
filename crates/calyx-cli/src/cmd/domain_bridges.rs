//! `calyx domain-bridges <vault>` — run scoped-kernel bridge mining (#876).

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use calyx_core::AnchorKind;
use calyx_lodestar::{
    DEFAULT_ASTER_ASSOC_COLLECTION, DomainBridgeMiningParams, DomainBridgeParams,
    DomainBridgeReport, DomainBridgeScopePair, DomainPair, FilterExpr, KernelParams,
    PhysicalAsterAssocSnapshot, Scope, mine_domain_bridges,
};
use serde_json::json;
use sha2::{Digest, Sha256};

use super::ingest::parse_anchor_kind;
use super::vault::{home_dir, now_ms, resolve_vault_info};
use super::{Subcommand, value};
use crate::error::{CliError, CliResult};
use crate::output::print_json;

const DEFAULT_SCOPE_RADIUS: usize = 2;
const DEFAULT_KERNEL_TARGET_FRACTION: f32 = 0.10;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct DomainBridgesArgs {
    pub vault: String,
    pub pairs: Vec<(String, String)>,
    pub anchor_kind: Option<AnchorKind>,
    pub min_gate_confidence: f32,
    pub max_per_pair: usize,
    pub max_evidence_hops: usize,
    pub scope_radius: usize,
    pub kernel_target_fraction: f32,
    pub out: Option<PathBuf>,
}

impl Default for DomainBridgesArgs {
    fn default() -> Self {
        Self {
            vault: String::new(),
            pairs: Vec::new(),
            anchor_kind: None,
            min_gate_confidence: DomainBridgeParams::default().min_gate_confidence,
            max_per_pair: DomainBridgeParams::default().max_per_pair,
            max_evidence_hops: DomainBridgeParams::default().max_evidence_hops,
            scope_radius: DEFAULT_SCOPE_RADIUS,
            kernel_target_fraction: DEFAULT_KERNEL_TARGET_FRACTION,
            out: None,
        }
    }
}

struct PersistedReport {
    path: PathBuf,
    bytes: u64,
    sha256: String,
    readback_pair_count: usize,
    readback_candidate_count: usize,
}

pub(crate) fn run(command: Subcommand) -> CliResult {
    let Subcommand::DomainBridges(args) = command else {
        unreachable!("non-domain-bridges command routed to domain_bridges module");
    };
    run_domain_bridges_with_home(&home_dir()?, args)
}

pub(crate) fn run_domain_bridges_with_home(home: &Path, args: DomainBridgesArgs) -> CliResult {
    let started = Instant::now();
    let resolved = resolve_vault_info(home, &args.vault)?;
    eprintln!(
        "domain-bridges: opening physical graph name={} id={} path={}",
        resolved.name,
        resolved.vault_id,
        resolved.path.display()
    );
    let store = PhysicalAsterAssocSnapshot::latest(&resolved.path, DEFAULT_ASTER_ASSOC_COLLECTION)?;
    let pairs = args
        .pairs
        .iter()
        .map(|(left, right)| pair_from_args(left, right, args.scope_radius))
        .collect::<CliResult<Vec<_>>>()?;
    let mut kernel = KernelParams {
        panel_version: 876,
        anchor_kind: args.anchor_kind.as_ref().map(anchor_kind_name),
        built_at_millis: now_ms(),
        ..KernelParams::default()
    };
    kernel.kernel_graph.target_fraction = args.kernel_target_fraction;
    let params = DomainBridgeMiningParams {
        ranking: DomainBridgeParams {
            min_gate_confidence: args.min_gate_confidence,
            max_per_pair: args.max_per_pair,
            max_evidence_hops: args.max_evidence_hops,
        },
        kernel,
        anchor_kind: args.anchor_kind.clone(),
    };
    let report = mine_domain_bridges(&store, &pairs, &params)?;
    let persisted = persist_report(&resolved.path, args.out.as_deref(), &report)?;
    eprintln!(
        "domain-bridges: persisted report={} bytes={} sha256={} elapsed_ms={}",
        persisted.path.display(),
        persisted.bytes,
        persisted.sha256,
        started.elapsed().as_millis()
    );
    print_json(&json!({
        "status": "ok",
        "vault": resolved.name,
        "vault_dir": resolved.path.display().to_string(),
        "pairs_requested": pairs.len(),
        "anchor_kind": args.anchor_kind.as_ref().map(anchor_kind_name),
        "scope_radius": args.scope_radius,
        "max_evidence_hops": args.max_evidence_hops,
        "kernel_target_fraction": args.kernel_target_fraction,
        "report": report,
        "artifacts": {
            "report_json": persisted.path,
            "report_json_bytes": persisted.bytes,
            "report_json_sha256": persisted.sha256,
            "readback": {
                "pair_count": persisted.readback_pair_count,
                "candidate_count": persisted.readback_candidate_count,
            }
        }
    }))
}

pub(crate) fn parse_domain_bridges(rest: &[String]) -> CliResult<Subcommand> {
    let vault = rest
        .first()
        .ok_or_else(|| CliError::usage("domain-bridges requires <vault>"))?
        .clone();
    let mut args = DomainBridgesArgs {
        vault,
        ..DomainBridgesArgs::default()
    };
    let mut idx = 1;
    while idx < rest.len() {
        match rest[idx].as_str() {
            "--pair" => {
                let left = value(rest, idx + 1, "--pair <left> <right>")?.to_string();
                let right = value(rest, idx + 2, "--pair <left> <right>")?.to_string();
                args.pairs.push((left, right));
                idx += 2;
            }
            "--anchor-kind" => {
                idx += 1;
                args.anchor_kind = Some(parse_anchor_kind(value(rest, idx, "--anchor-kind")?)?);
            }
            "--min-gate-confidence" => {
                idx += 1;
                args.min_gate_confidence = parse_unit(
                    value(rest, idx, "--min-gate-confidence")?,
                    "--min-gate-confidence",
                )?;
            }
            "--max-per-pair" => {
                idx += 1;
                args.max_per_pair =
                    parse_usize(value(rest, idx, "--max-per-pair")?, "--max-per-pair", 1)?;
            }
            "--max-evidence-hops" => {
                idx += 1;
                args.max_evidence_hops = parse_usize(
                    value(rest, idx, "--max-evidence-hops")?,
                    "--max-evidence-hops",
                    1,
                )?;
            }
            "--scope-radius" => {
                idx += 1;
                args.scope_radius =
                    parse_usize(value(rest, idx, "--scope-radius")?, "--scope-radius", 1)?;
            }
            "--kernel-target-fraction" => {
                idx += 1;
                args.kernel_target_fraction = parse_unit(
                    value(rest, idx, "--kernel-target-fraction")?,
                    "--kernel-target-fraction",
                )?;
                if args.kernel_target_fraction == 0.0 {
                    return Err(CliError::usage("--kernel-target-fraction must be > 0"));
                }
            }
            "--out" => {
                idx += 1;
                args.out = Some(value(rest, idx, "--out")?.into());
            }
            other => {
                return Err(CliError::usage(format!(
                    "unexpected domain-bridges flag {other}"
                )));
            }
        }
        idx += 1;
    }
    if args.pairs.is_empty() {
        return Err(CliError::usage(
            "domain-bridges requires at least one --pair <left-scope> <right-scope>",
        ));
    }
    Ok(Subcommand::DomainBridges(args))
}

fn pair_from_args(left: &str, right: &str, radius: usize) -> CliResult<DomainBridgeScopePair> {
    Ok(DomainBridgeScopePair {
        pair: DomainPair {
            left: left.to_string(),
            right: right.to_string(),
        },
        left_scope: parse_scope(left, radius)?,
        right_scope: parse_scope(right, radius)?,
    })
}

fn parse_scope(raw: &str, radius: usize) -> CliResult<Scope> {
    if raw == "all" {
        return Ok(Scope::AllAssociations);
    }
    if raw.starts_with("label:") {
        return Ok(Scope::Domain {
            anchor_kind: parse_anchor_kind(raw)?,
        });
    }
    if let Some(kind) = raw.strip_prefix("anchor:") {
        return Ok(Scope::Domain {
            anchor_kind: parse_anchor_kind(kind)?,
        });
    }
    if let Some(spec) = raw.strip_prefix("metadata:") {
        let (key, value) = spec.split_once('=').ok_or_else(|| {
            CliError::usage(format!(
                "metadata scope {raw} must be metadata:<key>=<value>"
            ))
        })?;
        require_nonempty(key, "metadata key")?;
        require_nonempty(value, "metadata value")?;
        return Ok(Scope::FilterReachable {
            expr: FilterExpr::MetadataEq {
                key: key.to_string(),
                value: value.to_string(),
            },
            radius,
        });
    }
    if let Some(name) = raw
        .strip_prefix("filter:")
        .or_else(|| raw.strip_prefix("named:"))
    {
        require_nonempty(name, "filter name")?;
        return Ok(Scope::FilterReachable {
            expr: FilterExpr::Named {
                name: name.to_string(),
            },
            radius,
        });
    }
    Err(CliError::usage(format!(
        "unknown domain scope {raw}; use all, label:<name>, anchor:<kind>, metadata:<key>=<value>, or filter:<name>"
    )))
}

fn persist_report(
    vault_dir: &Path,
    explicit: Option<&Path>,
    report: &DomainBridgeReport,
) -> CliResult<PersistedReport> {
    let bytes = serde_json::to_vec_pretty(report)?;
    let report_id = blake3::hash(&bytes).to_hex().to_string();
    let path = explicit.map(Path::to_path_buf).unwrap_or_else(|| {
        vault_dir
            .join("idx")
            .join("domain_bridges")
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
                "refusing to overwrite existing different domain bridge report {}",
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
            "domain bridge report readback mismatch at {}",
            path.display()
        )));
    }
    let decoded: DomainBridgeReport = serde_json::from_slice(&readback)?;
    let candidate_count = decoded
        .pair_reports
        .iter()
        .map(|pair| pair.candidate_count)
        .sum();
    Ok(PersistedReport {
        path,
        bytes: readback.len() as u64,
        sha256: sha256_hex(&readback),
        readback_pair_count: decoded.pair_reports.len(),
        readback_candidate_count: candidate_count,
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

fn require_nonempty(value: &str, label: &str) -> CliResult {
    if value.trim().is_empty() {
        Err(CliError::usage(format!("{label} must not be empty")))
    } else {
        Ok(())
    }
}

fn anchor_kind_name(kind: &AnchorKind) -> String {
    match kind {
        AnchorKind::Label(value) => format!("label:{value}"),
        AnchorKind::TestPass => "test-pass".to_string(),
        AnchorKind::TieFormed => "tie-formed".to_string(),
        AnchorKind::Thumbs => "thumbs".to_string(),
        AnchorKind::Reward => "reward".to_string(),
        AnchorKind::SpeakerMatch => "speaker-match".to_string(),
        AnchorKind::StyleHold => "style-hold".to_string(),
        AnchorKind::Recurrence => "recurrence".to_string(),
    }
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
