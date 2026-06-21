use std::path::PathBuf;
use std::process::Command;
use std::time::Instant;

use calyx_core::{CalyxError, Input, Lens, Placement, SlotVector, SparseEntry};
use calyx_registry::LensSpec;

use super::model::{BatchStability, Flags, LensAudit, ProbeEvidence, reject};
use super::probe::{probe_set, supports_probe_measurement};
use super::runtime::{association_family, is_content_modality, is_temporal_sidecar, runtime_lens};
use crate::lens_commands::support::{dim, hex_from_bytes, runtime_name, validate_vector_contract};

pub(super) fn audit_lens(manifest: PathBuf, spec: LensSpec, flags: &Flags) -> LensAudit {
    let temporal = is_temporal_sidecar(&spec);
    let counts = !temporal && is_content_modality(spec.modality);
    let runtime = runtime_name(&spec.runtime).to_string();
    let family = association_family(&spec).to_string();
    let lens_id = spec.lens_id().to_string();
    let weights_sha256 = hex_from_bytes(&spec.weights_sha256);
    let shape_dim = dim(spec.output);
    let mut rejections = Vec::new();

    let runtime_lens = match runtime_lens(&spec) {
        Ok(value) => value,
        Err(error) => {
            rejections.push(reject(
                "CALYX_LENS_SCALE_RUNTIME_LOAD",
                format!("{}: {}", error.code, error.message),
            ));
            return LensAudit {
                manifest,
                lens_id,
                name: spec.name,
                modality: spec.modality,
                runtime,
                runtime_detail: "runtime_load_failed".to_string(),
                provider: "unproven".to_string(),
                placement: Placement::Cpu,
                association_family: family,
                temporal_sidecar: temporal,
                counts_toward_content_floor: counts,
                weights_sha256,
                dim: shape_dim,
                max_batch: spec.max_batch,
                requested_batch_size: flags.batch_size,
                effective_batch_size: 0,
                native_batching: false,
                provider_placement_proof: String::new(),
                gpu_process_observed: None,
                probe_evidence: Vec::new(),
                rows_per_sec: None,
                batch_stability: None,
                accepted: false,
                rejections,
            };
        }
    };

    let effective = effective_batch(flags.batch_size, runtime_lens.max_batch);
    if counts && runtime_lens.placement == Placement::Gpu && effective < flags.min_effective_batch {
        rejections.push(reject(
            "CALYX_LENS_SCALE_BATCH_TOO_SMALL",
            format!(
                "lens {} effective_batch_size={} below min_effective_batch={}",
                spec.name, effective, flags.min_effective_batch
            ),
        ));
    }
    if counts && runtime_lens.placement == Placement::Gpu && runtime_lens.proof.trim().is_empty() {
        rejections.push(reject(
            "CALYX_LENS_SCALE_PROVIDER_PROOF_MISSING",
            format!(
                "lens {} has GPU placement without provider proof",
                spec.name
            ),
        ));
    }

    let mut rows_per_sec = None;
    let mut batch_stability = None;
    let mut gpu_process_observed = None;
    let mut probe_evidence = Vec::new();
    if counts && supports_probe_measurement(spec.modality) {
        match measure_runtime(&*runtime_lens.lens, &spec, flags, effective) {
            Ok(result) => {
                rows_per_sec = Some(result.rows_per_sec);
                batch_stability = Some(result.stability.clone());
                probe_evidence = result.probe_evidence;
                if !result.stability.acceptable {
                    rejections.push(reject(
                        "CALYX_LENS_SCALE_BATCH_STABILITY",
                        format!(
                            "lens {} batch stability min_cosine={} max_abs_delta={}",
                            spec.name, result.stability.min_cosine, result.stability.max_abs_delta
                        ),
                    ));
                }
            }
            Err(failure) => {
                probe_evidence = failure.probe_evidence;
                rejections.push(reject(
                    "CALYX_LENS_SCALE_MEASUREMENT_FAILED",
                    format!("{}: {}", failure.error.code, failure.error.message),
                ));
            }
        }
    } else if counts {
        rejections.push(reject(
            "CALYX_LENS_SCALE_MODALITY_UNSUPPORTED",
            format!(
                "lens {} modality {:?} has no scale-audit probe support",
                spec.name, spec.modality
            ),
        ));
    }
    if runtime_lens.gpu_process_required {
        match current_pid_seen_by_nvidia_smi() {
            Ok(seen) => {
                gpu_process_observed = Some(seen);
                if !seen {
                    rejections.push(reject(
                        "CALYX_LENS_SCALE_GPU_PROCESS_UNOBSERVED",
                        format!(
                            "lens {} claimed GPU provider {} but current pid was absent from nvidia-smi compute apps",
                            spec.name, runtime_lens.provider
                        ),
                    ));
                }
            }
            Err(error) => {
                gpu_process_observed = Some(false);
                rejections.push(reject(
                    "CALYX_LENS_SCALE_GPU_PROCESS_READBACK_FAILED",
                    format!(
                        "lens {} could not read nvidia-smi process evidence: {error}",
                        spec.name
                    ),
                ));
            }
        }
    }

    LensAudit {
        manifest,
        lens_id,
        name: spec.name,
        modality: spec.modality,
        runtime,
        runtime_detail: runtime_lens.detail,
        provider: runtime_lens.provider,
        placement: runtime_lens.placement,
        association_family: family,
        temporal_sidecar: temporal,
        counts_toward_content_floor: counts,
        weights_sha256,
        dim: shape_dim,
        max_batch: runtime_lens.max_batch,
        requested_batch_size: flags.batch_size,
        effective_batch_size: effective,
        native_batching: runtime_lens.native_batching,
        provider_placement_proof: runtime_lens.proof,
        gpu_process_observed,
        probe_evidence,
        rows_per_sec,
        batch_stability,
        accepted: rejections.is_empty(),
        rejections,
    }
}

