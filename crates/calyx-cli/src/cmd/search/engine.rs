//! CLI glue for `search` / `kernel-answer`: resolve + open the vault, delegate
//! the real search to the shared `calyx-search` crate, then render the CLI JSON.
//! All search logic (index load, recall, fusion, provenance, guard) lives in
//! `calyx-search` so the CLI and `calyx-web-api` share ONE path (#573).

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::str::FromStr;
use std::time::Instant;

use calyx_aster::vault::AsterVault;
use calyx_core::{
    AnchorKind, CalyxError, Constellation, CxId, Input, LensId, Modality, Placement, SlotId,
    SlotState, SlotVector,
};
use calyx_registry::{VaultPanelState, load_vault_panel_state, require_vault_registry_contracts};
use calyx_search::{
    FusionChoice, GuardChoice, load_docs, search_outcome, search_outcome_with_query_vectors,
};
use calyx_sextant::Hit;

use super::super::Subcommand;
use super::super::ingest::parse_anchor_kind;
use super::super::vault::{ResolvedVault, home_dir, resolve_vault_info, vault_salt};
use super::output;
use super::parse::{KernelAnswerArgs, SearchArgs, SearchFusionArg, SearchGuardArg};
use super::{base_read_cfs, latest_read_vault_options_for_cfs, panel_read_cfs};
use crate::error::CliResult;
use crate::output::print_json;
use crate::panel_commands::measure_resident_batch_at;
use crate::path_identity::vault_template_source;

pub(super) fn run(command: Subcommand) -> CliResult {
    match command {
        Subcommand::Search(args) => search_command(args),
        Subcommand::KernelAnswer(args) => kernel_answer_command(args),
        _ => unreachable!("non-search command routed to search module"),
    }
}

fn search_command(args: SearchArgs) -> CliResult {
    let resolved = resolve_cli_vault(&args.vault)?;
    require_vault_registry_contracts(&resolved.path)?;
    let state = load_vault_panel_state(&resolved.path)?;
    let fusion = fusion_choice(args.fusion);
    let guard = guard_choice(args.guard);
    let outcome = match args.resident_addr {
        Some(addr) => {
            let query_vectors =
                measure_search_query_vectors_via_resident(&state, &resolved, &args.query, addr)?;
            let vault = open_vault(&resolved, search_read_cfs(&state, guard))?;
            search_outcome_with_query_vectors(
                &vault,
                &resolved.path,
                &query_vectors,
                args.k,
                fusion,
                guard,
                args.filter.as_deref(),
                args.explain,
            )?
        }
        None => {
            require_resident_for_gpu_text_search(&state)?;
            let vault = open_vault(&resolved, search_read_cfs(&state, guard))?;
            search_outcome(
                &vault,
                &state,
                &resolved.path,
                &args.query,
                args.k,
                fusion,
                guard,
                args.filter.as_deref(),
                args.explain,
            )?
        }
    };
    print_json(&output::render_hits(
        &outcome.hits,
        args.explain,
        args.provenance,
        outcome.guard_tau,
    ))
}

