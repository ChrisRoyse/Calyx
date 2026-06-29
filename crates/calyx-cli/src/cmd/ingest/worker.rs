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

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct ResidentWorkerKey {
    lens_id: calyx_core::LensId,
    snapshot_sha256: [u8; 32],
}

struct ResidentLensWorker {
    lens_id: calyx_core::LensId,
    snapshot_sha256: [u8; 32],
    pid: u32,
    tx: mpsc::Sender<ResidentWorkerRequest>,
    stderr_tail: Arc<Mutex<Vec<u8>>>,
}

struct ResidentWorkerRequest {
    request: Vec<u8>,
    request_bytes: usize,
    reply: mpsc::Sender<Result<ResidentLensWorkerResponse>>,
}

static RESIDENT_LENS_WORKERS: OnceLock<
    Mutex<BTreeMap<ResidentWorkerKey, Arc<ResidentLensWorker>>>,
> = OnceLock::new();

pub(super) fn measure_lens_in_worker(
    snapshot: &RegistryLensSnapshot,
    inputs: &[Input],
    runtime_batch_limit: Option<usize>,
) -> Result<Vec<SlotVector>> {
    resident_lens_worker(snapshot)?.measure(inputs, runtime_batch_limit)
}

fn resident_lens_worker(snapshot: &RegistryLensSnapshot) -> Result<Arc<ResidentLensWorker>> {
    let key = ResidentWorkerKey {
        lens_id: snapshot.lens_id,
        snapshot_sha256: snapshot_sha256(snapshot)?,
    };
    let pool = RESIDENT_LENS_WORKERS.get_or_init(|| Mutex::new(BTreeMap::new()));
    let mut guard = pool.lock().map_err(|_| {
        CalyxError::lens_unreachable("resident ingest lens worker pool mutex was poisoned")
    })?;
    if let Some(worker) = guard.get(&key) {
        ingest_runtime_log(format_args!(
            "phase=measure_lens_worker_resident_reuse lens_id={} pid={} snapshot_sha256={}",
            worker.lens_id,
            worker.pid,
            hex_sha256(worker.snapshot_sha256)
        ));
        return Ok(worker.clone());
    }
    let worker = Arc::new(ResidentLensWorker::spawn(
        snapshot.clone(),
        key.snapshot_sha256,
    )?);
    guard.insert(key, worker.clone());
    Ok(worker)
}