fn current_pid_seen_by_nvidia_smi() -> Result<bool, String> {
    let output = Command::new("nvidia-smi")
        .args(["--query-compute-apps=pid", "--format=csv,noheader,nounits"])
        .output()
        .map_err(|error| format!("spawn nvidia-smi failed: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "nvidia-smi exited {:?}: {}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let pid = std::process::id().to_string();
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .any(|line| line.trim() == pid))
}

#[derive(Clone)]
struct Measurement {
    rows_per_sec: f64,
    stability: BatchStability,
    probe_evidence: Vec<ProbeEvidence>,
}

struct MeasurementFailure {
    error: CalyxError,
    probe_evidence: Vec<ProbeEvidence>,
}

fn measure_runtime(
    lens: &dyn Lens,
    spec: &LensSpec,
    flags: &Flags,
    effective_batch: usize,
) -> Result<Measurement, MeasurementFailure> {
    let probes = probe_set(flags, spec.modality, effective_batch.max(1)).map_err(|error| {
        MeasurementFailure {
            error,
            probe_evidence: Vec::new(),
        }
    })?;
    let probe_evidence = probes.evidence.clone();
    let started = Instant::now();
    let measured = measure_chunks(lens, &probes.inputs, effective_batch)
        .map_err(|error| measurement_failure(error, &probe_evidence))?;
    let elapsed = started.elapsed().as_secs_f64().max(0.000_001);
    for vector in &measured {
        validate_vector_contract(vector, spec.output, spec.norm_policy)
            .map_err(|error| CalyxError::lens_unreachable(error.to_string()))
            .map_err(|error| measurement_failure(error, &probe_evidence))?;
    }
    let singles = probes
        .inputs
        .iter()
        .map(|input| lens.measure(input))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| measurement_failure(error, &probe_evidence))?;
    let stability = compare_vectors(
        &singles,
        &measured,
        flags.min_batch_cosine,
        flags.max_abs_delta,
    )
    .map_err(|error| measurement_failure(error, &probe_evidence))?;
    Ok(Measurement {
        rows_per_sec: probes.inputs.len() as f64 / elapsed,
        stability,
        probe_evidence,
    })
}

fn measurement_failure(error: CalyxError, probe_evidence: &[ProbeEvidence]) -> MeasurementFailure {
    MeasurementFailure {
        error,
        probe_evidence: probe_evidence.to_vec(),
    }
}

pub(super) fn effective_batch(requested: usize, max_batch: Option<usize>) -> usize {
    max_batch
        .filter(|value| *value > 0)
        .map(|value| value.min(requested))
        .unwrap_or(requested)
}

fn measure_chunks(
    lens: &dyn Lens,
    inputs: &[Input],
    effective_batch: usize,
) -> Result<Vec<SlotVector>, CalyxError> {
    let mut out = Vec::with_capacity(inputs.len());
    for chunk in inputs.chunks(effective_batch.max(1)) {
        out.extend(lens.measure_batch(chunk)?);
    }
    Ok(out)
}

pub(super) fn compare_vectors(
    singles: &[SlotVector],
    batched: &[SlotVector],
    min_batch_cosine: f32,
    max_allowed_abs_delta: f32,
) -> Result<BatchStability, CalyxError> {
    if singles.len() != batched.len() {
        return Err(CalyxError::lens_dim_mismatch(format!(
            "single vector count {} != batched count {}",
            singles.len(),
            batched.len()
        )));
    }
    let mut min_cosine = f32::INFINITY;
    let mut max_abs_delta = 0.0_f32;
    for (single, batch) in singles.iter().zip(batched) {
        let metrics = vector_metrics(single, batch)?;
        min_cosine = min_cosine.min(metrics.cosine);
        max_abs_delta = max_abs_delta.max(metrics.max_abs_delta);
    }
    Ok(BatchStability {
        sample_rows: singles.len(),
        min_cosine,
        max_abs_delta,
        min_batch_cosine,
        max_allowed_abs_delta,
        acceptable: min_cosine >= min_batch_cosine && max_abs_delta <= max_allowed_abs_delta,
    })
}

