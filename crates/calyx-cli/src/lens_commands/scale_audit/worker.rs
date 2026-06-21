use std::env;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::ExitStatus;
use std::thread;
use std::time::{Duration, Instant};

use calyx_core::{Modality, Placement};
use calyx_registry::LensForgeManifest;
use sha2::{Digest, Sha256};

use super::measure::audit_lens;
use super::model::{Flags, LensAudit, reject};
use super::report::{ProgressUpdate, write_lens_audit, write_progress};
use super::runtime::is_content_modality;
use crate::error::{CliError, CliResult};
use crate::output::print_json;

pub(super) fn run_worker(flags: &Flags) -> CliResult {
    if flags.manifests.len() != 1 {
        return Err(CliError::usage(
            "calyx lens scale-audit --worker requires exactly one --manifest",
        ));
    }
    let manifest = flags.manifests[0].clone();
    let spec = calyx_registry::lens_spec_from_manifest_path(&manifest)?;
    let audit = audit_lens(manifest, spec, flags);
    write_lens_audit(&flags.out, &audit)?;
    print_json(&audit)?;
    Ok(())
}

pub(super) fn audit_lens_with_timeout(
    flags: &Flags,
    idx: usize,
    manifest: &PathBuf,
    lenses: &[LensAudit],
) -> CliResult<LensAudit> {
    let paths = worker_paths(&flags.out, idx);
    remove_stale_worker_report(&paths.report)?;
    let stdout = File::create(&paths.stdout)?;
    let stderr = File::create(&paths.stderr)?;
    let mut command = std::process::Command::new(env::current_exe()?);
    add_worker_args(&mut command, flags, manifest, &paths.report);
    let mut child = command.stdout(stdout).stderr(stderr).spawn()?;
    write_progress(
        flags,
        idx,
        manifest,
        ProgressUpdate::new("lens_worker_started", "runtime_load_measure").with_worker(
            format!("pid={} timeout={}s", child.id(), flags.lens_timeout_secs),
            Some(child.id()),
            paths.report.clone(),
            paths.stderr.clone(),
        ),
        lenses,
    )?;
    wait_for_worker(flags, idx, manifest, lenses, paths, &mut child)
}

fn wait_for_worker(
    flags: &Flags,
    idx: usize,
    manifest: &PathBuf,
    lenses: &[LensAudit],
    paths: WorkerPaths,
    child: &mut std::process::Child,
) -> CliResult<LensAudit> {
    let started = Instant::now();
    let timeout = Duration::from_secs(flags.lens_timeout_secs);
    loop {
        if let Some(status) = child.try_wait()? {
            write_progress(
                flags,
                idx,
                manifest,
                ProgressUpdate::new("lens_worker_exited", "read_worker_report").with_worker(
                    format!(
                        "status={status} elapsed_ms={}",
                        started.elapsed().as_millis()
                    ),
                    None,
                    paths.report.clone(),
                    paths.stderr.clone(),
                ),
                lenses,
            )?;
            return read_worker_report_or_reject(
                paths.report,
                paths.stderr,
                status,
                manifest.clone(),
                flags,
            );
        }
        if started.elapsed() >= timeout {
            let kill_result = child.kill();
            let wait_result = child.wait();
            let message = format!(
                "manifest {} exceeded timeout {}s; kill_result={kill_result:?}; wait_result={wait_result:?}",
                manifest.display(),
                flags.lens_timeout_secs
            );
            write_progress(
                flags,
                idx,
                manifest,
                ProgressUpdate::new("lens_worker_timeout", "timeout").with_worker(
                    message.clone(),
                    Some(child.id()),
                    paths.report,
                    paths.stderr,
                ),
                lenses,
            )?;
            return Ok(rejected_worker_audit(
                manifest.clone(),
                flags,
                "CALYX_LENS_SCALE_TIMEOUT",
                message,
            ));
        }
        thread::sleep(Duration::from_millis(250));
    }
}

fn read_worker_report_or_reject(
    report: PathBuf,
    stderr: PathBuf,
    status: ExitStatus,
    manifest: PathBuf,
    flags: &Flags,
) -> CliResult<LensAudit> {
    let bytes = match fs::read(&report) {
        Ok(bytes) => bytes,
        Err(error) => {
            let cause = worker_failure_cause(&stderr);
            return Ok(rejected_worker_audit(
                manifest,
                flags,
                "CALYX_LENS_SCALE_WORKER_REPORT_MISSING",
                format!(
                    "worker status={status}; read worker report {} failed: {error}; {cause}",
                    report.display(),
                ),
            ));
        }
    };
    match serde_json::from_slice(&bytes) {
        Ok(audit) => Ok(audit),
        Err(error) => {
            let cause = worker_failure_cause(&stderr);
            Ok(rejected_worker_audit(
                manifest,
                flags,
                "CALYX_LENS_SCALE_WORKER_REPORT_INVALID",
                format!(
                    "worker status={status}; parse report {} failed: {error}; {cause}",
                    report.display(),
                ),
            ))
        }
    }
}

fn worker_failure_cause(stderr: &Path) -> String {
    const TAIL_BYTES: usize = 4096;
    match fs::read(stderr) {
        Ok(bytes) if bytes.is_empty() => format!("stderr {} was empty", stderr.display()),
        Ok(bytes) => {
            let start = bytes.len().saturating_sub(TAIL_BYTES);
            let tail = String::from_utf8_lossy(&bytes[start..]).trim().to_string();
            format!("stderr_tail {}: {tail}", stderr.display())
        }
        Err(error) => format!("read stderr {} failed: {error}", stderr.display()),
    }
}

