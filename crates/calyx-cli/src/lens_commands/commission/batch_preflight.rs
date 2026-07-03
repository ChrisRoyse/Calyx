//! Commission-time batch preflight (#1157).
//!
//! GPU lenses pinned at `max_batch: 1` tax every downstream stream 4-8x in
//! wall clock (#791 gate evidence), so a commission must never default into
//! batch-1. This module measures the largest passing batch through the
//! freshly commissioned runtime (doubling probe with single-vs-batch
//! stability checks), records the evidence in the manifest `batch_policy`,
//! and refuses an unjustified batch-1 commission of a GPU-policy runtime
//! unless the operator supplies `--allow-batch-1 <reason>`. Skipping the
//! probe entirely (`--skip-batch-preflight <reason>`, e.g. commissioning on
//! a box without the runtime's GPU) is recorded as `operator-unverified` —
//! an auditable operator decision, never a silent fallback.

use std::path::Path;
use std::time::Instant;

use calyx_core::{CalyxError, Input, Lens, Modality};
use calyx_registry::{
    LensForgeBatchPolicy, LensForgeBatchProbeLevel, LensForgeManifest, lens_spec_from_manifest_path,
};
use serde_json::json;

use super::super::support::{prepare_manifest_runtime, validate_vector_contract};
use super::log::{ConversionLog, write_json_file};
use super::options::{CommissionFlags, CommissionRuntime};
use crate::error::{CliError, CliResult};
use crate::lens_commands::scale_audit::compare_vectors;

pub(super) const DEFAULT_PREFLIGHT_CAP: usize = 128;
const PREFLIGHT_MIN_COSINE: f32 = 0.999;
const PREFLIGHT_MAX_ABS_DELTA: f32 = 0.02;
/// Long enough that the fixed-width row suffix cannot move inputs across a
/// power-of-two sequence bucket, so every probe level runs as one real batch.
const PREFLIGHT_TEXT: &str = "Calyx commission batch preflight probe: measure the largest \
     passing batch for this lens runtime against single-input references before freezing \
     the manifest batch limit, row";

/// Resolve `max_batch` + `batch_policy` for the freshly written manifest,
/// enforcing the batch-1 justification gate, and rewrite the manifest file.
pub(super) fn apply(
    flags: &CommissionFlags,
    manifest_path: &Path,
    log: &mut ConversionLog,
) -> CliResult<(Option<usize>, LensForgeBatchPolicy)> {
    let policy = if let Some(skip_reason) = &flags.skip_batch_preflight {
        enforce_batch_1_gate(flags, flags.max_batch, None)?;
        LensForgeBatchPolicy {
            max_batch_source: "operator-unverified".to_string(),
            batch_1_reason: flags.allow_batch_1.clone(),
            preflight_skip_reason: Some(skip_reason.clone()),
            preflight_cap: None,
            preflight_levels: Vec::new(),
        }
    } else {
        run_probe(flags, manifest_path, log)?
    };
    let resolved_max_batch = resolved_max_batch(flags, &policy)?;
    let mut manifest = read_manifest(manifest_path)?;
    manifest.max_batch = resolved_max_batch;
    manifest.batch_policy = Some(policy.clone());
    write_json_file(manifest_path, &manifest)?;
    log.event(json!({
        "event": "batch_preflight_resolved",
        "max_batch": resolved_max_batch,
        "max_batch_source": policy.max_batch_source,
        "batch_1_reason": policy.batch_1_reason,
        "preflight_skip_reason": policy.preflight_skip_reason,
        "levels": policy.preflight_levels.len(),
    }))?;
    Ok((resolved_max_batch, policy))
}

