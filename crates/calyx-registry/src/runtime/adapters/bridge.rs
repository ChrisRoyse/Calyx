use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, TryRecvError};
use std::thread;
use std::time::Instant;

use calyx_core::{CalyxError, Input, Result};
use serde::{Deserialize, Serialize};

use super::config::MultimodalAdapterConfig;

#[derive(Serialize)]
struct AdapterRequest<'a> {
    inputs: Vec<&'a [u8]>,
}

#[derive(Deserialize)]
struct AdapterResponse {
    vectors: Vec<Vec<f32>>,
}

pub fn measure_batch(config: &MultimodalAdapterConfig, inputs: &[Input]) -> Result<Vec<Vec<f32>>> {
    let request = AdapterRequest {
        inputs: inputs.iter().map(|input| input.bytes.as_slice()).collect(),
    };
    let request = serde_json::to_vec(&request).map_err(|err| {
        CalyxError::lens_unreachable(format!("multimodal request encode failed: {err}"))
    })?;
    let body = run_frame(config, &request)?;
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

fn run_frame(config: &MultimodalAdapterConfig, request: &[u8]) -> Result<Vec<u8>> {
    if config.timeout.is_zero() {
        return Err(CalyxError::lens_unreachable(
            "multimodal adapter timed out before spawn",
        ));
    }
    let mut child = Command::new(&config.command)
        .arg(&config.helper)
        .arg("--config")
        .arg(&config.path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| {
            CalyxError::lens_unreachable(format!(
                "spawn multimodal adapter {} failed: {err}",
                config.command
            ))
        })?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| CalyxError::lens_unreachable("multimodal stdin pipe missing"))?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| CalyxError::lens_unreachable("multimodal stdout pipe missing"))?;
    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| CalyxError::lens_unreachable("multimodal stderr pipe missing"))?;

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

    let (stderr_tx, stderr_rx) = mpsc::channel();
    thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = stderr.read_to_end(&mut bytes).map(|_| bytes);
        let _ = stderr_tx.send(result);
    });

    let deadline = Instant::now() + config.timeout;
    let mut write_result = None;
    let mut body = None;
    let mut status = None;
    let mut stderr_bytes = None;
    loop {
        poll_write(&write_rx, &mut write_result, &mut child)?;
        poll_body(&read_rx, &mut body, &mut child)?;
        if status.is_none() {
            status = child.try_wait().map_err(|err| {
                CalyxError::lens_unreachable(format!("multimodal wait failed: {err}"))
            })?;
        }
        if stderr_bytes.is_none() {
            match stderr_rx.try_recv() {
                Ok(Ok(bytes)) => stderr_bytes = Some(bytes),
                Ok(Err(err)) => {
                    return Err(CalyxError::lens_unreachable(format!(
                        "multimodal stderr read failed: {err}"
                    )));
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => stderr_bytes = Some(Vec::new()),
            }
        }
        if write_result.is_some() && body.is_some() && status.is_some() {
            break;
        }
        let now = Instant::now();
        if now >= deadline {
            let _ = child.kill();
            finish_child(&mut child);
            return Err(CalyxError::lens_unreachable(format!(
                "multimodal adapter timed out after {} ms",
                config.timeout.as_millis()
            )));
        }
        thread::sleep((deadline - now).min(std::time::Duration::from_millis(5)));
    }

    write_result.expect("write result is set")?;
    let status = status.expect("child status is set");
    if !status.success() {
        let stderr = String::from_utf8_lossy(stderr_bytes.as_deref().unwrap_or_default());
        return Err(CalyxError::lens_unreachable(format!(
            "multimodal adapter exited with {status}: {}",
            stderr.trim()
        )));
    }
    body.expect("body result is set")
}

fn poll_write(
    rx: &mpsc::Receiver<Result<()>>,
    slot: &mut Option<Result<()>>,
    child: &mut std::process::Child,
) -> Result<()> {
    if slot.is_some() {
        return Ok(());
    }
    match rx.try_recv() {
        Ok(result) => *slot = Some(result),
        Err(TryRecvError::Empty) => {}
        Err(TryRecvError::Disconnected) => {
            let _ = child.kill();
            finish_child(child);
            return Err(CalyxError::lens_unreachable(
                "multimodal write worker stopped",
            ));
        }
    }
    Ok(())
}

fn poll_body(
    rx: &mpsc::Receiver<Result<Vec<u8>>>,
    slot: &mut Option<Result<Vec<u8>>>,
    child: &mut std::process::Child,
) -> Result<()> {
    if slot.is_some() {
        return Ok(());
    }
    match rx.try_recv() {
        Ok(result) => *slot = Some(result),
        Err(TryRecvError::Empty) => {}
        Err(TryRecvError::Disconnected) => {
            let _ = child.kill();
            finish_child(child);
            return Err(CalyxError::lens_unreachable(
                "multimodal read worker stopped",
            ));
        }
    }
    Ok(())
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
    let _ = child.wait();
}