fn measure_search_query_vectors_via_resident(
    state: &VaultPanelState,
    resolved: &ResolvedVault,
    query: &str,
    addr: SocketAddr,
) -> CliResult<Vec<(SlotId, SlotVector)>> {
    let started = Instant::now();
    let input = Input::new(Modality::Text, query.as_bytes().to_vec());
    eprintln!(
        "CALYX_SEARCH_RUNTIME phase=search_resident_service_start addr={} vault={} inputs=1",
        addr,
        resolved.path.display()
    );
    let resident = measure_resident_batch_at(addr, Modality::Text, &[input], None).map_err(
        |error| {
            CalyxError {
                code: "CALYX_PANEL_RESIDENT_UNAVAILABLE",
                message: format!(
                    "resident search measurement failed addr={addr} code={} message={}",
                    error.code(),
                    error.message()
                ),
                remediation: "start `calyx panel resident serve --vault <vault-path>` for the matching active vault, then retry search with --resident-addr",
            }
        },
    )?;
    let request_bytes = resident.request_bytes;
    let response_bytes = resident.response_bytes;
    let response = resident.response;
    if !response.ready {
        return Err(CalyxError::lens_unreachable(format!(
            "resident service {addr} returned ready=false for search measurement"
        ))
        .into());
    }
    let expected_template_source = vault_template_source(&resolved.path)?;
    if response.template_source != expected_template_source {
        return Err(CalyxError {
            code: "CALYX_SEARCH_RESIDENT_MISMATCH",
            message: format!(
                "resident service {addr} served template_source {}, expected {}",
                response.template_source, expected_template_source
            ),
            remediation: "restart the resident service with `calyx panel resident serve --vault <this-vault-path>` and retry search",
        }
        .into());
    }
    if response.modality != Modality::Text || response.input_count != 1 || response.rows.len() != 1
    {
        return Err(CalyxError::lens_dim_mismatch(format!(
            "resident search response mismatch: modality {:?} input_count {} rows {}, expected Text/1/1",
            response.modality,
            response.input_count,
            response.rows.len()
        ))
        .into());
    }
    let row = &response.rows[0];
    if row.input_index != 0 {
        return Err(CalyxError::lens_dim_mismatch(format!(
            "resident search response row index {}, expected 0",
            row.input_index
        ))
        .into());
    }

    let mut out = Vec::new();
    for slot in active_registered_text_slots(state) {
        let returned = row
            .slots
            .iter()
            .find(|returned| {
                returned.measured
                    && returned.slot == slot.slot_id.get()
                    && LensId::from_str(&returned.lens_id)
                        .map(|lens_id| lens_id == slot.lens_id)
                        .unwrap_or(false)
            })
            .ok_or_else(|| {
                CalyxError::lens_unreachable(format!(
                    "resident service {addr} did not return active text slot {} lens {}",
                    slot.slot_id.get(),
                    slot.lens_id
                ))
            })?;
        if returned.modality != Modality::Text || returned.placement != slot.resource.placement {
            return Err(CalyxError::lens_unreachable(format!(
                "resident service {addr} returned slot {} lens {} with modality {:?} placement {:?}, expected Text/{:?}",
                slot.slot_id.get(),
                slot.lens_id,
                returned.modality,
                returned.placement,
                slot.resource.placement
            ))
            .into());
        }
        let vector = returned.vector.clone().ok_or_else(|| {
            CalyxError::lens_unreachable(format!(
                "resident service {addr} measured slot {} lens {} without a vector",
                slot.slot_id.get(),
                slot.lens_id
            ))
        })?;
        if !indexable(&vector) {
            return Err(CalyxError::stale_derived(format!(
                "resident service {addr} returned non-indexable query vector for slot {} lens {}",
                slot.slot_id.get(),
                slot.lens_id
            ))
            .into());
        }
        let snapshot = registry_snapshot_for_lens(state, slot.lens_id).ok_or_else(|| {
            CalyxError::lens_unreachable(format!(
                "resident search requires persisted registry snapshot contract for lens {}",
                slot.lens_id
            ))
        })?;
        snapshot.contract.verify_vector(slot.lens_id, &vector)?;
        out.push((slot.slot_id, vector));
    }
    eprintln!(
        "CALYX_SEARCH_RUNTIME phase=search_resident_service_ok addr={} process_id={} template_source={} inputs=1 slots={} elapsed_ms={} resident_elapsed_ms={} protocol=binary request_bytes={} response_bytes={}",
        addr,
        response.process_id,
        response.template_source,
        out.len(),
        started.elapsed().as_millis(),
        response.elapsed_ms,
        request_bytes,
        response_bytes
    );
    Ok(out)
}

fn require_resident_for_gpu_text_search(state: &VaultPanelState) -> CliResult {
    let gpu_slots = active_registered_text_slots(state)
        .filter(|slot| slot.resource.placement == Placement::Gpu)
        .map(|slot| {
            format!(
                "slot={} key={} lens={} placement={:?}",
                slot.slot_id.get(),
                slot.slot_key.key(),
                slot.lens_id,
                slot.resource.placement
            )
        })
        .collect::<Vec<_>>();
    if gpu_slots.is_empty() {
        return Ok(());
    }
    Err(CalyxError {
        code: "CALYX_SEARCH_RESIDENT_REQUIRED",
        message: format!(
            "search refuses cold local query measurement for {} active GPU text lens(es): {}",
            gpu_slots.len(),
            gpu_slots.join(", ")
        ),
        remediation: "start `calyx panel resident serve --vault <vault-path>` and rerun search with --resident-addr",
    }
    .into())
}

fn active_registered_text_slots(
    state: &VaultPanelState,
) -> impl Iterator<Item = &calyx_core::Slot> {
    state.panel.slots.iter().filter(|slot| {
        slot.state == SlotState::Active
            && slot.modality == Modality::Text
            && state.registry.contains(slot.lens_id)
    })
}

fn registry_snapshot_for_lens(
    state: &VaultPanelState,
    lens_id: LensId,
) -> Option<&calyx_registry::RegistryLensSnapshot> {
    state
        .registry_snapshot
        .as_ref()?
        .lenses
        .iter()
        .find(|snapshot| snapshot.lens_id == lens_id)
}

fn indexable(vector: &SlotVector) -> bool {
    matches!(
        vector,
        SlotVector::Dense { .. } | SlotVector::Sparse { .. } | SlotVector::Multi { .. }
    )
}

fn kernel_answer_command(args: KernelAnswerArgs) -> CliResult {
    let anchor = args.anchor.as_deref().map(parse_anchor_kind).transpose()?;
    let resolved = resolve_cli_vault(&args.vault)?;
    require_vault_registry_contracts(&resolved.path)?;
    let state = load_vault_panel_state(&resolved.path)?;
    let vault = open_vault(&resolved, panel_read_cfs(&state.panel))?;
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

fn open_vault(
    resolved: &ResolvedVault,
    selected_cfs: Option<Vec<calyx_aster::cf::ColumnFamily>>,
) -> CliResult<AsterVault> {
    Ok(AsterVault::open(
        &resolved.path,
        resolved.vault_id,
        vault_salt(resolved.vault_id, &resolved.name),
        latest_read_vault_options_for_cfs(selected_cfs),
    )?)
}

fn search_read_cfs(
    state: &VaultPanelState,
    guard: GuardChoice,
) -> Option<Vec<calyx_aster::cf::ColumnFamily>> {
    match guard {
        GuardChoice::Off => Some(base_read_cfs()),
        GuardChoice::InRegion => panel_read_cfs(&state.panel),
    }
}