struct PairMetrics {
    cosine: f32,
    max_abs_delta: f32,
}

fn vector_metrics(left: &SlotVector, right: &SlotVector) -> Result<PairMetrics, CalyxError> {
    match (left, right) {
        (
            SlotVector::Dense {
                dim: a_dim,
                data: a,
            },
            SlotVector::Dense {
                dim: b_dim,
                data: b,
            },
        ) if a_dim == b_dim && a.len() == b.len() => dense_metrics(a, b),
        (
            SlotVector::Sparse {
                dim: a_dim,
                entries: a,
            },
            SlotVector::Sparse {
                dim: b_dim,
                entries: b,
            },
        ) if a_dim == b_dim => sparse_metrics(a, b),
        (
            SlotVector::Multi {
                token_dim: a_dim,
                tokens: a,
            },
            SlotVector::Multi {
                token_dim: b_dim,
                tokens: b,
            },
        ) if a_dim == b_dim => multi_metrics(a, b),
        _ => Err(CalyxError::lens_dim_mismatch(
            "batch stability requires matching slot-vector shapes",
        )),
    }
}

fn dense_metrics(a: &[f32], b: &[f32]) -> Result<PairMetrics, CalyxError> {
    Ok(PairMetrics {
        cosine: cosine(a, b)?,
        max_abs_delta: max_delta(a, b),
    })
}

fn sparse_metrics(a: &[SparseEntry], b: &[SparseEntry]) -> Result<PairMetrics, CalyxError> {
    let an = a.iter().map(|entry| entry.val * entry.val).sum::<f32>();
    let bn = b.iter().map(|entry| entry.val * entry.val).sum::<f32>();
    let mut dot = 0.0_f32;
    let mut max_abs_delta = 0.0_f32;
    let mut left = 0;
    let mut right = 0;
    while left < a.len() || right < b.len() {
        match (a.get(left), b.get(right)) {
            (Some(l), Some(r)) if l.idx == r.idx => {
                dot += l.val * r.val;
                max_abs_delta = max_abs_delta.max((l.val - r.val).abs());
                left += 1;
                right += 1;
            }
            (Some(l), Some(r)) if l.idx < r.idx => {
                max_abs_delta = max_abs_delta.max(l.val.abs());
                left += 1;
            }
            (Some(_), Some(r)) => {
                max_abs_delta = max_abs_delta.max(r.val.abs());
                right += 1;
            }
            (Some(l), None) => {
                max_abs_delta = max_abs_delta.max(l.val.abs());
                left += 1;
            }
            (None, Some(r)) => {
                max_abs_delta = max_abs_delta.max(r.val.abs());
                right += 1;
            }
            (None, None) => break,
        }
    }
    if an > 0.0 && bn > 0.0 {
        Ok(PairMetrics {
            cosine: dot / (an.sqrt() * bn.sqrt()),
            max_abs_delta,
        })
    } else {
        Err(CalyxError::lens_numerical_invariant(
            "batch stability saw zero-norm sparse vector",
        ))
    }
}

fn multi_metrics(a: &[Vec<f32>], b: &[Vec<f32>]) -> Result<PairMetrics, CalyxError> {
    if a.len() != b.len() {
        return Err(CalyxError::lens_dim_mismatch(format!(
            "single token count {} != batched token count {}",
            a.len(),
            b.len()
        )));
    }
    let left = a.iter().flatten().copied().collect::<Vec<_>>();
    let right = b.iter().flatten().copied().collect::<Vec<_>>();
    if left.len() != right.len() {
        return Err(CalyxError::lens_dim_mismatch(
            "batch stability saw mismatched multi-vector token dimensions",
        ));
    }
    dense_metrics(&left, &right)
}

fn cosine(a: &[f32], b: &[f32]) -> Result<f32, CalyxError> {
    let dot = a.iter().zip(b).map(|(x, y)| x * y).sum::<f32>();
    let an = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let bn = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if an > 0.0 && bn > 0.0 {
        Ok(dot / (an * bn))
    } else {
        Err(CalyxError::lens_numerical_invariant(
            "batch stability saw zero-norm vector",
        ))
    }
}

fn max_delta(a: &[f32], b: &[f32]) -> f32 {
    a.iter()
        .zip(b)
        .map(|(x, y)| (x - y).abs())
        .fold(0.0_f32, f32::max)
}
