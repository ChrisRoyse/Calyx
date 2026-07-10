use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use calyx_core::Clock;
use serde::Deserialize;

use crate::raw_large_corpus::read_large_corpus_manifest;
use crate::raw_large_corpus_profile::{LargeCorpusFieldProfile, LargeCorpusJoinProfile};
use crate::raw_large_corpus_readback::readback_large_corpus;
use crate::raw_large_corpus_types::LargeCorpusManifest;
use crate::raw_source_support::{now_unix_ms, sha256_hex, write_json};
use crate::schema_derivation_classify::{
    blocked_runtime_sources, dataset_contracts, derived_contracts, field_contracts,
    raw_retention_rules,
};
pub use crate::schema_derivation_types::{
    SchemaArtifactFileState, SchemaBlockedRuntimeSource, SchemaContract, SchemaDatasetContract,
    SchemaDerivationFailure, SchemaDerivationReport, SchemaDerivationRequest, SchemaEdgeAudit,
    SchemaEdgeCheck, SchemaFieldContract, SchemaJoinContract,
};
use crate::{PolyError, Result};

pub const SCHEMA_DERIVATION_SCHEMA_VERSION: &str = "poly.schema_derivation.v1";
pub const SCHEMA_DERIVATION_PASSED: &str = "POLY_SCHEMA_DERIVATION_PASSED";

const DEFAULT_REQUIRED_SOURCES: &[&str] = &[
    "gamma",
    "clob",
    "data-api",
    "historical-dump",
    "polygon-rpc",
    "goldsky-subgraph",
    "websocket-market",
    "websocket-rtds",
    "websocket-sports",
];

const DEFAULT_REQUIRED_JOIN_KEYS: &[&str] = &[
    "condition_id",
    "question_id",
    "token_or_asset_id",
    "transaction_hash",
];

impl SchemaDerivationRequest {
    pub fn new(corpus_root: impl Into<PathBuf>, output_root: impl Into<PathBuf>) -> Self {
        Self {
            corpus_root: corpus_root.into(),
            output_root: output_root.into(),
            required_sources: DEFAULT_REQUIRED_SOURCES
                .iter()
                .map(|source| (*source).to_string())
                .collect(),
            required_join_keys: DEFAULT_REQUIRED_JOIN_KEYS
                .iter()
                .map(|key| (*key).to_string())
                .collect(),
        }
    }
}

