use std::collections::{BTreeMap, HashSet};
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

use calyx_core::{CalyxError, SlotId};
use calyx_sextant::index::I32BinMatrix;
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::{Plan, report, slot_id};
use crate::error::{CliError, CliResult};

const FORMAT: &str = "calyx-partitioned-rrf-slot-ground-truth-v1";
const MODE: &str = "per_slot_ranked_rrf_reference";
const ROW_ID_SPACE: &str = "partitioned_rrf_plan_corpus_row_idx";

#[derive(Clone, Debug)]
pub(super) struct SlotTruth {
    rows_by_slot: BTreeMap<SlotId, Vec<Vec<u64>>>,
    source: Value,
    scale_suitable: bool,
}

#[derive(Clone, Debug)]
pub(super) struct Context<'a> {
    pub(super) manifest_file: &'a Path,
    pub(super) plan_path: &'a Path,
    pub(super) plan: &'a Plan,
    pub(super) truth_n: usize,
    pub(super) truth_depth: usize,
    pub(super) corpus_rows: usize,
}

#[derive(Clone, Debug, Deserialize)]
struct SlotTruthManifest {
    format: String,
    mode: String,
    row_id_space: String,
    plan_sha256: String,
    query_count: usize,
    truth_depth: usize,
    corpus_rows: usize,
    reference_backend: String,
    scale_suitable: bool,
    slots: Vec<ManifestSlot>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
struct ManifestSlot {
    slot: u16,
    lens_id: String,
    weights_sha256: String,
    #[serde(default)]
    signal_kind: String,
    file: PathBuf,
    file_sha256: String,
    rows: usize,
    width: usize,
}

impl SlotTruth {
    pub(super) fn load(ctx: Context<'_>) -> CliResult<Self> {
        let manifest_bytes = fs::read(ctx.manifest_file).map_err(|error| {
            st_error(
                "CALYX_FSV_PARTITIONED_RRF_SLOT_GROUND_TRUTH_MANIFEST_IO",
                format!("read {} failed: {error}", ctx.manifest_file.display()),
                "pass a per-slot truth manifest produced for this RRF plan",
            )
        })?;
        let manifest: SlotTruthManifest =
            serde_json::from_slice(&manifest_bytes).map_err(|error| {
                st_error(
                    "CALYX_FSV_PARTITIONED_RRF_SLOT_GROUND_TRUTH_MANIFEST_INVALID",
                    format!("parse {} failed: {error}", ctx.manifest_file.display()),
                    "regenerate the per-slot truth manifest",
                )
            })?;
        validate_manifest(&manifest, &ctx)?;
        let mut source_slots = Vec::with_capacity(manifest.slots.len());
        let mut rows_by_slot = BTreeMap::new();
        let base = ctx.manifest_file.parent().unwrap_or_else(|| Path::new(""));
        for spec in &manifest.slots {
            let file = resolve_manifest_path(base, &spec.file);
            let sha = sha256_file(&file)?;
            if spec.file_sha256 != sha {
                return Err(stale("slot file_sha256", &spec.file_sha256, &sha));
            }
            let matrix = I32BinMatrix::open(&file).map_err(|error| {
                st_error(
                    "CALYX_FSV_PARTITIONED_RRF_SLOT_GROUND_TRUTH_IO",
                    format!("open {} failed: {error}", file.display()),
                    "pass valid per-slot .i32bin ranked truth files",
                )
            })?;
            validate_matrix_shape(&matrix, spec, &ctx)?;
            let rows = load_rows(&matrix, spec.slot, &ctx)?;
            rows_by_slot.insert(slot_id(spec.slot), rows);
            source_slots.push(json!({
                "slot": spec.slot,
                "lens_id": spec.lens_id,
                "weights_sha256": spec.weights_sha256,
                "signal_kind": spec.signal_kind,
                "file": file,
                "file_sha256": sha,
                "rows": matrix.count(),
                "width": matrix.width(),
            }));
        }
        let source = json!({
            "mode": "precomputed_slot_rrf_i32bin",
            "metric_class": report::METRIC_CLASS,
            "metric_scope": report::METRIC_SCOPE,
            "truth_reference_class": report::TRUTH_REFERENCE_CLASS,
            "valid_real_outcome": false,
            "grounded_phase_exit_eligible": false,
            "format": FORMAT,
            "manifest": ctx.manifest_file,
            "manifest_sha256": sha256_bytes(&manifest_bytes),
            "plan": ctx.plan_path,
            "plan_sha256": manifest.plan_sha256,
            "row_id_space": manifest.row_id_space,
            "query_count_used": ctx.truth_n,
            "truth_depth": manifest.truth_depth,
            "corpus_rows": manifest.corpus_rows,
            "reference_backend": manifest.reference_backend,
            "scale_suitable": manifest.scale_suitable,
            "slots": source_slots,
        });
        Ok(Self {
            rows_by_slot,
            source,
            scale_suitable: manifest.scale_suitable,
        })
    }

    pub(super) fn row_ids(&self, slot: SlotId, query_idx: usize) -> &[u64] {
        &self.rows_by_slot[&slot][query_idx]
    }

