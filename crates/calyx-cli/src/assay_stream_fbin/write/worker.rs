use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::ExitStatus;
use std::time::Instant;

#[cfg(not(test))]
use std::{env, process::Command};

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::assay_corpus_build::lens::projection::projected_slot_dim;
use crate::error::CliResult;

use super::super::args::Args;
use super::super::rows::{self, RowStats};
use super::super::{io_error, local_error};
use super::evidence::LensEvidence;
use super::paths::{display, display_final, lens_prefix};
use super::progress::LensProgressMeta;
use super::selection::{SelectedLens, selected_lenses_for_worker};
use super::{LensStream, create_sink, elapsed_ms, finish_sink, stream_lens};

mod report;
use report::read_worker_report;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct StreamWorkerReport {
    pub(crate) slot: u16,
    pub(crate) name: String,
    pub(crate) lens_id: String,
    pub(crate) weights_sha256: String,
    pub(crate) signal_kind: String,
    pub(crate) bits_about: f32,
    pub(crate) dim: usize,
    pub(crate) native_dim: usize,
    pub(crate) assay_projection: String,
    pub(crate) max_batch: Option<usize>,
    pub(crate) effective_batch_size: usize,
    pub(crate) elapsed_ms: u64,
    pub(crate) ms_per_input: f64,
    pub(crate) manifest: String,
    pub(crate) corpus_rows_written: usize,
    pub(crate) query_rows_written: usize,
    pub(crate) worker_pid: Option<u32>,
    pub(crate) worker_report_path: Option<String>,
    pub(crate) worker_stderr_path: Option<String>,
}

impl StreamWorkerReport {
    pub(crate) fn into_lens_evidence(self, args: &Args) -> CliResult<LensEvidence> {
        let prefix = lens_prefix(self.slot as usize, &self.name);
        let ext = args.vector_format.extension();
        let manifest = if args.lens_template_cf_root.is_some() {
            args.lens_descriptor_ref(&self.name)
        } else {
            self.manifest.clone()
        };
        Ok(LensEvidence {
            slot: self.slot,
            name: self.name,
            lens_id: self.lens_id,
            weights_sha256: self.weights_sha256,
            signal_kind: self.signal_kind,
            bits_about: self.bits_about,
            dim: self.dim,
            native_dim: self.native_dim,
            assay_projection: self.assay_projection,
            max_batch: self.max_batch,
            effective_batch_size: self.effective_batch_size,
            elapsed_ms: self.elapsed_ms,
            ms_per_input: self.ms_per_input,
            manifest,
            corpus_path: display_final(
                args,
                &format!("{}/{prefix}_corpus.{ext}", args.vector_format.dir_name()),
            ),
            queries_path: display_final(
                args,
                &format!("{}/{prefix}_queries.{ext}", args.vector_format.dir_name()),
            ),
            vault_path: display_final(args, &format!("vaults/{prefix}")),
            corpus_rows_written: self.corpus_rows_written,
            query_rows_written: self.query_rows_written,
            worker_pid: self.worker_pid,
            worker_report_path: self.worker_report_path,
            worker_stderr_path: self.worker_stderr_path,
        })
    }
}

pub(super) fn worker_root(out_dir: &Path) -> PathBuf {
    let name = out_dir
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("assay-stream-fbin");
    out_dir.with_file_name(format!(".{name}.workers-{}", std::process::id()))
}

pub(super) fn lens_meta(
    args: &Args,
    slot: usize,
    selected: &SelectedLens,
) -> CliResult<LensProgressMeta> {
    let spec = &selected.spec;
    let effective_batch_size = spec
        .max_batch
        .filter(|value| *value > 0)
        .map(|value| value.min(args.batch_size))
        .unwrap_or(args.batch_size);
    let name = spec.name.clone();
    let lens_id = spec.lens_id().to_string();
    Ok(LensProgressMeta {
        slot,
        name,
        lens_id,
        weights_sha256: hex(&spec.weights_sha256),
        bits_about: selected.bits.bits_about,
        dim: projected_slot_dim(spec.output) as usize,
        max_batch: spec.max_batch,
        effective_batch_size,
        manifest: selected.descriptor_ref.clone(),
    })
}

