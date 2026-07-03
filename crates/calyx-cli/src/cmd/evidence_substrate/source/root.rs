use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::SourceLoadReport;
use crate::cmd::evidence_substrate::model::EvidenceGraphDraft;
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
pub(in crate::cmd::evidence_substrate) struct RootIndex {
    pub(super) family: String,
    pub(super) root: PathBuf,
    pub(super) root_key: String,
    pub(super) artifacts: BTreeMap<String, ArtifactInfo>,
}

pub(super) fn verify_root(
    family: &str,
    root: &Path,
    draft: &mut EvidenceGraphDraft,
    report: &mut SourceLoadReport,
) -> CliResult<RootIndex> {
    if !root.is_dir() {
        return Err(CliError::runtime(format!(
            "{family} FSV root is not a directory: {}",
            root.display()
        )));
    }
    let manifest_rel = "persisted_readback.json";
    let manifest_path = root.join(manifest_rel);
    let manifest = read_json_file(&manifest_path)?;
    let files = manifest
        .get("files")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            CliError::runtime(format!(
                "{family} persisted_readback.json missing object field files"
            ))
        })?;
    let source_key = draft.add_node(
        format!("source_family:{family}"),
        "source",
        family,
        BTreeMap::from([("family".to_string(), family.to_string())]),
    );
    let root_key = draft.add_node(
        format!("fsv_root:{family}:{}", root.display()),
        "fsv_root",
        root.display().to_string(),
        BTreeMap::from([
            ("family".to_string(), family.to_string()),
            ("path".to_string(), root.display().to_string()),
        ]),
    );
    draft.add_edge(&source_key, "has_fsv_root", &root_key, BTreeMap::new());
    let mut index = RootIndex {
        family: family.to_string(),
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
                "{family} artifact byte mismatch for {rel}: expected {:?} read {actual_len}",
                expected_len
            )));
        }
        let expected_sha = expected.get("sha256").and_then(Value::as_str);
        if is_self_manifest {
            self_hash_skipped.push(rel.clone());
        } else if expected_sha.is_some_and(|value| value != actual_sha) {
            return Err(CliError::runtime(format!(
                "{family} artifact sha256 mismatch for {rel}: expected {:?} read {actual_sha}",
                expected_sha
            )));
        }
        verified_file_count += 1;
        total_bytes += actual_len;
        aggregate.update(rel.as_bytes());
        aggregate.update(actual_sha.as_bytes());
        add_artifact_node(draft, family, &mut index, &rel, &actual_sha, actual_len);
    }
    report.roots.push(VerifiedRootReport {
        family: family.to_string(),
        root: root.display().to_string(),
        manifest: manifest_path.display().to_string(),
        file_count: files.len(),
        verified_file_count,
        self_hash_skipped,
        total_bytes,
        aggregate_sha256: format!("{:x}", aggregate.finalize()),
    });
    Ok(index)
}

fn add_artifact_node(
    draft: &mut EvidenceGraphDraft,
    family: &str,
    index: &mut RootIndex,
    rel: &str,
    actual_sha: &str,
    actual_len: u64,
) {
    let artifact_key = draft.add_node(
        format!("fsv_artifact:{family}:{rel}"),
        "fsv_artifact",
        rel,
        BTreeMap::from([
            ("family".to_string(), family.to_string()),
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
