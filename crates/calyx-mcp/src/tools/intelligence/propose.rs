use calyx_anneal::{
    AnchorId, AnnealLedger, AnnealSubstrate, AsterAnnealLedgerStore, AsterRollbackStorage,
    BudgetConfig, BudgetEnforcer, HeldOutReplay, ProposalOutcome, ProposalTerminalState,
    ProposeLens, ProposeLensRequest, ReplayAnchor, ReplayQuery, RollbackStore, TripwireRegistry,
    describe, record_proposal_outcome,
};
use calyx_assay::PanelResourceBudget;
use calyx_aster::cf::ColumnFamily;
use calyx_core::{CalyxError, Clock, CxId, SystemClock};
use calyx_ledger::{ActorId, LedgerAppender};
use calyx_registry::{SwapController, persist_vault_panel_state};
use serde::Serialize;
use serde_json::{Value, json};

use super::core::{VaultContext, active_slot_ids, load_context, load_docs, parse_anchor};
use super::metrics;
use super::model::{BitsOut, ProposeLensOut, assay_key, proposal_key};
use super::propose_backfill::{apply_slot_backfill, restore_slot_backfill};
use super::propose_live::{
    LiveAssay, LiveAssayView, LiveHotAdder, LivePairNmi, LiveProfiler, LiveProposalState,
};
use crate::server::{ToolError, ToolResult};

const PROPOSAL_ROLLBACK_SEED: u64 = 725;
const PROPOSAL_REPLAY_QUERIES: usize = 8;
const METRIC_EPSILON: f64 = 1e-12;

pub(super) fn run(
    vault_name: &str,
    anchor: &str,
    resource_budget: Option<PanelResourceBudget>,
) -> ToolResult<Value> {
    let ctx = load_context(vault_name)?;
    let docs = load_docs(&ctx.vault)?;
    let anchor = parse_anchor(anchor)?;
    let label = super::core::anchor_label(&anchor);
    let assay_key = assay_key(&label);
    let measured = metrics::bits(&ctx.state.panel, &docs, &anchor, &label, true, &assay_key)?;
    super::core::write_json_row(
        &ctx.vault,
        ColumnFamily::Assay,
        assay_key.clone(),
        &measured,
    )?;

    let clock = SystemClock;
    let anchor_id = AnchorId::new(label.clone())?;
    let corpus = docs.values().cloned().collect::<Vec<_>>();
    let live_state = LiveProposalState::default();
    let live_assay = LiveAssay::new(&ctx.state.panel, &docs, &anchor, &measured);
    let assay_view = LiveAssayView::new(&live_assay, &live_state);
    let mut controller = SwapController::new(ctx.state.panel.clone());
    let profiler = LiveProfiler::new(&ctx.vault_dir, &anchor, &live_state);
    let nmi = LivePairNmi::new(&ctx.state.panel, &live_state);
    let replay = replay_from_docs(&docs, &ctx.state.panel)?;
    let mut substrate = substrate(&ctx, &clock, replay)?;
    let mut registry = ctx.state.registry.clone();
    let mut hot_add = LiveHotAdder::new(
        &mut registry,
        &ctx.vault_dir,
        &docs,
        &anchor,
        live_assay.before_bits(),
        live_assay.entropy_bits(),
        &live_state,
    );

    let mut engine = ProposeLens::new(&clock);
    if let Some(budget) = resource_budget {
        engine = engine.with_resource_budget(budget);
    }
    let mut outcome = engine.propose_lens(ProposeLensRequest {
        anchor: &anchor_id,
        controller: &mut controller,
        substrate: &mut substrate,
        assay: &assay_view,
        hot_add: &mut hot_add,
        profiler: &profiler,
        nmi: &nmi,
        corpus: &corpus,
    })?;

    let mut backfill = None;
    let mut panel_write = None;
    if outcome.admitted {
        match finalize_admission(
            AdmissionInputs {
                ctx: &ctx,
                docs: &docs,
                anchor: &anchor,
                label: &label,
                controller: &controller,
                registry: &registry,
                live_state: &live_state,
            },
            &mut outcome,
            &mut substrate,
        ) {
            Ok(Some(finalized)) => {
                backfill = Some(json_value(&finalized.backfill)?);
                panel_write = Some(panel_write_json(&finalized.panel_write));
            }
            Ok(None) => {}
            Err(error) => return Err(error),
        }
    }

    let ledger_ref = record_proposal_outcome(
        &outcome,
        &mut substrate.ledger,
        clock.now(),
        live_assay.deficit_gap(),
    )?;
    let out = output(
        &label,
        measured,
        live_assay.deficit_gap(),
        &outcome,
        ledger_ref,
        panel_write,
        backfill,
    )?;
    super::core::write_json_row(
        &ctx.vault,
        ColumnFamily::AnnealOperators,
        proposal_key(&label),
        &out,
    )?;
    Ok(json!(out))
}