impl ResidentLensWorker {
    fn spawn(snapshot: RegistryLensSnapshot, snapshot_sha256: [u8; 32]) -> Result<Self> {
        let total_start = Instant::now();
        let paths = worker_paths(snapshot.lens_id)?;
        write_json(
            &paths.request,
            &ResidentLensWorkerInit {
                snapshot: snapshot.clone(),
            },
        )?;
        let mut child = spawn_resident_child(snapshot.lens_id, &paths.request)?;
        let pid = child.id();
        let stdin = child.stdin.take().ok_or_else(|| {
            CalyxError::lens_unreachable("resident ingest lens worker stdin pipe missing")
        })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            CalyxError::lens_unreachable("resident ingest lens worker stdout pipe missing")
        })?;
        let stderr = child.stderr.take().ok_or_else(|| {
            CalyxError::lens_unreachable("resident ingest lens worker stderr pipe missing")
        })?;
        let stderr_tail = Arc::new(Mutex::new(Vec::new()));
        spawn_stderr_reader(stderr, stderr_tail.clone());
        let (tx, rx) = mpsc::channel();
        let worker_stderr_tail = stderr_tail.clone();
        let worker_root = paths.root.clone();
        thread::spawn(move || {
            resident_worker_loop(child, stdin, stdout, rx, worker_stderr_tail, worker_root)
        });
        ingest_runtime_log(format_args!(
            "phase=measure_lens_worker_resident_spawned lens_id={} pid={} snapshot_sha256={} elapsed_ms={}",
            snapshot.lens_id,
            pid,
            hex_sha256(snapshot_sha256),
            total_start.elapsed().as_millis()
        ));
        Ok(Self {
            lens_id: snapshot.lens_id,
            snapshot_sha256,
            pid,
            tx,
            stderr_tail,
        })
    }

    fn measure(
        &self,
        inputs: &[Input],
        runtime_batch_limit: Option<usize>,
    ) -> Result<Vec<SlotVector>> {
        let timeout = lens_worker_timeout()?;
        let started = Instant::now();
        let request = ResidentLensWorkerRequest {
            protocol_version: RESIDENT_PROTOCOL_VERSION,
            inputs: inputs.to_vec(),
            runtime_batch_limit,
        };
        let request = encode_binary(&request)?;
        let request_bytes = request.len();
        let (reply, rx) = mpsc::channel();
        self.tx
            .send(ResidentWorkerRequest {
                request,
                request_bytes,
                reply,
            })
            .map_err(|_| {
                CalyxError::lens_unreachable(format!(
                    "resident ingest lens worker for lens {} stopped before request; pid={} snapshot_sha256={} stderr_tail={}",
                    self.lens_id,
                    self.pid,
                    hex_sha256(self.snapshot_sha256),
                    stderr_tail_text(&self.stderr_tail)
                ))
            })?;
        let response = match rx.recv_timeout(timeout) {
            Ok(result) => result?,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                return Err(CalyxError::lens_unreachable(format!(
                    "resident ingest lens worker for lens {} timed out after {} ms; pid={} snapshot_sha256={} stderr_tail={}",
                    self.lens_id,
                    timeout.as_millis(),
                    self.pid,
                    hex_sha256(self.snapshot_sha256),
                    stderr_tail_text(&self.stderr_tail)
                )));
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err(CalyxError::lens_unreachable(format!(
                    "resident ingest lens worker for lens {} disconnected; pid={} snapshot_sha256={} stderr_tail={}",
                    self.lens_id,
                    self.pid,
                    hex_sha256(self.snapshot_sha256),
                    stderr_tail_text(&self.stderr_tail)
                )));
            }
        };
        if response.protocol_version != RESIDENT_PROTOCOL_VERSION {
            return Err(CalyxError::lens_unreachable(format!(
                "resident ingest lens worker for lens {} returned protocol version {}, expected {}",
                self.lens_id, response.protocol_version, RESIDENT_PROTOCOL_VERSION
            )));
        }
        match response.result {
            ResidentLensWorkerResult::Ok { vectors, stats } => {
                ingest_runtime_log(format_args!(
                    "phase=measure_lens_worker_resident_ok lens_id={} pid={} inputs={} runtime_batch_limit={:?} effective_chunk_size={} chunk_count={} runtime_load_ms={} measure_ms={} worker_total_ms={} parent_total_ms={} request_bytes={} stderr_tail={}",
                    self.lens_id,
                    self.pid,
                    stats.input_count,
                    stats.runtime_batch_limit,
                    stats.effective_chunk_size,
                    stats.chunk_count,
                    stats.runtime_load_ms,
                    stats.measure_ms,
                    stats.total_ms,
                    started.elapsed().as_millis(),
                    request_bytes,
                    stderr_tail_text(&self.stderr_tail)
                ));
                Ok(vectors)
            }
            ResidentLensWorkerResult::Err {
                code,
                message,
                remediation,
            } => Err(CalyxError::lens_unreachable(format!(
                "resident ingest lens worker for lens {} returned {code}: {message}; remediation={remediation}; pid={} snapshot_sha256={} stderr_tail={}",
                self.lens_id,
                self.pid,
                hex_sha256(self.snapshot_sha256),
                stderr_tail_text(&self.stderr_tail)
            ))),
        }
    }
}

fn spawn_resident_child(lens_id: calyx_core::LensId, init_request: &Path) -> Result<Child> {
    let mut command = Command::new(std::env::current_exe().map_err(|error| {
        CalyxError::lens_unreachable(format!("resolve current calyx executable failed: {error}"))
    })?);
    command
        .arg("__ingest-lens-worker")
        .arg("--resident")
        .arg("--request")
        .arg(init_request)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command.spawn().map_err(|error| {
        CalyxError::lens_unreachable(format!(
            "spawn resident ingest lens worker for lens {lens_id} failed: {error}"
        ))
    })
}

fn resident_worker_loop(
    mut child: Child,
    mut stdin: ChildStdin,
    mut stdout: ChildStdout,
    rx: mpsc::Receiver<ResidentWorkerRequest>,
    stderr_tail: Arc<Mutex<Vec<u8>>>,
    root: PathBuf,
) {
    for item in rx {
        let result = write_frame(&mut stdin, &item.request)
            .and_then(|_| read_frame(&mut stdout))
            .and_then(|bytes| decode_binary::<ResidentLensWorkerResponse>(&bytes))
            .map_err(|error| {
                let status = child
                    .try_wait()
                    .ok()
                    .flatten()
                    .map(|status| status.to_string())
                    .unwrap_or_else(|| "still_running".to_string());
                CalyxError::lens_unreachable(format!(
                    "{}; child_status={status}; request_bytes={}; stderr_tail={}",
                    error.message,
                    item.request_bytes,
                    stderr_tail_text(&stderr_tail)
                ))
            });
        let failed = result.is_err();
        let _ = item.reply.send(result);
        if failed {
            break;
        }
    }
    drop(stdin);
    finish_child(&mut child);
    if std::env::var_os(KEEP_WORKER_ARTIFACTS_ENV).as_deref() != Some(OsStr::new("1")) {
        let _ = fs::remove_dir_all(root);
    }
}

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

