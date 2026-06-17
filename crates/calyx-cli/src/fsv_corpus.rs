use std::collections::BTreeSet;
use std::fs;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::CliResult;
use crate::output::print_json;

const SCHEMA: &str = "calyx.multimodal_fsv_corpus.v1";
const MIN_EDGE_CASES: usize = 3;

pub(crate) fn run(args: &[String]) -> CliResult {
    let root = parse_root(args)?;
    let readback = readback(&root)?;
    print_json(&readback)
}

fn parse_root(args: &[String]) -> Result<PathBuf, String> {
    match args {
        [flag, root] if flag == "--root" => Ok(PathBuf::from(root)),
        _ => Err("usage: calyx fsv corpus-readback --root <dir>".to_string()),
    }
}

fn readback(root: &Path) -> Result<CorpusReadback, String> {
    if !root.is_dir() {
        return Err(format!("CALYX_FSV_CORPUS_NOT_FOUND: {}", root.display()));
    }
    let manifest_path = root.join("manifest.json");
    if !manifest_path.is_file() {
        return Err(format!(
            "CALYX_FSV_CORPUS_MANIFEST_NOT_FOUND: {}",
            manifest_path.display()
        ));
    }
    let manifest_bytes = read_file(&manifest_path)?;
    let manifest_sha256 = sha256_hex(&manifest_bytes);
    let manifest: CorpusManifest = serde_json::from_slice(&manifest_bytes)
        .map_err(|error| format!("CALYX_FSV_CORPUS_INVALID_MANIFEST: {error}"))?;
    validate_manifest_shape(&manifest)?;

    let source_ids = validate_sources(&manifest.sources)?;
    let mut lane_readbacks = Vec::with_capacity(manifest.lanes.len());
    let mut total_samples = 0_usize;
    for lane in &manifest.lanes {
        lane_readbacks.push(readback_lane(root, lane, &source_ids)?);
        total_samples = total_samples.saturating_add(lane.sample_count);
    }

    if manifest.edge_cases.len() < MIN_EDGE_CASES {
        return Err(format!(
            "CALYX_FSV_CORPUS_EDGE_CASES_MISSING: need >={MIN_EDGE_CASES}, got {}",
            manifest.edge_cases.len()
        ));
    }
    let mut edge_readbacks = Vec::with_capacity(manifest.edge_cases.len());
    for edge in &manifest.edge_cases {
        edge_readbacks.push(readback_edge_case(root, edge)?);
    }

    Ok(CorpusReadback {
        source_of_truth: "multimodal FSV corpus manifest and listed files on disk".to_string(),
        root: root.display().to_string(),
        manifest_path: manifest_path.display().to_string(),
        manifest_bytes: manifest_bytes.len() as u64,
        manifest_sha256,
        schema: manifest.schema,
        bundle_id: manifest.bundle_id,
        sources: manifest.sources,
        lanes: lane_readbacks,
        edge_cases: edge_readbacks,
        total_samples,
    })
}

fn validate_manifest_shape(manifest: &CorpusManifest) -> Result<(), String> {
    if manifest.schema != SCHEMA {
        return Err(format!(
            "CALYX_FSV_CORPUS_INVALID_MANIFEST: schema must be {SCHEMA}, got {}",
            manifest.schema
        ));
    }
    if manifest.bundle_id.trim().is_empty() {
        return Err("CALYX_FSV_CORPUS_INVALID_MANIFEST: bundle_id is empty".to_string());
    }
    if manifest.sources.is_empty() {
        return Err("CALYX_FSV_CORPUS_INVALID_MANIFEST: sources is empty".to_string());
    }
    if manifest.lanes.is_empty() {
        return Err("CALYX_FSV_CORPUS_INVALID_MANIFEST: lanes is empty".to_string());
    }
    Ok(())
}

fn validate_sources(sources: &[CorpusSource]) -> Result<BTreeSet<String>, String> {
    let mut ids = BTreeSet::new();
    for source in sources {
        if source.id.trim().is_empty()
            || source.url.trim().is_empty()
            || source.license.trim().is_empty()
            || source.revision.trim().is_empty()
        {
            return Err(
                "CALYX_FSV_CORPUS_INVALID_MANIFEST: every source needs id/url/license/revision"
                    .to_string(),
            );
        }
        if !ids.insert(source.id.clone()) {
            return Err(format!(
                "CALYX_FSV_CORPUS_INVALID_MANIFEST: duplicate source id {}",
                source.id
            ));
        }
    }
    Ok(ids)
}

