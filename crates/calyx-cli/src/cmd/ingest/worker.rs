use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{Arc, Mutex, OnceLock, mpsc};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use std::{ffi::OsStr, thread};

use bincode::config;
use calyx_core::{CalyxError, Input, Result, SlotVector};
use calyx_registry::{
    LoadedRegistrySnapshotLens, RegistryLensSnapshot, RegistrySnapshotMeasureStats,
    measure_registry_snapshot_lens_batch_with_stats,
};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::command::ingest_runtime_log;
use crate::error::{CliError, CliResult};

const DEFAULT_LENS_WORKER_TIMEOUT_SECS: u64 = 300;
const LENS_WORKER_TIMEOUT_ENV: &str = "CALYX_INGEST_LENS_WORKER_TIMEOUT_SECS";
const KEEP_WORKER_ARTIFACTS_ENV: &str = "CALYX_KEEP_INGEST_WORKER_ARTIFACTS";
const RESIDENT_PROTOCOL_VERSION: u16 = 1;
const MAX_RESIDENT_FRAME_BYTES: usize = 2 * 1024 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
struct LensWorkerRequest {
    snapshot: RegistryLensSnapshot,
    inputs: Vec<Input>,
    runtime_batch_limit: Option<usize>,
}

#[derive(Serialize, Deserialize)]
struct LensWorkerResponse {
    vectors: Vec<SlotVector>,
    stats: RegistrySnapshotMeasureStats,
}

#[derive(Serialize, Deserialize)]
struct ResidentLensWorkerInit {
    snapshot: RegistryLensSnapshot,
}

#[derive(Serialize, Deserialize)]
struct ResidentLensWorkerRequest {
    protocol_version: u16,
    inputs: Vec<Input>,
    runtime_batch_limit: Option<usize>,
}

#[derive(Serialize, Deserialize)]
struct ResidentLensWorkerResponse {
    protocol_version: u16,
    result: ResidentLensWorkerResult,
}

#[derive(Serialize, Deserialize)]
enum ResidentLensWorkerResult {
    Ok {
        vectors: Vec<SlotVector>,
        stats: RegistrySnapshotMeasureStats,
    },
    Err {
        code: String,
        message: String,
        remediation: String,
    },
}

struct WorkerPaths {
    root: PathBuf,
    request: PathBuf,
}

mod frame;
mod resident;

#[cfg(test)]
mod tests;

use frame::{decode_binary, encode_binary, read_frame_or_eof, write_frame};
pub(crate) use resident::measure_lens_in_worker;

pub(crate) fn run_lens_worker(args: &[String]) -> CliResult {
    let total_start = Instant::now();
    let flags = parse_worker_flags(args)?;
    if flags.resident {
        return run_resident_lens_worker(flags);
    }
    let bytes = fs::read(&flags.request).map_err(|error| {
        CliError::io(format!(
            "read ingest lens worker request {} failed: {error}",
            flags.request.display()
        ))
    })?;
    let request: LensWorkerRequest = serde_json::from_slice(&bytes).map_err(|error| {
        CliError::usage(format!(
            "parse ingest lens worker request {} failed: {error}",
            flags.request.display()
        ))
    })?;
    let (vectors, stats) = measure_registry_snapshot_lens_batch_with_stats(
        &request.snapshot,
        &request.inputs,
        request.runtime_batch_limit,
    )?;
    eprintln!(
        "CALYX_INGEST_RUNTIME phase=measure_lens_worker_child_ok lens_id={} inputs={} runtime_batch_limit={:?} effective_chunk_size={} chunk_count={} runtime_load_ms={} measure_ms={} total_ms={} child_total_ms={}",
        request.snapshot.lens_id,
        stats.input_count,
        stats.runtime_batch_limit,
        stats.effective_chunk_size,
        stats.chunk_count,
        stats.runtime_load_ms,
        stats.measure_ms,
        stats.total_ms,
        total_start.elapsed().as_millis()
    );
    let out = flags
        .out
        .as_ref()
        .ok_or_else(|| CliError::usage("__ingest-lens-worker requires --out <json>"))?;
    write_json(out, &LensWorkerResponse { vectors, stats })?;
    Ok(())
}

