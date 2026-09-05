use super::super::{Subcommand, value};
use crate::error::{CliError, CliResult};
use std::net::{IpAddr, SocketAddr};

pub(super) const DEFAULT_K: usize = 10;
pub(crate) const DEFAULT_KERNEL_MAX_HOPS: usize = 32;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SearchArgs {
    pub vault: String,
    pub query: String,
    pub k: usize,
    pub fusion: SearchFusionArg,
    pub guard: SearchGuardArg,
    pub explain: bool,
    pub rerank: bool,
    pub provenance: bool,
    pub metadata: bool,
    pub freshness: SearchFreshnessArg,
    pub filter: Option<String>,
    pub resident_addr: Option<SocketAddr>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct KernelAnswerArgs {
    pub vault: String,
    pub query: String,
    pub anchor: Option<String>,
    pub explain: bool,
    pub resident_addr: Option<SocketAddr>,
    pub max_hops: usize,
    pub citation_target: Option<String>,
    pub citation_collection: String,
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
        rerank: false,
        provenance: true,
        metadata: false,
        freshness: SearchFreshnessArg::Fresh,
        filter: None,
        resident_addr: None,
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
            "--rerank" => args.rerank = true,
            "--provenance" => args.provenance = true,
            "--no-provenance" => args.provenance = false,
            "--metadata" => args.metadata = true,
            "--fresh" => set_freshness(&mut freshness_seen, &mut args, SearchFreshnessArg::Fresh)?,
            "--stale-ok" => {
                set_freshness(&mut freshness_seen, &mut args, SearchFreshnessArg::StaleOk)?
            }
            "--filter" => {
                idx += 1;
                let raw = value(rest, idx, "--filter")?.to_string();
                calyx_search::filters::parse(Some(&raw))?;
                args.filter = Some(raw);
            }
            "--resident-addr" => {
                idx += 1;
                args.resident_addr =
                    Some(parse_resident_addr(value(rest, idx, "--resident-addr")?)?);
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
    let mut resident_addr = None;
    let mut max_hops = DEFAULT_KERNEL_MAX_HOPS;
    let mut citation_target = None;
    let mut citation_collection = "legal-citations-v2".to_string();
    let mut idx = 2;
    while idx < rest.len() {
        match rest[idx].as_str() {
            "--anchor" => {
                idx += 1;
                anchor = Some(value(rest, idx, "--anchor")?.to_string());
            }
            "--explain" => explain = true,
            "--resident-addr" => {
                idx += 1;
                resident_addr = Some(parse_resident_addr(value(rest, idx, "--resident-addr")?)?);
            }
            "--max-hops" => {
                idx += 1;
                let raw = value(rest, idx, "--max-hops")?;
                max_hops = raw
                    .parse::<usize>()
                    .map_err(|error| CliError::usage(format!("parse --max-hops {raw}: {error}")))?;
                if max_hops == 0 || max_hops > DEFAULT_KERNEL_MAX_HOPS {
                    return Err(CliError::usage("--max-hops must be in 1..=32"));
                }
            }
            "--citation-target" => {
                idx += 1;
                let value = value(rest, idx, "--citation-target")?;
                if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
                    return Err(CliError::usage(
                        "--citation-target must be a CourtListener numeric opinion id",
                    ));
                }
                citation_target = Some(value.to_string());
            }
            "--citation-collection" => {
                idx += 1;
                citation_collection = value(rest, idx, "--citation-collection")?.to_string();
                if citation_collection.is_empty() {
                    return Err(CliError::usage("--citation-collection must not be empty"));
                }
            }
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
        resident_addr,
        max_hops,
        citation_target,
        citation_collection,
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

pub(crate) fn parse_resident_addr(raw: &str) -> CliResult<SocketAddr> {
    let addr = raw
        .parse::<SocketAddr>()
        .map_err(|error| CliError::usage(format!("parse --resident-addr {raw}: {error}")))?;
    match addr.ip() {
        IpAddr::V4(ip) if ip.is_loopback() => Ok(addr),
        IpAddr::V6(ip) if ip.is_loopback() => Ok(addr),
        _ => Err(CliError::from(calyx_core::CalyxError {
            code: "CALYX_SEARCH_RESIDENT_ADDR_REFUSED",
            message: format!("--resident-addr {addr} is not loopback"),
            remediation: "bind and use the resident measurement service only on 127.0.0.1 or [::1]",
        })),
    }
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
    if args.rerank {
        out.push("--rerank".to_string());
    }
    if !args.provenance {
        out.push("--no-provenance".to_string());
    }
    if args.metadata {
        out.push("--metadata".to_string());
    }
    if args.freshness == SearchFreshnessArg::StaleOk {
        out.push("--stale-ok".to_string());
    }
    if let Some(filter) = &args.filter {
        out.extend(["--filter".to_string(), filter.clone()]);
    }
    if let Some(addr) = args.resident_addr {
        out.extend(["--resident-addr".to_string(), addr.to_string()]);
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
    if let Some(addr) = args.resident_addr {
        out.extend(["--resident-addr".to_string(), addr.to_string()]);
    }
    if args.max_hops != DEFAULT_KERNEL_MAX_HOPS {
        out.extend(["--max-hops".to_string(), args.max_hops.to_string()]);
    }
    if let Some(target) = &args.citation_target {
        out.extend(["--citation-target".to_string(), target.clone()]);
        if args.citation_collection != "legal-citations-v2" {
            out.extend([
                "--citation-collection".to_string(),
                args.citation_collection.clone(),
            ]);
        }
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
