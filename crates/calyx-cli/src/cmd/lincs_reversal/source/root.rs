use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::FAMILY;
use crate::cmd::lincs_reversal::model::LincsGraphDraft;
use crate::error::{CliError, CliResult};

#[derive(Clone, Debug, Serialize)]
pub(crate) struct VerifiedRootReport {
    pub family: String,
    pub root: String,
    pub manifest: String,
    pub file_count: usize,
    pub verified_file_count: usize,
    pub self_hash_skipped: Vec<String>,
    pub total_bytes: u64,
    pub aggregate_sha256: String,
}

#[derive(Clone, Debug)]
pub(super) struct ArtifactInfo {
    pub(super) stable_key: String,
    pub(super) rel: String,
    pub(super) sha256: String,
    pub(super) bytes: u64,
}

#[derive(Clone, Debug)]
pub(super) struct RootIndex {
    pub(super) root: PathBuf,
    pub(super) root_key: String,
    pub(super) artifacts: BTreeMap<String, ArtifactInfo>,
}

pub(super) fn verify_root(
    root: &Path,
    draft: &mut LincsGraphDraft,
) -> CliResult<(RootIndex, VerifiedRootReport)> {
    if !root.is_dir() {
        return Err(CliError::runtime(format!(
            "LINCS/CMap FSV root is not a directory: {}",
            root.display()
        )));
    }
    let manifest_rel = "persisted_readback.json";
    let manifest_path = root.join(manifest_rel);
    let manifest = read_json_file(&manifest_path)?;
    let files = manifest
        .get("files")
        .and_then(Value::as_object)
        .ok_or_else(|| CliError::runtime("persisted_readback.json missing object field files"))?;
    let root_key = draft.add_node(
        format!("fsv_root:{FAMILY}:{}", root.display()),
        "fsv_root",
        root.display().to_string(),
        BTreeMap::from([
            ("family".to_string(), FAMILY.to_string()),
            ("path".to_string(), root.display().to_string()),
        ]),
    );
    let mut index = RootIndex {
        root: root.to_path_buf(),
        root_key,
        artifacts: BTreeMap::new(),
    };
    let mut verified_file_count = 0usize;
    let mut self_hash_skipped = Vec::new();
    let mut total_bytes = 0u64;
    let mut aggregate = Sha256::new();
    for (rel, expected) in files {
        let rel = normalize_rel(rel);
        let bytes = fs::read(root.join(&rel))?;
        let actual_sha = sha256_hex(&bytes);
        let actual_len = bytes.len() as u64;
        let is_self_manifest = rel == manifest_rel;
        let expected_len = expected.get("bytes").and_then(Value::as_u64);
        if !is_self_manifest && expected_len.is_some_and(|value| value != actual_len) {
            return Err(CliError::runtime(format!(
                "LINCS artifact byte mismatch for {rel}: expected {:?} read {actual_len}",
                expected_len
            )));
        }
        let expected_sha = expected.get("sha256").and_then(Value::as_str);
        if is_self_manifest {
            self_hash_skipped.push(rel.clone());
        } else if expected_sha.is_some_and(|value| value != actual_sha) {
            return Err(CliError::runtime(format!(
                "LINCS artifact sha256 mismatch for {rel}: expected {:?} read {actual_sha}",
                expected_sha
            )));
        }
        verified_file_count += 1;
        total_bytes += actual_len;
        aggregate.update(rel.as_bytes());
        aggregate.update(actual_sha.as_bytes());
        add_artifact_node(draft, &mut index, &rel, &actual_sha, actual_len);
    }
    Ok((
        index,
        VerifiedRootReport {
            family: FAMILY.to_string(),
            root: root.display().to_string(),
            manifest: manifest_path.display().to_string(),
            file_count: files.len(),
            verified_file_count,
            self_hash_skipped,
            total_bytes,
            aggregate_sha256: format!("{:x}", aggregate.finalize()),
        },
    ))
}