struct FinalizedAdmission {
    backfill: super::propose_backfill::BackfillWriteReport,
    panel_write: calyx_registry::VaultPanelWrite,
}

struct AdmissionInputs<'a> {
    ctx: &'a VaultContext,
    docs: &'a std::collections::BTreeMap<CxId, calyx_core::Constellation>,
    anchor: &'a calyx_core::AnchorKind,
    label: &'a str,
    controller: &'a SwapController,
    registry: &'a calyx_registry::Registry,
    live_state: &'a LiveProposalState,
}

fn finalize_admission(
    inputs: AdmissionInputs<'_>,
    outcome: &mut ProposalOutcome,
    substrate: &mut AnnealSubstrate<
        '_,
        AsterRollbackStorage<'_, SystemClock>,
        AsterAnnealLedgerStore<'_, SystemClock>,
        SystemClock,
    >,
) -> ToolResult<Option<FinalizedAdmission>> {
    let Some(candidate_backfill) = inputs.live_state.take_backfill() else {
        return Err(proposal_error("admitted proposal is missing candidate backfill").into());
    };
    let (backfill, undo) =
        apply_slot_backfill(&inputs.ctx.vault, inputs.docs, &candidate_backfill)?;
    let reloaded = load_docs(&inputs.ctx.vault)?;
    let after = metrics::bits(
        inputs.controller.panel(),
        &reloaded,
        inputs.anchor,
        inputs.label,
        false,
        &assay_key(inputs.label),
    )?;
    let after_bits = total_bits(&after).min(after.dpi_ceiling);
    if after_bits <= outcome.sufficiency_before + METRIC_EPSILON {
        restore_slot_backfill(&inputs.ctx.vault, undo)?;
        if let Some(change_id) = outcome.change_id {
            substrate.rollback_explicit(change_id)?;
        }
        outcome.admitted = false;
        outcome.sufficiency_after = Some(after_bits);
        outcome.terminal_state = ProposalTerminalState::NoSufficiencyGain;
        return Ok(None);
    }
    outcome.sufficiency_after = Some(after_bits);
    let panel_write = persist_vault_panel_state(
        &inputs.ctx.vault_dir,
        inputs.controller.panel(),
        inputs.registry,
    )?;
    Ok(Some(FinalizedAdmission {
        backfill,
        panel_write,
    }))
}

fn substrate<'a>(
    ctx: &'a VaultContext,
    clock: &'a SystemClock,
    replay: HeldOutReplay,
) -> ToolResult<
    AnnealSubstrate<
        'a,
        AsterRollbackStorage<'a, SystemClock>,
        AsterAnnealLedgerStore<'a, SystemClock>,
        SystemClock,
    >,
> {
    let rollback = RollbackStore::open(
        clock,
        PROPOSAL_ROLLBACK_SEED,
        AsterRollbackStorage::new(&ctx.vault),
    )?;
    let appender = LedgerAppender::open(AsterAnnealLedgerStore::new(&ctx.vault), SystemClock)?;
    let ledger = AnnealLedger::new(
        appender,
        ActorId::Service("calyx-mcp-propose-lens".to_string()),
    )?;
    let budget = BudgetEnforcer::new(BudgetConfig::load_from_vault(&ctx.vault_dir)?, clock)?;
    Ok(AnnealSubstrate::new(
        TripwireRegistry::load_from_vault(&ctx.vault_dir)?,
        replay,
        rollback,
        ledger,
        budget,
        clock,
    ))
}

