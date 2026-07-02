//! `calyx probe-matrix <vault>` -- run physical probe-matrix search (#879).

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use calyx_aster::cf::ColumnFamily;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{CalyxError, Panel, SlotId};
use calyx_lodestar::{
    ProbeFusionMode, ProbeHit, ProbeLength, ProbeLensEmphasis, ProbeMatrixLog, ProbeMatrixSpec,
    ProbePhrasing, ProbeRefusal, ProbeResponse, ProbeVariant,
};
use calyx_search::{
    FusionChoice, GuardChoice, SearchBudget, SearchFreshness,
    search_outcome_with_query_vectors_freshness_cached,
};
use calyx_sextant::{Hit, RrfProfile};

use super::Subcommand;
use super::vault::home_dir;
use crate::bounded_progress::Deadline;
use crate::error::{CliError, CliResult};

mod artifact;
mod diagnostics;
mod grounding;
mod guard_summary;
mod parse;
mod perf_budget;
mod persist;
mod progress;
mod resident;
mod runner;
mod slot_timings;
mod support;
mod trace;
pub(super) use artifact::ProbeMatrixArtifact;
#[cfg(test)]
pub(super) use diagnostics::ProbeMatrixArtifactStatus;
use diagnostics::{QueryVectorCache, variant_guard_diagnostic};
pub(crate) use parse::parse_probe_matrix;
pub(crate) use runner::run_probe_matrix_with_home;
use support::{
    accepted_hit_count, active_text_slots, hex_lower, invalid_params, validate_requested_slots,
};
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ProbeMatrixArgs {
    pub vault: String,
    pub frontier: String,
    pub slots: Vec<SlotId>,
    pub weighted_profiles: Vec<RrfProfile>,
    pub phrasings: Vec<ProbePhrasing>,
    pub lengths: Vec<ProbeLength>,
    pub top_k: usize,
    pub guard: GuardChoice,
    pub guard_tau: Option<f32>,
    pub out: Option<PathBuf>,
    pub resident_addr: Option<SocketAddr>,
    pub max_variants: Option<usize>,
    pub time_budget_ms: Option<u64>,
    pub search_miss_budget_ms: Option<u64>,
    pub search_hit_budget_ms: Option<u64>,
}

impl Default for ProbeMatrixArgs {
    fn default() -> Self {
        Self {
            vault: String::new(),
            frontier: String::new(),
            slots: Vec::new(),
            weighted_profiles: Vec::new(),
            phrasings: Vec::new(),
            lengths: Vec::new(),
            top_k: ProbeMatrixSpec::new("frontier", vec![SlotId::new(0)]).top_k,
            guard: GuardChoice::Off,
            guard_tau: None,
            out: None,
            resident_addr: None,
            max_variants: None,
            time_budget_ms: None,
            search_miss_budget_ms: None,
            search_hit_budget_ms: None,
        }
    }
}

pub(crate) fn run(command: Subcommand) -> CliResult {
    let Subcommand::ProbeMatrix(args) = command else {
        unreachable!("non-probe-matrix command routed to probe_matrix module");
    };
    run_probe_matrix_with_home(&home_dir()?, args)
}

fn probe_read_vault_options(panel: &Panel, guard: GuardChoice) -> VaultOptions {
    let mut selected_cfs = match guard {
        GuardChoice::Off => super::search::base_read_cfs(),
        GuardChoice::InRegion => {
            // Profile-backed guarding (#1094) reads the calibrated Ward
            // profile from the Guard CF; an unselected CF silently reads as
            // None, which would masquerade as a missing profile.
            let mut cfs = super::search::panel_read_cfs(panel)
                .expect("probe panel read cfs are always selected");
            cfs.push(ColumnFamily::Guard);
            cfs
        }
    };
    selected_cfs.push(ColumnFamily::Anchors);
    selected_cfs.sort();
    selected_cfs.dedup();
    super::search::latest_read_vault_options_for_cfs(Some(selected_cfs))
}

fn selected_active_slots(args: &ProbeMatrixArgs, panel: &Panel) -> CliResult<Vec<SlotId>> {
    if args.slots.is_empty() {
        active_text_slots(&panel.slots)
    } else {
        validate_requested_slots(&args.slots, &panel.slots)?;
        Ok(args.slots.clone())
    }
}

struct ProbeVariantContext<'a> {
    state: &'a calyx_registry::VaultPanelState,
    vault_dir: &'a Path,
    guard: GuardChoice,
    guard_tau: Option<f32>,
    query_cache: &'a mut QueryVectorCache,
    search_cache: &'a mut calyx_search::SearchSlotCache,
    guard_diagnostics: &'a mut Vec<diagnostics::ProbeMatrixVariantDiagnostic>,
    resident_addr: Option<SocketAddr>,
    deadline: &'a Deadline,
}

