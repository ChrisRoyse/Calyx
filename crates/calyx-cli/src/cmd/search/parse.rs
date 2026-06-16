use calyx_core::SlotId;
use calyx_sextant::{FusionStrategy, RrfProfile};

use super::super::{Subcommand, value};
use crate::error::{CliError, CliResult};

#[cfg(test)]
use super::filters;
#[cfg(test)]
use calyx_sextant::{FreshnessRequirement, Query};

pub(super) const DEFAULT_K: usize = 10;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SearchArgs {
    pub vault: String,
    pub query: String,
    pub k: usize,
    pub fusion: SearchFusionArg,
    pub guard: SearchGuardArg,
    pub explain: bool,
    pub provenance: bool,
    pub freshness: SearchFreshnessArg,
    pub filter: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct KernelAnswerArgs {
    pub vault: String,
    pub query: String,
    pub anchor: Option<String>,
    pub explain: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SearchFusionArg {
    Rrf,
    WeightedRrf,
    SingleLens,
    KernelFirst,
    Pipeline,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SearchGuardArg {
    Off,
    InRegion,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SearchFreshnessArg {
    Fresh,
    StaleOk,
}

pub(crate) fn parse_search(rest: &[String]) -> CliResult<Subcommand> {
    if rest.len() < 2 {
        return Err(CliError::usage("search requires <vault> <query>"));
    }
    let vault = rest[0].clone();
    let query = rest[1].clone();
    validate_query_text(&query)?;
    let mut args = SearchArgs {
        vault,
        query,
        k: DEFAULT_K,
        fusion: SearchFusionArg::Rrf,
        guard: SearchGuardArg::Off,
        explain: false,
        provenance: true,
        freshness: SearchFreshnessArg::Fresh,
        filter: None,
    };
    let mut freshness_seen = None;
    let mut idx = 2;
    while idx < rest.len() {
        match rest[idx].as_str() {
            "--k" => {
                idx += 1;
                parse_k(&mut args, value(rest, idx, "--k")?)?;
            }
            "--fusion" => {
                idx += 1;
                args.fusion = SearchFusionArg::parse(value(rest, idx, "--fusion")?)?;
            }
            "--guard" => {
                idx += 1;
                args.guard = SearchGuardArg::parse(value(rest, idx, "--guard")?)?;
            }
            "--explain" => args.explain = true,
            "--provenance" => args.provenance = true,
            "--no-provenance" => args.provenance = false,
            "--fresh" => set_freshness(&mut freshness_seen, &mut args, SearchFreshnessArg::Fresh)?,
            "--stale-ok" => {
                set_freshness(&mut freshness_seen, &mut args, SearchFreshnessArg::StaleOk)?
            }
            "--filter" => {
                idx += 1;
                args.filter = Some(value(rest, idx, "--filter")?.to_string());
            }
            other => return Err(CliError::usage(format!("unexpected search flag {other}"))),
        }
        idx += 1;
    }
    Ok(Subcommand::Search(args))
}

pub(crate) fn parse_kernel_answer(rest: &[String]) -> CliResult<Subcommand> {
    if rest.len() < 2 {
        return Err(CliError::usage("kernel-answer requires <vault> <query>"));
    }
    let vault = rest[0].clone();
    let query = rest[1].clone();
    validate_query_text(&query)?;
    let mut anchor = None;
    let mut explain = false;
    let mut idx = 2;
    while idx < rest.len() {
        match rest[idx].as_str() {
            "--anchor" => {
                idx += 1;
                anchor = Some(value(rest, idx, "--anchor")?.to_string());
            }
            "--explain" => explain = true,
            other => {
                return Err(CliError::usage(format!(
                    "unexpected kernel-answer flag {other}"
                )));
            }
        }
        idx += 1;
    }
    Ok(Subcommand::KernelAnswer(KernelAnswerArgs {
        vault,
        query,
        anchor,
        explain,
    }))
}

fn parse_k(args: &mut SearchArgs, raw: &str) -> CliResult {
    args.k = raw
        .parse::<usize>()
        .map_err(|err| CliError::usage(format!("parse --k {raw}: {err}")))?;
    if args.k == 0 {
        return Err(CliError::usage("--k must be greater than zero"));
    }
    Ok(())
}

fn validate_query_text(value: &str) -> CliResult {
    if value.is_empty() {
        return Err(CliError::usage("<query> must not be empty"));
    }
    Ok(())
}

fn set_freshness(
    seen: &mut Option<SearchFreshnessArg>,
    args: &mut SearchArgs,
    value: SearchFreshnessArg,
) -> CliResult {
    if seen.replace(value).is_some() {
        return Err(CliError::usage("use only one of --fresh or --stale-ok"));
    }
    args.freshness = value;
    Ok(())
}

impl SearchFusionArg {
    fn parse(value: &str) -> CliResult<Self> {
        match value {
            "rrf" => Ok(Self::Rrf),
            "weighted-rrf" => Ok(Self::WeightedRrf),
            "single-lens" => Ok(Self::SingleLens),
            "kernel-first" => Ok(Self::KernelFirst),
            "pipeline" => Ok(Self::Pipeline),
            other => Err(CliError::usage(format!("unknown --fusion {other}"))),
        }
    }

    pub(super) fn to_strategy(self, slots: &[SlotId]) -> CliResult<FusionStrategy> {
        match self {
            Self::Rrf => Ok(FusionStrategy::Rrf),
            Self::WeightedRrf => Ok(FusionStrategy::WeightedRrf {
                profile: RrfProfile::General,
            }),
            Self::SingleLens => slots
                .first()
                .copied()
                .map(|slot| FusionStrategy::SingleLens { slot })
                .ok_or_else(|| CliError::usage("single-lens search has no active lens slot")),
            Self::KernelFirst => Ok(FusionStrategy::WeightedRrf {
                profile: RrfProfile::Kernel,
            }),
            Self::Pipeline => Ok(FusionStrategy::Pipeline),
        }
    }
}

impl SearchGuardArg {
    fn parse(value: &str) -> CliResult<Self> {
        match value {
            "off" => Ok(Self::Off),
            "in-region" => Ok(Self::InRegion),
            other => Err(CliError::usage(format!("unknown --guard {other}"))),
        }
    }
}

#[cfg(test)]
impl SearchArgs {
    pub(crate) fn to_query(&self, slots: &[SlotId]) -> CliResult<Query> {
        let mut query = Query::new(self.query.clone());
        query.k = self.k;
        query.explain = self.explain;
        query.require_stored_provenance = self.provenance;
        query.freshness = match self.freshness {
            SearchFreshnessArg::Fresh => FreshnessRequirement::FreshDerived,
            SearchFreshnessArg::StaleOk => FreshnessRequirement::StaleOk { seq_lag: u64::MAX },
        };
        query.fusion = Some(self.fusion.to_strategy(slots)?);
        query.filters = filters::parse(self.filter.as_deref())?;
        Ok(query)
    }
}

#[cfg(test)]
pub(crate) fn search_tokens(args: &SearchArgs) -> Vec<String> {
    let mut out = vec![
        "search".to_string(),
        args.vault.clone(),
        args.query.clone(),
        "--k".to_string(),
        args.k.to_string(),
        "--fusion".to_string(),
        args.fusion.flag_value().to_string(),
        "--guard".to_string(),
        args.guard.flag_value().to_string(),
    ];
    if args.explain {
        out.push("--explain".to_string());
    }
    if !args.provenance {
        out.push("--no-provenance".to_string());
    }
    if args.freshness == SearchFreshnessArg::StaleOk {
        out.push("--stale-ok".to_string());
    }
    if let Some(filter) = &args.filter {
        out.extend(["--filter".to_string(), filter.clone()]);
    }
    out
}

#[cfg(test)]
pub(crate) fn kernel_answer_tokens(args: &KernelAnswerArgs) -> Vec<String> {
    let mut out = vec![
        "kernel-answer".to_string(),
        args.vault.clone(),
        args.query.clone(),
    ];
    if let Some(anchor) = &args.anchor {
        out.extend(["--anchor".to_string(), anchor.clone()]);
    }
    if args.explain {
        out.push("--explain".to_string());
    }
    out
}

#[cfg(test)]
impl SearchFusionArg {
    fn flag_value(self) -> &'static str {
        match self {
            Self::Rrf => "rrf",
            Self::WeightedRrf => "weighted-rrf",
            Self::SingleLens => "single-lens",
            Self::KernelFirst => "kernel-first",
            Self::Pipeline => "pipeline",
        }
    }
}

#[cfg(test)]
impl SearchGuardArg {
    fn flag_value(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::InRegion => "in-region",
        }
    }
}
