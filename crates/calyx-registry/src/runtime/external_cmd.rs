use std::fmt;
use std::io::{Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use calyx_core::{CalyxError, Input, Lens, LensId, Modality, Result, SlotShape, SlotVector};
use serde::{Deserialize, Serialize};

use crate::frozen::{FrozenLensContract, LensDType, NormPolicy, sha256_digest};
use crate::lens::ensure_input_modality;

#[derive(Clone)]
pub struct ExternalCmdLens {
    id: LensId,
    cmd: String,
    args: Vec<String>,
    modality: Modality,
    dim: u32,
    timeout: Duration,
    worker: Arc<Mutex<Option<Arc<ExternalWorker>>>>,
}

impl fmt::Debug for ExternalCmdLens {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExternalCmdLens")
            .field("id", &self.id)
            .field("cmd", &self.cmd)
            .field("args", &self.args)
            .field("modality", &self.modality)
            .field("dim", &self.dim)
            .field("timeout", &self.timeout)
            .finish_non_exhaustive()
    }
}

#[derive(Serialize)]
struct ExternalRequest<'a> {
    modality: Modality,
    inputs: Vec<&'a [u8]>,
}

#[derive(Deserialize)]
struct ExternalResponse {
    vectors: Vec<Vec<f32>>,
}

impl ExternalCmdLens {
    pub fn new(
        name: impl Into<String>,
        cmd: impl Into<String>,
        args: Vec<String>,
        modality: Modality,
        dim: u32,
    ) -> Self {
        let name = name.into();
        let cmd = cmd.into();
        let args_text = args.join("\0");
        let weights = sha256_digest(&[cmd.as_bytes(), args_text.as_bytes()]);
        let corpus = sha256_digest(&[b"external-cmd-runtime-v1"]);
        let contract = FrozenLensContract::new(
            name,
            weights,
            corpus,
            SlotShape::Dense(dim),
            modality,
            LensDType::F32,
            NormPolicy::None,
        );
        Self {
            id: contract.lens_id(),
            cmd,
            args,
            modality,
            dim,
            timeout: Duration::from_secs(30),
            worker: Arc::new(Mutex::new(None)),
        }
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn command(&self) -> (&str, &[String]) {
        (&self.cmd, &self.args)
    }
}

impl Lens for ExternalCmdLens {
    fn id(&self) -> LensId {
        self.id
    }

    fn shape(&self) -> SlotShape {
        SlotShape::Dense(self.dim)
    }

    fn modality(&self) -> Modality {
        self.modality
    }

    fn measure(&self, input: &Input) -> Result<SlotVector> {
        let mut batch = self.measure_batch(std::slice::from_ref(input))?;
        batch.pop().ok_or_else(|| {
            CalyxError::lens_unreachable(format!("external lens {} returned no vector", self.id))
        })
    }