    pub(super) fn source(&self) -> Value {
        self.source.clone()
    }

    pub(super) fn scale_suitable(&self) -> bool {
        self.scale_suitable
    }
}

fn validate_manifest(manifest: &SlotTruthManifest, ctx: &Context<'_>) -> CliResult {
    if manifest.format != FORMAT || manifest.mode != MODE || manifest.row_id_space != ROW_ID_SPACE {
        return Err(st_error(
            "CALYX_FSV_PARTITIONED_RRF_SLOT_GROUND_TRUTH_NOT_RRF_REFERENCE",
            "per-slot truth manifest is not a Calyx RRF reference artifact",
            "use a manifest produced for partitioned-rrf per-slot truth",
        ));
    }
    if manifest.reference_backend.trim().is_empty() {
        return Err(st_error(
            "CALYX_FSV_PARTITIONED_RRF_SLOT_GROUND_TRUTH_MANIFEST_INVALID",
            "reference_backend is empty",
            "record the exact/reference engine that produced the per-slot ranks",
        ));
    }
    let plan_sha256 = sha256_file(ctx.plan_path)?;
    if manifest.plan_sha256 != plan_sha256 {
        return Err(stale("plan_sha256", &manifest.plan_sha256, &plan_sha256));
    }
    if manifest.query_count < ctx.truth_n
        || manifest.truth_depth < ctx.truth_depth
        || manifest.corpus_rows != ctx.corpus_rows
    {
        return Err(st_error(
            "CALYX_FSV_PARTITIONED_RRF_SLOT_GROUND_TRUTH_MISMATCH",
            format!(
                "manifest query_count/truth_depth/corpus_rows = {}/{}/{} but run needs {}/{}/{}",
                manifest.query_count,
                manifest.truth_depth,
                manifest.corpus_rows,
                ctx.truth_n,
                ctx.truth_depth,
                ctx.corpus_rows
            ),
            "regenerate per-slot truth for this run depth and corpus row space",
        ));
    }
    if manifest_slots(manifest) != plan_slots(ctx.plan) {
        return Err(st_error(
            "CALYX_FSV_PARTITIONED_RRF_SLOT_GROUND_TRUTH_STALE",
            "manifest lens roster does not match the current RRF plan",
            "regenerate per-slot truth after changing lenses, weights, or slot order",
        ));
    }
    Ok(())
}

fn validate_matrix_shape(
    matrix: &I32BinMatrix,
    spec: &ManifestSlot,
    ctx: &Context<'_>,
) -> CliResult {
    if matrix.count() < ctx.truth_n as u64
        || matrix.width() < ctx.truth_depth
        || spec.rows < ctx.truth_n
        || spec.width < ctx.truth_depth
    {
        return Err(st_error(
            "CALYX_FSV_PARTITIONED_RRF_SLOT_GROUND_TRUTH_MISMATCH",
            format!(
                "slot {} rows/width matrix={}/{} manifest={}/{} needs {}/{}",
                spec.slot,
                matrix.count(),
                matrix.width(),
                spec.rows,
                spec.width,
                ctx.truth_n,
                ctx.truth_depth
            ),
            "regenerate per-slot truth with enough rows and rank depth",
        ));
    }
    Ok(())
}

fn load_rows(matrix: &I32BinMatrix, slot: u16, ctx: &Context<'_>) -> CliResult<Vec<Vec<u64>>> {
    (0..ctx.truth_n)
        .map(|idx| {
            let mut seen = HashSet::with_capacity(ctx.truth_depth);
            matrix
                .row(idx as u64)
                .into_iter()
                .take(ctx.truth_depth)
                .map(|value| checked_id(value, slot, idx, &mut seen, ctx.corpus_rows))
                .collect()
        })
        .collect()
}

fn checked_id(
    value: i32,
    slot: u16,
    row: usize,
    seen: &mut HashSet<u64>,
    corpus_rows: usize,
) -> CliResult<u64> {
    if value < 0 {
        return Err(st_error(
            "CALYX_FSV_PARTITIONED_RRF_SLOT_GROUND_TRUTH_INVALID",
            format!("slot {slot} row {row} contains negative id {value}"),
            "regenerate per-slot truth with non-negative corpus row ids",
        ));
    }
    let id = value as u64;
    if id >= corpus_rows as u64 || !seen.insert(id) {
        return Err(st_error(
            "CALYX_FSV_PARTITIONED_RRF_SLOT_GROUND_TRUTH_INVALID",
            format!("slot {slot} row {row} has out-of-range or duplicate id {id}"),
            "regenerate per-slot truth for the current corpus row space",
        ));
    }
    Ok(id)
}

fn manifest_slots(manifest: &SlotTruthManifest) -> Vec<(u16, String, String, String)> {
    manifest
        .slots
        .iter()
        .map(|slot| {
            (
                slot.slot,
                slot.lens_id.clone(),
                slot.weights_sha256.clone(),
                slot.signal_kind.clone(),
            )
        })
        .collect()
}

fn plan_slots(plan: &Plan) -> Vec<(u16, String, String, String)> {
    plan.slots
        .iter()
        .map(|slot| {
            (
                slot.slot,
                slot.lens_id.clone().unwrap_or_default(),
                slot.weights_sha256.clone().unwrap_or_default(),
                slot.signal_kind.clone().unwrap_or_default(),
            )
        })
        .collect()
}

fn resolve_manifest_path(base: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    }
}

