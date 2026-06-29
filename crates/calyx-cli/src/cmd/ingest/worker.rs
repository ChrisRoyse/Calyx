use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use calyx_core::{CalyxError, Input, Result, SlotVector};
use calyx_registry::{RegistryLensSnapshot, measure_registry_snapshot_lens_batch};
use serde::{Deserialize, Serialize};

use crate::error::{CliError, CliResult};

const DEFAULT_LENS_WORKER_TIMEOUT_SECS: u64 = 300;
const LENS_WORKER_TIMEOUT_ENV: &str = "CALYX_INGEST_LENS_WORKER_TIMEOUT_SECS";
const KEEP_WORKER_ARTIFACTS_ENV: &str = "CALYX_KEEP_INGEST_WORKER_ARTIFACTS";

#[derive(Serialize, Deserialize)]
struct LensWorkerRequest {
    snapshot: RegistryLensSnapshot,
    inputs: Vec<Input>,
}

#[derive(Serialize, Deserialize)]
struct LensWorkerResponse {
    vectors: Vec<SlotVector>,
}

struct WorkerPaths {
    root: PathBuf,
    request: PathBuf,
    response: PathBuf,
    stdout: PathBuf,
    stderr: PathBuf,
}

pub(super) fn measure_lens_in_worker(
    snapshot: &RegistryLensSnapshot,
    inputs: &[Input],
) -> Result<Vec<SlotVector>> {
    let timeout = lens_worker_timeout()?;
    let paths = worker_paths(snapshot.lens_id)?;
    let request = LensWorkerRequest {
        snapshot: snapshot.clone(),
        inputs: inputs.to_vec(),
    };
    write_json(&paths.request, &request)?;
    let stdout = File::create(&paths.stdout).map_err(|error| {
        CalyxError::lens_unreachable(format!("create ingest lens worker stdout failed: {error}"))
    })?;
    let stderr = File::create(&paths.stderr).map_err(|error| {
        CalyxError::lens_unreachable(format!("create ingest lens worker stderr failed: {error}"))
    })?;
    let mut command = Command::new(std::env::current_exe().map_err(|error| {
        CalyxError::lens_unreachable(format!("resolve current calyx executable failed: {error}"))
    })?);
    command
        .arg("__ingest-lens-worker")
        .arg("--request")
        .arg(&paths.request)
        .arg("--out")
        .arg(&paths.response);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = command
        .stdout(stdout)
        .stderr(stderr)
        .spawn()
        .map_err(|error| {
            CalyxError::lens_unreachable(format!(
                "spawn ingest lens worker for lens {} failed: {error}",
                snapshot.lens_id
            ))
        })?;
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait().map_err(|error| {
            CalyxError::lens_unreachable(format!(
                "poll ingest lens worker {} failed: {error}",
                snapshot.lens_id
            ))
        })? {
            let result = read_worker_response(status, snapshot, &paths);
            cleanup_worker_paths(&paths, result.is_ok());
            return result;
        }
        if started.elapsed() >= timeout {
            let kill_result = child.kill();
            let wait_result = child.wait();
            let stderr_tail = stderr_tail(&paths.stderr);
            let result = Err(CalyxError::lens_unreachable(format!(
                "ingest lens worker for lens {} timed out after {} ms; kill_result={kill_result:?}; wait_result={wait_result:?}; stderr_tail={stderr_tail}",
                snapshot.lens_id,
                timeout.as_millis()
            )));
            cleanup_worker_paths(&paths, false);
            return result;
        }
        thread::sleep(Duration::from_millis(100));
    }
}

pub(crate) fn run_lens_worker(args: &[String]) -> CliResult {
    let flags = parse_worker_flags(args)?;
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
    let vectors = measure_registry_snapshot_lens_batch(&request.snapshot, &request.inputs)?;
    write_json(&flags.out, &LensWorkerResponse { vectors })?;
    Ok(())
}

struct WorkerFlags {
    request: PathBuf,
    out: PathBuf,
}

fn parse_worker_flags(args: &[String]) -> CliResult<WorkerFlags> {
    let mut request = None;
    let mut out = None;
    let mut idx = 0;
    while idx < args.len() {
        match args[idx].as_str() {
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
        out: out.ok_or_else(|| CliError::usage("__ingest-lens-worker requires --out <json>"))?,
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
        response: root.join("response.json"),
        stdout: root.join("stdout.txt"),
        stderr: root.join("stderr.txt"),
        root,
    })
}

fn read_worker_response(
    status: ExitStatus,
    snapshot: &RegistryLensSnapshot,
    paths: &WorkerPaths,
) -> Result<Vec<SlotVector>> {
    let bytes = fs::read(&paths.response).map_err(|error| {
        CalyxError::lens_unreachable(format!(
            "ingest lens worker for lens {} exited {status} but response {} is unreadable: {error}; stderr_tail={}",
            snapshot.lens_id,
            paths.response.display(),
            stderr_tail(&paths.stderr)
        ))
    })?;
    let response: LensWorkerResponse = serde_json::from_slice(&bytes).map_err(|error| {
        CalyxError::lens_unreachable(format!(
            "ingest lens worker for lens {} wrote invalid response {}: {error}; stderr_tail={}",
            snapshot.lens_id,
            paths.response.display(),
            stderr_tail(&paths.stderr)
        ))
    })?;
    if !status.success() {
        return Err(CalyxError::lens_unreachable(format!(
            "ingest lens worker for lens {} exited {status}; stderr_tail={}",
            snapshot.lens_id,
            stderr_tail(&paths.stderr)
        )));
    }
    Ok(response.vectors)
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let bytes = serde_json::to_vec(value)
        .map_err(|error| CalyxError::lens_unreachable(format!("encode JSON failed: {error}")))?;
    fs::write(path, bytes).map_err(|error| {
        CalyxError::lens_unreachable(format!("write {} failed: {error}", path.display()))
    })
}

fn stderr_tail(path: &Path) -> String {
    const TAIL_BYTES: usize = 4096;
    match fs::read(path) {
        Ok(bytes) if bytes.is_empty() => "stderr was empty".to_string(),
        Ok(bytes) => {
            let start = bytes.len().saturating_sub(TAIL_BYTES);
            String::from_utf8_lossy(&bytes[start..]).trim().to_string()
        }
        Err(error) => format!("read stderr {} failed: {error}", path.display()),
    }
}

fn cleanup_worker_paths(paths: &WorkerPaths, _success: bool) {
    if std::env::var_os(KEEP_WORKER_ARTIFACTS_ENV).as_deref() != Some(std::ffi::OsStr::new("1")) {
        let _ = fs::remove_dir_all(&paths.root);
    }
}