fn readback_lane(
    root: &Path,
    lane: &CorpusLane,
    source_ids: &BTreeSet<String>,
) -> Result<LaneReadback, String> {
    if lane.name.trim().is_empty()
        || lane.modality.trim().is_empty()
        || lane.source.trim().is_empty()
    {
        return Err(
            "CALYX_FSV_CORPUS_INVALID_MANIFEST: lane needs name/modality/source".to_string(),
        );
    }
    if !source_ids.contains(&lane.source) {
        return Err(format!(
            "CALYX_FSV_CORPUS_INVALID_MANIFEST: lane {} references unknown source {}",
            lane.name, lane.source
        ));
    }
    if lane.sample_count == 0 {
        return Err(format!(
            "CALYX_FSV_CORPUS_INVALID_MANIFEST: lane {} sample_count must be > 0",
            lane.name
        ));
    }
    if lane.files.is_empty() {
        return Err(format!(
            "CALYX_FSV_CORPUS_INVALID_MANIFEST: lane {} has no files",
            lane.name
        ));
    }
    let mut files = Vec::with_capacity(lane.files.len());
    for file in &lane.files {
        files.push(readback_file(root, file)?);
    }
    Ok(LaneReadback {
        name: lane.name.clone(),
        modality: lane.modality.clone(),
        source: lane.source.clone(),
        sample_count: lane.sample_count,
        expected_labels: lane.expected_labels.clone(),
        files,
    })
}

fn readback_edge_case(root: &Path, edge: &CorpusEdgeCase) -> Result<EdgeCaseReadback, String> {
    if edge.name.trim().is_empty()
        || edge.lane.trim().is_empty()
        || edge.expected_error.trim().is_empty()
    {
        return Err(
            "CALYX_FSV_CORPUS_INVALID_MANIFEST: edge case needs name/lane/expected_error"
                .to_string(),
        );
    }
    if !edge.expected_error.starts_with("CALYX_") {
        return Err(format!(
            "CALYX_FSV_CORPUS_INVALID_MANIFEST: edge {} expected_error must be CALYX_*",
            edge.name
        ));
    }
    let file = ManifestFile {
        path: edge.path.clone(),
        role: "edge_case".to_string(),
        sha256: edge.sha256.clone(),
        bytes: edge.bytes,
        rows: edge.rows,
    };
    Ok(EdgeCaseReadback {
        name: edge.name.clone(),
        lane: edge.lane.clone(),
        expected_error: edge.expected_error.clone(),
        file: readback_file(root, &file)?,
    })
}

fn readback_file(root: &Path, file: &ManifestFile) -> Result<FileReadback, String> {
    if file.path.trim().is_empty() || file.role.trim().is_empty() {
        return Err("CALYX_FSV_CORPUS_INVALID_MANIFEST: file path/role is empty".to_string());
    }
    if file.sha256.len() != 64 || !file.sha256.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Err(format!(
            "CALYX_FSV_CORPUS_INVALID_MANIFEST: {} sha256 must be 64 hex chars",
            file.path
        ));
    }
    let full_path = safe_join(root, &file.path)?;
    if fs::symlink_metadata(&full_path)
        .map_err(|error| {
            format!(
                "CALYX_FSV_CORPUS_FILE_READ_FAILED: {}: {error}",
                full_path.display()
            )
        })?
        .file_type()
        .is_symlink()
    {
        return Err(format!(
            "CALYX_FSV_CORPUS_SYMLINK_REJECTED: {}",
            full_path.display()
        ));
    }
    if !full_path.is_file() {
        return Err(format!(
            "CALYX_FSV_CORPUS_FILE_NOT_FOUND: {}",
            full_path.display()
        ));
    }
    let bytes = read_file(&full_path)?;
    let byte_count = bytes.len() as u64;
    if byte_count != file.bytes {
        return Err(format!(
            "CALYX_FSV_CORPUS_BYTES_MISMATCH: {} expected={} got={}",
            file.path, file.bytes, byte_count
        ));
    }
    let sha256 = sha256_hex(&bytes);
    if !sha256.eq_ignore_ascii_case(&file.sha256) {
        return Err(format!(
            "CALYX_FSV_CORPUS_SHA_MISMATCH: {} expected={} got={}",
            file.path, file.sha256, sha256
        ));
    }
    let rows = count_nonempty_lines(&bytes);
    if let Some(expected) = file.rows
        && rows != expected
    {
        return Err(format!(
            "CALYX_FSV_CORPUS_ROWS_MISMATCH: {} expected={} got={}",
            file.path, expected, rows
        ));
    }
    Ok(FileReadback {
        path: file.path.clone(),
        role: file.role.clone(),
        bytes: byte_count,
        sha256,
        rows: file.rows.map(|_| rows),
    })
}