fn encode_binary(value: &impl Serialize) -> Result<Vec<u8>> {
    bincode::serde::encode_to_vec(value, config::standard()).map_err(|error| {
        CalyxError::lens_unreachable(format!("encode binary frame failed: {error}"))
    })
}

fn decode_binary<T: DeserializeOwned>(bytes: &[u8]) -> Result<T> {
    let (value, consumed) =
        bincode::serde::decode_from_slice(bytes, config::standard()).map_err(|error| {
            CalyxError::lens_unreachable(format!("decode binary frame failed: {error}"))
        })?;
    if consumed != bytes.len() {
        return Err(CalyxError::lens_unreachable(format!(
            "decode binary frame consumed {consumed} of {} bytes",
            bytes.len()
        )));
    }
    Ok(value)
}

fn write_frame(writer: &mut impl Write, bytes: &[u8]) -> Result<()> {
    if bytes.len() > MAX_RESIDENT_FRAME_BYTES {
        return Err(CalyxError::lens_unreachable(format!(
            "resident binary frame {} bytes exceeds max {}",
            bytes.len(),
            MAX_RESIDENT_FRAME_BYTES
        )));
    }
    let len = u64::try_from(bytes.len()).map_err(|_| {
        CalyxError::lens_unreachable(format!(
            "resident binary frame {} bytes overflows u64",
            bytes.len()
        ))
    })?;
    writer
        .write_all(&len.to_be_bytes())
        .and_then(|_| writer.write_all(bytes))
        .map_err(|error| {
            CalyxError::lens_unreachable(format!("write binary frame failed: {error}"))
        })
}

fn read_frame(reader: &mut impl Read) -> Result<Vec<u8>> {
    read_frame_or_eof(reader)?.ok_or_else(|| {
        CalyxError::lens_unreachable("read binary frame failed: stream closed before header")
    })
}

fn read_frame_or_eof(reader: &mut impl Read) -> Result<Option<Vec<u8>>> {
    let Some(header) = read_header_or_eof(reader)? else {
        return Ok(None);
    };
    let len = u64::from_be_bytes(header);
    let len = usize::try_from(len).map_err(|_| {
        CalyxError::lens_unreachable(format!(
            "resident binary frame length {len} overflows usize"
        ))
    })?;
    if len > MAX_RESIDENT_FRAME_BYTES {
        return Err(CalyxError::lens_unreachable(format!(
            "resident binary frame {len} bytes exceeds max {MAX_RESIDENT_FRAME_BYTES}"
        )));
    }
    let mut body = vec![0_u8; len];
    reader.read_exact(&mut body).map_err(|error| {
        CalyxError::lens_unreachable(format!(
            "read binary frame body ({len} bytes) failed: {error}"
        ))
    })?;
    Ok(Some(body))
}

fn read_header_or_eof(reader: &mut impl Read) -> Result<Option<[u8; 8]>> {
    let mut header = [0_u8; 8];
    let mut offset = 0;
    while offset < header.len() {
        match reader.read(&mut header[offset..]) {
            Ok(0) if offset == 0 => return Ok(None),
            Ok(0) => {
                return Err(CalyxError::lens_unreachable(format!(
                    "read binary frame header failed: stream closed after {offset} of 8 bytes"
                )));
            }
            Ok(n) => offset += n,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => {
                return Err(CalyxError::lens_unreachable(format!(
                    "read binary frame header failed: {error}"
                )));
            }
        }
    }
    Ok(Some(header))
}

fn spawn_stderr_reader(mut stderr: std::process::ChildStderr, tail: Arc<Mutex<Vec<u8>>>) {
    thread::spawn(move || {
        let mut chunk = [0_u8; 4096];
        loop {
            match stderr.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => append_tail(&tail, &chunk[..n]),
                Err(_) => break,
            }
        }
    });
}

fn append_tail(tail: &Arc<Mutex<Vec<u8>>>, bytes: &[u8]) {
    const CAP: usize = 16 * 1024;
    let Ok(mut tail) = tail.lock() else {
        return;
    };
    tail.extend_from_slice(bytes);
    if tail.len() > CAP {
        let overflow = tail.len() - CAP;
        tail.drain(0..overflow);
    }
}