pub fn run_schema_derivation(
    request: &SchemaDerivationRequest,
    clock: &dyn Clock,
) -> Result<SchemaDerivationReport> {
    fs::create_dir_all(&request.output_root).map_err(|err| {
        PolyError::raw_source(
            "POLY_SCHEMA_DERIVATION_OUTPUT_DIR_CREATE_FAILED",
            format!(
                "create schema derivation output dir {}: {err}",
                request.output_root.display()
            ),
        )
    })?;
    let corpus_root = canonicalize_path(
        &request.corpus_root,
        "POLY_SCHEMA_DERIVATION_CORPUS_ROOT_CANONICALIZE_FAILED",
    )?;
    let output_root = canonicalize_path(
        &request.output_root,
        "POLY_SCHEMA_DERIVATION_OUTPUT_ROOT_CANONICALIZE_FAILED",
    )?;
    let contract_path = output_root.join("schema-contract.json");
    let note_path = output_root.join("schema-decision.md");
    let edge_path = output_root.join("schema-edge-audit.json");
    let report_path = output_root.join("schema-derivation-report.json");
    let artifact_paths = vec![contract_path.clone(), note_path.clone(), edge_path.clone()];
    let before_files = file_states(&artifact_paths)?;

    let manifest_path = corpus_root.join("large-corpus-manifest.json");
    let manifest = read_large_corpus_manifest(&manifest_path)?;
    let readback = readback_large_corpus(&corpus_root)?;
    let profiles = read_field_profiles(&corpus_root, &manifest)?;
    let join_profile = read_join_profile(&corpus_root, &manifest.join_profile_path)?;
    let observed_sources = observed_sources(&manifest);
    let missing_required_sources = missing_values(&request.required_sources, &observed_sources);
    let missing_required_join_keys =
        missing_join_keys(&request.required_join_keys, &join_profile.identifier_counts);
    let blocked_runtime_sources = blocked_runtime_sources(&manifest);
    let field_contracts = field_contracts(&profiles);
    let nullable_or_union_field_count = field_contracts
        .iter()
        .filter(|field| field.variant_contract != "single_non_null_type")
        .count();
    let dataset_contracts = dataset_contracts(&profiles);
    let contract = SchemaContract {
        schema_version: "poly.schema_contract.v1".to_string(),
        source_of_truth: "large corpus field profiles, join profile, and raw readback files"
            .to_string(),
        corpus_root: corpus_root.display().to_string(),
        raw_retention_rules: raw_retention_rules(),
        dataset_contracts,
        field_contracts,
        join_contract: SchemaJoinContract {
            record_count: join_profile.record_count,
            identifier_counts: join_profile.identifier_counts.clone(),
            examples: join_profile.examples.clone(),
            required_join_keys: request.required_join_keys.clone(),
        },
        derived_contracts: derived_contracts(),
        blocked_runtime_sources: blocked_runtime_sources.clone(),
    };
    let edge_audit = edge_audit(
        &missing_required_sources,
        &missing_required_join_keys,
        nullable_or_union_field_count,
    );
    write_json(&contract_path, &contract)?;
    write_schema_note(&note_path, &contract, &manifest, &readback)?;
    write_json(&edge_path, &edge_audit)?;

    let failure = schema_failure(
        &readback.status_code,
        readback.passed,
        &missing_required_sources,
        &missing_required_join_keys,
    );
    let after_files = file_states(&artifact_paths)?;
    let report = SchemaDerivationReport {
        schema_version: SCHEMA_DERIVATION_SCHEMA_VERSION.to_string(),
        generated_at_unix_ms: now_unix_ms(clock),
        source_of_truth: "physical large-corpus artifacts read back from disk".to_string(),
        corpus_root: corpus_root.display().to_string(),
        output_root: output_root.display().to_string(),
        manifest_path: manifest_path.display().to_string(),
        readback_report_path: corpus_root
            .join("large-corpus-readback-report.json")
            .display()
            .to_string(),
        schema_contract_path: contract_path.display().to_string(),
        schema_decision_note_path: note_path.display().to_string(),
        edge_audit_path: edge_path.display().to_string(),
        dataset_count: profiles.len(),
        field_count: contract.field_contracts.len(),
        required_sources: request.required_sources.clone(),
        observed_sources,
        missing_required_sources,
        required_join_keys: request.required_join_keys.clone(),
        missing_required_join_keys,
        nullable_or_union_field_count,
        blocked_runtime_sources,
        before_files,
        after_files,
        passed: failure.is_none(),
        status_code: failure
            .as_ref()
            .map(|failure| failure.code.clone())
            .unwrap_or_else(|| SCHEMA_DERIVATION_PASSED.to_string()),
        failure,
    };
    write_json(&report_path, &report)?;
    Ok(report)
}

fn canonicalize_path(path: &Path, code: &str) -> Result<PathBuf> {
    fs::canonicalize(path).map_err(|err| {
        PolyError::raw_source(
            code,
            format!(
                "canonicalize schema derivation path {}: {err}",
                path.display()
            ),
        )
    })
}

pub fn read_schema_derivation_report(path: &Path) -> Result<SchemaDerivationReport> {
    let text = fs::read_to_string(path).map_err(|err| {
        PolyError::raw_source(
            "POLY_SCHEMA_DERIVATION_REPORT_READ_FAILED",
            format!("read schema derivation report {}: {err}", path.display()),
        )
    })?;
    serde_json::from_str(&text).map_err(|err| {
        PolyError::raw_source(
            "POLY_SCHEMA_DERIVATION_REPORT_PARSE_FAILED",
            format!("parse schema derivation report {}: {err}", path.display()),
        )
    })
}

pub fn require_schema_derivation_passed(report: &SchemaDerivationReport) -> Result<()> {
    if report.passed {
        return Ok(());
    }
    let failure = report
        .failure
        .as_ref()
        .map(|failure| format!("{}: {}", failure.code, failure.message))
        .unwrap_or_else(|| report.status_code.clone());
    Err(PolyError::raw_source(
        "POLY_SCHEMA_DERIVATION_REQUIRED_FAILED",
        format!("schema derivation did not pass: {failure}"),
    ))
}

