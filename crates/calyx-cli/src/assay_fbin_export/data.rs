use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use calyx_registry::lens_spec_from_manifest_path;
use serde::Deserialize;

use crate::a35_signal::{require_countable_content_signal_kind, runtime_signal_kind_name};
use crate::assay_anchor_audit::AnchorAudit;
use crate::assay_corpus_build::lens::projection::{projected_slot_dim, slot_projection_name};
use crate::error::CliResult;

use super::timeline::{TimelineScan, TimelineScanBuilder, timeline_row};
use super::{MIN_A35_LENSES, local_error};

#[derive(Debug)]
pub(super) struct VectorScan {
    pub(super) rows: usize,
    pub(super) lens_dims: BTreeMap<String, usize>,
    pub(super) timeline: TimelineScan,
}

#[derive(Debug)]
pub(super) struct LensMeta {
    pub(super) lens_id: String,
    pub(super) weights_sha256: String,
    pub(super) signal_kind: String,
    pub(super) native_dim: usize,
    pub(super) projected_dim: usize,
    pub(super) assay_projection: String,
}

#[derive(Debug)]
pub(super) struct LensCatalog {
    pub(super) order: Vec<String>,
    pub(super) meta: BTreeMap<String, LensMeta>,
}

#[derive(Debug, Deserialize)]
pub(super) struct BitsLens {
    pub(super) name: String,
    pub(super) bits_about: f32,
    pub(super) admitted: bool,
}

#[derive(Debug, Deserialize)]
pub(super) struct VectorRow {
    pub(super) id: String,
    pub(super) lenses: BTreeMap<String, Vec<f32>>,
    #[serde(default)]
    pub(super) source_event_time_secs: Option<i64>,
    #[serde(default)]
    pub(super) event_time_secs: Option<i64>,
    #[serde(default)]
    pub(super) source_event_time_raw: Option<String>,
    #[serde(default)]
    pub(super) event_time_raw: Option<String>,
    #[serde(default)]
    pub(super) temporal_lane_state: Option<String>,
    #[serde(default)]
    pub(super) temporal_inactive_reason: Option<String>,
    #[serde(default)]
    pub(super) source_sequence: Option<String>,
    #[serde(default)]
    pub(super) source_sequence_index: Option<usize>,
}

impl VectorRow {
    pub(super) fn event_time_secs(&self) -> CliResult<Option<i64>> {
        match (self.source_event_time_secs, self.event_time_secs) {
            (Some(source), Some(alias)) if source != alias => Err(local_error(
                "CALYX_FSV_ASSAY_FBIN_EXPORT_TEMPORAL_INVALID",
                format!("row {} has mismatched event time aliases", self.id),
                "keep source_event_time_secs and event_time_secs identical when both are present",
            )),
            (Some(source), _) => Ok(Some(source)),
            (_, Some(alias)) => Ok(Some(alias)),
            _ => Ok(None),
        }
    }

    pub(super) fn event_time_raw(&self) -> Option<&str> {
        self.source_event_time_raw
            .as_deref()
            .or(self.event_time_raw.as_deref())
    }
}

#[derive(Debug, Deserialize)]
struct BuildReport {
    lenses: Vec<BuildLensRef>,
}

#[derive(Debug, Deserialize)]
struct BuildLensRef {
    name: String,
    manifest: PathBuf,
}

#[derive(Debug, Deserialize)]
struct BitsReport {
    lenses: Option<Vec<BitsLens>>,
    anchor_audit: Option<AnchorAudit>,
    anchor_leaks_into_input: Option<bool>,
    trivial_anchor: Option<bool>,
    grounded_gate_eligible: Option<bool>,
    report: Option<BitsReportInner>,
}

#[derive(Debug, Deserialize)]
struct BitsReportInner {
    lenses: Vec<BitsLens>,
    anchor_audit: Option<AnchorAudit>,
    anchor_leaks_into_input: Option<bool>,
    trivial_anchor: Option<bool>,
    grounded_gate_eligible: Option<bool>,
}

pub(super) fn scan_vectors(path: &Path) -> CliResult<VectorScan> {
    let text = fs::read_to_string(path).map_err(|error| {
        local_error(
            "CALYX_FSV_ASSAY_FBIN_EXPORT_IO",
            format!("read {} failed: {error}", path.display()),
            "inspect the corpus-build output and rerun export-fbin",
        )
    })?;
    let mut rows = 0usize;
    let mut lens_dims: BTreeMap<String, usize> = BTreeMap::new();
    let mut timeline = TimelineScanBuilder::default();
    for (line_idx, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let row = parse_vector_row(line_idx, line)?;
        validate_row(line_idx, &row)?;
        timeline.push(&timeline_row(rows, &row, 0)?);
        let dims = row_dims(line_idx, &row)?;
        if rows == 0 {
            lens_dims = dims;
        } else if dims != lens_dims {
            return Err(local_error(
                "CALYX_FSV_ASSAY_FBIN_EXPORT_LENS_SET_MISMATCH",
                format!("line {line_idx} lens set or dimensions differ from line 0"),
                "rebuild the corpus so every row has the same frozen lens roster",
            ));
        }
        rows += 1;
    }
    if rows == 0 {
        return Err(local_error(
            "CALYX_FSV_ASSAY_FBIN_EXPORT_EMPTY",
            format!("{} contains no vector rows", path.display()),
            "rerun corpus-build and inspect vectors.jsonl",
        ));
    }
    Ok(VectorScan {
        rows,
        lens_dims,
        timeline: timeline.finish(),
    })
}

