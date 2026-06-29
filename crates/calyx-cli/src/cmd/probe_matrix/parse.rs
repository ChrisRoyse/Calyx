use calyx_core::SlotId;
use calyx_lodestar::{ProbeLength, ProbePhrasing};
use calyx_search::GuardChoice;
use calyx_sextant::RrfProfile;

use super::ProbeMatrixArgs;
use crate::cmd::{Subcommand, value};
use crate::error::{CliError, CliResult};

pub(crate) fn parse_probe_matrix(rest: &[String]) -> CliResult<Subcommand> {
    let vault = rest
        .first()
        .ok_or_else(|| CliError::usage("probe-matrix requires <vault>"))?
        .clone();
    let mut args = ProbeMatrixArgs {
        vault,
        ..ProbeMatrixArgs::default()
    };
    let mut frontier_seen = false;
    let mut idx = 1;
    while idx < rest.len() {
        match rest[idx].as_str() {
            "--frontier" => {
                idx += 1;
                args.frontier = value(rest, idx, "--frontier")?.to_string();
                frontier_seen = true;
            }
            "--slot" => {
                idx += 1;
                args.slots.push(parse_slot(value(rest, idx, "--slot")?)?);
            }
            "--weighted-profile" => {
                idx += 1;
                args.weighted_profiles.push(parse_rrf_profile(value(
                    rest,
                    idx,
                    "--weighted-profile",
                )?)?);
            }
            "--phrasing" => {
                idx += 1;
                args.phrasings
                    .push(parse_phrasing(value(rest, idx, "--phrasing")?)?);
            }
            "--length" => {
                idx += 1;
                args.lengths
                    .push(parse_length(value(rest, idx, "--length")?)?);
            }
            "--top-k" => {
                idx += 1;
                args.top_k = parse_usize(value(rest, idx, "--top-k")?, "--top-k", 1)?;
            }
            "--guard" => {
                idx += 1;
                args.guard = parse_guard(value(rest, idx, "--guard")?)?;
            }
            "--out" => {
                idx += 1;
                args.out = Some(value(rest, idx, "--out")?.into());
            }
            other => {
                return Err(CliError::usage(format!(
                    "unexpected probe-matrix flag {other}"
                )));
            }
        }
        idx += 1;
    }
    if !frontier_seen || args.frontier.trim().is_empty() {
        return Err(CliError::usage(
            "probe-matrix requires non-empty --frontier <text>",
        ));
    }
    dedupe_sorted(&mut args.slots);
    dedupe_sorted(&mut args.weighted_profiles);
    dedupe_sorted(&mut args.phrasings);
    dedupe_sorted(&mut args.lengths);
    Ok(Subcommand::ProbeMatrix(args))
}

fn parse_slot(raw: &str) -> CliResult<SlotId> {
    let value = parse_usize(raw, "--slot", 0)?;
    let value = u16::try_from(value)
        .map_err(|_| CliError::usage(format!("--slot {raw} exceeds u16 range")))?;
    Ok(SlotId::new(value))
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

fn parse_guard(raw: &str) -> CliResult<GuardChoice> {
    match raw {
        "off" => Ok(GuardChoice::Off),
        "in-region" => Ok(GuardChoice::InRegion),
        other => Err(CliError::usage(format!(
            "unknown --guard {other}; use off or in-region"
        ))),
    }
}

fn parse_phrasing(raw: &str) -> CliResult<ProbePhrasing> {
    match normalized(raw).as_str() {
        "terse" => Ok(ProbePhrasing::Terse),
        "clinical" => Ok(ProbePhrasing::Clinical),
        "mechanistic" => Ok(ProbePhrasing::Mechanistic),
        "analogical" => Ok(ProbePhrasing::Analogical),
        "contrast" => Ok(ProbePhrasing::Contrast),
        other => Err(CliError::usage(format!("unknown --phrasing {other}"))),
    }
}

fn parse_length(raw: &str) -> CliResult<ProbeLength> {
    match normalized(raw).as_str() {
        "entity" => Ok(ProbeLength::Entity),
        "phrase" => Ok(ProbeLength::Phrase),
        "paragraph" => Ok(ProbeLength::Paragraph),
        other => Err(CliError::usage(format!("unknown --length {other}"))),
    }
}

fn parse_rrf_profile(raw: &str) -> CliResult<RrfProfile> {
    match normalized(raw).as_str() {
        "causal" => Ok(RrfProfile::Causal),
        "code" => Ok(RrfProfile::Code),
        "entity" => Ok(RrfProfile::Entity),
        "temporal" => Ok(RrfProfile::Temporal),
        "speaker" => Ok(RrfProfile::Speaker),
        "style" => Ok(RrfProfile::Style),
        "civic" => Ok(RrfProfile::Civic),
        "media" => Ok(RrfProfile::Media),
        "bridge" => Ok(RrfProfile::Bridge),
        "kernel" => Ok(RrfProfile::Kernel),
        "semantic" => Ok(RrfProfile::Semantic),
        "lexical" => Ok(RrfProfile::Lexical),
        "multimodal" => Ok(RrfProfile::Multimodal),
        "general" => Ok(RrfProfile::General),
        other => Err(CliError::usage(format!(
            "unknown --weighted-profile {other}"
        ))),
    }
}

fn normalized(raw: &str) -> String {
    raw.trim().to_ascii_lowercase().replace('_', "-")
}

fn dedupe_sorted<T: Ord>(values: &mut Vec<T>) {
    values.sort();
    values.dedup();
}
