use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::json;

const SCHEMA_VERSION: u64 = 1;
const SURFACE: &str = "ph71-issue612-fsv";
const SOURCE_OF_TRUTH: &str = "PH71 flipped-read latency and control-plane pg_dump artifact";
const LATENCY_REGRESSION_LIMIT: f64 = 1.05;
pub(crate) const REQUIRED_TABLES: &[&str] = &[
    "creator_databases",
    "queries",
    "billing",
    "marketplace",
    "outbox",
];

#[derive(Clone, Debug)]
struct Args {
    baseline_latency: PathBuf,
    flipped_latency: PathBuf,
    pg_before: PathBuf,
    pg_after: PathBuf,
    out: PathBuf,
}

#[derive(Clone, Debug, Deserialize)]
struct LatencySamples {
    path: String,
    samples_us: Vec<u64>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
struct Issue612Evidence {
    schema_version: u64,
    surface: &'static str,
    source_of_truth: &'static str,
    latency: LatencyProof,
    control_plane: ControlPlaneProof,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
struct LatencyProof {
    baseline_path: String,
    flipped_path: String,
    baseline_sample_count: usize,
    flipped_sample_count: usize,
    baseline_p99_us: u64,
    flipped_p99_us: u64,
    max_allowed_flipped_p99_us: f64,
    non_regression: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
struct ControlPlaneProof {
    required_tables: Vec<&'static str>,
    matched_tables: usize,
    all_hashes_match: bool,
    tables: Vec<TableProof>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
struct TableProof {
    table: String,
    before_path: String,
    after_path: String,
    before_len: usize,
    after_len: usize,
    before_blake3: String,
    after_blake3: String,
    bytes_identical: bool,
}

pub(crate) fn run(args: &[String]) -> crate::error::CliResult {
    let args = parse_args(args)?;
    let evidence = build_evidence(&args)?;
    let bytes = serde_json::to_vec_pretty(&evidence).map_err(|error| error.to_string())?;
    if let Some(parent) = args.out.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("create {}: {error}", parent.display()))?;
    }
    fs::write(&args.out, &bytes)
        .map_err(|error| format!("write {}: {error}", args.out.display()))?;
    let readback = json!({
        "surface": SURFACE,
        "artifact": display_path(&args.out),
        "artifact_len": bytes.len(),
        "artifact_blake3": blake3::hash(&bytes).to_string(),
        "latency": evidence.latency,
        "control_plane": evidence.control_plane,
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&readback).map_err(|error| error.to_string())?
    );
    Ok(())
}

fn build_evidence(args: &Args) -> Result<Issue612Evidence, String> {
    let latency = latency_proof(
        &read_latency(&args.baseline_latency)?,
        &read_latency(&args.flipped_latency)?,
    )?;
    let control_plane = control_plane_proof(&args.pg_before, &args.pg_after)?;
    Ok(Issue612Evidence {
        schema_version: SCHEMA_VERSION,
        surface: SURFACE,
        source_of_truth: SOURCE_OF_TRUTH,
        latency,
        control_plane,
    })
}

fn latency_proof(
    baseline: &LatencySamples,
    flipped: &LatencySamples,
) -> Result<LatencyProof, String> {
    if baseline.samples_us.is_empty() {
        return Err("CALYX_LATENCY_SAMPLE_EMPTY baseline has zero samples".to_string());
    }
    if flipped.samples_us.is_empty() {
        return Err("CALYX_LATENCY_SAMPLE_EMPTY flipped path has zero samples".to_string());
    }
    let baseline_p99 = p99(&baseline.samples_us);
    let flipped_p99 = p99(&flipped.samples_us);
    let max_allowed = baseline_p99 as f64 * LATENCY_REGRESSION_LIMIT;
    if flipped_p99 as f64 > max_allowed {
        return Err(format!(
            "CALYX_LATENCY_REGRESSION flipped_p99_us={flipped_p99} baseline_p99_us={baseline_p99} max_allowed={max_allowed:.2}"
        ));
    }
    Ok(LatencyProof {
        baseline_path: baseline.path.clone(),
        flipped_path: flipped.path.clone(),
        baseline_sample_count: baseline.samples_us.len(),
        flipped_sample_count: flipped.samples_us.len(),
        baseline_p99_us: baseline_p99,
        flipped_p99_us: flipped_p99,
        max_allowed_flipped_p99_us: max_allowed,
        non_regression: true,
    })
}

fn control_plane_proof(before: &Path, after: &Path) -> Result<ControlPlaneProof, String> {
    let mut tables = Vec::with_capacity(REQUIRED_TABLES.len());
    for table in REQUIRED_TABLES {
        let before_path = before.join(format!("{table}.dump"));
        let after_path = after.join(format!("{table}.dump"));
        if !before_path.exists() || !after_path.exists() {
            return Err(format!(
                "CALYX_PG_SNAPSHOT_INCOMPLETE table={table} before_exists={} after_exists={}",
                before_path.exists(),
                after_path.exists()
            ));
        }
        let before_bytes = fs::read(&before_path)
            .map_err(|error| format!("read {}: {error}", before_path.display()))?;
        let after_bytes = fs::read(&after_path)
            .map_err(|error| format!("read {}: {error}", after_path.display()))?;
        let before_hash = blake3::hash(&before_bytes).to_string();
        let after_hash = blake3::hash(&after_bytes).to_string();
        let bytes_identical = before_bytes == after_bytes;
        if !bytes_identical {
            return Err(format!(
                "CALYX_PG_STATE_CHANGED table={table} before_blake3={before_hash} after_blake3={after_hash}"
            ));
        }
        tables.push(TableProof {
            table: (*table).to_string(),
            before_path: display_path(&before_path),
            after_path: display_path(&after_path),
            before_len: before_bytes.len(),
            after_len: after_bytes.len(),
            before_blake3: before_hash,
            after_blake3: after_hash,
            bytes_identical,
        });
    }
    Ok(ControlPlaneProof {
        required_tables: REQUIRED_TABLES.to_vec(),
        matched_tables: tables.len(),
        all_hashes_match: true,
        tables,
    })
}

fn read_latency(path: &Path) -> Result<LatencySamples, String> {
    let bytes = fs::read(path).map_err(|error| format!("read {}: {error}", path.display()))?;
    serde_json::from_slice(&bytes).map_err(|error| format!("parse {}: {error}", path.display()))
}

fn p99(samples: &[u64]) -> u64 {
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    let rank = (sorted.len() as f64 * 0.99).ceil() as usize;
    let index = rank.saturating_sub(1).min(sorted.len() - 1);
    sorted[index]
}

fn parse_args(args: &[String]) -> Result<Args, String> {
    let mut parsed = ParsedArgs::default();
    let mut i = 0;
    while i < args.len() {
        let flag = args[i].as_str();
        i += 1;
        let value = args
            .get(i)
            .ok_or_else(|| format!("leapable issue612-fsv missing value for {flag}"))?;
        match flag {
            "--baseline-latency" => parsed.baseline_latency = Some(PathBuf::from(value)),
            "--flipped-latency" => parsed.flipped_latency = Some(PathBuf::from(value)),
            "--pg-before" => parsed.pg_before = Some(PathBuf::from(value)),
            "--pg-after" => parsed.pg_after = Some(PathBuf::from(value)),
            "--out" => parsed.out = Some(PathBuf::from(value)),
            other => return Err(format!("unknown leapable issue612-fsv arg: {other}")),
        }
        i += 1;
    }
    Ok(Args {
        baseline_latency: parsed.baseline_latency.ok_or_else(|| {
            "leapable issue612-fsv requires --baseline-latency <json>".to_string()
        })?,
        flipped_latency: parsed
            .flipped_latency
            .ok_or_else(|| "leapable issue612-fsv requires --flipped-latency <json>".to_string())?,
        pg_before: parsed
            .pg_before
            .ok_or_else(|| "leapable issue612-fsv requires --pg-before <dir>".to_string())?,
        pg_after: parsed
            .pg_after
            .ok_or_else(|| "leapable issue612-fsv requires --pg-after <dir>".to_string())?,
        out: parsed
            .out
            .ok_or_else(|| "leapable issue612-fsv requires --out <json>".to_string())?,
    })
}

#[derive(Default)]
struct ParsedArgs {
    baseline_latency: Option<PathBuf>,
    flipped_latency: Option<PathBuf>,
    pg_before: Option<PathBuf>,
    pg_after: Option<PathBuf>,
    out: Option<PathBuf>,
}

fn display_path(path: &Path) -> String {
    path.canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .display()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn p99_uses_nearest_rank() {
        let samples: Vec<_> = (1..=100).collect();
        assert_eq!(p99(&samples), 99);
    }
}
