use super::WeaveLoomArgs;
use crate::bounded_progress::parse_nonzero_u64;
use crate::cmd::{Subcommand, value};
use crate::error::{CliError, CliResult};

pub(crate) fn parse_weave_loom(rest: &[String]) -> CliResult<Subcommand> {
    let vault = rest
        .first()
        .ok_or_else(|| CliError::usage("weave-loom requires <vault>"))?
        .clone();
    let mut args = WeaveLoomArgs {
        vault,
        ..WeaveLoomArgs::default()
    };
    let mut idx = 1;
    while idx < rest.len() {
        match rest[idx].as_str() {
            "--content-slot" => {
                idx += 1;
                args.content_slot = Some(parse_u16(value(rest, idx, "--content-slot")?)?);
            }
            "--knn" => {
                idx += 1;
                args.knn = parse_usize(value(rest, idx, "--knn")?, "--knn", 1)?;
            }
            "--edge-cos-threshold" => {
                idx += 1;
                args.edge_cos_threshold =
                    parse_threshold(value(rest, idx, "--edge-cos-threshold")?)?;
            }
            "--max-groundedness-distance" => {
                idx += 1;
                args.max_groundedness_distance = parse_usize(
                    value(rest, idx, "--max-groundedness-distance")?,
                    "--max-groundedness-distance",
                    1,
                )?;
            }
            "--batch" => {
                idx += 1;
                args.batch = parse_usize(value(rest, idx, "--batch")?, "--batch", 1)?;
            }
            "--limit" => {
                idx += 1;
                args.limit = parse_usize(value(rest, idx, "--limit")?, "--limit", 0)?;
            }
            "--time-budget-ms" => {
                idx += 1;
                args.time_budget_ms = Some(parse_nonzero_u64(
                    value(rest, idx, "--time-budget-ms")?,
                    "--time-budget-ms",
                )?);
            }
            other => {
                return Err(CliError::usage(format!(
                    "unexpected weave-loom flag {other}"
                )));
            }
        }
        idx += 1;
    }
    Ok(Subcommand::WeaveLoom(args))
}

fn parse_u16(raw: &str) -> CliResult<u16> {
    raw.parse::<u16>()
        .map_err(|err| CliError::usage(format!("parse u16 {raw}: {err}")))
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

fn parse_threshold(raw: &str) -> CliResult<f32> {
    let value = raw
        .parse::<f32>()
        .map_err(|err| CliError::usage(format!("parse --edge-cos-threshold {raw}: {err}")))?;
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        return Err(CliError::usage(
            "--edge-cos-threshold must be finite and in [0,1]",
        ));
    }
    Ok(value)
}