fn probe_variant(
    vault: &AsterVault,
    variant: &ProbeVariant,
    ctx: &mut ProbeVariantContext<'_>,
) -> CliResult<ProbeResponse> {
    ctx.deadline.check(
        "probe-matrix",
        "before_query_measurement",
        variant.id as u64,
    )?;
    let (query_text_sha256, query_vectors) = ctx.query_cache.query_vectors(
        ctx.state,
        ctx.vault_dir,
        &variant.query_text,
        ctx.resident_addr,
    )?;
    ctx.deadline
        .check("probe-matrix", "after_query_measurement", variant.id as u64)?;
    let mut events = Vec::new();
    let mut trace_sink = |event: calyx_search::SearchTraceEvent| {
        events.push(event.clone());
        trace::emit_search_trace_event(event);
    };
    let mut budget_check = |phase: &'static str, processed: usize| {
        ctx.deadline
            .check("probe-matrix", phase, processed as u64)
            .map_err(search_budget_error)
    };
    let outcome = search_outcome_with_query_vectors_freshness_cached(
        vault,
        ctx.vault_dir,
        query_vectors,
        variant.top_k,
        fusion_choice(variant),
        ctx.guard,
        ctx.guard_tau,
        Some(u64::from(ctx.state.panel.version)),
        None,
        false,
        SearchFreshness::Fresh,
        SearchBudget::new(&mut budget_check),
        Some(ctx.search_cache),
        Some(&mut trace_sink),
    )?;
    ctx.guard_diagnostics.push(variant_guard_diagnostic(
        variant.id,
        query_text_sha256,
        &events,
    ));
    let mut hits = Vec::with_capacity(outcome.hits.len());
    let calyx_search::SearchOutcome {
        hits: outcome_hits,
        docs: verified_docs,
        ..
    } = outcome;
    for hit in outcome_hits {
        let cx = verified_docs.get(&hit.cx_id).ok_or_else(|| {
            CalyxError::stale_derived(format!(
                "probe-matrix search outcome missing verified source document for hit {}",
                hit.cx_id
            ))
        })?;
        hits.push(probe_hit(&hit, cx));
    }
    let refusals = probe_refusals(variant, &hits);
    Ok(ProbeResponse { hits, refusals })
}

fn search_budget_error(error: CliError) -> calyx_search::SearchError {
    calyx_core::CalyxError {
        code: error.code(),
        message: error.message().to_string(),
        remediation: error.remediation(),
    }
    .into()
}

fn validate_response(response: &ProbeResponse) -> CliResult {
    for hit in &response.hits {
        if !hit.score.is_finite() {
            return Err(invalid_params("probe hit score must be finite"));
        }
    }
    for refusal in &response.refusals {
        if refusal.code.trim().is_empty() {
            return Err(invalid_params("probe refusal code must not be empty"));
        }
        if refusal
            .deficit_bits
            .is_some_and(|bits| !bits.is_finite() || bits < 0.0)
        {
            return Err(invalid_params(
                "probe refusal deficit_bits must be finite and non-negative",
            ));
        }
    }
    Ok(())
}

fn fusion_choice(variant: &ProbeVariant) -> FusionChoice {
    match variant.fusion {
        ProbeFusionMode::KernelFirst => FusionChoice::KernelFirst,
        ProbeFusionMode::Rrf => FusionChoice::Rrf,
        ProbeFusionMode::WeightedRrf => match variant.lens_emphasis {
            ProbeLensEmphasis::WeightedProfile(profile) => {
                FusionChoice::WeightedRrfProfile(profile)
            }
            _ => FusionChoice::WeightedRrf,
        },
        ProbeFusionMode::SingleLens => match variant.lens_emphasis {
            ProbeLensEmphasis::Slot(slot) => FusionChoice::SingleLensSlot(slot),
            _ => FusionChoice::SingleLens,
        },
        ProbeFusionMode::Pipeline => FusionChoice::Pipeline,
    }
}

fn probe_hit(hit: &Hit, cx: &calyx_core::Constellation) -> ProbeHit {
    let mut provenance = vec![
        format!("rank={}", hit.rank),
        format!("ledger_seq={}", hit.provenance.seq),
        format!("ledger_hash={}", hex_lower(&hit.provenance.hash)),
        format!("provenance_source={:?}", hit.provenance_source),
    ];
    provenance.push(format!(
        "grounding:anchor_count={} flags_ungrounded={} flags_degraded={}",
        cx.anchors.len(),
        cx.flags.ungrounded,
        cx.flags.degraded
    ));
    for (key, value) in &cx.metadata {
        if matches!(
            key.as_str(),
            "source_dataset" | "source_id" | "source_url" | "doi" | "pmid" | "license"
        ) {
            provenance.push(format!("metadata:{key}={value}"));
        }
    }
    for lens in &hit.per_lens {
        provenance.push(format!(
            "lens:{} rank={} contribution={}",
            lens.slot, lens.rank, lens.contribution
        ));
    }
    ProbeHit {
        cx_id: hit.cx_id,
        score: hit.score,
        grounded: !cx.anchors.is_empty(),
        provenance,
    }
}

fn probe_refusals(variant: &ProbeVariant, hits: &[ProbeHit]) -> Vec<ProbeRefusal> {
    if hits.is_empty() {
        return vec![ProbeRefusal {
            code: "CALYX_PROBE_NO_HITS".to_string(),
            reason: format!("variant {} returned zero physical search hits", variant.id),
            deficit_bits: None,
        }];
    }
    if hits.iter().all(|hit| !hit.grounded) {
        return vec![ProbeRefusal {
            code: "CALYX_PROBE_UNGROUNDED_HITS".to_string(),
            reason: format!(
                "variant {} returned hits, but none had persisted anchors",
                variant.id
            ),
            deficit_bits: None,
        }];
    }
    Vec::new()
}

fn ensure_useful_log(log: &ProbeMatrixLog) -> CliResult {
    if log.records.is_empty() {
        return Err(invalid_params("probe matrix produced no records"));
    }
    let accepted = accepted_hit_count(log);
    if accepted == 0 {
        return Err(invalid_params(
            "probe matrix produced no grounded accepted hits",
        ));
    }
    if log.productive.is_empty() {
        return Err(invalid_params(
            "probe matrix produced no productive variants with grounded accepted hits",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