fn replay_from_docs(
    docs: &std::collections::BTreeMap<CxId, calyx_core::Constellation>,
    panel: &calyx_core::Panel,
) -> ToolResult<HeldOutReplay> {
    let slots = active_slot_ids(panel);
    let mut queries = Vec::new();
    for (idx, cx) in docs.values().enumerate() {
        let Some(vector) = slots.iter().find_map(|slot| super::core::dense(cx, *slot)) else {
            continue;
        };
        queries.push(ReplayQuery {
            query_id: idx as u64,
            query_vector: vector.to_vec(),
            expected_top_k: vec![ReplayAnchor {
                cx_id: cx.cx_id,
                similarity: 1.0,
            }],
        });
        if queries.len() >= PROPOSAL_REPLAY_QUERIES {
            break;
        }
    }
    if queries.is_empty() {
        return Err(proposal_error("proposal shadow replay has no dense stored vectors").into());
    }
    Ok(HeldOutReplay { queries, seed: 725 })
}

fn output(
    label: &str,
    measured: BitsOut,
    predicted_gain: f64,
    outcome: &ProposalOutcome,
    ledger_ref: Option<calyx_core::LedgerRef>,
    panel_write: Option<Value>,
    backfill: Option<Value>,
) -> ToolResult<ProposeLensOut> {
    let candidate = outcome.candidate.as_ref();
    let candidate_json = candidate
        .map(json_value)
        .transpose()?
        .unwrap_or(Value::Null);
    Ok(ProposeLensOut {
        name: candidate
            .map(candidate_name)
            .unwrap_or_else(|| format!("none::{label}")),
        rationale: candidate
            .map(describe)
            .unwrap_or_else(|| "no positive lens deficit localized".to_string()),
        predicted_bits_gain: predicted_gain,
        runtime_hint: candidate.map(runtime_hint).unwrap_or("none").to_string(),
        estimated_cost: "measured from retained corpus inputs; zero external service cost"
            .to_string(),
        candidate: candidate_json,
        admitted: outcome.admitted,
        terminal_state: terminal_state(&outcome.terminal_state).to_string(),
        sufficiency_before: outcome.sufficiency_before,
        sufficiency_after: outcome.sufficiency_after,
        gate_outcome: outcome.gate_outcome.as_ref().map(json_value).transpose()?,
        hot_add: outcome.hot_add.as_ref().map(json_value).transpose()?,
        ledger_ref: ledger_ref.map(json_value).transpose()?,
        panel_write,
        backfill,
        measured,
    })
}

fn total_bits(report: &BitsOut) -> f64 {
    report.per_slot.iter().map(|slot| slot.bits).sum()
}

fn candidate_name(candidate: &calyx_anneal::CandidateLens) -> String {
    match candidate {
        calyx_anneal::CandidateLens::Algorithmic { kind, .. } => {
            format!("algorithmic::{kind:?}")
        }
        calyx_anneal::CandidateLens::Commission { spec } => {
            format!("commission::{}", spec.axis)
        }
    }
}

fn runtime_hint(candidate: &calyx_anneal::CandidateLens) -> &'static str {
    match candidate {
        calyx_anneal::CandidateLens::Algorithmic { .. } => "algorithmic",
        calyx_anneal::CandidateLens::Commission { .. } => "commission",
    }
}

fn terminal_state(state: &ProposalTerminalState) -> &'static str {
    match state {
        ProposalTerminalState::NoDeficit => "no_deficit",
        ProposalTerminalState::GateRejected => "gate_rejected",
        ProposalTerminalState::HotAddFailed { .. } => "hot_add_failed",
        ProposalTerminalState::SubstrateReverted { .. } => "substrate_reverted",
        ProposalTerminalState::NoSufficiencyGain => "no_sufficiency_gain",
        ProposalTerminalState::Admitted => "admitted",
    }
}

fn json_value<T: Serialize>(value: T) -> ToolResult<Value> {
    serde_json::to_value(value)
        .map_err(|err| ToolError::invalid_params(format!("serialize proposal result: {err}")))
}

fn panel_write_json(write: &calyx_registry::VaultPanelWrite) -> Value {
    json!({
        "manifest_seq": write.manifest_seq,
        "durable_seq": write.durable_seq,
        "panel_ref": write.panel_ref.logical_path,
        "registry_ref": write.registry_ref.logical_path,
    })
}

fn proposal_error(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: "CALYX_PROPOSE_DRIVER_FAILED",
        message: message.into(),
        remediation: "read the proposal ledger, backfill rows, and panel manifest before retrying",
    }
}
