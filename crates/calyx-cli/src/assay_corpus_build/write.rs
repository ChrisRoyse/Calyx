use std::collections::BTreeMap;
use std::fs;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process;

use calyx_core::SlotShape;
use serde::Serialize;
use serde_json::json;

use crate::assay_anchor_audit::AnchorAudit;

use super::data::BuildRows;
use super::lens::MeasuredLens;
use super::request::CorpusBuildRequest;

#[derive(Clone, Debug, Serialize)]
pub(crate) struct CorpusBuildEvidence {
    pub(crate) out_dir: String,
    pub(crate) manifest_path: String,
    pub(crate) vectors_path: String,
    pub(crate) cost_path: String,
    pub(crate) n_samples: usize,
    pub(crate) batch_size: usize,
    pub(crate) label_counts: BTreeMap<String, usize>,
    pub(crate) temporal: TemporalEvidence,
    pub(crate) anchor_audit: AnchorAudit,
    pub(crate) lenses: Vec<LensEvidence>,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct TemporalEvidence {
    pub(crate) active_rows: usize,
    pub(crate) inactive_rows: usize,
    pub(crate) source_sequence: String,
    pub(crate) accepted_fields: Vec<&'static str>,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct LensEvidence {
    pub(crate) name: String,
    pub(crate) runtime: String,
    pub(crate) output_shape: String,
    pub(crate) assay_projection: String,
    pub(crate) manifest: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) max_batch: Option<usize>,
    pub(crate) vram_mb: f32,
    pub(crate) ram_mb: f32,
    pub(crate) ms_per_input: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) worker_pid: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) worker_report_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) worker_stderr_path: Option<String>,
}

pub(crate) fn ensure_fresh_output(request: &CorpusBuildRequest) -> Result<(), String> {
    fail_if_final_exists(&request.out_dir)?;
    fail_if_final_exists(&staging_dir(&request.out_dir))
}

pub(crate) fn write_outputs(
    request: &CorpusBuildRequest,
    rows: &BuildRows,
    lenses: &[MeasuredLens],
) -> Result<CorpusBuildEvidence, String> {
    validate_measurements(rows, lenses)?;
    ensure_fresh_output(request)?;
    let staging = staging_dir(&request.out_dir);
    if let Some(parent) = request.out_dir.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).map_err(io_error)?;
    }
    fs::create_dir(&staging).map_err(io_error)?;
    let result = write_all_files(request, rows, lenses, &staging);
    match result {
        Ok(evidence) => {
            fs::rename(&staging, &request.out_dir).map_err(io_error)?;
            Ok(CorpusBuildEvidence {
                out_dir: display(&request.out_dir),
                ..evidence
            })
        }
        Err(error) => {
            let _ = fs::remove_dir_all(&staging);
            Err(error)
        }
    }
}

fn write_all_files(
    request: &CorpusBuildRequest,
    rows: &BuildRows,
    lenses: &[MeasuredLens],
    dir: &Path,
) -> Result<CorpusBuildEvidence, String> {
    let manifest_path = dir.join("manifest.json");
    let vectors_path = dir.join("vectors.jsonl");
    let cost_path = dir.join("cost.json");
    let report_path = dir.join("corpus_build_report.json");
    write_manifest(request, rows, lenses, &manifest_path)?;
    write_vectors(rows, lenses, &vectors_path)?;
    write_cost(lenses, &cost_path)?;
    let evidence = CorpusBuildEvidence {
        out_dir: display(&request.out_dir),
        manifest_path: display_final(request, "manifest.json"),
        vectors_path: display_final(request, "vectors.jsonl"),
        cost_path: display_final(request, "cost.json"),
        n_samples: rows.rows.len(),
        batch_size: request.batch_size,
        label_counts: rows.label_counts.clone(),
        temporal: temporal_evidence(rows),
        anchor_audit: rows.anchor_audit.clone(),
        lenses: lenses.iter().map(lens_evidence).collect(),
    };
    fs::write(
        report_path,
        serde_json::to_vec_pretty(&evidence).map_err(json_error)?,
    )
    .map_err(io_error)?;
    Ok(evidence)
}

fn write_manifest(
    request: &CorpusBuildRequest,
    rows: &BuildRows,
    lenses: &[MeasuredLens],
    path: &Path,
) -> Result<(), String> {
    let lens_names: Vec<String> = lenses.iter().map(|lens| lens.name.clone()).collect();
    let manifest_lenses: Vec<_> = lenses
        .iter()
        .map(|lens| json!({ "name": lens.name, "redundant": false }))
        .collect();
    let manifest = json!({
        "dataset": request.dataset,
        "embedding_model_id": request.embedding_model_id(&lens_names),
        "n_samples": rows.rows.len(),
        "label_counts": rows.label_counts,
        "target_class": request.target_class,
        "temporal": temporal_evidence(rows),
        "anchor_leaks_into_input": rows.anchor_audit.anchor_leaks_into_input,
        "trivial_anchor": rows.anchor_audit.trivial_anchor,
        "grounded_gate_eligible": rows.anchor_audit.grounded_gate_eligible,
        "anchor_audit": rows.anchor_audit.clone(),
        "lenses": manifest_lenses
    });
    fs::write(
        path,
        serde_json::to_vec_pretty(&manifest).map_err(json_error)?,
    )
    .map_err(io_error)
}