fn sha256_file(path: &Path) -> CliResult<String> {
    let mut file = File::open(path).map_err(|error| {
        st_error(
            "CALYX_FSV_PARTITIONED_RRF_SLOT_GROUND_TRUTH_IO",
            format!("open {} failed: {error}", path.display()),
            "check the per-slot truth manifest and file paths",
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
    st_error(
        "CALYX_FSV_PARTITIONED_RRF_SLOT_GROUND_TRUTH_STALE",
        format!("{field} manifest={expected} actual={actual}"),
        "regenerate per-slot truth from the current plan and source files",
    )
}

fn st_error(code: &'static str, message: impl Into<String>, remediation: &'static str) -> CliError {
    CliError::Calyx(CalyxError {
        code,
        message: message.into(),
        remediation,
    })
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::super::load_plan;
    use super::*;

    #[test]
    fn slot_truth_loads_rank_rows_and_rejects_stale_plan() {
        let root = temp_root("slot-truth");
        let plan_path = root.join("plan.json");
        write_plan(&plan_path, 0);
        let plan = load_plan(&plan_path).unwrap();
        for slot in 0..4 {
            write_i32bin(
                &root.join(format!("slot-{slot}.i32bin")),
                &[vec![slot, 4, 5, 6], vec![6, 5, 4, slot]],
            );
        }
        let manifest_path = root.join("slot-truth.manifest.json");
        write_manifest(&manifest_path, &plan_path, &root, 0);

        let loaded = SlotTruth::load(Context {
            manifest_file: &manifest_path,
            plan_path: &plan_path,
            plan: &plan,
            truth_n: 2,
            truth_depth: 4,
            corpus_rows: 8,
        })
        .unwrap();

        assert_eq!(loaded.source()["mode"], "precomputed_slot_rrf_i32bin");
        assert_eq!(loaded.source()["metric_class"], report::METRIC_CLASS);
        assert_eq!(loaded.source()["valid_real_outcome"], false);
        assert_eq!(loaded.source()["grounded_phase_exit_eligible"], false);
        assert_eq!(loaded.row_ids(slot_id(2), 0), &[2, 4, 5, 6]);

        write_plan(&plan_path, 10);
        let changed_plan = load_plan(&plan_path).unwrap();
        let err = SlotTruth::load(Context {
            manifest_file: &manifest_path,
            plan_path: &plan_path,
            plan: &changed_plan,
            truth_n: 2,
            truth_depth: 4,
            corpus_rows: 8,
        })
        .unwrap_err();
        assert_eq!(
            err.code(),
            "CALYX_FSV_PARTITIONED_RRF_SLOT_GROUND_TRUTH_STALE"
        );
        let _ = fs::remove_dir_all(root);
    }

    fn write_manifest(path: &Path, plan_path: &Path, root: &Path, offset: u16) {
        let slots = (0..4)
            .map(|idx| {
                let file = root.join(format!("slot-{idx}.i32bin"));
                json!({
                    "slot": idx,
                    "lens_id": format!("{:032x}", idx + offset),
                    "weights_sha256": format!("{:064x}", idx + offset),
                    "signal_kind": "learned_encoder",
                    "file": format!("slot-{idx}.i32bin"),
                    "file_sha256": sha256_file(&file).unwrap(),
                    "rows": 2,
                    "width": 4,
                })
            })
            .collect::<Vec<_>>();
        let manifest = json!({
            "format": FORMAT,
            "mode": MODE,
            "row_id_space": ROW_ID_SPACE,
            "plan_sha256": sha256_file(plan_path).unwrap(),
            "query_count": 2,
            "truth_depth": 4,
            "corpus_rows": 8,
            "reference_backend": "unit-test-exact",
            "scale_suitable": false,
            "slots": slots,
        });
        fs::write(path, serde_json::to_vec_pretty(&manifest).unwrap()).unwrap();
    }

    fn write_i32bin(path: &Path, rows: &[Vec<u16>]) {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&(rows.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&(rows[0].len() as u32).to_le_bytes());
        for row in rows {
            for value in row {
                bytes.extend_from_slice(&i32::from(*value).to_le_bytes());
            }
        }
        fs::write(path, bytes).unwrap();
    }

    fn write_plan(path: &Path, offset: u16) {
        let slots = (0..4)
            .map(|idx| {
                format!(
                    r#"{{"slot":{idx},"lens_id":"{:032x}","weights_sha256":"{:064x}","signal_kind":"learned_encoder","bits_about":0.1,"vault":"vault-{idx}","queries":"queries-{idx}.fbin","corpus":"corpus-{idx}.fbin"}}"#,
                    idx + offset,
                    idx + offset
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        fs::write(path, format!(r#"{{"slots":[{slots}]}}"#)).unwrap();
    }

    fn temp_root(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("{name}-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        root
    }
}