fn read_field_profiles(
    root: &Path,
    manifest: &LargeCorpusManifest,
) -> Result<Vec<LargeCorpusFieldProfile>> {
    manifest
        .field_profile_paths
        .iter()
        .map(|path| read_json(&resolve_path(root, path), "POLY_SCHEMA_FIELD_PROFILE"))
        .collect()
}

fn read_join_profile(root: &Path, path: &str) -> Result<LargeCorpusJoinProfile> {
    read_json(&resolve_path(root, path), "POLY_SCHEMA_JOIN_PROFILE")
}

fn read_json<T>(path: &Path, code_prefix: &str) -> Result<T>
where
    T: for<'de> Deserialize<'de>,
{
    let text = fs::read_to_string(path).map_err(|err| {
        PolyError::raw_source(
            format!("{code_prefix}_READ_FAILED"),
            format!("read {}: {err}", path.display()),
        )
    })?;
    serde_json::from_str(&text).map_err(|err| {
        PolyError::raw_source(
            format!("{code_prefix}_PARSE_FAILED"),
            format!("parse {}: {err}", path.display()),
        )
    })
}

fn resolve_path(root: &Path, path: &str) -> PathBuf {
    let parsed = PathBuf::from(path);
    if parsed.is_absolute() {
        parsed
    } else {
        root.join(parsed)
    }
}

fn observed_sources(manifest: &LargeCorpusManifest) -> Vec<String> {
    let mut sources = BTreeSet::new();
    for page in &manifest.pages {
        sources.insert(page.source.clone());
    }
    sources.into_iter().collect()
}

fn missing_values(required: &[String], observed: &[String]) -> Vec<String> {
    let observed: BTreeSet<&str> = observed.iter().map(String::as_str).collect();
    required
        .iter()
        .filter(|value| !observed.contains(value.as_str()))
        .cloned()
        .collect()
}

fn missing_join_keys(required: &[String], observed: &BTreeMap<String, usize>) -> Vec<String> {
    required
        .iter()
        .filter(|key| observed.get(*key).copied().unwrap_or_default() == 0)
        .cloned()
        .collect()
}

fn schema_failure(
    readback_status: &str,
    readback_passed: bool,
    missing_sources: &[String],
    missing_join_keys: &[String],
) -> Option<SchemaDerivationFailure> {
    if !readback_passed {
        return Some(SchemaDerivationFailure {
            code: "POLY_SCHEMA_DERIVATION_CORPUS_READBACK_FAILED".to_string(),
            message: format!("large corpus readback did not pass: {readback_status}"),
        });
    }
    if !missing_sources.is_empty() {
        return Some(SchemaDerivationFailure {
            code: "POLY_SCHEMA_DERIVATION_SOURCE_MISSING".to_string(),
            message: format!("missing required sources: {}", missing_sources.join(", ")),
        });
    }
    if !missing_join_keys.is_empty() {
        return Some(SchemaDerivationFailure {
            code: "POLY_SCHEMA_DERIVATION_JOIN_KEY_MISSING".to_string(),
            message: format!(
                "missing required join keys: {}",
                missing_join_keys.join(", ")
            ),
        });
    }
    None
}

fn edge_audit(
    missing_sources: &[String],
    missing_join_keys: &[String],
    nullable_or_union_field_count: usize,
) -> SchemaEdgeAudit {
    SchemaEdgeAudit {
        schema_version: "poly.schema_derivation.edge_audit.v1".to_string(),
        source_of_truth: "schema derivation validator state read from physical corpus artifacts"
            .to_string(),
        checks: vec![
            SchemaEdgeCheck {
                name: "missing_required_source".to_string(),
                before_state: "required source list compared with manifest page sources"
                    .to_string(),
                action: "fail closed if any required source is absent".to_string(),
                after_state: format!("missing_sources={}", missing_sources.len()),
                expectation_met: missing_sources.is_empty(),
            },
            SchemaEdgeCheck {
                name: "conflicting_or_nullable_field_type".to_string(),
                before_state: "field profile type_counts/null/missing counters read from disk"
                    .to_string(),
                action: "record union/nullable contract instead of collapsing to one type"
                    .to_string(),
                after_state: format!("nullable_or_union_fields={nullable_or_union_field_count}"),
                expectation_met: nullable_or_union_field_count > 0,
            },
            SchemaEdgeCheck {
                name: "missing_required_join_key".to_string(),
                before_state: "required join keys compared with join-profile identifier counts"
                    .to_string(),
                action: "fail closed if a required join key is absent".to_string(),
                after_state: format!("missing_join_keys={}", missing_join_keys.len()),
                expectation_met: missing_join_keys.is_empty(),
            },
        ],
    }
}

