//! Token emitters for the probe/discovery/chain-walk subcommands, split out
//! of token_roundtrip.rs to satisfy the 500-line file gate.

use super::super::super::*;
use super::{anchor_kind_name, push_opt};

pub(super) fn probe_matrix_tokens(args: &probe_matrix::ProbeMatrixArgs) -> Vec<String> {
    let mut out = vec![
        "probe-matrix".to_string(),
        args.vault.clone(),
        "--frontier".to_string(),
        args.frontier.clone(),
    ];
    for slot in &args.slots {
        out.extend(["--slot".to_string(), slot.to_string()]);
    }
    for profile in &args.weighted_profiles {
        out.extend(["--weighted-profile".to_string(), rrf_profile_name(*profile)]);
    }
    for phrasing in &args.phrasings {
        out.extend(["--phrasing".to_string(), phrasing_name(*phrasing)]);
    }
    for length in &args.lengths {
        out.extend(["--length".to_string(), length_name(*length)]);
    }
    out.extend(["--top-k".to_string(), args.top_k.to_string()]);
    out.extend(["--guard".to_string(), guard_name(args.guard).to_string()]);
    if let Some(guard_tau) = args.guard_tau {
        out.extend(["--guard-tau".to_string(), guard_tau.to_string()]);
    }
    push_opt(
        &mut out,
        "--out",
        args.out.as_ref().and_then(|p| p.to_str()),
    );
    if let Some(addr) = args.resident_addr {
        out.extend(["--resident-addr".to_string(), addr.to_string()]);
    }
    if let Some(max_variants) = args.max_variants {
        out.extend(["--max-variants".to_string(), max_variants.to_string()]);
    }
    if let Some(time_budget_ms) = args.time_budget_ms {
        out.extend(["--time-budget-ms".to_string(), time_budget_ms.to_string()]);
    }
    out
}

pub(super) fn discovery_chain_tokens(args: &discovery_chain::DiscoveryChainArgs) -> Vec<String> {
    let mut out = vec!["discovery-chain".to_string(), args.vault.clone()];
    for start in &args.starts {
        out.extend(["--start".to_string(), start.to_string()]);
    }
    for anchor in &args.anchors {
        out.extend(["--anchor".to_string(), anchor.to_string()]);
    }
    for path in &args.anchor_files {
        push_opt(&mut out, "--anchor-file", path.to_str());
    }
    out.extend([
        "--max-hops".to_string(),
        args.max_hops.to_string(),
        "--branch-width".to_string(),
        args.branch_width.to_string(),
        "--probe-width".to_string(),
        args.probe_width.to_string(),
        "--max-groundedness-distance".to_string(),
        args.max_groundedness_distance.to_string(),
        "--min-gate-confidence".to_string(),
        args.min_gate_confidence.to_string(),
        "--novelty-weight".to_string(),
        args.novelty_weight.to_string(),
        "--assay-domain".to_string(),
        args.assay_domain.clone(),
        "--assay-anchor".to_string(),
        anchor_kind_name(&args.assay_anchor),
    ]);
    push_opt(
        &mut out,
        "--out",
        args.out.as_ref().and_then(|p| p.to_str()),
    );
    out
}

pub(super) fn chain_walks_tokens(args: &chain_walks::ChainWalksArgs) -> Vec<String> {
    let mut out = vec!["chain-walks".to_string(), args.vault.clone()];
    push_opt(&mut out, "--seed-file", args.seed_file.to_str());
    for anchor in &args.anchors {
        out.extend(["--anchor".to_string(), anchor.to_string()]);
    }
    for path in &args.anchor_files {
        push_opt(&mut out, "--anchor-file", path.to_str());
    }
    out.extend([
        "--max-hops".to_string(),
        args.max_hops.to_string(),
        "--branch-width".to_string(),
        args.branch_width.to_string(),
        "--probe-width".to_string(),
        args.probe_width.to_string(),
        "--max-groundedness-distance".to_string(),
        args.max_groundedness_distance.to_string(),
        "--min-gate-confidence".to_string(),
        args.min_gate_confidence.to_string(),
        "--novelty-weight".to_string(),
        args.novelty_weight.to_string(),
        "--max-hypotheses-per-seed".to_string(),
        args.max_hypotheses_per_seed.to_string(),
        "--min-terminal-confidence".to_string(),
        args.min_terminal_confidence.to_string(),
        "--assay-domain".to_string(),
        args.assay_domain.clone(),
        "--assay-anchor".to_string(),
        anchor_kind_name(&args.assay_anchor),
    ]);
    push_opt(
        &mut out,
        "--out",
        args.out.as_ref().and_then(|p| p.to_str()),
    );
    out
}

pub(super) fn graph_collection_generations_tokens(
    args: &graph_lifecycle::GraphCollectionGenerationsArgs,
) -> Vec<String> {
    let mut out = vec![
        "graph-collection-generations".to_string(),
        args.vault.clone(),
    ];
    push_opt(&mut out, "--collection", args.collection.as_deref());
    push_opt(
        &mut out,
        "--home",
        args.home.as_ref().and_then(|p| p.to_str()),
    );
    out
}

pub(super) fn graph_collection_state_tokens(
    args: &graph_lifecycle::GraphCollectionStateArgs,
) -> Vec<String> {
    let mut out = vec![
        "graph-collection-state".to_string(),
        args.vault.clone(),
        "--collection".to_string(),
        args.collection.clone(),
        "--generation".to_string(),
        args.generation.clone(),
        "--state".to_string(),
        args.state.as_str().to_string(),
        "--command".to_string(),
        args.command.clone(),
    ];
    push_opt(&mut out, "--reason", args.reason.as_deref());
    for (key, value) in &args.detail {
        out.extend(["--detail".to_string(), format!("{key}={value}")]);
    }
    push_opt(
        &mut out,
        "--home",
        args.home.as_ref().and_then(|p| p.to_str()),
    );
    out
}

fn rrf_profile_name(value: calyx_sextant::RrfProfile) -> String {
    format!("{value:?}").to_ascii_lowercase()
}

fn phrasing_name(value: calyx_lodestar::ProbePhrasing) -> String {
    format!("{value:?}").to_ascii_lowercase()
}

fn length_name(value: calyx_lodestar::ProbeLength) -> String {
    format!("{value:?}").to_ascii_lowercase()
}

fn guard_name(value: calyx_search::GuardChoice) -> &'static str {
    match value {
        calyx_search::GuardChoice::Off => "off",
        calyx_search::GuardChoice::InRegion => "in-region",
    }
}
