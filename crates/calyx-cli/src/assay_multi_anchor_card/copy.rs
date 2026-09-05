use std::path::PathBuf;

use serde::Serialize;

use crate::a37_admission_store::{self, A37AdmissionDbReadback};

use super::readback::load_report;

const DEFAULT_ASSOCIATION_KEY: &str = "a37_multi_anchor_admission";
const CODE_INVALID_CONFIG: &str = "CALYX_FSV_ASSAY_MULTI_ANCHOR_COPY_INVALID_CONFIG";
const CODE_MISMATCH: &str = "CALYX_FSV_A37_ADMISSION_DB_MISMATCH";

#[derive(Debug)]
struct Request {
    from_cf_root: PathBuf,
    to_cf_root: PathBuf,
    from_association_key: String,
    to_association_key: String,
}

#[derive(Serialize)]
struct CopyReport {
    schema: &'static str,
    source: A37AdmissionDbReadback,
    target: A37AdmissionDbReadback,
    source_gate_passed: bool,
    target_gate_passed: bool,
    lens_count: usize,
    passing_lens_count: usize,
    no_collapse_pass: bool,
    stored_value_hash_match: bool,
}

pub(super) fn run(args: &[String]) -> Result<(), String> {
    let request = Request::parse(args)?;
    if request.from_cf_root == request.to_cf_root
        && request.from_association_key == request.to_association_key
    {
        return Err(format!(
            "{CODE_INVALID_CONFIG}: source and target A37 rows must be distinct"
        ));
    }

    let (source_report, source_readback) =
        load_report(&request.from_cf_root, &request.from_association_key)?;
    let target_write = a37_admission_store::write(
        &request.to_cf_root,
        &request.to_association_key,
        &source_report,
    )
    .map_err(|error| format!("{}: {}", error.code, error.message))?;
    let (target_report, target_readback) =
        load_report(&request.to_cf_root, &request.to_association_key)?;

    let stored_value_hash_match = source_readback.value_sha256 == target_write.value_sha256
        && target_write.value_sha256 == target_readback.value_sha256;
    if !stored_value_hash_match
        || source_report.lens_count != target_report.lens_count
        || source_report.passing_lens_count != target_report.passing_lens_count
        || source_report.no_collapse_pass != target_report.no_collapse_pass
    {
        return Err(format!(
            "{CODE_MISMATCH}: source and target A37 Graph CF readbacks differ"
        ));
    }

    let output = CopyReport {
        schema: "calyx-a37-multi-anchor-db-copy-v1",
        source: source_readback,
        target: target_readback,
        source_gate_passed: source_report.gate_passed,
        target_gate_passed: target_report.gate_passed,
        lens_count: target_report.lens_count,
        passing_lens_count: target_report.passing_lens_count,
        no_collapse_pass: target_report.no_collapse_pass,
        stored_value_hash_match,
    };
    println!(
        "{}",
        serde_json::to_string(&output)
            .map_err(|error| format!("{CODE_MISMATCH}: serialize copy report: {error}"))?
    );
    Ok(())
}

impl Request {
    fn parse(args: &[String]) -> Result<Self, String> {
        let mut from_cf_root = None;
        let mut to_cf_root = None;
        let mut from_association_key = DEFAULT_ASSOCIATION_KEY.to_string();
        let mut to_association_key = DEFAULT_ASSOCIATION_KEY.to_string();
        let mut idx = 0;
        while idx < args.len() {
            match args[idx].as_str() {
                "--from-cf-root" => {
                    from_cf_root = Some(PathBuf::from(value(args, idx, "--from-cf-root")?));
                    idx += 2;
                }
                "--to-cf-root" => {
                    to_cf_root = Some(PathBuf::from(value(args, idx, "--to-cf-root")?));
                    idx += 2;
                }
                "--from-association-key" => {
                    from_association_key = value(args, idx, "--from-association-key")?.to_string();
                    idx += 2;
                }
                "--to-association-key" => {
                    to_association_key = value(args, idx, "--to-association-key")?.to_string();
                    idx += 2;
                }
                other => {
                    return Err(format!("{CODE_INVALID_CONFIG}: unknown arg {other}"));
                }
            }
        }
        let from_cf_root = from_cf_root
            .ok_or_else(|| format!("{CODE_INVALID_CONFIG}: --from-cf-root is required"))?;
        let to_cf_root =
            to_cf_root.ok_or_else(|| format!("{CODE_INVALID_CONFIG}: --to-cf-root is required"))?;
        if from_association_key.trim().is_empty() || to_association_key.trim().is_empty() {
            return Err(format!(
                "{CODE_INVALID_CONFIG}: association keys must be non-empty"
            ));
        }
        Ok(Self {
            from_cf_root,
            to_cf_root,
            from_association_key,
            to_association_key,
        })
    }
}

fn value<'a>(args: &'a [String], idx: usize, flag: &str) -> Result<&'a str, String> {
    args.get(idx + 1)
        .map(String::as_str)
        .ok_or_else(|| format!("{CODE_INVALID_CONFIG}: {flag} requires a value"))
}