pub(super) fn add_source_nodes(index: &RootIndex, draft: &mut LincsGraphDraft) {
    let screen = draft.add_node(
        "source:lincs_cmap_reversal_screen",
        "source",
        "LINCS/CMap reversal screen",
        BTreeMap::from([
            ("family".to_string(), FAMILY.to_string()),
            (
                "boundary".to_string(),
                "lead_signal_only_not_clinical_actionability".to_string(),
            ),
        ]),
    );
    let creeds = draft.add_node(
        "source:creeds",
        "source",
        "CREEDS disease signatures",
        BTreeMap::from([
            (
                "url".to_string(),
                "https://maayanlab.cloud/CREEDS/".to_string(),
            ),
            (
                "role".to_string(),
                "disease_expression_signatures".to_string(),
            ),
        ]),
    );
    let l1000 = draft.add_node(
        "source:l1000cds2",
        "source",
        "L1000CDS2 reverse perturbation search",
        BTreeMap::from([
            (
                "url".to_string(),
                "https://maayanlab.cloud/L1000CDS2/".to_string(),
            ),
            (
                "role".to_string(),
                "perturbation_reversal_scores".to_string(),
            ),
        ]),
    );
    draft.add_edge(&screen, "has_fsv_root", &index.root_key, BTreeMap::new());
    draft.add_edge(&screen, "uses_source", &creeds, BTreeMap::new());
    draft.add_edge(&screen, "uses_source", &l1000, BTreeMap::new());
}

fn add_artifact_node(
    draft: &mut LincsGraphDraft,
    index: &mut RootIndex,
    rel: &str,
    actual_sha: &str,
    actual_len: u64,
) {
    let artifact_key = draft.add_node(
        format!("fsv_artifact:{FAMILY}:{rel}"),
        "fsv_artifact",
        rel,
        BTreeMap::from([
            ("family".to_string(), FAMILY.to_string()),
            ("relative_path".to_string(), rel.to_string()),
            ("sha256".to_string(), actual_sha.to_string()),
            ("bytes".to_string(), actual_len.to_string()),
        ]),
    );
    let hash_key = draft.add_node(
        format!("hash:sha256:{actual_sha}"),
        "hash",
        actual_sha,
        BTreeMap::from([("algorithm".to_string(), "sha256".to_string())]),
    );
    draft.add_edge(
        &index.root_key,
        "contains_artifact",
        &artifact_key,
        BTreeMap::new(),
    );
    draft.add_edge(&artifact_key, "has_hash", &hash_key, BTreeMap::new());
    index.artifacts.insert(
        rel.to_string(),
        ArtifactInfo {
            stable_key: artifact_key,
            rel: rel.to_string(),
            sha256: actual_sha.to_string(),
            bytes: actual_len,
        },
    );
}

pub(super) fn link_artifact_field(
    index: &RootIndex,
    draft: &mut LincsGraphDraft,
    from_key: &str,
    value: &Value,
    field: &str,
    edge_type: &str,
) -> CliResult {
    let Some(path) = super::rows::str_field(value, field) else {
        return Ok(());
    };
    let rel = manifest_rel_from_path(index, &path);
    link_artifact(index, draft, from_key, &rel, edge_type)
}

pub(super) fn link_artifact(
    index: &RootIndex,
    draft: &mut LincsGraphDraft,
    from_key: &str,
    rel: &str,
    edge_type: &str,
) -> CliResult {
    let artifact = artifact(index, rel)?;
    draft.add_edge(
        from_key,
        edge_type,
        &artifact.stable_key,
        artifact_meta(artifact),
    );
    Ok(())
}

pub(super) fn artifact<'a>(index: &'a RootIndex, rel: &str) -> CliResult<&'a ArtifactInfo> {
    let rel = normalize_rel(rel);
    index.artifacts.get(&rel).ok_or_else(|| {
        CliError::runtime(format!(
            "LINCS referenced artifact {rel} is missing from persisted readback manifest"
        ))
    })
}

pub(super) fn artifact_meta(artifact: &ArtifactInfo) -> BTreeMap<String, String> {
    BTreeMap::from([
        ("relative_path".to_string(), artifact.rel.clone()),
        ("sha256".to_string(), artifact.sha256.clone()),
        ("bytes".to_string(), artifact.bytes.to_string()),
    ])
}

fn manifest_rel_from_path(index: &RootIndex, path: &str) -> String {
    let normalized = normalize_rel(path);
    let root = normalize_rel(&index.root.display().to_string());
    if let Some(stripped) = normalized.strip_prefix(&format!("{root}/")) {
        stripped.to_string()
    } else if let Some(position) = normalized.find("/raw/") {
        normalized[position + 1..].to_string()
    } else if let Some(position) = normalized.find("/parsed/") {
        normalized[position + 1..].to_string()
    } else {
        normalized
    }
}

pub(super) fn read_json_file(path: &Path) -> CliResult<Value> {
    let bytes = fs::read(path)?;
    serde_json::from_slice(&bytes)
        .map_err(|error| CliError::runtime(format!("parse {} as JSON: {error}", path.display())))
}

pub(super) fn normalize_rel(rel: &str) -> String {
    rel.replace('\\', "/").trim_start_matches("./").to_string()
}

pub(super) fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