fn write_vectors(rows: &BuildRows, lenses: &[MeasuredLens], path: &Path) -> Result<(), String> {
    let file = fs::File::create(path).map_err(io_error)?;
    let mut writer = BufWriter::new(file);
    for (row_idx, row) in rows.rows.iter().enumerate() {
        let mut lens_map = serde_json::Map::new();
        for lens in lenses {
            lens_map.insert(lens.name.clone(), json!(lens.vectors[row_idx]));
        }
        let line = json!({
            "id": row.id,
            "split": row.split,
            "label": row.label,
            "source_event_time_secs": row.event_time_secs,
            "source_event_time_raw": row.event_time_raw,
            "temporal_lane_state": row.temporal_lane_state,
            "temporal_inactive_reason": row.temporal_inactive_reason,
            "source_sequence": row.source_sequence,
            "source_sequence_index": row.source_sequence_index,
            "anchor_leaks_into_input": row.anchor_audit.anchor_leaks_into_input,
            "anchor_audit": row.anchor_audit.clone(),
            "lenses": lens_map
        });
        serde_json::to_writer(&mut writer, &line).map_err(json_error)?;
        writer.write_all(b"\n").map_err(io_error)?;
    }
    writer.flush().map_err(io_error)
}

fn temporal_evidence(rows: &BuildRows) -> TemporalEvidence {
    let active_rows = rows
        .rows
        .iter()
        .filter(|row| row.event_time_secs.is_some())
        .count();
    TemporalEvidence {
        active_rows,
        inactive_rows: rows.rows.len().saturating_sub(active_rows),
        source_sequence: "jsonl_line".to_string(),
        accepted_fields: vec![
            "event_time",
            "event_time_secs",
            "source_event_time_secs",
            "created_at",
            "timestamp",
        ],
    }
}

fn write_cost(lenses: &[MeasuredLens], path: &Path) -> Result<(), String> {
    let costs: BTreeMap<String, _> = lenses
        .iter()
        .map(|lens| (lens.name.clone(), lens.cost))
        .collect();
    fs::write(path, serde_json::to_vec_pretty(&costs).map_err(json_error)?).map_err(io_error)
}

fn validate_measurements(rows: &BuildRows, lenses: &[MeasuredLens]) -> Result<(), String> {
    if lenses.is_empty() {
        return Err(
            "CALYX_FSV_ASSAY_CORPUS_BUILD_INVALID_MEASUREMENT: need at least one measured lens"
                .to_string(),
        );
    }
    for lens in lenses {
        if lens.vectors.len() != rows.rows.len() {
            return Err(format!(
                "CALYX_FSV_ASSAY_CORPUS_BUILD_INVALID_MEASUREMENT: lens={} vectors={} rows={}",
                lens.name,
                lens.vectors.len(),
                rows.rows.len()
            ));
        }
    }
    Ok(())
}

fn lens_evidence(lens: &MeasuredLens) -> LensEvidence {
    LensEvidence {
        name: lens.name.clone(),
        runtime: lens.runtime.clone(),
        output_shape: shape_text(lens.output),
        assay_projection: lens.assay_projection.clone(),
        manifest: display(&lens.manifest),
        max_batch: lens.max_batch,
        vram_mb: lens.cost.vram_mb,
        ram_mb: lens.cost.ram_mb,
        ms_per_input: lens.cost.ms_per_input,
        worker_pid: lens.worker_pid,
        worker_report_path: lens.worker_report_path.as_ref().map(|path| display(path)),
        worker_stderr_path: lens.worker_stderr_path.as_ref().map(|path| display(path)),
    }
}

fn fail_if_final_exists(path: &Path) -> Result<(), String> {
    if path.exists() {
        return Err(format!(
            "CALYX_FSV_ASSAY_CORPUS_BUILD_OUTPUT_EXISTS: {}",
            path.display()
        ));
    }
    Ok(())
}

fn staging_dir(out_dir: &Path) -> PathBuf {
    let name = out_dir
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("assay-corpus");
    out_dir.with_file_name(format!(".{name}.tmp-{}", process::id()))
}

fn display_final(request: &CorpusBuildRequest, file: &str) -> String {
    display(&request.out_dir.join(file))
}

fn display(path: &Path) -> String {
    path.display().to_string()
}

fn shape_text(shape: SlotShape) -> String {
    match shape {
        SlotShape::Dense(dim) => format!("dense:{dim}"),
        SlotShape::Sparse(dim) => format!("sparse:{dim}"),
        SlotShape::Multi { token_dim } => format!("multi:{token_dim}"),
    }
}

fn io_error(error: std::io::Error) -> String {
    format!("CALYX_FSV_ASSAY_CORPUS_BUILD_IO: {error}")
}

fn json_error(error: serde_json::Error) -> String {
    format!("CALYX_FSV_ASSAY_CORPUS_BUILD_JSON: {error}")
}
