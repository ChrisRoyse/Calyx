use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

#[derive(Clone, Debug)]
pub(crate) struct LoadedPlan {
    pub(crate) slots: Vec<PlanSlot>,
}

#[derive(Clone, Debug)]
pub(crate) struct PlanSlot {
    pub(crate) slot: u16,
    pub(crate) name: String,
    pub(crate) lens_id: String,
    pub(crate) weights_sha256: String,
    pub(crate) bits_about: f32,
    pub(crate) corpus: PathBuf,
    pub(crate) queries: PathBuf,
    pub(crate) vault: PathBuf,
    pub(crate) manifest: Option<PathBuf>,
    pub(crate) runtime: Option<String>,
    pub(crate) max_batch: Option<usize>,
    pub(crate) elapsed_ms: Option<u64>,
    pub(crate) dim: Option<usize>,
    pub(crate) corpus_rows_written: Option<usize>,
    pub(crate) query_rows_written: Option<usize>,
}

impl LoadedPlan {
    pub(crate) fn load(plan_path: &Path, stream_report: Option<&Path>) -> Result<Self, String> {
        if !plan_path.is_file() {
            return Err(format!(
                "CALYX_FSV_ASSAY_I8BIN_CARD_NOT_FOUND: {}",
                plan_path.display()
            ));
        }
        let bytes = fs::read(plan_path)
            .map_err(|error| format!("CALYX_FSV_ASSAY_I8BIN_CARD_NOT_FOUND: {error}"))?;
        let plan: PlanJson = serde_json::from_slice(&bytes).map_err(|error| {
            format!(
                "CALYX_FSV_ASSAY_I8BIN_CARD_INVALID_PLAN: {}: {error}",
                plan_path.display()
            )
        })?;
        let report = match stream_report {
            Some(path) => Some(StreamReport::load(path)?),
            None => None,
        };
        let mut slots = Vec::with_capacity(plan.slots.len());
        for slot in plan.slots {
            let report_slot = report
                .as_ref()
                .and_then(|report| report.by_slot.get(&slot.slot));
            let manifest = report_slot.and_then(|slot| slot.manifest.clone());
            let runtime = manifest
                .as_ref()
                .and_then(|path| manifest_runtime(path).ok());
            slots.push(PlanSlot {
                slot: slot.slot,
                name: slot.name,
                lens_id: slot.lens_id,
                weights_sha256: slot.weights_sha256,
                bits_about: slot.bits_about,
                corpus: slot.corpus,
                queries: slot.queries,
                vault: slot.vault,
                manifest,
                runtime,
                max_batch: report_slot.and_then(|slot| slot.max_batch),
                elapsed_ms: report_slot.and_then(|slot| slot.elapsed_ms),
                dim: report_slot.map(|slot| slot.dim),
                corpus_rows_written: report_slot.map(|slot| slot.corpus_rows_written),
                query_rows_written: report_slot.map(|slot| slot.query_rows_written),
            });
        }
        validate_slots(&slots)?;
        Ok(Self { slots })
    }
}

fn validate_slots(slots: &[PlanSlot]) -> Result<(), String> {
    let mut slot_ids = BTreeSet::new();
    let mut lens_ids = BTreeSet::new();
    for slot in slots {
        if slot.name.trim().is_empty() {
            return Err("CALYX_FSV_ASSAY_I8BIN_CARD_INVALID_PLAN: empty lens name".to_string());
        }
        if !slot_ids.insert(slot.slot) {
            return Err(format!(
                "CALYX_FSV_ASSAY_I8BIN_CARD_INVALID_PLAN: duplicate slot {}",
                slot.slot
            ));
        }
        if !lens_ids.insert(slot.lens_id.clone()) {
            return Err(format!(
                "CALYX_FSV_ASSAY_I8BIN_CARD_INVALID_PLAN: duplicate lens_id {}",
                slot.lens_id
            ));
        }
        if !slot.bits_about.is_finite() {
            return Err(format!(
                "CALYX_FSV_ASSAY_I8BIN_CARD_INVALID_PLAN: non-finite bits_about for {}",
                slot.name
            ));
        }
        if !slot.corpus.is_file() {
            return Err(format!(
                "CALYX_FSV_ASSAY_I8BIN_CARD_NOT_FOUND: {}",
                slot.corpus.display()
            ));
        }
    }
    Ok(())
}

fn manifest_runtime(path: &Path) -> Result<String, String> {
    let bytes =
        fs::read(path).map_err(|error| format!("CALYX_FSV_ASSAY_I8BIN_CARD_NOT_FOUND: {error}"))?;
    let manifest: serde_json::Value = serde_json::from_slice(&bytes).map_err(|error| {
        format!(
            "CALYX_FSV_ASSAY_I8BIN_CARD_INVALID_PLAN: manifest {}: {error}",
            path.display()
        )
    })?;
    for key in ["runtime", "runtime_id", "backend"] {
        if let Some(value) = manifest.get(key).and_then(|value| value.as_str()) {
            return Ok(value.to_string());
        }
    }
    Ok("unknown".to_string())
}

#[derive(Deserialize)]
struct PlanJson {
    slots: Vec<PlanSlotJson>,
}

#[derive(Deserialize)]
struct PlanSlotJson {
    slot: u16,
    name: String,
    lens_id: String,
    weights_sha256: String,
    bits_about: f32,
    corpus: PathBuf,
    queries: PathBuf,
    vault: PathBuf,
}

struct StreamReport {
    by_slot: BTreeMap<u16, ReportSlot>,
}

impl StreamReport {
    fn load(path: &Path) -> Result<Self, String> {
        if !path.is_file() {
            return Err(format!(
                "CALYX_FSV_ASSAY_I8BIN_CARD_NOT_FOUND: {}",
                path.display()
            ));
        }
        let bytes = fs::read(path)
            .map_err(|error| format!("CALYX_FSV_ASSAY_I8BIN_CARD_NOT_FOUND: {error}"))?;
        let report: StreamReportJson = serde_json::from_slice(&bytes).map_err(|error| {
            format!(
                "CALYX_FSV_ASSAY_I8BIN_CARD_INVALID_PLAN: {}: {error}",
                path.display()
            )
        })?;
        Ok(Self {
            by_slot: report
                .lens_roster
                .into_iter()
                .map(|slot| (slot.slot, slot))
                .collect(),
        })
    }
}

#[derive(Deserialize)]
struct StreamReportJson {
    lens_roster: Vec<ReportSlot>,
}

#[derive(Clone, Debug, Deserialize)]
struct ReportSlot {
    slot: u16,
    manifest: Option<PathBuf>,
    dim: usize,
    max_batch: Option<usize>,
    #[serde(default)]
    elapsed_ms: Option<u64>,
    corpus_rows_written: usize,
    query_rows_written: usize,
}
