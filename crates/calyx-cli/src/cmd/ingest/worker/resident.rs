use super::frame::{decode_binary, encode_binary, read_frame, write_frame};
use super::*;

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

pub(crate) fn measure_lens_in_worker(
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

pub(crate) fn append_tail(tail: &Arc<Mutex<Vec<u8>>>, bytes: &[u8]) {
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

pub(crate) fn stderr_tail_text(tail: &Arc<Mutex<Vec<u8>>>) -> String {
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
