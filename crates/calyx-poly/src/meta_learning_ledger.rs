//! Append-only meta-learning ledger for self-evolution changes (issue #113).
//!
//! Every self-evolution change needs an auditable row that says what changed, why, what happened to
//! the measured metrics, where the guardrail report lives, and how to roll back. The source of truth
//! is a local JSONL ledger read back from disk; missing guardrail/rollback evidence or non-finite
//! measured effects fail closed before appending.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{
    PolyError, Result, SelfEvolutionGuardrailReport, read_self_evolution_guardrail_report,
};

pub const META_LEARNING_LEDGER_SCHEMA_VERSION: &str = "poly.meta_learning_ledger.v1";
pub const META_LEARNING_LEDGER_FILE: &str = "meta_learning_ledger.jsonl";

pub const ERR_META_LEARNING_INVALID_REQUEST: &str = "CALYX_POLY_META_LEARNING_INVALID_REQUEST";
pub const ERR_META_LEARNING_MISSING_GUARDRAIL: &str = "CALYX_POLY_META_LEARNING_MISSING_GUARDRAIL";
pub const ERR_META_LEARNING_MISSING_ROLLBACK: &str = "CALYX_POLY_META_LEARNING_MISSING_ROLLBACK";
pub const ERR_META_LEARNING_LEDGER_IO: &str = "CALYX_POLY_META_LEARNING_LEDGER_IO";
pub const ERR_META_LEARNING_LEDGER_DECODE: &str = "CALYX_POLY_META_LEARNING_LEDGER_DECODE";
pub const ERR_META_LEARNING_READBACK_MISMATCH: &str = "CALYX_POLY_META_LEARNING_READBACK_MISMATCH";

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct MetaLearningEffect {
    pub objective_score_delta: f64,
    pub kernel_recall_delta: f64,
    pub guard_far_delta: f64,
    pub p95_latency_delta_ms: f64,
}

pub struct MetaLearningLedgerRequest<'a> {
    pub ledger_dir: &'a Path,
    pub change_id: &'a str,
    pub changed_surface: &'a str,
    pub rationale: &'a str,
    pub responsible_actor: &'a str,
    pub effect: MetaLearningEffect,
    pub guardrail_report_path: &'a Path,
    pub rollback_artifact_path: &'a Path,
    pub fsv_artifact_path: &'a Path,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MetaLearningLedgerEntry {
    pub schema_version: String,
    pub sequence: u64,
    pub change_id: String,
    pub changed_surface: String,
    pub rationale: String,
    pub responsible_actor: String,
    pub effect: MetaLearningEffect,
    pub goodhart_risk: bool,
    pub regression_flags: Vec<String>,
    pub guardrail_report_path: String,
    pub guardrail_report_blake3: String,
    pub guardrail_status: String,
    pub rollback_artifact_path: String,
    pub rollback_artifact_blake3: String,
    pub fsv_artifact_path: String,
    pub entry_blake3: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MetaLearningLedgerRun {
    pub ledger_path: PathBuf,
    pub appended: MetaLearningLedgerEntry,
    pub readback_entries: Vec<MetaLearningLedgerEntry>,
}

pub fn append_meta_learning_ledger_entry(
    request: &MetaLearningLedgerRequest<'_>,
) -> Result<MetaLearningLedgerRun> {
    validate_request(request)?;
    fs::create_dir_all(request.ledger_dir).map_err(|err| {
        PolyError::diagnostics(
            ERR_META_LEARNING_LEDGER_IO,
            format!("create ledger dir {}: {err}", request.ledger_dir.display()),
        )
    })?;
    let ledger_path = request.ledger_dir.join(META_LEARNING_LEDGER_FILE);
    let before = read_meta_learning_ledger_entries(&ledger_path)?;
    let entry = build_entry(request, before.len() as u64)?;
    append_entry_line(&ledger_path, &entry)?;
    let after = read_meta_learning_ledger_entries(&ledger_path)?;
    if after.last() != Some(&entry) || after.len() != before.len() + 1 {
        return Err(PolyError::diagnostics(
            ERR_META_LEARNING_READBACK_MISMATCH,
            format!(
                "meta-learning ledger {} did not read back appended entry",
                ledger_path.display()
            ),
        ));
    }
    Ok(MetaLearningLedgerRun {
        ledger_path,
        appended: entry,
        readback_entries: after,
    })
}

pub fn read_meta_learning_ledger_entries(path: &Path) -> Result<Vec<MetaLearningLedgerEntry>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let text = fs::read_to_string(path).map_err(|err| {
        PolyError::diagnostics(
            ERR_META_LEARNING_LEDGER_IO,
            format!("read meta-learning ledger {}: {err}", path.display()),
        )
    })?;
    let mut entries = Vec::new();
    for (idx, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            return Err(PolyError::diagnostics(
                ERR_META_LEARNING_LEDGER_DECODE,
                format!("ledger {} line {} is empty", path.display(), idx + 1),
            ));
        }
        entries.push(serde_json::from_str(line).map_err(|err| {
            PolyError::diagnostics(
                ERR_META_LEARNING_LEDGER_DECODE,
                format!("decode ledger {} line {}: {err}", path.display(), idx + 1),
            )
        })?);
    }
    Ok(entries)
}

