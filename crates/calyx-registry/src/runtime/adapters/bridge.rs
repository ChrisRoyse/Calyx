use std::collections::BTreeMap;
use std::env;
use std::ffi::OsString;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, OnceLock, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use calyx_core::{CalyxError, Input, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::config::{MultimodalAdapterConfig, MultimodalAdapterProvider};

const DEFAULT_SHARED_GPU_WORKERS: usize = 4;
const MAX_SHARED_GPU_WORKERS: usize = 16;
const GPU_WORKERS_ENV: &str = "CALYX_MULTIMODAL_GPU_WORKERS";

#[derive(Serialize)]
struct AdapterRequest<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    config: Option<&'a Path>,
    inputs: Vec<&'a [u8]>,
}

#[derive(Deserialize)]
struct AdapterResponse {
    vectors: Vec<Vec<f32>>,
}

pub struct AdapterWorker {
    tx: mpsc::Sender<WorkerRequest>,
    stderr_tail: Arc<Mutex<Vec<u8>>>,
}

struct WorkerRequest {
    request: Vec<u8>,
    reply: mpsc::Sender<Result<Vec<u8>>>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct SharedWorkerKey {
    command: String,
    helper_sha256: [u8; 32],
    provider: MultimodalAdapterProvider,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WorkerMode {
    SingleConfig,
    Mux,
}

static SHARED_GPU_WORKERS: OnceLock<Mutex<BTreeMap<SharedWorkerKey, SharedWorkerPool>>> =
    OnceLock::new();

struct SharedWorkerPool {
    workers: Vec<Option<Arc<AdapterWorker>>>,
}

impl SharedWorkerPool {
    fn new(worker_count: usize) -> Self {
        Self {
            workers: vec![None; worker_count],
        }
    }

    fn worker(
        &mut self,
        slot: usize,
        config: &MultimodalAdapterConfig,
        helper_sha256: [u8; 32],
    ) -> Result<Arc<AdapterWorker>> {
        let worker_count = self.workers.len();
        let Some(entry) = self.workers.get_mut(slot) else {
            return Err(CalyxError::lens_unreachable(format!(
                "multimodal shared worker slot {slot} outside pool size {worker_count}"
            )));
        };
        if let Some(worker) = entry {
            return Ok(worker.clone());
        }
        eprintln!(
            "calyx multimodal shared worker spawn command={} helper={} helper_sha256={} provider={} worker_slot={} worker_count={}",
            config.command,
            config.helper.display(),
            hex_sha256(helper_sha256),
            config.provider.detail(),
            slot,
            worker_count,
        );
        let worker = Arc::new(AdapterWorker::spawn(config, WorkerMode::Mux)?);
        *entry = Some(worker.clone());
        Ok(worker)
    }
}

pub(super) fn shutdown_shared_gpu_workers() {
    if let Some(pool) = SHARED_GPU_WORKERS.get()
        && let Ok(mut guard) = pool.lock()
    {
        guard.clear();
    }
}

pub fn measure_batch(
    config: &MultimodalAdapterConfig,
    inputs: &[Input],
    worker: &Mutex<Option<AdapterWorker>>,
) -> Result<Vec<Vec<f32>>> {
    let request = AdapterRequest {
        config: config.provider.is_gpu().then_some(config.path.as_path()),
        inputs: inputs.iter().map(|input| input.bytes.as_slice()).collect(),
    };
    let request = serde_json::to_vec(&request).map_err(|err| {
        CalyxError::lens_unreachable(format!("multimodal request encode failed: {err}"))
    })?;
    let body = if config.provider.is_gpu() {
        shared_gpu_worker(config)?.request(config, request)?
    } else {
        let mut guard = worker.lock().map_err(|_| {
            CalyxError::lens_unreachable("multimodal adapter worker mutex was poisoned")
        })?;
        if guard.is_none() {
            *guard = Some(AdapterWorker::spawn(config, WorkerMode::SingleConfig)?);
        }
        guard
            .as_ref()
            .expect("adapter worker initialized")
            .request(config, request)?
    };
    let response: AdapterResponse = serde_json::from_slice(&body).map_err(|err| {
        CalyxError::lens_unreachable(format!("multimodal response decode failed: {err}"))
    })?;
    if response.vectors.len() != inputs.len() {
        return Err(CalyxError::lens_dim_mismatch(format!(
            "multimodal adapter returned {} vectors for {} inputs",
            response.vectors.len(),
            inputs.len()
        )));
    }
    Ok(response.vectors)
}

impl AdapterWorker {
    fn spawn(config: &MultimodalAdapterConfig, mode: WorkerMode) -> Result<Self> {
        if config.timeout.is_zero() {
            return Err(CalyxError::lens_unreachable(
                "multimodal adapter timed out before spawn",
            ));
        }
        let mut child = spawn_child(config, mode)?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| CalyxError::lens_unreachable("multimodal stdin pipe missing"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| CalyxError::lens_unreachable("multimodal stdout pipe missing"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| CalyxError::lens_unreachable("multimodal stderr pipe missing"))?;
        let stderr_tail = Arc::new(Mutex::new(Vec::new()));
        spawn_stderr_reader(stderr, stderr_tail.clone());

        let (tx, rx) = mpsc::channel();
        let stderr_for_worker = stderr_tail.clone();
        thread::spawn(move || worker_loop(child, stdin, stdout, rx, stderr_for_worker));
        Ok(Self { tx, stderr_tail })
    }

    fn request(&self, config: &MultimodalAdapterConfig, request: Vec<u8>) -> Result<Vec<u8>> {
        let (reply, rx) = mpsc::channel();
        self.tx
            .send(WorkerRequest { request, reply })
            .map_err(|_| {
                CalyxError::lens_unreachable(format!(
                    "multimodal adapter worker stopped before request; stderr_tail={}",
                    stderr_tail_text(&self.stderr_tail)
                ))
            })?;
        match rx.recv_timeout(config.timeout) {
            Ok(result) => result,
            Err(mpsc::RecvTimeoutError::Timeout) => Err(CalyxError::lens_unreachable(format!(
                "multimodal adapter timed out after {} ms; stderr_tail={}",
                config.timeout.as_millis(),
                stderr_tail_text(&self.stderr_tail)
            ))),
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                Err(CalyxError::lens_unreachable(format!(
                    "multimodal adapter worker disconnected; stderr_tail={}",
                    stderr_tail_text(&self.stderr_tail)
                )))
            }
        }
    }
}

fn shared_gpu_worker(config: &MultimodalAdapterConfig) -> Result<Arc<AdapterWorker>> {
    let helper_sha256 = file_sha256(&config.helper)?;
    let key = SharedWorkerKey {
        command: config.command.clone(),
        helper_sha256,
        provider: config.provider,
    };
    let worker_count = shared_gpu_worker_count()?;
    let slot = config_worker_slot(&config.path, worker_count);
    let pool = SHARED_GPU_WORKERS.get_or_init(|| Mutex::new(BTreeMap::new()));
    let mut guard = pool.lock().map_err(|_| {
        CalyxError::lens_unreachable("multimodal shared worker pool mutex was poisoned")
    })?;
    let entry = guard
        .entry(key)
        .or_insert_with(|| SharedWorkerPool::new(worker_count));
    if entry.workers.len() != worker_count {
        return Err(CalyxError::lens_unreachable(format!(
            "{GPU_WORKERS_ENV} changed after multimodal GPU worker pool initialization: existing={} requested={worker_count}",
            entry.workers.len()
        )));
    }
    entry.worker(slot, config, helper_sha256)
}

fn shared_gpu_worker_count() -> Result<usize> {
    let Some(raw) = env::var_os(GPU_WORKERS_ENV) else {
        return Ok(DEFAULT_SHARED_GPU_WORKERS);
    };
    let raw = raw.to_string_lossy();
    let value = raw.parse::<usize>().map_err(|err| {
        CalyxError::lens_unreachable(format!("parse {GPU_WORKERS_ENV}={raw}: {err}"))
    })?;
    if value == 0 || value > MAX_SHARED_GPU_WORKERS {
        return Err(CalyxError::lens_unreachable(format!(
            "{GPU_WORKERS_ENV} must be between 1 and {MAX_SHARED_GPU_WORKERS}, got {value}"
        )));
    }
    Ok(value)
}

fn config_worker_slot(path: &Path, worker_count: usize) -> usize {
    let digest = Sha256::digest(path.to_string_lossy().as_bytes());
    let mut bytes = [0_u8; 8];
    bytes.copy_from_slice(&digest[..8]);
    (u64::from_be_bytes(bytes) as usize) % worker_count
}

fn file_sha256(path: &Path) -> Result<[u8; 32]> {
    let bytes = std::fs::read(path).map_err(|err| {
        CalyxError::lens_unreachable(format!(
            "hash multimodal helper {} failed: {err}",
            path.display()
        ))
    })?;
    Ok(Sha256::digest(bytes).into())
}

fn hex_sha256(bytes: [u8; 32]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn spawn_child(config: &MultimodalAdapterConfig, mode: WorkerMode) -> Result<std::process::Child> {
    let mut command = Command::new(&config.command);
    command.arg(&config.helper);
    match mode {
        WorkerMode::SingleConfig => {
            command.arg("--config").arg(&config.path);
        }
        WorkerMode::Mux => {
            command.arg("--mux");
        }
    }
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    if config.provider.is_gpu()
        && let Some(path) = cuda_ld_library_path(&config.command)
    {
        command.env("LD_LIBRARY_PATH", path);
    }
    #[cfg(windows)]
    if config.provider.is_gpu()
        && let Some(path) = gpu_dll_path(&config.command)
    {
        command.env("PATH", path);
    }
    let mut child = command.spawn().map_err(|err| {
        CalyxError::lens_unreachable(format!(
            "spawn multimodal adapter {} failed: {err}",
            config.command
        ))
    })?;
    if let Err(error) = assign_child_to_cleanup_job(&mut child) {
        let _ = child.kill();
        let _ = child.wait();
        return Err(error);
    }
    Ok(child)
}

fn worker_loop(
    mut child: std::process::Child,
    mut stdin: std::process::ChildStdin,
    mut stdout: std::process::ChildStdout,
    rx: mpsc::Receiver<WorkerRequest>,
    stderr_tail: Arc<Mutex<Vec<u8>>>,
) {
    for item in rx {
        let result = write_request(&mut stdin, &item.request)
            .and_then(|_| read_response(&mut stdout))
            .map_err(|error| enrich_worker_error(error, &mut child, &stderr_tail));
        let failed = result.is_err();
        let _ = item.reply.send(result);
        if failed {
            break;
        }
    }
    drop(stdin);
    finish_child(&mut child);
}

fn enrich_worker_error(
    error: CalyxError,
    child: &mut std::process::Child,
    stderr_tail: &Arc<Mutex<Vec<u8>>>,
) -> CalyxError {
    let status = child.try_wait().ok().flatten();
    let status = status
        .map(|status| status.to_string())
        .unwrap_or_else(|| "still_running".to_string());
    CalyxError::lens_unreachable(format!(
        "{}; child_status={status}; stderr_tail={}",
        error.message,
        stderr_tail_text(stderr_tail)
    ))
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
    decode_stderr_tail(&tail).trim().to_string()
}

fn decode_stderr_tail(bytes: &[u8]) -> String {
    if looks_like_utf16le(bytes) {
        let words = bytes
            .chunks_exact(2)
            .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
            .collect::<Vec<_>>();
        return String::from_utf16_lossy(&words);
    }
    String::from_utf8_lossy(bytes).to_string()
}

fn looks_like_utf16le(bytes: &[u8]) -> bool {
    let pairs = bytes.len() / 2;
    if pairs < 8 {
        return false;
    }
    let nul_high = bytes
        .chunks_exact(2)
        .filter(|chunk| chunk[1] == 0 && chunk[0] != 0)
        .count();
    nul_high * 2 >= pairs
}

fn write_request(stdin: &mut impl Write, request: &[u8]) -> Result<()> {
    let len = u32::try_from(request.len())
        .map_err(|_| CalyxError::lens_dim_mismatch("multimodal request too large"))?;
    stdin
        .write_all(&len.to_be_bytes())
        .and_then(|_| stdin.write_all(request))
        .map_err(|err| CalyxError::lens_unreachable(format!("multimodal write failed: {err}")))
}

fn read_response(stdout: &mut impl Read) -> Result<Vec<u8>> {
    let mut header = [0_u8; 4];
    stdout.read_exact(&mut header).map_err(|err| {
        CalyxError::lens_unreachable(format!("multimodal response header read failed: {err}"))
    })?;
    let len = u32::from_be_bytes(header) as usize;
    let mut body = vec![0_u8; len];
    stdout.read_exact(&mut body).map_err(|err| {
        CalyxError::lens_unreachable(format!("multimodal response body read failed: {err}"))
    })?;
    Ok(body)
}

fn finish_child(child: &mut std::process::Child) {
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

mod cleanup_job;
use cleanup_job::assign_child_to_cleanup_job;

mod gpu_paths;
use gpu_paths::cuda_ld_library_path;
#[cfg(windows)]
use gpu_paths::gpu_dll_path;
