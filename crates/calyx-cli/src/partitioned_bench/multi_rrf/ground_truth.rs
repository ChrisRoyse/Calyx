use std::collections::{BTreeSet, HashSet};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use calyx_core::CalyxError;
use calyx_sextant::index::I32BinMatrix;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::{Plan, report};
use crate::error::{CliError, CliResult};

const FORMAT: &str = "calyx-partitioned-rrf-ground-truth-v1";
const MODE: &str = "fused_rrf";
const ROW_ID_SPACE: &str = "partitioned_rrf_plan_corpus_row_idx";

#[derive(Clone, Debug)]
pub(super) struct PrecomputedTruth {
    rows: Vec<Vec<u64>>,
    source: Value,
    scale_suitable: bool,
}

#[derive(Clone, Debug)]
pub(super) struct Context<'a> {
    pub(super) truth_file: &'a Path,
    pub(super) manifest_file: &'a Path,
    pub(super) plan_path: &'a Path,
    pub(super) plan: &'a Plan,
    pub(super) truth_n: usize,
    pub(super) k: usize,
    pub(super) truth_depth: usize,
    pub(super) corpus_rows: usize,
}

#[derive(Clone, Debug, Deserialize)]
struct TruthManifest {
    format: String,
    mode: String,
    row_id_space: String,
    truth_file_sha256: String,
    plan_sha256: String,
    query_count: usize,
    k: usize,
    truth_depth: usize,
    corpus_rows: usize,
    slots: Vec<TruthSlot>,
    #[serde(default)]
    reference_backend: String,
    #[serde(default)]
    scale_suitable: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
struct TruthSlot {
    slot: u16,
    lens_id: String,
    weights_sha256: String,
    #[serde(default)]
    signal_kind: String,
}

impl PrecomputedTruth {
    pub(super) fn load(ctx: Context<'_>) -> CliResult<Self> {
        let truth_sha256 = sha256_file(ctx.truth_file)?;
        let manifest_bytes = fs::read(ctx.manifest_file).map_err(|error| {
            gt_error(
                "CALYX_FSV_PARTITIONED_RRF_GROUND_TRUTH_MANIFEST_IO",
                format!("read {} failed: {error}", ctx.manifest_file.display()),
                "pass the manifest written with the fused RRF truth file",
            )
        })?;
        let manifest: TruthManifest = serde_json::from_slice(&manifest_bytes).map_err(|error| {
            gt_error(
                "CALYX_FSV_PARTITIONED_RRF_GROUND_TRUTH_MANIFEST_INVALID",
                format!("parse {} failed: {error}", ctx.manifest_file.display()),
                "regenerate the fused RRF truth manifest from the current plan",
            )
        })?;
        validate_manifest(&manifest, &truth_sha256, &ctx)?;
        let matrix = I32BinMatrix::open(ctx.truth_file).map_err(|error| {
            gt_error(
                "CALYX_FSV_PARTITIONED_RRF_GROUND_TRUTH_IO",
                format!("open {} failed: {error}", ctx.truth_file.display()),
                "pass a valid .i32bin fused RRF truth file",
            )
        })?;
        validate_matrix_shape(&matrix, &ctx)?;
        let rows = load_rows(&matrix, &ctx)?;
        let source = json!({
            "mode": "precomputed_fused_rrf_i32bin",
            "metric_class": report::METRIC_CLASS,
            "metric_scope": report::METRIC_SCOPE,
            "truth_reference_class": report::TRUTH_REFERENCE_CLASS,
            "valid_real_outcome": false,
            "grounded_phase_exit_eligible": false,
            "format": FORMAT,
            "file": ctx.truth_file,
            "file_sha256": truth_sha256,
            "file_bytes": fs::metadata(ctx.truth_file).map(|meta| meta.len()).unwrap_or(0),
            "manifest": ctx.manifest_file,
            "manifest_sha256": sha256_bytes(&manifest_bytes),
            "plan": ctx.plan_path,
            "plan_sha256": manifest.plan_sha256,
            "row_id_space": manifest.row_id_space,
            "rows": matrix.count(),
            "width": matrix.width(),
            "query_count_used": ctx.truth_n,
            "k_used": ctx.k,
            "truth_depth": manifest.truth_depth,
            "corpus_rows": manifest.corpus_rows,
            "reference_backend": manifest.reference_backend,
            "scale_suitable": manifest.scale_suitable,
            "slots": manifest.slots,
        });
        Ok(Self {
            rows,
            source,
            scale_suitable: manifest.scale_suitable,
        })
    }

    pub(super) fn row_ids(&self, query_idx: usize) -> &[u64] {
        &self.rows[query_idx]
    }

    pub(super) fn source(&self) -> Value {
        self.source.clone()
    }