fn build_entry(
    request: &MetaLearningLedgerRequest<'_>,
    sequence: u64,
) -> Result<MetaLearningLedgerEntry> {
    let guardrail = read_guardrail(request.guardrail_report_path)?;
    let guardrail_bytes = fs::read(request.guardrail_report_path).map_err(|err| {
        PolyError::diagnostics(
            ERR_META_LEARNING_MISSING_GUARDRAIL,
            format!(
                "read guardrail report {}: {err}",
                request.guardrail_report_path.display()
            ),
        )
    })?;
    let rollback_bytes = fs::read(request.rollback_artifact_path).map_err(|err| {
        PolyError::diagnostics(
            ERR_META_LEARNING_MISSING_ROLLBACK,
            format!(
                "read rollback artifact {}: {err}",
                request.rollback_artifact_path.display()
            ),
        )
    })?;
    let regression_flags = regression_flags(request.effect);
    let goodhart_risk = request.effect.objective_score_delta > 0.0 && !regression_flags.is_empty();
    let mut entry = MetaLearningLedgerEntry {
        schema_version: META_LEARNING_LEDGER_SCHEMA_VERSION.to_string(),
        sequence,
        change_id: request.change_id.to_string(),
        changed_surface: request.changed_surface.to_string(),
        rationale: request.rationale.to_string(),
        responsible_actor: request.responsible_actor.to_string(),
        effect: request.effect,
        goodhart_risk,
        regression_flags,
        guardrail_report_path: request.guardrail_report_path.display().to_string(),
        guardrail_report_blake3: blake3::hash(&guardrail_bytes).to_hex().to_string(),
        guardrail_status: format!("{:?}", guardrail.status),
        rollback_artifact_path: request.rollback_artifact_path.display().to_string(),
        rollback_artifact_blake3: blake3::hash(&rollback_bytes).to_hex().to_string(),
        fsv_artifact_path: request.fsv_artifact_path.display().to_string(),
        entry_blake3: String::new(),
    };
    entry.entry_blake3 = entry_hash(&entry)?;
    Ok(entry)
}

fn read_guardrail(path: &Path) -> Result<SelfEvolutionGuardrailReport> {
    read_self_evolution_guardrail_report(path).map_err(|err| {
        PolyError::diagnostics(
            ERR_META_LEARNING_MISSING_GUARDRAIL,
            format!("read guardrail report {}: {err}", path.display()),
        )
    })
}

fn append_entry_line(path: &Path, entry: &MetaLearningLedgerEntry) -> Result<()> {
    let line = serde_json::to_string(entry).map_err(|err| {
        PolyError::diagnostics(
            ERR_META_LEARNING_LEDGER_DECODE,
            format!("encode meta-learning entry: {err}"),
        )
    })?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|err| {
            PolyError::diagnostics(
                ERR_META_LEARNING_LEDGER_IO,
                format!("open meta-learning ledger {}: {err}", path.display()),
            )
        })?;
    writeln!(file, "{line}").map_err(|err| {
        PolyError::diagnostics(
            ERR_META_LEARNING_LEDGER_IO,
            format!("append meta-learning ledger {}: {err}", path.display()),
        )
    })
}

fn validate_request(request: &MetaLearningLedgerRequest<'_>) -> Result<()> {
    if request.change_id.trim().is_empty()
        || request.changed_surface.trim().is_empty()
        || request.rationale.trim().is_empty()
        || request.responsible_actor.trim().is_empty()
    {
        return invalid(
            "change id, changed surface, rationale, and responsible actor are required",
        );
    }
    for value in [
        request.effect.objective_score_delta,
        request.effect.kernel_recall_delta,
        request.effect.guard_far_delta,
        request.effect.p95_latency_delta_ms,
    ] {
        if !value.is_finite() {
            return invalid("measured effect values must be finite");
        }
    }
    Ok(())
}

fn regression_flags(effect: MetaLearningEffect) -> Vec<String> {
    let mut flags = Vec::new();
    if effect.kernel_recall_delta < 0.0 {
        flags.push("kernel_recall_regressed".to_string());
    }
    if effect.guard_far_delta > 0.0 {
        flags.push("guard_far_increased".to_string());
    }
    if effect.p95_latency_delta_ms > 0.0 {
        flags.push("latency_increased".to_string());
    }
    flags
}

fn entry_hash(entry: &MetaLearningLedgerEntry) -> Result<String> {
    let mut clone = entry.clone();
    clone.entry_blake3.clear();
    let bytes = serde_json::to_vec(&clone).map_err(|err| {
        PolyError::diagnostics(
            ERR_META_LEARNING_LEDGER_DECODE,
            format!("encode meta-learning entry hash payload: {err}"),
        )
    })?;
    Ok(blake3::hash(&bytes).to_hex().to_string())
}

fn invalid<T>(message: impl Into<String>) -> Result<T> {
    Err(PolyError::diagnostics(
        ERR_META_LEARNING_INVALID_REQUEST,
        message.into(),
    ))
}