pub(super) fn parse_vector_row(line_idx: usize, line: &str) -> CliResult<VectorRow> {
    serde_json::from_str(line).map_err(|error| {
        local_error(
            "CALYX_FSV_ASSAY_FBIN_EXPORT_INVALID_VECTOR_ROW",
            format!("line {line_idx}: {error}"),
            "fix vectors.jsonl so every row has id and lenses",
        )
    })
}

pub(super) fn validate_row(line_idx: usize, row: &VectorRow) -> CliResult {
    if row.id.trim().is_empty() {
        return Err(local_error(
            "CALYX_FSV_ASSAY_FBIN_EXPORT_INVALID_VECTOR_ROW",
            format!("line {line_idx} id is empty"),
            "fix vectors.jsonl so every row has a stable id",
        ));
    }
    if row.lenses.is_empty() {
        return Err(local_error(
            "CALYX_FSV_ASSAY_FBIN_EXPORT_INVALID_VECTOR_ROW",
            format!("line {line_idx} has no lenses"),
            "rerun corpus-build with a real multi-lens panel",
        ));
    }
    Ok(())
}

pub(super) fn row_dims(line_idx: usize, row: &VectorRow) -> CliResult<BTreeMap<String, usize>> {
    let mut dims = BTreeMap::new();
    for (name, vector) in &row.lenses {
        if vector.is_empty() || vector.iter().any(|value| !value.is_finite()) {
            return Err(local_error(
                "CALYX_FSV_ASSAY_FBIN_EXPORT_INVALID_VECTOR",
                format!("line {line_idx} lens {name} has empty or non-finite vector"),
                "rerun corpus-build and inspect the offending vector row",
            ));
        }
        dims.insert(name.clone(), vector.len());
    }
    Ok(dims)
}

pub(super) fn load_lens_catalog(
    corpus_dir: &Path,
    dims: &BTreeMap<String, usize>,
) -> CliResult<LensCatalog> {
    let path = corpus_dir.join("corpus_build_report.json");
    let report: BuildReport = serde_json::from_slice(&fs::read(&path).map_err(|error| {
        local_error(
            "CALYX_FSV_ASSAY_FBIN_EXPORT_BUILD_REPORT_IO",
            format!("read {} failed: {error}", path.display()),
            "export-fbin requires the corpus_build_report.json source of truth",
        )
    })?)
    .map_err(|error| {
        local_error(
            "CALYX_FSV_ASSAY_FBIN_EXPORT_BUILD_REPORT_INVALID",
            format!("parse {} failed: {error}", path.display()),
            "rerun assay corpus-build and inspect corpus_build_report.json",
        )
    })?;
    let mut order = Vec::new();
    let mut meta = BTreeMap::new();
    let mut seen = BTreeSet::new();
    for lens in report.lenses {
        if !seen.insert(lens.name.clone()) {
            return Err(local_error(
                "CALYX_FSV_ASSAY_FBIN_EXPORT_METADATA_DUPLICATE",
                format!(
                    "lens {} appears more than once in corpus_build_report.json",
                    lens.name
                ),
                "rerun assay corpus-build so the frozen lens roster has unique names",
            ));
        }
        if dims.contains_key(&lens.name) {
            let name = lens.name.clone();
            let lens_meta = lens_meta(corpus_dir, &lens)?;
            let actual_dim = dims[&name];
            if actual_dim != lens_meta.projected_dim {
                return Err(local_error(
                    "CALYX_FSV_ASSAY_FBIN_EXPORT_PROJECTION_MISMATCH",
                    format!(
                        "lens {name} vector dim={actual_dim} but manifest native_dim={} projects to {} via {}",
                        lens_meta.native_dim, lens_meta.projected_dim, lens_meta.assay_projection
                    ),
                    "rerun corpus-build or stream-fbin with the bounded sparse projection build",
                ));
            }
            meta.insert(name.clone(), lens_meta);
            order.push(name);
        }
    }
    for name in dims.keys() {
        if !meta.contains_key(name) {
            return Err(local_error(
                "CALYX_FSV_ASSAY_FBIN_EXPORT_METADATA_MISSING",
                format!("lens {name} missing from corpus_build_report.json"),
                "rerun assay corpus-build so every vector lens has a frozen manifest",
            ));
        }
    }
    Ok(LensCatalog { order, meta })
}