pub(super) fn run_one_worker(
    args: &Args,
    stats: &RowStats,
    slot: usize,
    selected: &SelectedLens,
    staging: &Path,
    root: &Path,
) -> CliResult<StreamWorkerReport> {
    if selected.manifest.is_none() {
        let mut runtime_args = args.clone();
        runtime_args.out_dir = staging.to_path_buf();
        eprintln!(
            "{}",
            json!({
                "event": "assay_stream_fbin_worker_start",
                "slot": slot,
                "descriptor": selected.descriptor_ref,
                "rows": stats.rows
            })
        );
        let report = stream_selected(&runtime_args, stats, slot, selected, None)?;
        verify_worker_counts(args, stats, slot, &report)?;
        eprintln!(
            "{}",
            json!({
                "event": "assay_stream_fbin_worker_finish",
                "slot": slot,
                "lens": report.name,
                "worker_pid": report.worker_pid,
                "corpus_rows": report.corpus_rows_written,
                "query_rows": report.query_rows_written
            })
        );
        return Ok(report);
    }
    let report_path = root.join(format!("lens-{slot:02}.json"));
    let stdout_path = root.join(format!("lens-{slot:02}.stdout.json"));
    let stderr_path = root.join(format!("lens-{slot:02}.stderr.log"));
    remove_stale(&report_path)?;
    let stdout = File::create(&stdout_path).map_err(io_error)?;
    let stderr = File::create(&stderr_path).map_err(io_error)?;
    eprintln!(
        "{}",
        json!({
            "event": "assay_stream_fbin_worker_start",
            "slot": slot,
            "descriptor": selected.descriptor_ref,
            "worker_report": report_path,
            "rows": stats.rows
        })
    );
    let status = run_worker_process(args, selected, slot, staging, &report_path, stdout, stderr)?;
    let mut report =
        read_worker_report(&report_path, &stderr_path, status, &selected.descriptor_ref)?;
    verify_worker_counts(args, stats, slot, &report)?;
    report.worker_report_path = Some(display(&report_path));
    report.worker_stderr_path = Some(display(&stderr_path));
    eprintln!(
        "{}",
        json!({
            "event": "assay_stream_fbin_worker_finish",
            "slot": slot,
            "lens": report.name,
            "worker_pid": report.worker_pid,
            "corpus_rows": report.corpus_rows_written,
            "query_rows": report.query_rows_written
        })
    );
    Ok(report)
}

#[cfg(not(test))]
fn run_worker_process(
    args: &Args,
    selected: &SelectedLens,
    slot: usize,
    staging: &Path,
    report: &Path,
    stdout: File,
    stderr: File,
) -> Result<ExitStatus, crate::error::CliError> {
    let mut command = Command::new(env::current_exe().map_err(io_error)?);
    add_worker_args(&mut command, args, selected, slot, staging, report);
    if let Some(limit_mib) = args.worker_gpu_mem_limit_mib {
        // #1160/#1143: every co-resident worker session gets an explicit CUDA
        // arena budget so K-way parallelism fails at a defined limit.
        command.env(
            crate::assay_corpus_build::parallel::GPU_MEM_LIMIT_ENV,
            limit_mib.to_string(),
        );
    }
    command
        .stdout(stdout)
        .stderr(stderr)
        .status()
        .map_err(io_error)
}

#[cfg(test)]
fn run_worker_process(
    args: &Args,
    selected: &SelectedLens,
    slot: usize,
    staging: &Path,
    report: &Path,
    _stdout: File,
    _stderr: File,
) -> Result<ExitStatus, crate::error::CliError> {
    let mut worker_args = worker_args(args, selected, slot, staging, report);
    worker_args.worker_report = Some(report.to_path_buf());
    worker_args.worker_slot = Some(slot);
    run_worker(&worker_args)?;
    Ok(success_status())
}

pub(crate) fn run_worker(args: &Args) -> CliResult<StreamWorkerReport> {
    let report_path = args.worker_report.as_ref().ok_or_else(|| {
        local_error(
            "CALYX_FSV_ASSAY_STREAM_FBIN_WORKER_CONFIG",
            "missing --worker-report",
            "rerun worker through the parent stream-fbin command",
        )
    })?;
    let slot = args.worker_slot.ok_or_else(|| {
        local_error(
            "CALYX_FSV_ASSAY_STREAM_FBIN_WORKER_CONFIG",
            "missing --worker-slot",
            "rerun worker through the parent stream-fbin command",
        )
    })?;
    let mut selected = selected_lenses_for_worker(args)?;
    let selected = selected.remove(0);
    let stats = rows::scan(args)?;
    stream_selected(args, &stats, slot, &selected, Some(report_path))
}