fn run_probe(
    flags: &CommissionFlags,
    manifest_path: &Path,
    log: &mut ConversionLog,
) -> CliResult<LensForgeBatchPolicy> {
    let cap = probe_cap(flags);
    let mut spec = lens_spec_from_manifest_path(manifest_path)?;
    if spec.modality != Modality::Text {
        return Err(CliError::from(CalyxError {
            code: "CALYX_LENS_COMMISSION_PREFLIGHT_MODALITY",
            message: format!(
                "batch preflight supports Text lenses, but {} is {:?}",
                spec.name, spec.modality
            ),
            remediation: "add a modality-specific preflight input generator, or record an explicit \
                 --skip-batch-preflight <reason>",
        }));
    }
    // The probe must exercise the real batch capability, not the manifest's
    // requested cap: chunking at the operator's max_batch would silently turn
    // a batch-8 probe into eight batch-1 runs.
    spec.max_batch = Some(cap);
    let prepared = prepare_manifest_runtime(spec)?;
    let levels = probe_levels(
        &*prepared.lens,
        prepared.spec.output,
        prepared.spec.norm_policy,
        cap,
        log,
    )?;
    let largest = largest_passing(&levels);
    if largest == 0 {
        return Err(CliError::from(CalyxError {
            code: "CALYX_LENS_COMMISSION_BATCH_PREFLIGHT_FAILED",
            message: format!(
                "lens {} failed the batch preflight at every probed batch size including 1: {}",
                prepared.spec.name,
                first_failure(&levels)
            ),
            remediation: "the commissioned runtime cannot measure a single input; fix the runtime or \
                 artifacts before commissioning",
        }));
    }
    if let Some(requested) = flags.max_batch
        && requested > largest
    {
        return Err(CliError::from(CalyxError {
            code: "CALYX_LENS_COMMISSION_BATCH_PREFLIGHT_FAILED",
            message: format!(
                "--max-batch {requested} exceeds the largest passing preflight batch {largest}: {}",
                first_failure(&levels)
            ),
            remediation: "lower --max-batch to a measured passing level or fix the runtime before \
                 commissioning",
        }));
    }
    enforce_batch_1_gate(flags, flags.max_batch, Some(largest))?;
    Ok(LensForgeBatchPolicy {
        max_batch_source: if flags.max_batch.is_some() {
            "operator-verified".to_string()
        } else {
            "preflight-measured".to_string()
        },
        batch_1_reason: flags.allow_batch_1.clone(),
        preflight_skip_reason: None,
        preflight_cap: Some(cap),
        preflight_levels: levels,
    })
}

fn probe_levels(
    lens: &dyn Lens,
    output: calyx_core::SlotShape,
    norm_policy: calyx_registry::NormPolicy,
    cap: usize,
    log: &mut ConversionLog,
) -> CliResult<Vec<LensForgeBatchProbeLevel>> {
    let inputs: Vec<Input> = (0..cap)
        .map(|idx| {
            Input::new(
                Modality::Text,
                format!("{PREFLIGHT_TEXT} {idx:04}").into_bytes(),
            )
        })
        .collect();
    let singles = inputs
        .iter()
        .map(|input| lens.measure(input))
        .collect::<Result<Vec<_>, _>>()?;
    for vector in &singles {
        validate_vector_contract(vector, output, norm_policy)?;
    }
    let mut levels = Vec::new();
    let mut batch = 1usize;
    loop {
        let level = probe_one_level(
            lens,
            output,
            norm_policy,
            &inputs[..batch],
            &singles[..batch],
            batch,
        );
        let passed = level.passed;
        log.event(json!({
            "event": "batch_preflight_level",
            "batch": level.batch,
            "passed": level.passed,
            "elapsed_ms": level.elapsed_ms,
            "ms_per_row": level.ms_per_row,
            "min_cosine_vs_single": level.min_cosine_vs_single,
            "max_abs_delta_vs_single": level.max_abs_delta_vs_single,
            "failure": level.failure,
        }))?;
        levels.push(level);
        if !passed || batch >= cap {
            break;
        }
        batch = (batch * 2).min(cap);
    }
    Ok(levels)
}