fn rejected_worker_audit(
    manifest_path: PathBuf,
    flags: &Flags,
    code: &'static str,
    message: String,
) -> LensAudit {
    let metadata = read_manifest_metadata(&manifest_path).ok();
    let name = metadata
        .as_ref()
        .map(|manifest| manifest.name.clone())
        .unwrap_or_else(|| manifest_path.display().to_string());
    let modality = metadata
        .as_ref()
        .map(|manifest| manifest.modality)
        .unwrap_or(Modality::Mixed);
    let runtime = metadata
        .as_ref()
        .map(|manifest| manifest.runtime.clone())
        .unwrap_or_else(|| "unparsed".to_string());
    let temporal = metadata
        .as_ref()
        .is_some_and(|manifest| manifest_name_is_temporal(&manifest.name));
    let counts = !temporal && is_content_modality(modality);
    LensAudit {
        manifest: manifest_path.clone(),
        lens_id: synthetic_lens_id(&manifest_path, &name),
        name,
        modality,
        runtime: runtime.clone(),
        runtime_detail: "worker_timeout_or_missing_report".to_string(),
        provider: "unproven".to_string(),
        placement: Placement::Cpu,
        association_family: metadata_family(&runtime, temporal),
        temporal_sidecar: temporal,
        counts_toward_content_floor: counts,
        weights_sha256: metadata
            .as_ref()
            .map(|manifest| manifest.weights_sha256.clone())
            .unwrap_or_else(|| "unverified".to_string()),
        dim: metadata.as_ref().map(|manifest| manifest.dim).unwrap_or(0),
        max_batch: metadata.as_ref().and_then(|manifest| manifest.max_batch),
        requested_batch_size: flags.batch_size,
        effective_batch_size: 0,
        native_batching: false,
        provider_placement_proof: String::new(),
        gpu_process_observed: None,
        probe_evidence: Vec::new(),
        rows_per_sec: None,
        batch_stability: None,
        accepted: false,
        rejections: vec![reject(code, message)],
    }
}

fn read_manifest_metadata(path: &Path) -> Result<LensForgeManifest, String> {
    let bytes = fs::read(path)
        .map_err(|error| format!("read manifest {} failed: {error}", path.display()))?;
    serde_json::from_slice(&bytes)
        .map_err(|error| format!("parse manifest {} failed: {error}", path.display()))
}

fn metadata_family(runtime: &str, temporal: bool) -> String {
    if temporal {
        return "temporal_sidecar".to_string();
    }
    let runtime = runtime.to_ascii_lowercase();
    if runtime.contains("cameo") || runtime.contains("actor") || runtime.contains("geo") {
        "entity_cameo_graph".to_string()
    } else if runtime.contains("token") || runtime.contains("multi") {
        "late_interaction_token".to_string()
    } else if runtime.contains("sparse") {
        "lexical_sparse".to_string()
    } else if runtime.contains("byte") {
        "byte_char".to_string()
    } else if runtime.contains("algorithmic") {
        "algorithmic".to_string()
    } else if runtime.contains("model2vec") || runtime.contains("static") {
        "static_lookup_semantic".to_string()
    } else if runtime.contains("adapter") {
        "multimodal_adapter".to_string()
    } else if runtime.contains("onnx") || runtime.contains("candle") || runtime.contains("tei") {
        "dense_semantic".to_string()
    } else {
        "unproven".to_string()
    }
}

fn manifest_name_is_temporal(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name.contains("temporal") || name.contains("as-of")
}

fn synthetic_lens_id(path: &Path, name: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(path.display().to_string().as_bytes());
    hasher.update(name.as_bytes());
    let digest: [u8; 32] = hasher.finalize().into();
    format!("unverified-{}", hex_prefix(&digest, 8))
}

fn hex_prefix(bytes: &[u8], count: usize) -> String {
    bytes
        .iter()
        .take(count)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn add_worker_args(
    command: &mut std::process::Command,
    flags: &Flags,
    manifest: &Path,
    report: &Path,
) {
    command
        .arg("lens")
        .arg("scale-audit")
        .arg("--worker")
        .arg("--out")
        .arg(report)
        .arg("--manifest")
        .arg(manifest)
        .arg("--batch-size")
        .arg(flags.batch_size.to_string())
        .arg("--min-content-lenses")
        .arg(flags.min_content_lenses.to_string())
        .arg("--min-gpu-content-lenses")
        .arg(flags.min_gpu_content_lenses.to_string())
        .arg("--min-effective-batch")
        .arg(flags.min_effective_batch.to_string())
        .arg("--min-batch-cosine")
        .arg(flags.min_batch_cosine.to_string())
        .arg("--max-abs-delta")
        .arg(flags.max_abs_delta.to_string())
        .arg("--lens-timeout-secs")
        .arg(flags.lens_timeout_secs.to_string());
    for probe in &flags.probes {
        command.arg("--probe").arg(probe);
    }
    for probe_file in &flags.probe_files {
        command.arg("--probe-file").arg(probe_file);
    }
}

struct WorkerPaths {
    report: PathBuf,
    stdout: PathBuf,
    stderr: PathBuf,
}

fn worker_paths(panel_report: &Path, idx: usize) -> WorkerPaths {
    let parent = panel_report.parent().unwrap_or_else(|| Path::new("."));
    let stem = panel_report
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("scale_audit");
    WorkerPaths {
        report: parent.join(format!("{stem}.lens-{idx:02}.json")),
        stdout: parent.join(format!("{stem}.lens-{idx:02}.stdout.jsonl")),
        stderr: parent.join(format!("{stem}.lens-{idx:02}.stderr.log")),
    }
}

fn remove_stale_worker_report(path: &Path) -> CliResult {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}
