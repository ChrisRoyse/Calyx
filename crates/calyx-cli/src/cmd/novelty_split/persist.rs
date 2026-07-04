use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::{Value, json};

use super::model::{
    ArtifactReadback, BeforeAfterTopK, CLINICAL_BOUNDARY, InputScope, ManualSpotChecks,
    PersistedReadback, SCHEMA_VERSION, SourceArtifact, SplitRow, ValidationMetrics,
};
use super::scoring::{flag_counts, issue_counts, slim, text_blob};
use crate::cmd::artifact_hash::sha256_hex;
use crate::error::{CliError, CliResult};

pub(super) struct PersistedSplit {
    pub root: PathBuf,
    pub readback: PersistedReadback,
    pub readback_sha256: String,
}

pub(super) fn input_scope(artifacts: Vec<SourceArtifact>, command: &str) -> InputScope {
    InputScope {
        schema_version: SCHEMA_VERSION,
        command: command.to_string(),
        source_artifacts: artifacts,
    }
}

pub(super) fn metrics(
    artifacts: &[SourceArtifact],
    combined: &[SplitRow],
    calibration: &[SplitRow],
    novelty: &[SplitRow],
) -> ValidationMetrics {
    ValidationMetrics {
        schema_version: SCHEMA_VERSION,
        status: "ok".to_string(),
        input_row_counts: artifacts
            .iter()
            .map(|artifact| {
                (
                    format!("{}_{}", artifact.issue, artifact.domain),
                    artifact.row_count,
                )
            })
            .collect(),
        total_rows: combined.len(),
        calibration_known_positive_rows: calibration.len(),
        novelty_prioritized_rows: novelty.len(),
        calibration_by_issue: issue_counts(calibration),
        novelty_by_issue: issue_counts(novelty),
        calibration_flag_counts: flag_counts(calibration),
        top_original_candidate: combined.first().map(slim),
        top_novelty_candidate: novelty.first().map(slim),
        top_calibration_candidate: calibration.first().map(slim),
        clinical_boundary: CLINICAL_BOUNDARY.to_string(),
        interpretation: "split ranking views only; no row is promoted to clinical actionability or cure evidence".to_string(),
    }
}

pub(super) fn before_after(
    combined: &[SplitRow],
    calibration: &[SplitRow],
    novelty: &[SplitRow],
    top_k: usize,
) -> BeforeAfterTopK {
    BeforeAfterTopK {
        schema_version: SCHEMA_VERSION,
        before_combined_original_top_k: combined.iter().take(top_k).map(slim).collect(),
        after_novelty_prioritized_top_k: novelty.iter().take(top_k).map(slim).collect(),
        calibration_known_positive_top_k: calibration.iter().take(top_k).map(slim).collect(),
    }
}

pub(super) fn manual_spot_checks(rows: &[SplitRow]) -> ManualSpotChecks {
    let checks = [
        (
            "tnf_psoriatic_calibration",
            &["tnf", "psoriatic arthritis"][..],
        ),
        ("cd4_hiv_calibration", &["cd4", "hiv infectious disease"]),
        (
            "civic_level_a_b_calibration",
            &["oncology-civic:11176", "oncology-civic:1958"],
        ),
        (
            "streptomycin_infectious_novelty_view",
            &["streptomycin", "klebsiella", "rhinoscleroma"],
        ),
        (
            "dpp4_schizophrenia_novelty_view",
            &["dpp4", "schizophrenia", "metformin"],
        ),
    ];
    ManualSpotChecks {
        schema_version: SCHEMA_VERSION,
        checks: checks
            .iter()
            .map(|(name, needles)| {
                let mut matches = rows
                    .iter()
                    .filter(|row| {
                        let text = text_blob(row);
                        needles.iter().any(|needle| text.contains(needle))
                    })
                    .collect::<Vec<_>>();
                matches.sort_by(|a, b| {
                    b.original_rank_percentile_within_domain
                        .total_cmp(&a.original_rank_percentile_within_domain)
                        .then(a.candidate_id.cmp(&b.candidate_id))
                });
                super::model::SpotCheck {
                    check: (*name).to_string(),
                    match_count: matches.len(),
                    examples: matches.into_iter().take(6).map(slim).collect(),
                }
            })
            .collect(),
    }
}