fn probe_one_level(
    lens: &dyn Lens,
    output: calyx_core::SlotShape,
    norm_policy: calyx_registry::NormPolicy,
    inputs: &[Input],
    singles: &[calyx_core::SlotVector],
    batch: usize,
) -> LensForgeBatchProbeLevel {
    let started = Instant::now();
    let outcome = lens.measure_batch(inputs);
    let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    let ms_per_row = elapsed_ms as f64 / batch.max(1) as f64;
    let mut level = LensForgeBatchProbeLevel {
        batch,
        passed: false,
        elapsed_ms,
        ms_per_row,
        min_cosine_vs_single: None,
        max_abs_delta_vs_single: None,
        failure: None,
    };
    let vectors = match outcome {
        Ok(vectors) => vectors,
        Err(error) => {
            level.failure = Some(format!("{}: {}", error.code, error.message));
            return level;
        }
    };
    if vectors.len() != inputs.len() {
        level.failure = Some(format!(
            "runtime returned {} vectors for {} inputs",
            vectors.len(),
            inputs.len()
        ));
        return level;
    }
    for vector in &vectors {
        if let Err(error) = validate_vector_contract(vector, output, norm_policy) {
            level.failure = Some(error.to_string());
            return level;
        }
    }
    match compare_vectors(
        singles,
        &vectors,
        PREFLIGHT_MIN_COSINE,
        PREFLIGHT_MAX_ABS_DELTA,
    ) {
        Ok(stability) => {
            level.min_cosine_vs_single = Some(stability.min_cosine);
            level.max_abs_delta_vs_single = Some(stability.max_abs_delta);
            if stability.acceptable {
                level.passed = true;
            } else {
                level.failure = Some(format!(
                    "batch output diverged from single-input references: min_cosine={} max_abs_delta={}",
                    stability.min_cosine, stability.max_abs_delta
                ));
            }
        }
        Err(error) => {
            level.failure = Some(format!("{}: {}", error.code, error.message));
        }
    }
    level
}

/// The batch-1 refusal (#1157): a GPU-policy runtime commissioned at
/// `max_batch = 1` — whether requested or measured — requires an explicit
/// operator justification.
fn enforce_batch_1_gate(
    flags: &CommissionFlags,
    requested: Option<usize>,
    measured_largest: Option<usize>,
) -> CliResult<()> {
    if !is_gpu_policy_runtime(flags.runtime) || flags.allow_batch_1.is_some() {
        return Ok(());
    }
    let effective = requested.or(measured_largest);
    if effective == Some(1) {
        return Err(CliError::from(CalyxError {
            code: "CALYX_LENS_COMMISSION_BATCH1_UNJUSTIFIED",
            message: format!(
                "GPU-policy runtime {} would be commissioned at max_batch=1 (requested={requested:?}, measured_largest_passing={measured_largest:?}) — batch-1 GPU lenses run 4-8x slower than batched siblings",
                flags.runtime.manifest_runtime()
            ),
            remediation: "fix the runtime/manifest so real batches pass preflight, or record the \
                 justification with --allow-batch-1 <reason>",
        }));
    }
    Ok(())
}

/// Every commissionable runtime except the remote TEI service runs its ONNX /
/// Candle graph on the local GPU under fail-loud CUDA policy.
pub(super) const fn is_gpu_policy_runtime(runtime: CommissionRuntime) -> bool {
    !matches!(runtime, CommissionRuntime::Tei)
}

fn probe_cap(flags: &CommissionFlags) -> usize {
    flags
        .preflight_cap
        .or(flags.max_batch)
        .unwrap_or(DEFAULT_PREFLIGHT_CAP)
        .max(1)
}

fn resolved_max_batch(
    flags: &CommissionFlags,
    policy: &LensForgeBatchPolicy,
) -> CliResult<Option<usize>> {
    if let Some(requested) = flags.max_batch {
        return Ok(Some(requested));
    }
    match policy.max_batch_source.as_str() {
        "preflight-measured" => Ok(Some(largest_passing(&policy.preflight_levels))),
        "operator-unverified" => Ok(None),
        other => Err(CliError::runtime(format!(
            "batch preflight produced unknown max_batch_source {other}"
        ))),
    }
}

fn largest_passing(levels: &[LensForgeBatchProbeLevel]) -> usize {
    levels
        .iter()
        .filter(|level| level.passed)
        .map(|level| level.batch)
        .max()
        .unwrap_or(0)
}

fn first_failure(levels: &[LensForgeBatchProbeLevel]) -> String {
    levels
        .iter()
        .find_map(|level| level.failure.clone())
        .unwrap_or_else(|| "no failure detail recorded".to_string())
}

fn read_manifest(path: &Path) -> CliResult<LensForgeManifest> {
    let bytes = std::fs::read(path)?;
    serde_json::from_slice(&bytes).map_err(|error| {
        CliError::runtime(format!(
            "re-read lensforge manifest {} for batch policy failed: {error}",
            path.display()
        ))
    })
}

#[cfg(test)]
mod tests;
