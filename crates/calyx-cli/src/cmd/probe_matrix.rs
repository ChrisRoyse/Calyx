//! `calyx probe-matrix <vault>` -- run physical probe-matrix search (#879).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::Instant;

use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{CalyxError, CxId, Panel, SlotId};
use calyx_lodestar::{
    PROBE_MATRIX_SCHEMA_VERSION, ProbeFusionMode, ProbeHit, ProbeLength, ProbeLensEmphasis,
    ProbeMatrixLog, ProbeMatrixSpec, ProbePhrasing, ProbeProductivity, ProbeRecord, ProbeRefusal,
    ProbeResponse, ProbeVariant, build_probe_matrix,
};
use calyx_registry::{load_vault_panel_state, require_vault_registry_contracts};
use calyx_search::{
    FusionChoice, GuardChoice, SearchFreshness, search_outcome_with_query_vectors_freshness,
};
use calyx_sextant::{Hit, RrfProfile};
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::Subcommand;
use super::vault::{home_dir, resolve_vault_info, vault_salt};
use crate::error::CliResult;
use crate::output::print_json;

mod diagnostics;
mod parse;
mod persist;
mod support;
mod trace;
use diagnostics::{
    ProbeMatrixArtifactStatus, ProbeMatrixDiagnostics, QueryVectorCache, variant_guard_diagnostic,
};
pub(crate) use parse::parse_probe_matrix;
use persist::persist_probe_matrix;
use support::{
    accepted_hit_count, active_text_slots, hex_lower, invalid_params, validate_requested_slots,
    with_persisted_artifact_error,
};
const PROBE_MATRIX_ARTIFACT_SCHEMA_VERSION: u32 = 2;

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
    pub out: Option<PathBuf>,
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
            out: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct ProbeMatrixArtifact {
    schema_version: u32,
    status: ProbeMatrixArtifactStatus,
    vault: String,
    vault_id: String,
    vault_dir: String,
    active_slots: Vec<SlotId>,
    diagnostics: ProbeMatrixDiagnostics,
    log: ProbeMatrixLog,
}

pub(crate) fn run(command: Subcommand) -> CliResult {
    let Subcommand::ProbeMatrix(args) = command else {
        unreachable!("non-probe-matrix command routed to probe_matrix module");
    };
    run_probe_matrix_with_home(&home_dir()?, args)
}

pub(crate) fn run_probe_matrix_with_home(home: &Path, args: ProbeMatrixArgs) -> CliResult {
    let started = Instant::now();
    let resolved = resolve_vault_info(home, &args.vault)?;
    eprintln!(
        "probe-matrix: opening physical vault name={} id={} path={}",
        resolved.name,
        resolved.vault_id,
        resolved.path.display()
    );
    let state = load_vault_panel_state(&resolved.path)?;
    let active_slots = if args.slots.is_empty() {
        active_text_slots(&state.panel.slots)?
    } else {
        validate_requested_slots(&args.slots, &state.panel.slots)?;
        args.slots.clone()
    };
    let vault = AsterVault::open(
        &resolved.path,
        resolved.vault_id,
        vault_salt(resolved.vault_id, &resolved.name),
        probe_read_vault_options(&state.panel, args.guard),
    )?;
    eprintln!(
        "probe-matrix: opened vault snapshot_seq={} elapsed_ms={}",
        vault.latest_seq(),
        started.elapsed().as_millis()
    );
    let audit = require_vault_registry_contracts(&resolved.path)?;
    eprintln!(
        "probe-matrix: registry contracts valid checked_count={} elapsed_ms={}",
        audit.checked_count,
        started.elapsed().as_millis()
    );
    eprintln!(
        "probe-matrix: loaded panel slots={} registry_lenses={} elapsed_ms={}",
        state.panel.slots.len(),
        state
            .registry_snapshot
            .as_ref()
            .map_or(0, |snapshot| snapshot.lenses.len()),
        started.elapsed().as_millis()
    );
    let spec = ProbeMatrixSpec {
        frontier: args.frontier.clone(),
        active_slots,
        weighted_profiles: if args.weighted_profiles.is_empty() {
            ProbeMatrixSpec::new(&args.frontier, vec![SlotId::new(0)]).weighted_profiles
        } else {
            args.weighted_profiles.clone()
        },
        phrasings: if args.phrasings.is_empty() {
            ProbePhrasing::all()
        } else {
            args.phrasings.clone()
        },
        lengths: if args.lengths.is_empty() {
            ProbeLength::all()
        } else {
            args.lengths.clone()
        },
        top_k: args.top_k,
    };
    eprintln!(
        "probe-matrix: running frontier={:?} slots={} profiles={} phrasings={} lengths={} top_k={} guard={:?} rayon_threads={}",
        spec.frontier,
        spec.active_slots.len(),
        spec.weighted_profiles.len(),
        spec.phrasings.len(),
        spec.lengths.len(),
        spec.top_k,
        args.guard,
        rayon::current_num_threads()
    );
    let allowed_slots = spec.active_slots.iter().copied().collect::<BTreeSet<_>>();
    let mut query_cache = QueryVectorCache::new(allowed_slots.clone());
    let mut guard_diagnostics = Vec::new();
    let log = run_physical_probe_matrix(&spec, |variant| {
        probe_variant(
            &vault,
            &state,
            &resolved.path,
            variant,
            args.guard,
            &mut query_cache,
            &mut guard_diagnostics,
        )
    })?;
    let status = ProbeMatrixArtifactStatus::from_log(&log);
    let artifact = ProbeMatrixArtifact {
        schema_version: PROBE_MATRIX_ARTIFACT_SCHEMA_VERSION,
        status,
        vault: resolved.name.clone(),
        vault_id: resolved.vault_id.to_string(),
        vault_dir: resolved.path.display().to_string(),
        active_slots: spec.active_slots.clone(),
        diagnostics: ProbeMatrixDiagnostics {
            query_measurements: query_cache.diagnostics(),
            variant_guard_counts: guard_diagnostics,
        },
        log,
    };
    let persisted = persist_probe_matrix(&resolved.path, args.out.as_deref(), &artifact)?;
    eprintln!(
        "probe-matrix: persisted matrix={} bytes={} sha256={} elapsed_ms={}",
        persisted.path.display(),
        persisted.bytes,
        persisted.sha256,
        started.elapsed().as_millis()
    );
    if let Err(error) = ensure_useful_log(&artifact.log) {
        return Err(with_persisted_artifact_error(error, &persisted));
    }
    print_json(&json!({
        "status": "ok",
        "vault": resolved.name,
        "vault_dir": resolved.path.display().to_string(),
        "artifact": artifact,
        "artifacts": {
            "matrix_json": persisted.path,
            "matrix_json_bytes": persisted.bytes,
            "matrix_json_sha256": persisted.sha256,
            "readback": {
                "record_count": persisted.readback_record_count,
                "productive_count": persisted.readback_productive_count,
                "accepted_hit_count": persisted.readback_accepted_hit_count,
                "refusal_count": persisted.readback_refusal_count,
            }
        }
    }))
}