pub(super) fn persist_all(
    out_dir: &Path,
    scope: &InputScope,
    combined: &[SplitRow],
    calibration: &[SplitRow],
    novelty: &[SplitRow],
    top_k: usize,
) -> CliResult<PersistedSplit> {
    fs::create_dir_all(out_dir)?;
    let before_after = before_after(combined, calibration, novelty, top_k);
    let checks = manual_spot_checks(combined);
    let metrics = metrics(&scope.source_artifacts, combined, calibration, novelty);
    let mut artifacts = BTreeMap::new();
    write_json(out_dir, "input_scope.json", scope, &mut artifacts)?;
    write_jsonl(
        out_dir,
        "combined_original_ranked.jsonl",
        combined,
        &mut artifacts,
    )?;
    write_jsonl(
        out_dir,
        "calibration_known_positive_rows.jsonl",
        calibration,
        &mut artifacts,
    )?;
    write_jsonl(
        out_dir,
        "novelty_prioritized_research_leads.jsonl",
        novelty,
        &mut artifacts,
    )?;
    write_json(
        out_dir,
        "before_after_topk.json",
        &before_after,
        &mut artifacts,
    )?;
    write_json(out_dir, "manual_spot_checks.json", &checks, &mut artifacts)?;
    write_json(out_dir, "validation_metrics.json", &metrics, &mut artifacts)?;
    let assertions = assertions(combined, calibration, novelty, &before_after);
    let readback = PersistedReadback {
        schema_version: SCHEMA_VERSION,
        status: "ok".to_string(),
        root: out_dir.display().to_string(),
        artifacts,
        assertions,
    };
    let readback_bytes = serde_json::to_vec_pretty(&readback)
        .map_err(|err| CliError::runtime(format!("serialize split readback: {err}")))?;
    write_if_same(&out_dir.join("persisted_readback.json"), &readback_bytes)?;
    let manifest = json!({
        "schema_version": SCHEMA_VERSION,
        "status": "ok",
        "readback_path": out_dir.join("persisted_readback.json"),
        "readback_sha256": sha256_hex(&readback_bytes),
        "total_rows": combined.len(),
        "calibration_known_positive_rows": calibration.len(),
        "novelty_prioritized_rows": novelty.len(),
        "clinical_boundary": CLINICAL_BOUNDARY,
    });
    write_json(
        out_dir,
        "output_manifest.json",
        &manifest,
        &mut BTreeMap::new(),
    )?;
    Ok(PersistedSplit {
        root: out_dir.to_path_buf(),
        readback,
        readback_sha256: sha256_hex(&readback_bytes),
    })
}

fn assertions(
    combined: &[SplitRow],
    calibration: &[SplitRow],
    novelty: &[SplitRow],
    before_after: &BeforeAfterTopK,
) -> BTreeMap<String, Value> {
    BTreeMap::from([
        ("combined_rows_readback".to_string(), json!(combined.len())),
        (
            "calibration_rows_readback".to_string(),
            json!(calibration.len()),
        ),
        ("novelty_rows_readback".to_string(), json!(novelty.len())),
        (
            "total_rows_match".to_string(),
            json!(combined.len() == calibration.len() + novelty.len()),
        ),
        (
            "split_rows_sum_to_total".to_string(),
            json!(calibration.len() + novelty.len() == combined.len()),
        ),
        (
            "before_topk_count".to_string(),
            json!(before_after.before_combined_original_top_k.len()),
        ),
        (
            "after_topk_count".to_string(),
            json!(before_after.after_novelty_prioritized_top_k.len()),
        ),
        (
            "calibration_topk_count".to_string(),
            json!(before_after.calibration_known_positive_top_k.len()),
        ),
        (
            "top_after_not_calibration".to_string(),
            json!(
                novelty
                    .first()
                    .is_some_and(|row| !row.calibration_known_positive)
            ),
        ),
        (
            "calibration_rows_available".to_string(),
            json!(!calibration.is_empty()),
        ),
        (
            "novelty_rows_available".to_string(),
            json!(!novelty.is_empty()),
        ),
    ])
}

fn write_json<T: Serialize>(
    out_dir: &Path,
    name: &str,
    value: &T,
    artifacts: &mut BTreeMap<String, ArtifactReadback>,
) -> CliResult {
    let bytes = serde_json::to_vec_pretty(value)
        .map_err(|err| CliError::runtime(format!("serialize {name}: {err}")))?;
    write_artifact(out_dir, name, &bytes, artifacts)
}

fn write_jsonl<T: Serialize>(
    out_dir: &Path,
    name: &str,
    rows: &[T],
    artifacts: &mut BTreeMap<String, ArtifactReadback>,
) -> CliResult {
    let mut bytes = Vec::new();
    for row in rows {
        serde_json::to_writer(&mut bytes, row)
            .map_err(|err| CliError::runtime(format!("serialize {name} row: {err}")))?;
        bytes.push(b'\n');
    }
    write_artifact(out_dir, name, &bytes, artifacts)
}

fn write_artifact(
    out_dir: &Path,
    name: &str,
    bytes: &[u8],
    artifacts: &mut BTreeMap<String, ArtifactReadback>,
) -> CliResult {
    let path = out_dir.join(name);
    write_if_same(&path, bytes)?;
    let readback = fs::read(&path)?;
    if readback != bytes {
        return Err(CliError::runtime(format!(
            "novelty split artifact readback mismatch at {}",
            path.display()
        )));
    }
    artifacts.insert(
        format!("out/{name}"),
        ArtifactReadback {
            bytes: readback.len() as u64,
            sha256: sha256_hex(&readback),
        },
    );
    Ok(())
}

fn write_if_same(path: &Path, bytes: &[u8]) -> CliResult {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    if path.exists() {
        if fs::read(path)? != bytes {
            return Err(CliError::runtime(format!(
                "refusing to overwrite existing different novelty split artifact {}",
                path.display()
            )));
        }
        return Ok(());
    }
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path)?;
    Ok(())
}
