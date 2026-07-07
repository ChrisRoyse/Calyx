use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchemaDerivationRequest {
    pub corpus_root: PathBuf,
    pub output_root: PathBuf,
    pub required_sources: Vec<String>,
    pub required_join_keys: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchemaDerivationReport {
    pub schema_version: String,
    pub generated_at_unix_ms: u128,
    pub source_of_truth: String,
    pub corpus_root: String,
    pub output_root: String,
    pub manifest_path: String,
    pub readback_report_path: String,
    pub schema_contract_path: String,
    pub schema_decision_note_path: String,
    pub edge_audit_path: String,
    pub dataset_count: usize,
    pub field_count: usize,
    pub required_sources: Vec<String>,
    pub observed_sources: Vec<String>,
    pub missing_required_sources: Vec<String>,
    pub required_join_keys: Vec<String>,
    pub missing_required_join_keys: Vec<String>,
    pub nullable_or_union_field_count: usize,
    pub blocked_runtime_sources: Vec<SchemaBlockedRuntimeSource>,
    pub before_files: Vec<SchemaArtifactFileState>,
    pub after_files: Vec<SchemaArtifactFileState>,
    pub passed: bool,
    pub status_code: String,
    pub failure: Option<SchemaDerivationFailure>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchemaContract {
    pub schema_version: String,
    pub source_of_truth: String,
    pub corpus_root: String,
    pub raw_retention_rules: Vec<String>,
    pub dataset_contracts: Vec<SchemaDatasetContract>,
    pub field_contracts: Vec<SchemaFieldContract>,
    pub join_contract: SchemaJoinContract,
    pub derived_contracts: Vec<String>,
    pub blocked_runtime_sources: Vec<SchemaBlockedRuntimeSource>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchemaDatasetContract {
    pub dataset: String,
    pub source: String,
    pub record_count: usize,
    pub field_count: usize,
    pub storage_family: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchemaFieldContract {
    pub dataset: String,
    pub source: String,
    pub field: String,
    pub present_count: usize,
    pub missing_count: usize,
    pub null_count: usize,
    pub type_counts: BTreeMap<String, usize>,
    pub json_string_count: usize,
    pub roles: Vec<String>,
    pub variant_contract: String,
    pub example_sha256: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchemaJoinContract {
    pub record_count: usize,
    pub identifier_counts: BTreeMap<String, usize>,
    pub examples: BTreeMap<String, Vec<String>>,
    pub required_join_keys: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchemaBlockedRuntimeSource {
    pub source: String,
    pub issue: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchemaEdgeAudit {
    pub schema_version: String,
    pub source_of_truth: String,
    pub checks: Vec<SchemaEdgeCheck>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchemaEdgeCheck {
    pub name: String,
    pub before_state: String,
    pub action: String,
    pub after_state: String,
    pub expectation_met: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchemaArtifactFileState {
    pub path: String,
    pub exists: bool,
    pub bytes: u64,
    pub sha256: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchemaDerivationFailure {
    pub code: String,
    pub message: String,
}