fn probe_read_vault_options(panel: &Panel, guard: GuardChoice) -> VaultOptions {
    let selected_cfs = match guard {
        GuardChoice::Off => Some(super::search::base_read_cfs()),
        GuardChoice::InRegion => super::search::panel_read_cfs(panel),
    };
    super::search::latest_read_vault_options_for_cfs(selected_cfs)
}

fn run_physical_probe_matrix<F>(spec: &ProbeMatrixSpec, mut probe: F) -> CliResult<ProbeMatrixLog>
where
    F: FnMut(&ProbeVariant) -> CliResult<ProbeResponse>,
{
    let variants = build_probe_matrix(spec)?;
    let mut records = Vec::with_capacity(variants.len());
    for variant in variants {
        let variant_started = Instant::now();
        eprintln!(
            "probe-matrix: variant start fusion={:?} emphasis={:?} phrasing={:?} length={:?} top_k={}",
            variant.fusion, variant.lens_emphasis, variant.phrasing, variant.length, variant.top_k
        );
        let response = probe(&variant)?;
        validate_response(&response)?;
        let accepted_hit_count = response.hits.iter().filter(|hit| hit.grounded).count();
        eprintln!(
            "probe-matrix: variant ok hits={} accepted_hits={} refusals={} elapsed_ms={}",
            response.hits.len(),
            accepted_hit_count,
            response.refusals.len(),
            variant_started.elapsed().as_millis()
        );
        records.push(ProbeRecord {
            variant,
            hits: response.hits,
            refusals: response.refusals,
            accepted_hit_count,
            unique_grounded_hits: Vec::new(),
        });
    }
    attach_unique_hits(&mut records);
    let productive = productive_rows(&records);
    Ok(ProbeMatrixLog {
        schema_version: PROBE_MATRIX_SCHEMA_VERSION,
        spec: spec.clone(),
        records,
        productive,
    })
}

fn probe_variant(
    vault: &AsterVault,
    state: &calyx_registry::VaultPanelState,
    vault_dir: &Path,
    variant: &ProbeVariant,
    guard: GuardChoice,
    query_cache: &mut QueryVectorCache,
    guard_diagnostics: &mut Vec<diagnostics::ProbeMatrixVariantDiagnostic>,
) -> CliResult<ProbeResponse> {
    let (query_text_sha256, query_vectors) =
        query_cache.query_vectors(state, &variant.query_text)?;
    let mut events = Vec::new();
    let mut trace_sink = |event: calyx_search::SearchTraceEvent| {
        events.push(event.clone());
        trace::emit_search_trace_event(event);
    };
    let outcome = search_outcome_with_query_vectors_freshness(
        vault,
        vault_dir,
        query_vectors,
        variant.top_k,
        fusion_choice(variant),
        guard,
        None,
        false,
        SearchFreshness::Fresh,
        Some(&mut trace_sink),
    )?;
    guard_diagnostics.push(variant_guard_diagnostic(
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

fn attach_unique_hits(records: &mut [ProbeRecord]) {
    let mut counts = BTreeMap::<CxId, usize>::new();
    for record in records.iter() {
        for hit in record.hits.iter().filter(|hit| hit.grounded) {
            *counts.entry(hit.cx_id).or_default() += 1;
        }
    }
    for record in records {
        record.unique_grounded_hits = record
            .hits
            .iter()
            .filter(|hit| hit.grounded && counts.get(&hit.cx_id) == Some(&1))
            .map(|hit| hit.cx_id)
            .collect();
    }
}

fn productive_rows(records: &[ProbeRecord]) -> Vec<ProbeProductivity> {
    let mut rows: Vec<_> = records
        .iter()
        .filter(|record| record.accepted_hit_count > 0)
        .map(|record| ProbeProductivity {
            variant_id: record.variant.id,
            fusion: record.variant.fusion.clone(),
            phrasing: record.variant.phrasing,
            length: record.variant.length,
            lens_emphasis: record.variant.lens_emphasis.clone(),
            unique_hit_count: record.unique_grounded_hits.len(),
            accepted_hit_count: record.accepted_hit_count,
            refusal_count: record.refusals.len(),
        })
        .collect();
    rows.sort_by(|left, right| {
        right
            .unique_hit_count
            .cmp(&left.unique_hit_count)
            .then_with(|| right.accepted_hit_count.cmp(&left.accepted_hit_count))
            .then_with(|| left.variant_id.cmp(&right.variant_id))
    });
    rows
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