fn stream_selected(
    args: &Args,
    stats: &RowStats,
    slot: usize,
    selected: &SelectedLens,
    report_path: Option<&Path>,
) -> CliResult<StreamWorkerReport> {
    let lens = selected.load_runtime()?;
    let vector_dir = args.out_dir.join(args.vector_format.dir_name());
    let vault_root = args.out_dir.join("vaults");
    fs::create_dir_all(&vector_dir).map_err(io_error)?;
    fs::create_dir_all(&vault_root).map_err(io_error)?;
    let prefix = lens_prefix(slot, lens.name());
    let ext = args.vector_format.extension();
    let corpus_path = vector_dir.join(format!("{prefix}_corpus.{ext}"));
    let queries_path = vector_dir.join(format!("{prefix}_queries.{ext}"));
    let mut sink = create_sink(&corpus_path, &queries_path, lens.dim(), stats.rows, args)?;
    let effective_batch_size = lens.effective_batch_size(args.batch_size);
    let started = Instant::now();
    stream_lens(
        LensStream {
            args,
            stats,
            lens: &lens,
            effective_batch_size,
            sink: &mut sink,
            progress: None,
        },
        slot == 0,
        &args.out_dir.join("timeline.jsonl"),
    )?;
    let elapsed_ms = elapsed_ms(started.elapsed())?;
    finish_sink(&mut sink)?;
    let report = StreamWorkerReport {
        slot: u16::try_from(slot).map_err(|_| {
            local_error(
                "CALYX_FSV_ASSAY_STREAM_FBIN_WORKER_CONFIG",
                "slot exceeds u16",
                "use fewer stream-fbin slots",
            )
        })?,
        name: lens.name().to_string(),
        lens_id: lens.lens_id(),
        weights_sha256: lens.weights_sha256_hex(),
        signal_kind: lens.signal_kind().to_string(),
        bits_about: selected.bits.bits_about,
        dim: lens.dim(),
        native_dim: lens.native_dim(),
        assay_projection: lens.assay_projection().to_string(),
        max_batch: lens.max_batch(),
        effective_batch_size,
        elapsed_ms,
        ms_per_input: elapsed_ms as f64 / stats.rows.max(1) as f64,
        manifest: display(lens.manifest()),
        corpus_rows_written: sink.corpus_written,
        query_rows_written: sink.query_written,
        worker_pid: Some(std::process::id()),
        worker_report_path: report_path.map(display),
        worker_stderr_path: None,
    };
    if let Some(report_path) = report_path {
        if let Some(parent) = report_path.parent() {
            fs::create_dir_all(parent).map_err(io_error)?;
        }
        fs::write(
            report_path,
            serde_json::to_vec(&report).map_err(|error| {
                crate::error::CliError::runtime(format!("serialize worker report: {error}"))
            })?,
        )
        .map_err(io_error)?;
    }
    Ok(report)
}

fn verify_worker_counts(
    args: &Args,
    stats: &RowStats,
    slot: usize,
    report: &StreamWorkerReport,
) -> CliResult {
    if report.corpus_rows_written != stats.rows || report.query_rows_written != args.query_count {
        return Err(local_error(
            "CALYX_FSV_ASSAY_STREAM_FBIN_WORKER_COUNT_MISMATCH",
            format!(
                "slot={slot} corpus={} queries={} expected corpus={} queries={}",
                report.corpus_rows_written, report.query_rows_written, stats.rows, args.query_count
            ),
            "inspect worker report and streamed row selection before trusting FBIN output",
        ));
    }
    Ok(())
}

#[cfg(not(test))]
fn add_worker_args(
    command: &mut Command,
    args: &Args,
    selected: &SelectedLens,
    slot: usize,
    staging: &Path,
    report: &Path,
) {
    command
        .arg("assay")
        .arg("stream-fbin")
        .arg("--rows-jsonl")
        .arg(&args.rows_jsonl)
        .arg("--out-dir")
        .arg(staging)
        .arg("--dataset")
        .arg(&args.dataset)
        .arg("--target-class")
        .arg(args.target_class.to_string())
        .arg("--query-count")
        .arg(args.query_count.to_string())
        .arg("--batch-size")
        .arg(args.batch_size.to_string())
        .arg("--manifest")
        .arg(
            selected
                .manifest
                .as_ref()
                .expect("file-mode worker must have a manifest"),
        )
        .arg("--min-bits")
        .arg(args.min_bits.to_string())
        .arg("--vector-format")
        .arg(args.vector_format.as_str())
        .arg("--mode")
        .arg(args.mode.as_str())
        .arg("--worker-report")
        .arg(report)
        .arg("--worker-slot")
        .arg(slot.to_string());
    add_admission_args(command, args);
    if let Some(limit) = args.limit_per_class {
        command.arg("--limit-per-class").arg(limit.to_string());
    }
    if let Some(path) = &args.cost_override_json {
        command.arg("--cost-override-json").arg(path);
    }
    if let Some(id) = &args.embedding_model_id {
        command.arg("--embedding-model-id").arg(id);
    }
    if !args.emit_artifacts {
        command.arg("--db-only");
    }
}

#[cfg(not(test))]
fn add_admission_args(command: &mut Command, args: &Args) {
    if let Some(path) = &args.bits_report {
        command.arg("--bits-report").arg(path);
    }
    if let Some(path) = &args.a37_admission_cf_root {
        command.arg("--a37-admission-cf-root").arg(path);
        command
            .arg("--a37-admission-key")
            .arg(&args.a37_admission_key);
    }
}

#[cfg(test)]
fn worker_args(
    args: &Args,
    selected: &SelectedLens,
    slot: usize,
    staging: &Path,
    report: &Path,
) -> Args {
    let mut worker_args = args.clone();
    worker_args.out_dir = staging.to_path_buf();
    worker_args.manifests = vec![
        selected
            .manifest
            .clone()
            .expect("file-mode worker must have a manifest"),
    ];
    worker_args.worker_report = Some(report.to_path_buf());
    worker_args.worker_slot = Some(slot);
    worker_args
}

fn remove_stale(path: &Path) -> CliResult {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(io_error(error)),
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
fn success_status() -> ExitStatus {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        ExitStatus::from_raw(0)
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::ExitStatusExt;
        ExitStatus::from_raw(0)
    }
}