fn lens_meta(corpus_dir: &Path, lens: &BuildLensRef) -> CliResult<LensMeta> {
    let manifest = resolve_path(corpus_dir, &lens.manifest);
    let spec = lens_spec_from_manifest_path(&manifest).map_err(|error| {
        local_error(
            "CALYX_FSV_ASSAY_FBIN_EXPORT_MANIFEST_INVALID",
            format!("{}: {}", manifest.display(), error.message),
            "fix the frozen lens manifest referenced by corpus_build_report.json",
        )
    })?;
    Ok(LensMeta {
        lens_id: spec.lens_id().to_string(),
        weights_sha256: hex32(&spec.weights_sha256),
        signal_kind: runtime_signal_kind_name(&spec.runtime).to_string(),
        native_dim: crate::lens_commands::support::dim(spec.output) as usize,
        projected_dim: projected_slot_dim(spec.output) as usize,
        assay_projection: slot_projection_name(spec.output).to_string(),
    })
}

pub(super) fn load_bits_report(path: &Path) -> CliResult<BTreeMap<String, BitsLens>> {
    let report: BitsReport = serde_json::from_slice(&fs::read(path).map_err(|error| {
        local_error(
            "CALYX_FSV_ASSAY_FBIN_EXPORT_BITS_IO",
            format!("read {} failed: {error}", path.display()),
            "run assay bits-validate and pass its assay_abundance.json or evidence JSON",
        )
    })?)
    .map_err(|error| {
        local_error(
            "CALYX_FSV_ASSAY_FBIN_EXPORT_BITS_INVALID",
            format!("parse {} failed: {error}", path.display()),
            "pass a valid assay bits report with per-lens bits_about",
        )
    })?;
    report_anchor_audit(&report).require_gate_eligible("assay export-fbin grounded anchor gate")?;
    let lenses = report
        .lenses
        .or_else(|| report.report.map(|inner| inner.lenses))
        .ok_or_else(|| {
            local_error(
                "CALYX_FSV_ASSAY_FBIN_EXPORT_BITS_INVALID",
                "bits report missing lenses".to_string(),
                "pass assay_abundance.json or the full bits-validate evidence JSON",
            )
        })?;
    Ok(lenses
        .into_iter()
        .map(|lens| (lens.name.clone(), lens))
        .collect())
}

fn report_anchor_audit(report: &BitsReport) -> AnchorAudit {
    let inner = report.report.as_ref();
    AnchorAudit::from_parts(
        report
            .anchor_audit
            .clone()
            .or_else(|| inner.and_then(|value| value.anchor_audit.clone())),
        report
            .anchor_leaks_into_input
            .or_else(|| inner.and_then(|value| value.anchor_leaks_into_input)),
        report
            .trivial_anchor
            .or_else(|| inner.and_then(|value| value.trivial_anchor)),
        report
            .grounded_gate_eligible
            .or_else(|| inner.and_then(|value| value.grounded_gate_eligible)),
    )
}

pub(super) fn selected_lenses(
    lens_order: &[String],
    meta: &BTreeMap<String, LensMeta>,
    bits: &BTreeMap<String, BitsLens>,
    min_bits: f32,
) -> CliResult<Vec<String>> {
    let mut selected = Vec::new();
    for name in lens_order {
        let Some(lens_meta) = meta.get(name) else {
            continue;
        };
        if admitted(bits.get(name), min_bits) {
            require_countable_content_signal_kind(
                name,
                &lens_meta.signal_kind,
                "assay-fbin-export A35 gate",
            )?;
            selected.push(name.clone());
        }
    }
    if selected.len() < MIN_A35_LENSES {
        return Err(local_error(
            "CALYX_FSV_ASSAY_FBIN_EXPORT_PANEL_TOO_SMALL",
            format!(
                "selected {} admitted lenses; A35 requires at least {MIN_A35_LENSES}",
                selected.len()
            ),
            "run bits-validate on a real panel and export at least ten admitted signal-bearing content lenses",
        ));
    }
    Ok(selected)
}

pub(super) fn ensure_selected_present(
    row: &VectorRow,
    selected: &BTreeSet<String>,
    line_idx: usize,
) -> CliResult {
    for name in selected {
        if !row.lenses.contains_key(name) {
            return Err(local_error(
                "CALYX_FSV_ASSAY_FBIN_EXPORT_LENS_MISSING",
                format!("line {line_idx} missing lens {name}"),
                "rebuild the corpus so every row has the same selected lens roster",
            ));
        }
    }
    Ok(())
}

fn admitted(lens: Option<&BitsLens>, min_bits: f32) -> bool {
    lens.map(|lens| lens.admitted && lens.bits_about.is_finite() && lens.bits_about >= min_bits)
        .unwrap_or(false)
}

fn resolve_path(base: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    }
}

fn hex32(bytes: &[u8; 32]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
