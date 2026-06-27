//! CLI glue for `search` / `kernel-answer`: resolve + open the vault, delegate
//! the real search to the shared `calyx-search` crate, then render the CLI JSON.
//! All search logic (index load, recall, fusion, provenance, guard) lives in
//! `calyx-search` so the CLI and `calyx-web-api` share ONE path (#573).

use std::collections::BTreeMap;

use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{AnchorKind, CalyxError, Constellation, CxId};
use calyx_registry::load_vault_panel_state;
use calyx_search::{FusionChoice, GuardChoice, load_docs, search_outcome};
use calyx_sextant::Hit;

use super::super::Subcommand;
use super::super::ingest::parse_anchor_kind;
use super::super::vault::{ResolvedVault, home_dir, resolve_vault_info, vault_salt};
use super::output;
use super::parse::{KernelAnswerArgs, SearchArgs, SearchFusionArg, SearchGuardArg};
use crate::error::CliResult;
use crate::output::print_json;

pub(super) fn run(command: Subcommand) -> CliResult {
    match command {
        Subcommand::Search(args) => search_command(args),
        Subcommand::KernelAnswer(args) => kernel_answer_command(args),
        _ => unreachable!("non-search command routed to search module"),
    }
}

fn search_command(args: SearchArgs) -> CliResult {
    let resolved = resolve_cli_vault(&args.vault)?;
    let vault = open_vault(&resolved)?;
    let state = load_vault_panel_state(&resolved.path)?;
    let outcome = search_outcome(
        &vault,
        &state,
        &resolved.path,
        &args.query,
        args.k,
        fusion_choice(args.fusion),
        guard_choice(args.guard),
        args.filter.as_deref(),
        args.explain,
    )?;
    print_json(&output::render_hits(
        &outcome.hits,
        args.explain,
        args.provenance,
        outcome.guard_tau,
    ))
}

fn kernel_answer_command(args: KernelAnswerArgs) -> CliResult {
    let anchor = args.anchor.as_deref().map(parse_anchor_kind).transpose()?;
    let resolved = resolve_cli_vault(&args.vault)?;
    let vault = open_vault(&resolved)?;
    let state = load_vault_panel_state(&resolved.path)?;
    let docs = load_docs(&vault)?;
    let outcome = search_outcome(
        &vault,
        &state,
        &resolved.path,
        &args.query,
        super::parse::DEFAULT_K,
        FusionChoice::KernelFirst,
        GuardChoice::Off,
        None,
        args.explain,
    )?;
    let report = kernel_report_from_docs(&docs, &outcome.hits, anchor.as_ref())?;
    print_json(&report)
}

fn fusion_choice(arg: SearchFusionArg) -> FusionChoice {
    match arg {
        SearchFusionArg::Rrf => FusionChoice::Rrf,
        SearchFusionArg::WeightedRrf => FusionChoice::WeightedRrf,
        SearchFusionArg::SingleLens => FusionChoice::SingleLens,
        SearchFusionArg::KernelFirst => FusionChoice::KernelFirst,
        SearchFusionArg::Pipeline => FusionChoice::Pipeline,
    }
}

fn guard_choice(arg: SearchGuardArg) -> GuardChoice {
    match arg {
        SearchGuardArg::Off => GuardChoice::Off,
        SearchGuardArg::InRegion => GuardChoice::InRegion,
    }
}

pub(super) fn kernel_report_from_docs(
    docs: &BTreeMap<CxId, Constellation>,
    hits: &[Hit],
    anchor: Option<&AnchorKind>,
) -> CliResult<output::KernelAnswerOut> {
    let grounded = docs
        .values()
        .filter(|cx| has_grounding(cx, anchor))
        .map(|cx| cx.cx_id)
        .collect::<Vec<_>>();
    if grounded.is_empty() {
        return Err(CalyxError::kernel_ungrounded("kernel-answer has no grounded anchors").into());
    }
    let mut kernel_ids = hits
        .iter()
        .map(|hit| hit.cx_id)
        .filter(|cx_id| grounded.contains(cx_id))
        .take(5)
        .collect::<Vec<_>>();
    if kernel_ids.is_empty() {
        kernel_ids.extend(grounded.iter().copied().take(5));
    }
    let gap_count = docs.len().saturating_sub(grounded.len());
    let gaps = (gap_count > 0)
        .then(|| format!("grounding_gaps:{gap_count}"))
        .into_iter()
        .collect();
    Ok(output::KernelAnswerOut {
        answer: format!(
            "grounded kernel answer over {} anchored constellations",
            grounded.len()
        ),
        kernel_cx_ids: kernel_ids.into_iter().map(|id| id.to_string()).collect(),
        recall: grounded.len() as f32 / docs.len().max(1) as f32,
        gaps,
    })
}

fn has_grounding(cx: &Constellation, anchor: Option<&AnchorKind>) -> bool {
    cx.anchors
        .iter()
        .any(|item| anchor.is_none_or(|kind| &item.kind == kind))
}

fn resolve_cli_vault(vault: &str) -> CliResult<ResolvedVault> {
    resolve_vault_info(&home_dir()?, vault)
}

fn open_vault(resolved: &ResolvedVault) -> CliResult<AsterVault> {
    Ok(AsterVault::open(
        &resolved.path,
        resolved.vault_id,
        vault_salt(resolved.vault_id, &resolved.name),
        VaultOptions::default(),
    )?)
}