fn stderr_tail_text(tail: &Arc<Mutex<Vec<u8>>>) -> String {
    let Ok(tail) = tail.lock() else {
        return "stderr_tail_mutex_poisoned".to_string();
    };
    let raw = String::from_utf8_lossy(&tail);
    let mut out = String::with_capacity(raw.len());
    for ch in raw.trim().chars() {
        match ch {
            '\r' => out.push_str("\\r"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            _ => out.push(ch),
        }
    }
    out
}

fn finish_child(child: &mut Child) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if matches!(child.try_wait(), Ok(Some(_))) {
            return;
        }
        if Instant::now() >= deadline {
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Write};

    use calyx_core::Modality;

    use super::*;

    #[test]
    fn resident_binary_request_roundtrips_without_json_shape() {
        let request = ResidentLensWorkerRequest {
            protocol_version: RESIDENT_PROTOCOL_VERSION,
            inputs: vec![
                Input::new(Modality::Text, b"alpha".to_vec()),
                Input::new(Modality::Text, b"beta".to_vec()),
            ],
            runtime_batch_limit: Some(4),
        };

        let bytes = encode_binary(&request).unwrap();
        let decoded: ResidentLensWorkerRequest = decode_binary(&bytes).unwrap();
        println!(
            "resident_binary_request_roundtrip bytes={} inputs={} runtime_batch_limit={:?}",
            bytes.len(),
            decoded.inputs.len(),
            decoded.runtime_batch_limit
        );

        assert_eq!(decoded.protocol_version, RESIDENT_PROTOCOL_VERSION);
        assert_eq!(decoded.inputs, request.inputs);
        assert_eq!(decoded.runtime_batch_limit, Some(4));
        assert!(
            !String::from_utf8_lossy(&bytes).contains("runtime_batch_limit"),
            "binary IPC must not carry JSON field names"
        );
    }

    #[test]
    fn resident_binary_frame_readback_is_length_prefixed() {
        let response = ResidentLensWorkerResponse {
            protocol_version: RESIDENT_PROTOCOL_VERSION,
            result: ResidentLensWorkerResult::Err {
                code: "CALYX_TEST".to_string(),
                message: "synthetic frame edge".to_string(),
                remediation: "fix test input".to_string(),
            },
        };
        let payload = encode_binary(&response).unwrap();
        let mut stream = Cursor::new(Vec::new());

        write_frame(&mut stream, &payload).unwrap();
        let stored = stream.into_inner();
        println!(
            "resident_binary_frame_state header_bytes=8 payload_bytes={} stored_bytes={}",
            payload.len(),
            stored.len()
        );

        assert_eq!(stored.len(), payload.len() + 8);
        assert_eq!(
            u64::from_be_bytes(stored[..8].try_into().unwrap()) as usize,
            payload.len()
        );
        let mut readback = Cursor::new(stored);
        let decoded_payload = read_frame(&mut readback).unwrap();
        let decoded: ResidentLensWorkerResponse = decode_binary(&decoded_payload).unwrap();
        assert_eq!(decoded.protocol_version, RESIDENT_PROTOCOL_VERSION);
        assert!(matches!(
            decoded.result,
            ResidentLensWorkerResult::Err { ref code, .. } if code == "CALYX_TEST"
        ));
    }

    #[test]
    fn resident_binary_frame_truncated_body_fails_loud() {
        let mut stream = Cursor::new(Vec::new());
        stream.write_all(&16_u64.to_be_bytes()).unwrap();
        stream.write_all(b"short").unwrap();
        stream.set_position(0);

        let error = read_frame(&mut stream).unwrap_err();
        println!(
            "resident_binary_frame_truncated_error code={} message={}",
            error.code, error.message
        );
        assert_eq!(error.code, "CALYX_LENS_UNREACHABLE");
        assert!(error.message.contains("read binary frame body"));
    }

    #[test]
    fn stderr_tail_text_is_single_line_for_runtime_logs() {
        let tail = Arc::new(Mutex::new(Vec::new()));
        append_tail(
            &tail,
            b"line one\r\nCALYX_INGEST_RUNTIME phase=child_ready\tok\n",
        );

        let text = stderr_tail_text(&tail);

        println!("stderr_tail_sanitized={text}");
        assert_eq!(
            text,
            "line one\\r\\nCALYX_INGEST_RUNTIME phase=child_ready\\tok"
        );
        assert!(!text.contains('\n'));
        assert!(!text.contains('\r'));
        assert!(!text.contains('\t'));
    }
}
