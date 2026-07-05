use std::env;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};

use serde_json::json;

use super::data::{self, BuildRows};
use super::lens::{self, MeasuredLens};
use super::request::CorpusBuildRequest;
use crate::error::{CliError, CliResult};
use crate::output::print_json;

pub(crate) fn run_worker(request: &CorpusBuildRequest) -> CliResult {
    let rows = data::load_rows(request).map_err(CliError::runtime)?;
    let mut measured = lens::measure_requested_lenses(request, &rows).map_err(CliError::runtime)?;
    let Some(mut lens) = measured.pop() else {
        return Err(CliError::runtime(worker_error(
            "CALYX_FSV_ASSAY_CORPUS_BUILD_WORKER_EMPTY",
            "worker measured no lens".to_string(),
        )));
    };
    lens.worker_pid = Some(std::process::id());
    let report = request
        .worker_report
        .as_ref()
        .expect("worker_report checked by request validation");
    if let Some(parent) = report.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(
        report,
        serde_json::to_vec(&lens)
            .map_err(|error| CliError::runtime(format!("serialize worker report: {error}")))?,
    )?;
    print_json(&json!({
        "worker_report": report,
        "worker_pid": std::process::id(),
        "lens": lens.name,
        "vectors": lens.vectors.len()
    }))?;
    Ok(())
}

pub(crate) fn measure_requested_lenses(
    request: &CorpusBuildRequest,
    rows: &BuildRows,
) -> Result<Vec<MeasuredLens>, String> {
    let root = worker_root(&request.out_dir);
    fs::create_dir_all(&root).map_err(io_error)?;
    if request.lens_parallelism <= 1 {
        let mut measured = Vec::with_capacity(request.manifests.len());
        for (idx, manifest) in request.manifests.iter().enumerate() {
            measured.push(run_one_worker(request, rows, idx, manifest, &root)?);
        }
        return Ok(measured);
    }
    measure_lenses_parallel(request, rows, &root)
}

/// #1160: schedule up to `lens_parallelism` single-lens worker processes
/// concurrently. Lens indices, per-lens reports, and evidence files are
/// identical to the sequential path; only start order and wall clock change.
/// The first failing lens fails the build with its own attribution after
/// in-flight workers drain.
fn measure_lenses_parallel(
    request: &CorpusBuildRequest,
    rows: &BuildRows,
    root: &Path,
) -> Result<Vec<MeasuredLens>, String> {
    use std::sync::mpsc;

    super::parallel::ensure_worker_vram_safety(
        request.lens_parallelism,
        request.worker_gpu_mem_limit_mib,
        &request.manifests,
    )
    .map_err(|message| worker_error("CALYX_FSV_ASSAY_CORPUS_BUILD_PARALLEL_VRAM", message))?;
    let order = super::parallel::interleaved_start_order(&request.manifests);
    let limit = request.lens_parallelism.min(request.manifests.len());
    eprintln!(
        "{}",
        json!({
            "event": "assay_corpus_build_lens_parallelism",
            "lens_parallelism": limit,
            "lens_total": request.manifests.len(),
            "start_order": order,
        })
    );
    let mut measured: Vec<Option<MeasuredLens>> = Vec::new();
    measured.resize_with(request.manifests.len(), || None);
    let mut failures: Vec<(usize, String)> = Vec::new();
    std::thread::scope(|scope| {
        let (sender, receiver) = mpsc::channel::<(usize, Result<MeasuredLens, String>)>();
        let mut queue = order.into_iter();
        let mut in_flight = 0usize;
        loop {
            while in_flight < limit && failures.is_empty() {
                let Some(idx) = queue.next() else { break };
                let sender = sender.clone();
                let manifest = &request.manifests[idx];
                scope.spawn(move || {
                    let outcome = run_one_worker(request, rows, idx, manifest, root);
                    let _ = sender.send((idx, outcome));
                });
                in_flight += 1;
            }
            if in_flight == 0 {
                break;
            }
            let Ok((idx, outcome)) = receiver.recv() else {
                failures.push((
                    usize::MAX,
                    "lens worker scheduler channel closed unexpectedly".to_string(),
                ));
                break;
            };
            in_flight -= 1;
            match outcome {
                Ok(lens) => measured[idx] = Some(lens),
                Err(error) => failures.push((idx, error)),
            }
        }
    });
    if let Some((idx, error)) = failures.first() {
        for (other_idx, other_error) in failures.iter().skip(1) {
            eprintln!(
                "{}",
                json!({
                    "event": "assay_corpus_build_worker_also_failed",
                    "lens_index": other_idx,
                    "error": other_error,
                })
            );
        }
        return Err(format!("lens_index={idx}: {error}"));
    }
    measured
        .into_iter()
        .enumerate()
        .map(|(idx, lens)| {
            lens.ok_or_else(|| {
                worker_error(
                    "CALYX_FSV_ASSAY_CORPUS_BUILD_WORKER_MISSING",
                    format!("lens worker {idx} produced no report and no error"),
                )
            })
        })
        .collect()
}