    fn measure_batch(&self, inputs: &[Input]) -> Result<Vec<SlotVector>> {
        for input in inputs {
            ensure_input_modality(self, input)?;
        }
        let request = ExternalRequest {
            modality: self.modality,
            inputs: inputs.iter().map(|input| input.bytes.as_slice()).collect(),
        };
        let request = serde_json::to_vec(&request).map_err(|err| {
            CalyxError::lens_unreachable(format!("external request encode failed: {err}"))
        })?;
        let response = self.request_frame(request)?;
        let response: ExternalResponse = serde_json::from_slice(&response).map_err(|err| {
            CalyxError::lens_unreachable(format!("external response decode failed: {err}"))
        })?;
        if response.vectors.len() != inputs.len() {
            return Err(CalyxError::lens_dim_mismatch(format!(
                "external lens returned {} vectors for {} inputs",
                response.vectors.len(),
                inputs.len()
            )));
        }
        response
            .vectors
            .into_iter()
            .map(|data| self.slot_from_row(data))
            .collect()
    }
}

struct ExternalWorker {
    tx: mpsc::Sender<WorkerRequest>,
    child: Arc<Mutex<Child>>,
    stderr_tail: Arc<Mutex<Vec<u8>>>,
}

struct WorkerRequest {
    request: Vec<u8>,
    reply: mpsc::Sender<Result<Vec<u8>>>,
}

enum RequestFailure {
    Error(CalyxError),
    Timeout(CalyxError),
}

impl RequestFailure {
    fn into_error(self) -> CalyxError {
        match self {
            Self::Error(error) | Self::Timeout(error) => error,
        }
    }
}

impl ExternalCmdLens {
    fn request_frame(&self, request: Vec<u8>) -> Result<Vec<u8>> {
        if self.timeout.is_zero() {
            return Err(CalyxError::lens_unreachable(
                "external process timed out before spawn",
            ));
        }
        let worker = self.worker()?;
        match worker.request(self.timeout, request.clone()) {
            Ok(body) => Ok(body),
            Err(RequestFailure::Timeout(error)) => {
                self.clear_worker(&worker);
                Err(error)
            }
            Err(RequestFailure::Error(_)) => {
                self.clear_worker(&worker);
                let retry = self.worker()?;
                retry
                    .request(self.timeout, request)
                    .map_err(RequestFailure::into_error)
                    .inspect_err(|_| self.clear_worker(&retry))
            }
        }
    }

    fn worker(&self) -> Result<Arc<ExternalWorker>> {
        let mut guard = self.worker.lock().map_err(|_| {
            CalyxError::lens_unreachable("external worker cache mutex was poisoned")
        })?;
        if let Some(worker) = guard.as_ref() {
            return Ok(worker.clone());
        }
        let worker = Arc::new(ExternalWorker::spawn(&self.cmd, &self.args)?);
        *guard = Some(worker.clone());
        Ok(worker)
    }