fn write_schema_note(
    path: &Path,
    contract: &SchemaContract,
    manifest: &LargeCorpusManifest,
    readback: &crate::LargeCorpusReadbackReport,
) -> Result<()> {
    let mut text = String::new();
    text.push_str("# Poly Schema Decision\n\n");
    text.push_str(
        "Source of truth: persisted large-corpus bytes and independent readback artifacts.\n\n",
    );
    text.push_str("## Corpus Readback\n\n");
    text.push_str(&format!("- Status: `{}`\n", readback.status_code));
    text.push_str(&format!("- Pages: `{}`\n", readback.total_pages));
    text.push_str(&format!("- Records: `{}`\n", readback.total_records));
    text.push_str(&format!("- Body bytes: `{}`\n", readback.total_body_bytes));
    text.push_str(&format!(
        "- Checked files: `{}`\n",
        readback.checked_file_count
    ));
    text.push_str("\n## Durable Rules\n\n");
    for rule in &contract.raw_retention_rules {
        text.push_str(&format!("- {rule}\n"));
    }
    text.push_str("\n## Dataset Families\n\n");
    for dataset in &contract.dataset_contracts {
        text.push_str(&format!(
            "- `{}` (`{}`): `{}` records, `{}` fields, storage `{}`.\n",
            dataset.dataset,
            dataset.source,
            dataset.record_count,
            dataset.field_count,
            dataset.storage_family
        ));
    }
    text.push_str("\n## Join Keys\n\n");
    for (key, count) in &contract.join_contract.identifier_counts {
        text.push_str(&format!("- `{key}`: `{count}` observations.\n"));
    }
    text.push_str("\n## Blocked Runtime Sources\n\n");
    if contract.blocked_runtime_sources.is_empty() {
        text.push_str("- None.\n");
    } else {
        for source in &contract.blocked_runtime_sources {
            text.push_str(&format!(
                "- `{}` remains blocked by {}: {}.\n",
                source.source, source.issue, source.reason
            ));
        }
    }
    text.push_str("\n## Schema Lock Rule\n\n");
    text.push_str("Do not create docs-inferred RTDS equity payload tables. Model only payload families with real persisted bytes; keep #179 as blocked-runtime until reality produces `equity_prices` frames.\n");
    text.push_str(&format!(
        "\nManifest status at derivation time: `{}`.\n",
        manifest.status_code
    ));
    fs::write(path, text).map_err(|err| {
        PolyError::raw_source(
            "POLY_SCHEMA_DERIVATION_NOTE_WRITE_FAILED",
            format!("write schema decision note {}: {err}", path.display()),
        )
    })
}

fn file_states(paths: &[PathBuf]) -> Result<Vec<SchemaArtifactFileState>> {
    paths.iter().map(|path| file_state(path)).collect()
}

fn file_state(path: &Path) -> Result<SchemaArtifactFileState> {
    if !path.exists() {
        return Ok(SchemaArtifactFileState {
            path: path.display().to_string(),
            exists: false,
            bytes: 0,
            sha256: None,
        });
    }
    let bytes = fs::read(path).map_err(|err| {
        PolyError::raw_source(
            "POLY_SCHEMA_DERIVATION_FILE_READ_FAILED",
            format!("read schema artifact {}: {err}", path.display()),
        )
    })?;
    Ok(SchemaArtifactFileState {
        path: path.display().to_string(),
        exists: true,
        bytes: bytes.len() as u64,
        sha256: Some(sha256_hex(&bytes)),
    })
}
