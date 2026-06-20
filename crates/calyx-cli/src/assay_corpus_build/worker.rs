use std::env;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};

use serde_json::json;

use super::data::{self, BuildRows};
use super::lens::{self, MeasuredLens};
use super::request::CorpusBuildRequest;
use crate::error::CliResult;
use crate::output::print_json;

pub(crate) fn run_worker(request: &CorpusBuildRequest) -> CliResult {
    let rows = data::load_rows(request)?;
    let mut measured = lens::measure_requested_lenses(request, &rows)?;
    let Some(mut lens) = measured.pop() else {
        return Err(worker_error(
            "CALYX_FSV_ASSAY_CORPUS_BUILD_WORKER_EMPTY",
            "worker measured no lens".to_string(),
        )
        .into());
    };
    lens.worker_pid = Some(std::process::id());
    let report = request
        .worker_report
        .as_ref()
        .expect("worker_report checked by request validation");
    if let Some(parent) = report.parent() {
        fs::create_dir_all(parent).map_err(io_error)?;
    }
    fs::write(report, serde_json::to_vec(&lens).map_err(json_error)?).map_err(io_error)?;
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
    let mut measured = Vec::with_capacity(request.manifests.len());
    for (idx, manifest) in request.manifests.iter().enumerate() {
        measured.push(run_one_worker(request, rows, idx, manifest, &root)?);
    }
    Ok(measured)
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

fn json_error(error: serde_json::Error) -> String {
    error.to_string()
}