fn run_one_worker(
    request: &CorpusBuildRequest,
    rows: &BuildRows,
    idx: usize,
    manifest: &Path,
    root: &Path,
) -> Result<MeasuredLens, String> {
    let report = root.join(format!("lens-{idx:02}.json"));
    let stdout_path = root.join(format!("lens-{idx:02}.stdout.json"));
    let stderr_path = root.join(format!("lens-{idx:02}.stderr.log"));
    remove_stale(&report)?;
    let stdout = File::create(&stdout_path).map_err(io_error)?;
    let stderr = File::create(&stderr_path).map_err(io_error)?;
    let mut command = Command::new(env::current_exe().map_err(io_error)?);
    add_worker_args(&mut command, request, manifest, &report);
    if let Some(limit_mib) = request.worker_gpu_mem_limit_mib {
        // #1160/#1143: every co-resident worker session gets an explicit CUDA
        // arena budget so K-way parallelism fails at a defined limit.
        command.env(super::parallel::GPU_MEM_LIMIT_ENV, limit_mib.to_string());
    }
    eprintln!(
        "{}",
        json!({
            "event": "assay_corpus_build_worker_start",
            "lens_index": idx,
            "manifest": manifest,
            "worker_report": report,
            "input_count": rows.rows.len()
        })
    );
    let status = command
        .stdout(stdout)
        .stderr(stderr)
        .status()
        .map_err(io_error)?;
    let mut lens = read_worker_report(&report, &stderr_path, status, manifest)?;
    lens.worker_report_path = Some(report);
    lens.worker_stderr_path = Some(stderr_path);
    eprintln!(
        "{}",
        json!({
            "event": "assay_corpus_build_worker_finish",
            "lens_index": idx,
            "lens": lens.name,
            "worker_pid": lens.worker_pid,
            "vectors": lens.vectors.len()
        })
    );
    Ok(lens)
}

fn add_worker_args(
    command: &mut Command,
    request: &CorpusBuildRequest,
    manifest: &Path,
    report: &Path,
) {
    command
        .arg("assay")
        .arg("corpus-build")
        .arg("--rows-jsonl")
        .arg(&request.rows_jsonl)
        .arg("--out-dir")
        .arg(&request.out_dir)
        .arg("--dataset")
        .arg(&request.dataset)
        .arg("--target-class")
        .arg(request.target_class.to_string())
        .arg("--batch-size")
        .arg(request.batch_size.to_string())
        .arg("--manifest")
        .arg(manifest)
        .arg("--worker-report")
        .arg(report);
    if let Some(limit) = request.limit_per_class {
        command.arg("--limit-per-class").arg(limit.to_string());
    }
    if let Some(path) = &request.cost_override_json {
        command.arg("--cost-override-json").arg(path);
    }
    if let Some(id) = &request.embedding_model_id {
        command.arg("--embedding-model-id").arg(id);
    }
}

fn read_worker_report(
    report: &Path,
    stderr: &Path,
    status: ExitStatus,
    manifest: &Path,
) -> Result<MeasuredLens, String> {
    let bytes = fs::read(report).map_err(|error| {
        worker_error(
            "CALYX_FSV_ASSAY_CORPUS_BUILD_WORKER_REPORT_MISSING",
            format!(
                "manifest={} status={status}; read {} failed: {error}; {}",
                manifest.display(),
                report.display(),
                stderr_tail(stderr)
            ),
        )
    })?;
    if !status.success() {
        return Err(worker_error(
            "CALYX_FSV_ASSAY_CORPUS_BUILD_WORKER_FAILED",
            format!(
                "manifest={} status={status}; {}; report_bytes={}",
                manifest.display(),
                stderr_tail(stderr),
                bytes.len()
            ),
        ));
    }
    serde_json::from_slice(&bytes).map_err(|error| {
        worker_error(
            "CALYX_FSV_ASSAY_CORPUS_BUILD_WORKER_REPORT_INVALID",
            format!("parse {} failed: {error}", report.display()),
        )
    })
}

fn worker_root(out_dir: &Path) -> PathBuf {
    let name = out_dir
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("assay-corpus");
    out_dir.with_file_name(format!(".{name}.workers-{}", std::process::id()))
}

fn remove_stale(path: &Path) -> Result<(), String> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(io_error(error)),
    }
}

fn stderr_tail(path: &Path) -> String {
    const TAIL_BYTES: usize = 4096;
    match fs::read(path) {
        Ok(bytes) if bytes.is_empty() => format!("stderr {} was empty", path.display()),
        Ok(bytes) => {
            let start = bytes.len().saturating_sub(TAIL_BYTES);
            format!(
                "stderr_tail {}: {}",
                path.display(),
                String::from_utf8_lossy(&bytes[start..]).trim()
            )
        }
        Err(error) => format!("read stderr {} failed: {error}", path.display()),
    }
}

fn worker_error(code: &'static str, message: String) -> String {
    format!("{code}: {message}")
}

fn io_error(error: std::io::Error) -> String {
    error.to_string()
}