    fn clear_worker(&self, worker: &Arc<ExternalWorker>) {
        if let Ok(mut guard) = self.worker.lock()
            && guard
                .as_ref()
                .is_some_and(|cached| Arc::ptr_eq(cached, worker))
        {
            *guard = None;
        }
    }
}

impl ExternalWorker {
    fn spawn(cmd: &str, args: &[String]) -> Result<Self> {
        let mut child = spawn_child(cmd, args)?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| CalyxError::lens_unreachable("external stdin pipe missing"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| CalyxError::lens_unreachable("external stdout pipe missing"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| CalyxError::lens_unreachable("external stderr pipe missing"))?;
        let child = Arc::new(Mutex::new(child));
        let stderr_tail = Arc::new(Mutex::new(Vec::new()));
        spawn_stderr_reader(stderr, stderr_tail.clone());

        let (tx, rx) = mpsc::channel();
        let child_for_worker = child.clone();
        let stderr_for_worker = stderr_tail.clone();
        thread::spawn(move || worker_loop(child_for_worker, stdin, stdout, rx, stderr_for_worker));
        Ok(Self {
            tx,
            child,
            stderr_tail,
        })
    }

    fn request(
        &self,
        timeout: Duration,
        request: Vec<u8>,
    ) -> std::result::Result<Vec<u8>, RequestFailure> {
        let (reply, rx) = mpsc::channel();
        self.tx
            .send(WorkerRequest { request, reply })
            .map_err(|_| {
                RequestFailure::Error(CalyxError::lens_unreachable(format!(
                    "external worker stopped before request; stderr_tail={}",
                    stderr_tail_text(&self.stderr_tail)
                )))
            })?;
        match rx.recv_timeout(timeout) {
            Ok(result) => result.map_err(RequestFailure::Error),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                kill_child(&self.child);
                Err(RequestFailure::Timeout(CalyxError::lens_unreachable(
                    format!(
                        "external process timed out after {} ms; stderr_tail={}",
                        timeout.as_millis(),
                        stderr_tail_text(&self.stderr_tail)
                    ),
                )))
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(RequestFailure::Error(
                CalyxError::lens_unreachable(format!(
                    "external worker disconnected; stderr_tail={}",
                    stderr_tail_text(&self.stderr_tail)
                )),
            )),
        }
    }
}

fn spawn_child(cmd: &str, args: &[String]) -> Result<Child> {
    let mut command = Command::new(cmd);
    command.args(args);
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
    command
        .spawn()
        .map_err(|err| CalyxError::lens_unreachable(format!("spawn {cmd} failed: {err}")))
}

fn worker_loop(
    child: Arc<Mutex<Child>>,
    mut stdin: std::process::ChildStdin,
    mut stdout: std::process::ChildStdout,
    rx: mpsc::Receiver<WorkerRequest>,
    stderr_tail: Arc<Mutex<Vec<u8>>>,
) {
    for item in rx {
        let result = write_request(&mut stdin, &item.request)
            .and_then(|_| read_response(&mut stdout))
            .map_err(|error| enrich_worker_error(error, &child, &stderr_tail));
        let failed = result.is_err();
        let _ = item.reply.send(result);
        if failed {
            break;
        }
    }
    drop(stdin);
    finish_child(&child);
}

fn enrich_worker_error(
    error: CalyxError,
    child: &Arc<Mutex<Child>>,
    stderr_tail: &Arc<Mutex<Vec<u8>>>,
) -> CalyxError {
    CalyxError::lens_unreachable(format!(
        "{}; child_status={}; stderr_tail={}",
        error.message,
        child_status(child),
        stderr_tail_text(stderr_tail)
    ))
}

fn child_status(child: &Arc<Mutex<Child>>) -> String {
    let Ok(mut child) = child.lock() else {
        return "child_mutex_poisoned".to_string();
    };
    child
        .try_wait()
        .ok()
        .flatten()
        .map(|status| status.to_string())
        .unwrap_or_else(|| "still_running".to_string())
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
    String::from_utf8_lossy(&tail).trim().to_string()
}

fn write_request(stdin: &mut impl Write, request: &[u8]) -> Result<()> {
    let len = u32::try_from(request.len())
        .map_err(|_| CalyxError::lens_dim_mismatch("external request too large"))?;
    stdin
        .write_all(&len.to_be_bytes())
        .and_then(|_| stdin.write_all(request))
        .map_err(|err| CalyxError::lens_unreachable(format!("external write failed: {err}")))
}

fn read_response(stdout: &mut impl Read) -> Result<Vec<u8>> {
    let mut header = [0_u8; 4];
    stdout.read_exact(&mut header).map_err(|err| {
        CalyxError::lens_unreachable(format!("external response header read failed: {err}"))
    })?;
    let len = u32::from_be_bytes(header) as usize;
    let mut body = vec![0_u8; len];
    stdout.read_exact(&mut body).map_err(|err| {
        CalyxError::lens_unreachable(format!("external response body read failed: {err}"))
    })?;
    Ok(body)
}

fn kill_child(child: &Arc<Mutex<Child>>) {
    if let Ok(mut child) = child.lock() {
        let _ = child.kill();
    }
}

fn finish_child(child: &Arc<Mutex<Child>>) {
    let Ok(mut child) = child.lock() else {
        return;
    };
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

impl ExternalCmdLens {
    fn slot_from_row(&self, data: Vec<f32>) -> Result<SlotVector> {
        if data.len() != self.dim as usize {
            return Err(CalyxError::lens_dim_mismatch(format!(
                "external dim {} != expected {}",
                data.len(),
                self.dim
            )));
        }
        if data.iter().any(|value| !value.is_finite()) {
            return Err(CalyxError::lens_numerical_invariant(
                "external vector contains NaN or Inf",
            ));
        }
        Ok(SlotVector::Dense {
            dim: self.dim,
            data,
        })
    }
}

#[cfg(all(test, unix))]
mod tests;
