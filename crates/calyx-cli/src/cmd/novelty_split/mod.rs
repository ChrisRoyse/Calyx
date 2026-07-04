//! `calyx novelty-calibration-split` -- split disease-hunt atlas rows into
//! calibration proof rows and novelty-prioritized research leads (#1227).

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::artifact_hash::sha256_hex;
use super::discovery_run_preflight::{
    PreflightInput, RUN_MANIFEST_FLAG, RUN_STAGE_ID_FLAG, preflight_input_files,
};
use super::value;
use crate::error::{CliError, CliResult};
use crate::output::print_json;

mod model;
mod persist;
mod row_access;
mod scoring;

use model::{AtlasInputArg, COMMAND, LoadedAtlas, NoveltySplitArgs, SourceArtifact};

pub(crate) fn try_run(args: &[String]) -> Option<CliResult> {
    let (command, rest) = args.split_first()?;
    if command != COMMAND {
        return None;
    }
    if matches!(rest, [flag] if flag == "--help" || flag == "-h") {
        return Some(crate::usage::print_command_usage(command));
    }
    Some(parse_novelty_split(rest).and_then(run_novelty_split))
}

pub(crate) fn run_novelty_split(args: NoveltySplitArgs) -> CliResult {
    let loaded = load_atlases(&args.atlases)?;
    let preflight_inputs = loaded
        .iter()
        .map(|atlas| PreflightInput::new(&atlas.arg.path, &atlas.bytes))
        .collect::<Vec<_>>();
    let preflight = preflight_input_files(&args.preflight, &preflight_inputs)?;
    let source_artifacts = loaded
        .iter()
        .map(|atlas| SourceArtifact {
            issue: atlas.arg.issue.clone(),
            domain: atlas.arg.domain.clone(),
            path: atlas.arg.path.display().to_string(),
            bytes: atlas.bytes.len() as u64,
            sha256: atlas.sha256.clone(),
            row_count: atlas.rows.len(),
        })
        .collect::<Vec<_>>();
    let rows = scoring::build_split_rows(&loaded)?;
    let combined = scoring::combined_original(rows);
    let (calibration, novelty) = scoring::split_views(&combined);
    let scope = persist::input_scope(source_artifacts, COMMAND);
    let persisted = persist::persist_all(
        &args.out_dir,
        &scope,
        &combined,
        &calibration,
        &novelty,
        args.top_k,
    )?;
    print_json(&serde_json::json!({
        "status": "ok",
        "command": COMMAND,
        "out_dir": persisted.root,
        "readback": args.out_dir.join("persisted_readback.json"),
        "readback_sha256": persisted.readback_sha256,
        "combined_rows": combined.len(),
        "calibration_known_positive_rows": calibration.len(),
        "novelty_prioritized_rows": novelty.len(),
        "preflight": preflight,
        "assertions": persisted.readback.assertions,
    }))
}

fn parse_novelty_split(rest: &[String]) -> CliResult<NoveltySplitArgs> {
    let mut args = NoveltySplitArgs {
        top_k: 25,
        ..NoveltySplitArgs::default()
    };
    let mut idx = 0;
    while idx < rest.len() {
        match rest[idx].as_str() {
            "--atlas" => {
                idx += 1;
                args.atlases
                    .push(parse_atlas_arg(value(rest, idx, "--atlas")?)?);
            }
            "--out-dir" => {
                idx += 1;
                args.out_dir = PathBuf::from(value(rest, idx, "--out-dir")?);
            }
            "--top-k" => {
                idx += 1;
                args.top_k = parse_usize(value(rest, idx, "--top-k")?, "--top-k", 1)?;
            }
            RUN_MANIFEST_FLAG => {
                idx += 1;
                args.preflight.manifest = Some(PathBuf::from(value(rest, idx, RUN_MANIFEST_FLAG)?));
            }
            RUN_STAGE_ID_FLAG => {
                idx += 1;
                args.preflight.stage_id = Some(value(rest, idx, RUN_STAGE_ID_FLAG)?.to_string());
            }
            other => {
                return Err(CliError::usage(format!(
                    "unexpected {COMMAND} flag {other}"
                )));
            }
        }
        idx += 1;
    }
    if args.atlases.is_empty() {
        return Err(CliError::usage(format!(
            "{COMMAND} requires at least one --atlas <issue>|<domain>|<jsonl>"
        )));
    }
    if args.out_dir.as_os_str().is_empty() {
        return Err(CliError::usage(format!(
            "{COMMAND} requires --out-dir <dir>"
        )));
    }
    args.preflight.validate_for_command(COMMAND)?;
    Ok(args)
}

fn parse_atlas_arg(raw: &str) -> CliResult<AtlasInputArg> {
    let mut parts = raw.splitn(3, '|');
    let issue = parts.next().unwrap_or_default();
    let domain = parts.next().unwrap_or_default();
    let path = parts.next().unwrap_or_default();
    if issue.is_empty() || domain.is_empty() || path.is_empty() {
        return Err(CliError::usage(format!(
            "--atlas must be <issue>|<domain>|<jsonl>, got {raw}"
        )));
    }
    Ok(AtlasInputArg {
        issue: issue.to_string(),
        domain: domain.to_string(),
        path: PathBuf::from(path),
    })
}

fn load_atlases(args: &[AtlasInputArg]) -> CliResult<Vec<LoadedAtlas>> {
    args.iter()
        .map(|arg| {
            let bytes = fs::read(&arg.path).map_err(|err| {
                CliError::io(format!("read --atlas {}: {err}", arg.path.display()))
            })?;
            let rows = parse_jsonl(&arg.path, &bytes)?;
            if rows.is_empty() {
                return Err(CliError::usage(format!(
                    "--atlas {} did not contain any rows",
                    arg.path.display()
                )));
            }
            Ok(LoadedAtlas {
                arg: arg.clone(),
                sha256: sha256_hex(&bytes),
                bytes,
                rows,
            })
        })
        .collect()
}

fn parse_jsonl(path: &Path, bytes: &[u8]) -> CliResult<Vec<Value>> {
    let text = std::str::from_utf8(bytes)
        .map_err(|err| CliError::runtime(format!("decode {} as UTF-8: {err}", path.display())))?;
    let mut rows = Vec::new();
    for (index, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let row = serde_json::from_str::<Value>(line).map_err(|err| {
            CliError::runtime(format!(
                "parse JSONL row {} in {}: {err}",
                index + 1,
                path.display()
            ))
        })?;
        rows.push(row);
    }
    Ok(rows)
}

fn parse_usize(raw: &str, flag: &str, min: usize) -> CliResult<usize> {
    let value = raw
        .parse::<usize>()
        .map_err(|err| CliError::usage(format!("parse {flag} {raw}: {err}")))?;
    if value < min {
        return Err(CliError::usage(format!("{flag} must be >= {min}")));
    }
    Ok(value)
}

#[cfg(test)]
mod tests;
