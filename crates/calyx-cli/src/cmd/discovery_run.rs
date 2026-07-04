//! `calyx discovery-run` -- seal and verify discovery-run manifests.

use std::fs;
use std::path::{Path, PathBuf};

use calyx_core::SystemClock;
use calyx_ledger::{
    DirectoryLedgerStore, LedgerAppender, LedgerCfStore, VerifyResult, decode, verify_chain,
};
use calyx_lodestar::{
    DiscoveryRunManifest, DiscoveryRunReproductionReport, DiscoveryRunSeal, ObservedStageOutput,
    manifest_sha256, reproduce_discovery_run_manifest, seal_discovery_run_manifest,
    validate_discovery_run_manifest,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::json;
use sha2::{Digest, Sha256};

use super::value;
use crate::error::{CliError, CliResult};
use crate::output::print_json;

const DISCOVERY_RUN_CLI_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Eq)]
enum DiscoveryRunCommand {
    Seal(SealArgs),
    Reproduce(ReproduceArgs),
    Verify(VerifyArgs),
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SealArgs {
    manifest: PathBuf,
    ledger: PathBuf,
    out: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ReproduceArgs {
    manifest: PathBuf,
    observed: PathBuf,
    out: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct VerifyArgs {
    manifest: PathBuf,
    ledger: PathBuf,
    seq: u64,
    out: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct SealArtifact {
    schema_version: u32,
    manifest_sha256: String,
    ledger_ref_seq: u64,
    ledger_ref_hash: String,
    verify_chain: String,
    ledger_payload: serde_json::Value,
    manifest: DiscoveryRunManifest,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct ReproduceArtifact {
    schema_version: u32,
    report: DiscoveryRunReproductionReport,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct VerifyArtifact {
    schema_version: u32,
    manifest_sha256: String,
    ledger_seq: u64,
    verify_chain: String,
    ledger_payload: serde_json::Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct PersistedArtifact<T> {
    path: PathBuf,
    bytes: u64,
    sha256: String,
    readback: T,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
struct ObservedFile {
    observed: Vec<ObservedStageOutput>,
}

pub(crate) fn try_run(args: &[String]) -> Option<CliResult> {
    let (command, rest) = args.split_first()?;
    if command != "discovery-run" {
        return None;
    }
    if matches!(rest, [flag] if flag == "--help" || flag == "-h") {
        return Some(crate::usage::print_command_usage(command));
    }
    Some(parse_discovery_run(rest).and_then(run_discovery_run))
}

fn run_discovery_run(command: DiscoveryRunCommand) -> CliResult {
    match command {
        DiscoveryRunCommand::Seal(args) => run_seal(args),
        DiscoveryRunCommand::Reproduce(args) => run_reproduce(args),
        DiscoveryRunCommand::Verify(args) => run_verify(args),
    }
}

fn run_seal(args: SealArgs) -> CliResult {
    let manifest = read_json::<DiscoveryRunManifest>(&args.manifest)?;
    validate_discovery_run_manifest(&manifest)?;
    let store = DirectoryLedgerStore::open(&args.ledger)?;
    let mut appender = LedgerAppender::open(store, SystemClock)?;
    let seal = seal_discovery_run_manifest(&mut appender, manifest)?;
    let store = appender.into_store();
    let verify = verify_chain(&store, 0..seal.ledger_ref.seq + 1)?;
    let payload = ledger_payload(&store, seal.ledger_ref.seq)?;
    let artifact = seal_artifact(seal, verify, payload);
    let persisted = persist_json(&args.out, &artifact)?;
    print_json(&json!({
        "status": "ok",
        "out": persisted.path,
        "out_sha256": persisted.sha256,
        "manifest_sha256": persisted.readback.manifest_sha256,
        "ledger_ref_seq": persisted.readback.ledger_ref_seq,
        "verify_chain": persisted.readback.verify_chain,
    }))
}

fn run_reproduce(args: ReproduceArgs) -> CliResult {
    let manifest = read_json::<DiscoveryRunManifest>(&args.manifest)?;
    let observed = read_observed(&args.observed)?;
    let report = reproduce_discovery_run_manifest(&manifest, &observed)?;
    let artifact = ReproduceArtifact {
        schema_version: DISCOVERY_RUN_CLI_SCHEMA_VERSION,
        report,
    };
    let persisted = persist_json(&args.out, &artifact)?;
    print_json(&json!({
        "status": "ok",
        "out": persisted.path,
        "out_sha256": persisted.sha256,
        "manifest_sha256": persisted.readback.report.manifest_sha256,
        "stage_count": persisted.readback.report.stage_count,
    }))
}

fn run_verify(args: VerifyArgs) -> CliResult {
    let manifest = read_json::<DiscoveryRunManifest>(&args.manifest)?;
    let expected_manifest_sha256 = manifest_sha256(&manifest)?;
    let store = DirectoryLedgerStore::open(&args.ledger)?;
    let verify = verify_chain(&store, 0..args.seq + 1)?;
    let payload = ledger_payload(&store, args.seq)?;
    let observed = payload
        .get("manifest_sha256")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| CliError::runtime("ledger payload missing manifest_sha256"))?;
    if observed != expected_manifest_sha256 {
        return Err(CliError::runtime(format!(
            "ledger manifest hash mismatch: expected {expected_manifest_sha256}, observed {observed}"
        )));
    }
    let artifact = VerifyArtifact {
        schema_version: DISCOVERY_RUN_CLI_SCHEMA_VERSION,
        manifest_sha256: expected_manifest_sha256,
        ledger_seq: args.seq,
        verify_chain: format!("{verify:?}"),
        ledger_payload: payload,
    };
    let persisted = persist_json(&args.out, &artifact)?;
    print_json(&json!({
        "status": "ok",
        "out": persisted.path,
        "out_sha256": persisted.sha256,
        "manifest_sha256": persisted.readback.manifest_sha256,
        "verify_chain": persisted.readback.verify_chain,
    }))
}

fn parse_discovery_run(rest: &[String]) -> CliResult<DiscoveryRunCommand> {
    let (subcommand, args) = rest
        .split_first()
        .ok_or_else(|| CliError::usage("discovery-run requires seal|reproduce|verify"))?;
    match subcommand.as_str() {
        "seal" => parse_seal(args).map(DiscoveryRunCommand::Seal),
        "reproduce" => parse_reproduce(args).map(DiscoveryRunCommand::Reproduce),
        "verify" => parse_verify(args).map(DiscoveryRunCommand::Verify),
        other => Err(CliError::usage(format!(
            "unexpected discovery-run subcommand {other}"
        ))),
    }
}

fn parse_seal(rest: &[String]) -> CliResult<SealArgs> {
    let mut manifest = None;
    let mut ledger = None;
    let mut out = None;
    parse_paths(rest, |flag, value| {
        match flag {
            "--manifest" => manifest = Some(value.into()),
            "--ledger" => ledger = Some(value.into()),
            "--out" => out = Some(value.into()),
            _ => {
                return Err(CliError::usage(format!(
                    "unexpected discovery-run seal flag {flag}"
                )));
            }
        }
        Ok(())
    })?;
    Ok(SealArgs {
        manifest: required_path(manifest, "--manifest")?,
        ledger: required_path(ledger, "--ledger")?,
        out: required_path(out, "--out")?,
    })
}

fn parse_reproduce(rest: &[String]) -> CliResult<ReproduceArgs> {
    let mut manifest = None;
    let mut observed = None;
    let mut out = None;
    parse_paths(rest, |flag, value| {
        match flag {
            "--manifest" => manifest = Some(value.into()),
            "--observed" => observed = Some(value.into()),
            "--out" => out = Some(value.into()),
            _ => {
                return Err(CliError::usage(format!(
                    "unexpected discovery-run reproduce flag {flag}"
                )));
            }
        }
        Ok(())
    })?;
    Ok(ReproduceArgs {
        manifest: required_path(manifest, "--manifest")?,
        observed: required_path(observed, "--observed")?,
        out: required_path(out, "--out")?,
    })
}

fn parse_verify(rest: &[String]) -> CliResult<VerifyArgs> {
    let mut manifest = None;
    let mut ledger = None;
    let mut seq = None;
    let mut out = None;
    let mut idx = 0;
    while idx < rest.len() {
        match rest[idx].as_str() {
            "--manifest" => {
                idx += 1;
                manifest = Some(value(rest, idx, "--manifest")?.into());
            }
            "--ledger" => {
                idx += 1;
                ledger = Some(value(rest, idx, "--ledger")?.into());
            }
            "--seq" => {
                idx += 1;
                seq = Some(
                    value(rest, idx, "--seq")?
                        .parse::<u64>()
                        .map_err(|err| CliError::usage(format!("parse --seq: {err}")))?,
                );
            }
            "--out" => {
                idx += 1;
                out = Some(value(rest, idx, "--out")?.into());
            }
            other => {
                return Err(CliError::usage(format!(
                    "unexpected discovery-run verify flag {other}"
                )));
            }
        }
        idx += 1;
    }
    Ok(VerifyArgs {
        manifest: required_path(manifest, "--manifest")?,
        ledger: required_path(ledger, "--ledger")?,
        seq: seq.ok_or_else(|| CliError::usage("discovery-run verify requires --seq <n>"))?,
        out: required_path(out, "--out")?,
    })
}

fn parse_paths<F>(rest: &[String], mut set: F) -> CliResult
where
    F: FnMut(&str, &str) -> CliResult,
{
    let mut idx = 0;
    while idx < rest.len() {
        let flag = rest[idx].as_str();
        idx += 1;
        set(flag, value(rest, idx, flag)?)?;
        idx += 1;
    }
    Ok(())
}

fn required_path(value: Option<PathBuf>, flag: &str) -> CliResult<PathBuf> {
    value.ok_or_else(|| CliError::usage(format!("discovery-run requires {flag} <path>")))
}

fn seal_artifact(
    seal: DiscoveryRunSeal,
    verify: VerifyResult,
    ledger_payload: serde_json::Value,
) -> SealArtifact {
    SealArtifact {
        schema_version: DISCOVERY_RUN_CLI_SCHEMA_VERSION,
        manifest_sha256: seal.manifest_sha256,
        ledger_ref_seq: seal.ledger_ref.seq,
        ledger_ref_hash: hex(&seal.ledger_ref.hash),
        verify_chain: format!("{verify:?}"),
        ledger_payload,
        manifest: seal.manifest,
    }
}

fn ledger_payload(store: &DirectoryLedgerStore, seq: u64) -> CliResult<serde_json::Value> {
    let row = store
        .read_seq(seq)?
        .ok_or_else(|| CliError::runtime(format!("missing ledger row seq {seq}")))?;
    let entry = decode(&row.bytes)?;
    serde_json::from_slice(&entry.payload)
        .map_err(|err| CliError::runtime(format!("parse discovery-run ledger payload: {err}")))
}

fn read_observed(path: &Path) -> CliResult<Vec<ObservedStageOutput>> {
    let bytes = fs::read(path)
        .map_err(|error| CliError::io(format!("read {}: {error}", path.display())))?;
    if let Ok(rows) = serde_json::from_slice::<Vec<ObservedStageOutput>>(&bytes) {
        return Ok(rows);
    }
    serde_json::from_slice::<ObservedFile>(&bytes)
        .map(|file| file.observed)
        .map_err(|err| CliError::runtime(format!("parse observed {}: {err}", path.display())))
}

fn read_json<T: DeserializeOwned>(path: &Path) -> CliResult<T> {
    let bytes = fs::read(path)
        .map_err(|error| CliError::io(format!("read {}: {error}", path.display())))?;
    serde_json::from_slice(&bytes)
        .map_err(|err| CliError::runtime(format!("parse {}: {err}", path.display())))
}

fn persist_json<T>(path: &Path, artifact: &T) -> CliResult<PersistedArtifact<T>>
where
    T: Serialize + DeserializeOwned,
{
    let bytes = serde_json::to_vec_pretty(artifact)
        .map_err(|err| CliError::runtime(format!("serialize discovery-run artifact: {err}")))?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    if path.exists() {
        let existing = fs::read(path)?;
        if existing != bytes {
            return Err(CliError::usage(format!(
                "refusing to overwrite existing different discovery-run artifact {}",
                path.display()
            )));
        }
    } else {
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, &bytes)?;
        fs::rename(&tmp, path)?;
    }
    let readback = fs::read(path)?;
    if readback != bytes {
        return Err(CliError::runtime(format!(
            "discovery-run artifact readback mismatch at {}",
            path.display()
        )));
    }
    Ok(PersistedArtifact {
        path: path.to_path_buf(),
        bytes: readback.len() as u64,
        sha256: sha256_hex(&readback),
        readback: serde_json::from_slice(&readback).map_err(|err| {
            CliError::runtime(format!("parse discovery-run artifact readback: {err}"))
        })?,
    })
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests;
