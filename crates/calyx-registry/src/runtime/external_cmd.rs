use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};

use calyx_core::{CalyxError, Input, Lens, LensId, Modality, Result, SlotShape, SlotVector};
use serde::{Deserialize, Serialize};

use crate::frozen::{FrozenLensContract, LensDType, NormPolicy, sha256_digest};
use crate::lens::ensure_input_modality;

#[derive(Clone, Debug)]
pub struct ExternalCmdLens {
    id: LensId,
    cmd: String,
    args: Vec<String>,
    modality: Modality,
    dim: u32,
    timeout: Duration,
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
        let response = run_frame(&self.cmd, &self.args, &request, self.timeout)?;
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

fn run_frame(cmd: &str, args: &[String], request: &[u8], timeout: Duration) -> Result<Vec<u8>> {
    if timeout.is_zero() {
        return Err(CalyxError::lens_unreachable(
            "external process timed out before spawn",
        ));
    }
    let mut child = Command::new(cmd)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| CalyxError::lens_unreachable(format!("spawn {cmd} failed: {err}")))?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| CalyxError::lens_unreachable("external stdin pipe missing"))?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| CalyxError::lens_unreachable("external stdout pipe missing"))?;

    let (write_tx, write_rx) = mpsc::channel();
    let request = request.to_vec();
    thread::spawn(move || {
        let result = write_request(&mut stdin, &request);
        let _ = write_tx.send(result);
    });

    let (read_tx, read_rx) = mpsc::channel();
    thread::spawn(move || {
        let result = read_response(&mut stdout);
        let _ = read_tx.send(result);
    });

    let deadline = Instant::now() + timeout;
    let mut write_result = None;
    let mut body = None;
    let mut status = None;
    loop {
        if write_result.is_none() {
            match write_rx.try_recv() {
                Ok(result) => write_result = Some(result),
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    let _ = child.kill();
                    finish_child(&mut child);
                    return Err(CalyxError::lens_unreachable(
                        "external write worker stopped",
                    ));
                }
            }
        }
        if body.is_none() {
            match read_rx.try_recv() {
                Ok(result) => body = Some(result),
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    let _ = child.kill();
                    finish_child(&mut child);
                    return Err(CalyxError::lens_unreachable("external read worker stopped"));
                }
            }
        }
        if status.is_none() {
            status = child.try_wait().map_err(|err| {
                CalyxError::lens_unreachable(format!("external wait failed: {err}"))
            })?;
        }
        if write_result.is_some() && body.is_some() && status.is_some() {
            break;
        }
        let now = Instant::now();
        if now >= deadline {
            let _ = child.kill();
            finish_child(&mut child);
            return Err(CalyxError::lens_unreachable(format!(
                "external process timed out after {} ms",
                timeout.as_millis()
            )));
        }
        let remaining = deadline.saturating_duration_since(now);
        thread::sleep(remaining.min(Duration::from_millis(5)));
    }

    write_result.expect("write result is set")?;
    let body = body.expect("body result is set")?;
    let status = status.expect("child status is set");
    if !status.success() {
        return Err(CalyxError::lens_unreachable(format!(
            "external process exited with {status}"
        )));
    }
    Ok(body)
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

fn finish_child(child: &mut std::process::Child) {
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
mod tests {
    use super::*;
    use calyx_core::Input;
    use serde_json::json;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn timeout_kills_slow_external_process_before_finished_marker() {
        let fsv_root = std::env::var_os("CALYX_FSV_ROOT").map(PathBuf::from);
        let dir = fsv_root.as_ref().map_or_else(
            || test_dir("external-timeout"),
            |root| {
                let dir = root.join("external-timeout");
                let _ = fs::remove_dir_all(&dir);
                fs::create_dir_all(&dir).unwrap();
                dir
            },
        );
        let marker = dir.join("marker.txt");
        let script = format!(
            "import pathlib,time; p=pathlib.Path({}); p.write_text('started\\n'); time.sleep(5); p.write_text(p.read_text() + 'finished\\n')",
            serde_json::to_string(marker.to_str().unwrap()).unwrap()
        );
        let lens = ExternalCmdLens::new(
            "external-timeout",
            "python3",
            vec!["-c".to_string(), script],
            Modality::Text,
            4,
        )
        .with_timeout(Duration::from_millis(100));

        let started = Instant::now();
        let error = lens
            .measure(&Input::new(Modality::Text, b"slow".to_vec()))
            .expect_err("slow command times out");
        let elapsed = started.elapsed();
        let marker_text = fs::read_to_string(&marker).unwrap();

        assert_eq!(error.code, "CALYX_LENS_UNREACHABLE");
        assert!(error.message.contains("timed out"));
        assert!(elapsed < Duration::from_secs(2));
        assert_eq!(marker_text, "started\n");

        if let Some(root) = fsv_root {
            write_timeout_readback(&root, &marker, &marker_text, elapsed, &error);
        } else {
            cleanup(dir);
        }
    }

    fn write_timeout_readback(
        root: &Path,
        marker: &Path,
        marker_text: &str,
        elapsed: Duration,
        error: &CalyxError,
    ) {
        fs::create_dir_all(root).unwrap();
        let readback = json!({
            "marker": marker,
            "marker_text": marker_text,
            "elapsed_ms": elapsed.as_millis(),
            "error_code": error.code,
            "error_message": error.message,
        });
        fs::write(
            root.join("external-cmd-timeout-readback.json"),
            serde_json::to_vec_pretty(&readback).unwrap(),
        )
        .unwrap();
    }

    fn test_dir(name: &str) -> PathBuf {
        let id = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("calyx-registry-{name}-{}-{id}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn cleanup(dir: PathBuf) {
        fs::remove_dir_all(dir).unwrap();
    }
}