    pub(super) fn scale_suitable(&self) -> bool {
        self.scale_suitable
    }
}

pub(super) fn write(rows: &[Vec<u64>], ctx: Context<'_>) -> CliResult<Value> {
    if ctx.truth_file.exists() || ctx.manifest_file.exists() {
        return Err(gt_error(
            "CALYX_FSV_PARTITIONED_RRF_GROUND_TRUTH_EXISTS",
            "fused RRF ground-truth output already exists",
            "write to a fresh path so stale truth cannot be overwritten silently",
        ));
    }
    let truth_bytes = i32bin_bytes(rows, ctx.k)?;
    write_atomic(ctx.truth_file, &truth_bytes)?;
    let truth_sha256 = sha256_bytes(&truth_bytes);
    let plan_sha256 = sha256_file(ctx.plan_path)?;
    let manifest = json!({
        "format": FORMAT,
        "mode": MODE,
        "row_id_space": ROW_ID_SPACE,
        "truth_file": ctx.truth_file,
        "truth_file_sha256": truth_sha256,
        "plan": ctx.plan_path,
        "plan_sha256": plan_sha256,
        "query_count": rows.len(),
        "k": ctx.k,
        "truth_depth": ctx.truth_depth,
        "corpus_rows": ctx.corpus_rows,
        "reference_backend": "calyx-bench-partitioned-rrf-diagnostic-v1",
        "scale_suitable": false,
        "slots": plan_slots(ctx.plan),
        "generator": "calyx bench partitioned-rrf",
    });
    let manifest_bytes = serde_json::to_vec_pretty(&manifest)?;
    write_atomic(ctx.manifest_file, &manifest_bytes)?;
    Ok(json!({
        "mode": "generated_fused_rrf_i32bin",
        "metric_class": report::METRIC_CLASS,
        "metric_scope": report::METRIC_SCOPE,
        "truth_reference_class": report::TRUTH_REFERENCE_CLASS,
        "valid_real_outcome": false,
        "grounded_phase_exit_eligible": false,
        "format": FORMAT,
        "file": ctx.truth_file,
        "file_sha256": truth_sha256,
        "file_bytes": truth_bytes.len(),
        "manifest": ctx.manifest_file,
        "manifest_sha256": sha256_bytes(&manifest_bytes),
        "plan_sha256": plan_sha256,
        "rows": rows.len(),
        "width": ctx.k,
        "truth_depth": ctx.truth_depth,
        "corpus_rows": ctx.corpus_rows,
        "reference_backend": "calyx-bench-partitioned-rrf-diagnostic-v1",
        "scale_suitable": false,
    }))
}

fn validate_manifest(manifest: &TruthManifest, truth_sha256: &str, ctx: &Context<'_>) -> CliResult {
    if manifest.format != FORMAT || manifest.mode != MODE || manifest.row_id_space != ROW_ID_SPACE {
        return Err(gt_error(
            "CALYX_FSV_PARTITIONED_RRF_GROUND_TRUTH_NOT_FUSED_PANEL",
            "ground-truth manifest is not a fused RRF panel truth artifact",
            "use a manifest produced for calyx partitioned-rrf fused truth",
        ));
    }
    if manifest.truth_file_sha256 != truth_sha256 {
        return Err(stale(
            "truth_file_sha256",
            &manifest.truth_file_sha256,
            truth_sha256,
        ));
    }
    let plan_sha256 = sha256_file(ctx.plan_path)?;
    if manifest.plan_sha256 != plan_sha256 {
        return Err(stale("plan_sha256", &manifest.plan_sha256, &plan_sha256));
    }
    if manifest.query_count < ctx.truth_n
        || manifest.k < ctx.k
        || manifest.corpus_rows != ctx.corpus_rows
    {
        return Err(gt_error(
            "CALYX_FSV_PARTITIONED_RRF_GROUND_TRUTH_MISMATCH",
            format!(
                "manifest query_count/k/corpus_rows = {}/{}/{} but run needs {}/{}/{}",
                manifest.query_count,
                manifest.k,
                manifest.corpus_rows,
                ctx.truth_n,
                ctx.k,
                ctx.corpus_rows
            ),
            "regenerate fused truth for this query count, k, and corpus row count",
        ));
    }
    if manifest.slots != plan_slots(ctx.plan) {
        return Err(gt_error(
            "CALYX_FSV_PARTITIONED_RRF_GROUND_TRUTH_STALE",
            "manifest lens roster does not match the current RRF plan",
            "regenerate fused truth after changing lenses, weights, or slot order",
        ));
    }
    Ok(())
}

fn validate_matrix_shape(matrix: &I32BinMatrix, ctx: &Context<'_>) -> CliResult {
    if matrix.count() < ctx.truth_n as u64 {
        return Err(gt_error(
            "CALYX_FSV_PARTITIONED_RRF_GROUND_TRUTH_MISMATCH",
            format!(
                "truth rows={} but run needs {}",
                matrix.count(),
                ctx.truth_n
            ),
            "regenerate fused truth with enough query rows",
        ));
    }
    if matrix.width() < ctx.k {
        return Err(gt_error(
            "CALYX_FSV_PARTITIONED_RRF_GROUND_TRUTH_MISMATCH",
            format!("truth width={} is smaller than k {}", matrix.width(), ctx.k),
            "regenerate fused truth with width at least k",
        ));
    }
    Ok(())
}

fn load_rows(matrix: &I32BinMatrix, ctx: &Context<'_>) -> CliResult<Vec<Vec<u64>>> {
    (0..ctx.truth_n)
        .map(|idx| {
            let mut seen = HashSet::with_capacity(ctx.k);
            matrix
                .row(idx as u64)
                .into_iter()
                .take(ctx.k)
                .map(|value| {
                    if value < 0 {
                        return Err(gt_error(
                            "CALYX_FSV_PARTITIONED_RRF_GROUND_TRUTH_INVALID",
                            format!("truth row {idx} contains negative id {value}"),
                            "regenerate fused truth with non-negative corpus row ids",
                        ));
                    }
                    let id = value as u64;
                    if id >= ctx.corpus_rows as u64 {
                        return Err(gt_error(
                            "CALYX_FSV_PARTITIONED_RRF_GROUND_TRUTH_STALE",
                            format!(
                                "truth row {idx} id {id} outside corpus rows {}",
                                ctx.corpus_rows
                            ),
                            "regenerate fused truth for the current corpus row space",
                        ));
                    }
                    if !seen.insert(id) {
                        return Err(gt_error(
                            "CALYX_FSV_PARTITIONED_RRF_GROUND_TRUTH_INVALID",
                            format!("truth row {idx} repeats id {id}"),
                            "regenerate fused truth with unique top-k ids per query",
                        ));
                    }
                    Ok(id)
                })
                .collect()
        })
        .collect()
}

fn i32bin_bytes(rows: &[Vec<u64>], width: usize) -> CliResult<Vec<u8>> {
    let mut bytes = Vec::with_capacity(8 + rows.len() * width * 4);
    bytes.extend_from_slice(&(rows.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&(width as u32).to_le_bytes());
    for (row_idx, row) in rows.iter().enumerate() {
        if row.len() < width {
            return Err(gt_error(
                "CALYX_FSV_PARTITIONED_RRF_GROUND_TRUTH_INVALID",
                format!(
                    "generated truth row {row_idx} has {} ids, need {width}",
                    row.len()
                ),
                "increase truth-depth so fused exact top-k is complete",
            ));
        }
        let mut seen = BTreeSet::new();
        for &id in row.iter().take(width) {
            if id > i32::MAX as u64 || !seen.insert(id) {
                return Err(gt_error(
                    "CALYX_FSV_PARTITIONED_RRF_GROUND_TRUTH_INVALID",
                    format!("generated truth row {row_idx} has invalid id {id}"),
                    "regenerate truth with unique i32-addressable row ids",
                ));
            }
            bytes.extend_from_slice(&(id as i32).to_le_bytes());
        }
    }
    Ok(bytes)
}

fn plan_slots(plan: &Plan) -> Vec<TruthSlot> {
    plan.slots
        .iter()
        .map(|slot| TruthSlot {
            slot: slot.slot,
            lens_id: slot.lens_id.clone().unwrap_or_default(),
            weights_sha256: slot.weights_sha256.clone().unwrap_or_default(),
            signal_kind: slot.signal_kind.clone().unwrap_or_default(),
        })
        .collect()
}

fn write_atomic(path: &Path, bytes: &[u8]) -> CliResult {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    {
        let mut file = File::create(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    fs::rename(&tmp, path).inspect_err(|_| {
        let _ = fs::remove_file(&tmp);
    })?;
    Ok(())
}

fn sha256_file(path: &Path) -> CliResult<String> {
    let mut file = File::open(path).map_err(|error| {
        gt_error(
            "CALYX_FSV_PARTITIONED_RRF_GROUND_TRUTH_IO",
            format!("open {} failed: {error}", path.display()),
            "check the fused truth, manifest, and plan paths",
        )
    })?;
    let mut hasher = Sha256::new();
    let mut buf = [0_u8; 1024 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex_lower(&hasher.finalize()))
}

fn sha256_bytes(bytes: &[u8]) -> String {
    hex_lower(&Sha256::digest(bytes))
}

fn hex_lower(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        write!(&mut out, "{byte:02x}").expect("hex write");
    }
    out
}

fn stale(field: &'static str, expected: &str, actual: &str) -> CliError {
    gt_error(
        "CALYX_FSV_PARTITIONED_RRF_GROUND_TRUTH_STALE",
        format!("{field} manifest={expected} actual={actual}"),
        "regenerate fused truth from the current plan and source files",
    )
}

fn gt_error(code: &'static str, message: impl Into<String>, remediation: &'static str) -> CliError {
    CliError::Calyx(CalyxError {
        code,
        message: message.into(),
        remediation,
    })
}

#[cfg(test)]
#[path = "ground_truth_tests.rs"]
mod tests;
