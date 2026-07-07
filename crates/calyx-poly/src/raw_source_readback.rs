//! Physical readback verifier for raw-source inventory artifacts.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::raw_source_support::{sha256_hex, write_json};
use crate::raw_sources::{RawEndpointSample, read_raw_source_inventory};
use crate::{PolyError, Result};

pub const RAW_SOURCE_READBACK_SCHEMA_VERSION: &str = "poly.raw_source_readback.v1";
pub const RAW_SOURCE_READBACK_FILE: &str = "raw-source-readback-report.json";
pub const RAW_SOURCE_READBACK_PASSED: &str = "POLY_RAW_SOURCE_READBACK_PASSED";
pub const ERR_RAW_SOURCE_READBACK_FAILED: &str = "POLY_RAW_SOURCE_READBACK_FAILED";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawSourceReadbackFile {
    pub sample_name: String,
    pub role: String,
    pub path: String,
    pub exists: bool,
    pub expected_exists: bool,
    pub bytes: u64,
    pub expected_bytes: Option<u64>,
    pub sha256: Option<String>,
    pub expected_sha256: Option<String>,
    pub passed: bool,
    pub failure_code: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawSourceReadbackReport {
    pub schema_version: String,
    pub source_of_truth: String,
    pub inventory_path: String,
    pub sample_count: usize,
    pub checked_file_count: usize,
    pub failure_count: usize,
    pub files: Vec<RawSourceReadbackFile>,
    pub passed: bool,
    pub status_code: String,
}

pub fn readback_raw_source_inventory(root: &Path) -> Result<RawSourceReadbackReport> {
    let inventory_path = root.join("source-inventory.json");
    let inventory = read_raw_source_inventory(&inventory_path)?;
    let mut files = Vec::new();
    for sample in &inventory.samples {
        files.push(check_metadata(sample)?);
        files.push(check_body(sample)?);
        if sample.request_body_exists {
            files.push(check_request(sample)?);
        }
    }
    let failure_count = files.iter().filter(|file| !file.passed).count();
    let report = RawSourceReadbackReport {
        schema_version: RAW_SOURCE_READBACK_SCHEMA_VERSION.to_string(),
        source_of_truth: "physical raw body/request/metadata files listed by source-inventory.json"
            .to_string(),
        inventory_path: inventory_path.display().to_string(),
        sample_count: inventory.samples.len(),
        checked_file_count: files.len(),
        failure_count,
        files,
        passed: failure_count == 0,
        status_code: if failure_count == 0 {
            RAW_SOURCE_READBACK_PASSED.to_string()
        } else {
            ERR_RAW_SOURCE_READBACK_FAILED.to_string()
        },
    };
    write_json(&root.join(RAW_SOURCE_READBACK_FILE), &report)?;
    Ok(report)
}

pub fn require_raw_source_readback_passed(report: &RawSourceReadbackReport) -> Result<()> {
    if report.passed {
        return Ok(());
    }
    let first = report
        .files
        .iter()
        .find(|file| !file.passed)
        .map(|file| format!("{} {} failed", file.sample_name, file.role))
        .unwrap_or_else(|| "raw-source readback failed".to_string());
    Err(PolyError::raw_source(report.status_code.clone(), first))
}

fn check_metadata(sample: &RawEndpointSample) -> Result<RawSourceReadbackFile> {
    let path = PathBuf::from(&sample.metadata_path);
    let bytes = fs::read(&path).map_err(|err| {
        PolyError::raw_source(
            "POLY_RAW_SOURCE_METADATA_READBACK_FAILED",
            format!("read metadata {}: {err}", path.display()),
        )
    })?;
    let parsed = serde_json::from_slice::<RawEndpointSample>(&bytes).map_err(|err| {
        PolyError::raw_source(
            "POLY_RAW_SOURCE_METADATA_DECODE_FAILED",
            format!("decode metadata {}: {err}", path.display()),
        )
    })?;
    let passed = &parsed == sample;
    Ok(RawSourceReadbackFile {
        sample_name: sample.name.clone(),
        role: "metadata".to_string(),
        path: path.display().to_string(),
        exists: true,
        expected_exists: true,
        bytes: bytes.len() as u64,
        expected_bytes: None,
        sha256: Some(sha256_hex(&bytes)),
        expected_sha256: None,
        passed,
        failure_code: (!passed).then(|| "POLY_RAW_SOURCE_METADATA_MISMATCH".to_string()),
    })
}

fn check_body(sample: &RawEndpointSample) -> Result<RawSourceReadbackFile> {
    check_file(
        sample,
        "body",
        &sample.body_path,
        sample.body_exists,
        sample.body_bytes,
        sample.body_sha256.clone(),
    )
}

fn check_request(sample: &RawEndpointSample) -> Result<RawSourceReadbackFile> {
    let path = sample.request_body_path.as_deref().ok_or_else(|| {
        PolyError::raw_source(
            "POLY_RAW_SOURCE_REQUEST_PATH_MISSING",
            format!(
                "sample {} has request_body_exists without path",
                sample.name
            ),
        )
    })?;
    check_file(
        sample,
        "request",
        path,
        true,
        sample.request_body_bytes,
        sample.request_body_sha256.clone(),
    )
}

fn check_file(
    sample: &RawEndpointSample,
    role: &str,
    path: &str,
    expected_exists: bool,
    expected_bytes: u64,
    expected_sha256: Option<String>,
) -> Result<RawSourceReadbackFile> {
    let path = PathBuf::from(path);
    let exists = path.exists();
    let (bytes, sha256) = if exists {
        let data = fs::read(&path).map_err(|err| {
            PolyError::raw_source(
                "POLY_RAW_SOURCE_FILE_READBACK_FAILED",
                format!("read {role} {}: {err}", path.display()),
            )
        })?;
        (data.len() as u64, Some(sha256_hex(&data)))
    } else {
        (0, None)
    };
    let passed = exists == expected_exists
        && (!expected_exists
            || (bytes == expected_bytes
                && normalize_sha(&sha256) == normalize_sha(&expected_sha256)));
    Ok(RawSourceReadbackFile {
        sample_name: sample.name.clone(),
        role: role.to_string(),
        path: path.display().to_string(),
        exists,
        expected_exists,
        bytes,
        expected_bytes: expected_exists.then_some(expected_bytes),
        sha256,
        expected_sha256,
        passed,
        failure_code: (!passed).then(|| "POLY_RAW_SOURCE_FILE_MISMATCH".to_string()),
    })
}

fn normalize_sha(value: &Option<String>) -> Option<String> {
    value.as_ref().map(|sha| sha.to_ascii_lowercase())
}