fn safe_join(root: &Path, rel: &str) -> Result<PathBuf, String> {
    let rel_path = Path::new(rel);
    if rel_path.is_absolute() {
        return Err(format!("CALYX_FSV_CORPUS_PATH_ESCAPE: {rel}"));
    }
    for component in rel_path.components() {
        match component {
            Component::Normal(_) | Component::CurDir => {}
            _ => return Err(format!("CALYX_FSV_CORPUS_PATH_ESCAPE: {rel}")),
        }
    }
    Ok(root.join(rel_path))
}

fn read_file(path: &Path) -> Result<Vec<u8>, String> {
    fs::read(path).map_err(|error| {
        format!(
            "CALYX_FSV_CORPUS_FILE_READ_FAILED: {}: {error}",
            path.display()
        )
    })
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex_lower(&Sha256::digest(bytes))
}

fn hex_lower(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(hex_digit(byte >> 4));
        out.push(hex_digit(byte & 0x0f));
    }
    out
}

fn hex_digit(value: u8) -> char {
    match value {
        0..=9 => char::from(b'0' + value),
        10..=15 => char::from(b'a' + value - 10),
        _ => unreachable!("nibble"),
    }
}

fn count_nonempty_lines(bytes: &[u8]) -> usize {
    String::from_utf8_lossy(bytes)
        .lines()
        .filter(|line| !line.trim().is_empty())
        .count()
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CorpusManifest {
    schema: String,
    bundle_id: String,
    sources: Vec<CorpusSource>,
    lanes: Vec<CorpusLane>,
    #[serde(default)]
    edge_cases: Vec<CorpusEdgeCase>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct CorpusSource {
    id: String,
    url: String,
    license: String,
    revision: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CorpusLane {
    name: String,
    modality: String,
    source: String,
    sample_count: usize,
    #[serde(default)]
    expected_labels: Vec<String>,
    files: Vec<ManifestFile>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestFile {
    path: String,
    role: String,
    sha256: String,
    bytes: u64,
    #[serde(default)]
    rows: Option<usize>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CorpusEdgeCase {
    name: String,
    lane: String,
    path: String,
    expected_error: String,
    sha256: String,
    bytes: u64,
    #[serde(default)]
    rows: Option<usize>,
}

#[derive(Clone, Debug, Serialize)]
struct CorpusReadback {
    source_of_truth: String,
    root: String,
    manifest_path: String,
    manifest_bytes: u64,
    manifest_sha256: String,
    schema: String,
    bundle_id: String,
    sources: Vec<CorpusSource>,
    lanes: Vec<LaneReadback>,
    edge_cases: Vec<EdgeCaseReadback>,
    total_samples: usize,
}

#[derive(Clone, Debug, Serialize)]
struct LaneReadback {
    name: String,
    modality: String,
    source: String,
    sample_count: usize,
    expected_labels: Vec<String>,
    files: Vec<FileReadback>,
}

#[derive(Clone, Debug, Serialize)]
struct EdgeCaseReadback {
    name: String,
    lane: String,
    expected_error: String,
    file: FileReadback,
}

#[derive(Clone, Debug, Serialize)]
struct FileReadback {
    path: String,
    role: String,
    bytes: u64,
    sha256: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    rows: Option<usize>,
}

#[cfg(test)]
#[path = "fsv_corpus_tests.rs"]
mod fsv_corpus_tests;
