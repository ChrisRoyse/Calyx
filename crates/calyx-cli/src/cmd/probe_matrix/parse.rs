use calyx_core::SlotId;
use calyx_lodestar::{ProbeLength, ProbePhrasing};
use calyx_search::GuardChoice;
use calyx_sextant::RrfProfile;

use super::ProbeMatrixArgs;
use crate::bounded_progress::parse_nonzero_u64;
use crate::cmd::search::parse_resident_addr;
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
            "--guard-tau" => {
                idx += 1;
                args.guard_tau = Some(parse_guard_tau(value(rest, idx, "--guard-tau")?)?);
            }
            "--out" => {
                idx += 1;
                args.out = Some(value(rest, idx, "--out")?.into());
            }
            "--resident-addr" => {
                idx += 1;
                args.resident_addr =
                    Some(parse_resident_addr(value(rest, idx, "--resident-addr")?)?);
            }
            "--max-variants" => {
                idx += 1;
                args.max_variants = Some(parse_usize(
                    value(rest, idx, "--max-variants")?,
                    "--max-variants",
                    1,
                )?);
            }
            "--time-budget-ms" => {
                idx += 1;
                args.time_budget_ms = Some(parse_nonzero_u64(
                    value(rest, idx, "--time-budget-ms")?,
                    "--time-budget-ms",
                )?);
            }
            "--search-miss-budget-ms" => {
                idx += 1;
                args.search_miss_budget_ms = Some(parse_nonzero_u64(
                    value(rest, idx, "--search-miss-budget-ms")?,
                    "--search-miss-budget-ms",
                )?);
            }
            "--search-hit-budget-ms" => {
                idx += 1;
                args.search_hit_budget_ms = Some(parse_nonzero_u64(
                    value(rest, idx, "--search-hit-budget-ms")?,
                    "--search-hit-budget-ms",
                )?);
            }
            other => {
                return Err(CliError::usage(format!(
                    "unexpected probe-matrix flag {other}"
                )));
            }
        }
        idx += 1;
    }
    if !frontier_seen {
        return Err(CliError::usage("probe-matrix requires --frontier <text>"));
    }
    if args.guard_tau.is_some() && args.guard != GuardChoice::InRegion {
        return Err(CliError::usage(
            "--guard-tau requires --guard in-region; the tau calibrates the in-region cosine threshold",
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

fn parse_guard_tau(raw: &str) -> CliResult<f32> {
    let tau = raw
        .parse::<f32>()
        .map_err(|err| CliError::usage(format!("parse --guard-tau {raw}: {err}")))?;
    if !tau.is_finite() || tau <= 0.0 || tau > 1.0 {
        return Err(CliError::usage(format!(
            "--guard-tau {raw} is out of range; supply a finite cosine threshold in (0.0, 1.0]"
        )));
    }
    Ok(tau)
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