fn run_resident_lens_worker(flags: WorkerFlags) -> CliResult {
    let bytes = fs::read(&flags.request).map_err(|error| {
        CliError::io(format!(
            "read resident ingest lens worker init {} failed: {error}",
            flags.request.display()
        ))
    })?;
    let init: ResidentLensWorkerInit = serde_json::from_slice(&bytes).map_err(|error| {
        CliError::usage(format!(
            "parse resident ingest lens worker init {} failed: {error}",
            flags.request.display()
        ))
    })?;
    let load_started = Instant::now();
    let loaded = LoadedRegistrySnapshotLens::load(init.snapshot)?;
    eprintln!(
        "CALYX_INGEST_RUNTIME phase=measure_lens_worker_resident_child_ready lens_id={} runtime_load_ms={} child_load_total_ms={}",
        loaded.lens_id(),
        loaded.runtime_load_ms(),
        load_started.elapsed().as_millis()
    );
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut stdin = stdin.lock();
    let mut stdout = stdout.lock();
    while let Some(bytes) = read_frame_or_eof(&mut stdin)? {
        let request: ResidentLensWorkerRequest = decode_binary(&bytes)?;
        if request.protocol_version != RESIDENT_PROTOCOL_VERSION {
            return Err(CliError::from(CalyxError::lens_unreachable(format!(
                "resident ingest lens worker protocol version {} does not match expected {}",
                request.protocol_version, RESIDENT_PROTOCOL_VERSION
            ))));
        }
        let started = Instant::now();
        let result =
            match loaded.measure_batch_with_stats(&request.inputs, request.runtime_batch_limit) {
                Ok((vectors, stats)) => ResidentLensWorkerResult::Ok { vectors, stats },
                Err(error) => ResidentLensWorkerResult::Err {
                    code: error.code.to_string(),
                    message: error.message,
                    remediation: error.remediation.to_string(),
                },
            };
        let response = ResidentLensWorkerResponse {
            protocol_version: RESIDENT_PROTOCOL_VERSION,
            result,
        };
        let bytes = encode_binary(&response)?;
        eprintln!(
            "CALYX_INGEST_RUNTIME phase=measure_lens_worker_resident_child_response lens_id={} inputs={} elapsed_ms={} response_bytes={}",
            loaded.lens_id(),
            request.inputs.len(),
            started.elapsed().as_millis(),
            bytes.len()
        );
        write_frame(&mut stdout, &bytes)?;
        stdout.flush().map_err(|error| {
            CalyxError::lens_unreachable(format!(
                "resident ingest lens worker stdout flush failed: {error}"
            ))
        })?;
    }
    Ok(())
}

struct WorkerFlags {
    request: PathBuf,
    out: Option<PathBuf>,
    resident: bool,
}

fn parse_worker_flags(args: &[String]) -> CliResult<WorkerFlags> {
    let mut request = None;
    let mut out = None;
    let mut resident = false;
    let mut idx = 0;
    while idx < args.len() {
        match args[idx].as_str() {
            "--resident" => {
                resident = true;
            }
            "--request" => {
                idx += 1;
                request = Some(PathBuf::from(value(args, idx, "--request")?));
            }
            "--out" => {
                idx += 1;
                out = Some(PathBuf::from(value(args, idx, "--out")?));
            }
            other => {
                return Err(CliError::usage(format!(
                    "unexpected __ingest-lens-worker flag {other}"
                )));
            }
        }
        idx += 1;
    }
    Ok(WorkerFlags {
        request: request
            .ok_or_else(|| CliError::usage("__ingest-lens-worker requires --request <json>"))?,
        out,
        resident,
    })
}

fn value<'a>(args: &'a [String], index: usize, flag: &str) -> CliResult<&'a str> {
    args.get(index)
        .map(String::as_str)
        .ok_or_else(|| CliError::usage(format!("{flag} requires a value")))
}

fn lens_worker_timeout() -> Result<Duration> {
    let Some(raw) = std::env::var_os(LENS_WORKER_TIMEOUT_ENV) else {
        return Ok(Duration::from_secs(DEFAULT_LENS_WORKER_TIMEOUT_SECS));
    };
    let raw = raw.to_string_lossy();
    let secs = raw.parse::<u64>().map_err(|error| {
        CalyxError::lens_unreachable(format!("parse {LENS_WORKER_TIMEOUT_ENV}={raw}: {error}"))
    })?;
    if secs == 0 {
        return Err(CalyxError::lens_unreachable(format!(
            "{LENS_WORKER_TIMEOUT_ENV} must be > 0"
        )));
    }
    Ok(Duration::from_secs(secs))
}

fn worker_paths(lens_id: calyx_core::LensId) -> Result<WorkerPaths> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| CalyxError::lens_unreachable(format!("system clock error: {error}")))?
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "calyx-ingest-lens-worker-{}-{lens_id}-{now}",
        std::process::id()
    ));
    fs::create_dir_all(&root).map_err(|error| {
        CalyxError::lens_unreachable(format!(
            "create ingest lens worker dir {} failed: {error}",
            root.display()
        ))
    })?;
    Ok(WorkerPaths {
        request: root.join("request.json"),
        root,
    })
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let bytes = serde_json::to_vec(value)
        .map_err(|error| CalyxError::lens_unreachable(format!("encode JSON failed: {error}")))?;
    fs::write(path, bytes).map_err(|error| {
        CalyxError::lens_unreachable(format!("write {} failed: {error}", path.display()))
    })
}

fn snapshot_sha256(snapshot: &RegistryLensSnapshot) -> Result<[u8; 32]> {
    let bytes = serde_json::to_vec(snapshot).map_err(|error| {
        CalyxError::lens_unreachable(format!(
            "encode registry lens snapshot {} for resident worker hash failed: {error}",
            snapshot.lens_id
        ))
    })?;
    Ok(Sha256::digest(bytes).into())
}

fn hex_sha256(bytes: [u8; 32]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
