use std::collections::BTreeMap;

use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{
    AnchorKind, CalyxError, Constellation, CxId, Input, Modality, SlotId, SlotState, SlotVector,
    VaultStore,
};
use calyx_registry::{VaultPanelState, load_vault_panel_state};
use calyx_sextant::fusion;
use calyx_sextant::{
    FreshnessTag, FusionContext, FusionStrategy, Hit, ProvenanceSource, RrfProfile,
};

use super::super::Subcommand;
use super::super::ingest::parse_anchor_kind;
use super::super::vault::{ResolvedVault, home_dir, resolve_vault_info, vault_salt};
use super::output;
use super::parse::{DEFAULT_K, KernelAnswerArgs, SearchArgs, SearchFreshnessArg, SearchGuardArg};
use super::persisted::{PersistedSearchIndexes, load_docs};
use crate::error::CliResult;
use crate::output::print_json;

const GUARD_TAU: f32 = 0.999;

pub(super) fn run(command: Subcommand) -> CliResult {
    match command {
        Subcommand::Search(args) => search_command(args),
        Subcommand::KernelAnswer(args) => kernel_answer_command(args),
        _ => unreachable!("non-search command routed to search module"),
    }
}

fn search_command(args: SearchArgs) -> CliResult {
    let outcome = search_outcome(&args)?;
    print_json(&output::render_hits(
        &outcome.hits,
        args.explain,
        args.provenance,
        outcome.guard_tau,
    ))
}

fn kernel_answer_command(args: KernelAnswerArgs) -> CliResult {
    let search_args = SearchArgs {
        vault: args.vault,
        query: args.query,
        k: DEFAULT_K,
        fusion: super::parse::SearchFusionArg::KernelFirst,
        guard: SearchGuardArg::Off,
        explain: args.explain,
        provenance: true,
        freshness: SearchFreshnessArg::Fresh,
        filter: None,
    };
    let anchor = args.anchor.as_deref().map(parse_anchor_kind).transpose()?;
    let resolved = resolve_cli_vault(&search_args.vault)?;
    let vault = open_vault(&resolved)?;
    let docs = load_docs(&vault)?;
    let outcome = search_outcome(&search_args)?;
    let report = kernel_report_from_docs(&docs, &outcome.hits, anchor.as_ref())?;
    print_json(&report)
}

fn search_outcome(args: &SearchArgs) -> CliResult<SearchOutcome> {
    let resolved = resolve_cli_vault(&args.vault)?;
    let vault = open_vault(&resolved)?;
    let state = load_vault_panel_state(&resolved.path)?;
    let filters = super::filters::parse(args.filter.as_deref())?;
    let indexes = match PersistedSearchIndexes::open(&resolved.path) {
        Ok(indexes) => indexes,
        Err(err) if err.code() == "CALYX_STALE_DERIVED" && vault_base_count(&vault)? == 0 => {
            return Ok(SearchOutcome::empty());
        }
        Err(err) => return Err(err),
    };
    if indexes.max_len() == 0 {
        return Ok(SearchOutcome::empty());
    }
    let query_vectors = measure_query_vectors(&state, &args.query)?;
    if query_vectors.is_empty() {
        return Err(no_indexable_query_vectors().into());
    }
    let filter_candidates = indexes.filter_candidates(&filters)?;
    if filter_candidates.as_ref().is_some_and(|ids| ids.is_empty()) {
        return Ok(SearchOutcome::empty());
    }
    let search_k = filter_candidates
        .as_ref()
        .map(|ids| ids.len())
        .unwrap_or_else(|| args.k.max(64));
    let per_slot = search_slots(
        &indexes,
        &query_vectors,
        search_k,
        filter_candidates.as_ref(),
    )?;
    let slots = per_slot.keys().copied().collect::<Vec<_>>();
    if slots.is_empty() {
        return Err(no_indexable_stored_vectors().into());
    }
    let strategy = args.fusion.to_strategy(&slots)?;
    let context = FusionContext {
        k: args.k.max(64),
        explain: args.explain,
        strategy: strategy.clone(),
        weights: weights_for(&strategy, &slots),
        stage1_slots: stage1_slots(&strategy, &query_vectors, &slots),
    };
    let mut hits = fusion::fuse(&per_slot, &context);
    let hit_docs = hit_docs(&vault, &hits)?;
    attach_stored_provenance(&mut hits, &hit_docs, vault.latest_seq())?;
    let guard_tau = if args.guard == SearchGuardArg::InRegion {
        hits = apply_in_region_guard(hits, &hit_docs, &query_vectors)?;
        Some(GUARD_TAU)
    } else {
        None
    };
    renumber_and_truncate(&mut hits, args.k);
    Ok(SearchOutcome { hits, guard_tau })
}

pub(super) fn measure_query_vectors(
    state: &VaultPanelState,
    query: &str,
) -> CliResult<Vec<(SlotId, SlotVector)>> {
    let input = Input::new(Modality::Text, query.as_bytes().to_vec());
    let mut out = Vec::new();
    for slot in &state.panel.slots {
        if slot.state == SlotState::Active
            && slot.modality == Modality::Text
            && state.registry.contains(slot.lens_id)
        {
            let vector = state.registry.measure(slot.lens_id, &input)?;
            if indexable(&vector) {
                out.push((slot.slot_id, vector));
            }
        }
    }
    Ok(out)
}

pub(super) fn no_indexable_query_vectors() -> CalyxError {
    CalyxError::stale_derived(
        "search has no indexable query vectors from active text lenses; re-enable a concrete lens or remeasure the panel",
    )
}

pub(super) fn no_indexable_stored_vectors() -> CalyxError {
    CalyxError::stale_derived(
        "search has no indexable stored slot vectors matching active query lenses; reingest or backfill stale slot rows",
    )
}

fn search_slots(
    indexes: &PersistedSearchIndexes,
    query_vectors: &[(SlotId, SlotVector)],
    k: usize,
    filter_candidates: Option<&std::collections::BTreeSet<CxId>>,
) -> CliResult<BTreeMap<SlotId, Vec<calyx_sextant::IndexSearchHit>>> {
    let mut out = BTreeMap::new();
    for (slot, query) in query_vectors {
        let hits = if let Some(candidates) = filter_candidates {
            indexes.search_filtered(*slot, query, k, candidates)?
        } else {
            indexes.search(*slot, query, k)?
        };
        if !hits.is_empty() {
            out.insert(*slot, hits);
        }
    }
    Ok(out)
}

fn apply_in_region_guard(
    hits: Vec<Hit>,
    docs: &BTreeMap<CxId, Constellation>,
    query_vectors: &[(SlotId, SlotVector)],
) -> CliResult<Vec<Hit>> {
    let mut kept = Vec::new();
    for hit in hits {
        let cosine = guard_cosine(&hit, docs, query_vectors);
        if cosine.is_some_and(|value| value >= GUARD_TAU) {
            kept.push(hit);
        } else {
            output::warn_guard_blocked(hit.cx_id, cosine, GUARD_TAU)?;
        }
    }
    Ok(kept)
}

fn guard_cosine(
    hit: &Hit,
    docs: &BTreeMap<CxId, Constellation>,
    query_vectors: &[(SlotId, SlotVector)],
) -> Option<f32> {
    let cx = docs.get(&hit.cx_id)?;
    hit.per_lens
        .iter()
        .filter_map(|item| {
            let query = query_vectors
                .iter()
                .find(|(slot, _)| *slot == item.slot)?
                .1
                .as_dense()?;
            let doc = cx.slots.get(&item.slot)?.as_dense()?;
            cosine(query, doc)
        })
        .max_by(f32::total_cmp)
}

fn attach_stored_provenance(
    hits: &mut [Hit],
    docs: &BTreeMap<CxId, Constellation>,
    seq: u64,
) -> CliResult {
    for hit in hits {
        let cx = docs.get(&hit.cx_id).ok_or_else(|| {
            CalyxError::vault_access_denied(format!(
                "stored constellation missing for hit {}",
                hit.cx_id
            ))
        })?;
        hit.provenance = cx.provenance.clone();
        hit.provenance_source = ProvenanceSource::Stored;
        hit.freshness = FreshnessTag::fresh(seq);
    }
    Ok(())
}

fn hit_docs(vault: &AsterVault, hits: &[Hit]) -> CliResult<BTreeMap<CxId, Constellation>> {
    let snapshot = vault.snapshot();
    let mut docs = BTreeMap::new();
    for hit in hits {
        let cx_id = hit.cx_id;
        docs.insert(cx_id, vault.get(cx_id, snapshot)?);
    }
    Ok(docs)
}

fn vault_base_count(vault: &AsterVault) -> CliResult<usize> {
    Ok(load_docs(vault)?.len())
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

fn renumber_and_truncate(hits: &mut Vec<Hit>, k: usize) {
    hits.truncate(k);
    for (idx, hit) in hits.iter_mut().enumerate() {
        hit.rank = idx + 1;
    }
}

fn indexable(vector: &SlotVector) -> bool {
    matches!(
        vector,
        SlotVector::Dense { .. } | SlotVector::Sparse { .. } | SlotVector::Multi { .. }
    )
}

fn cosine(left: &[f32], right: &[f32]) -> Option<f32> {
    if left.len() != right.len() || left.is_empty() {
        return None;
    }
    let (mut dot, mut l2, mut r2) = (0.0f32, 0.0f32, 0.0f32);
    for (l, r) in left.iter().zip(right) {
        dot += l * r;
        l2 += l * l;
        r2 += r * r;
    }
    (l2 > 0.0 && r2 > 0.0).then(|| dot / (l2.sqrt() * r2.sqrt()))
}

fn weights_for(strategy: &FusionStrategy, slots: &[SlotId]) -> BTreeMap<SlotId, f32> {
    let Some(profile) = weighted_profile(strategy) else {
        return BTreeMap::new();
    };
    let profile_weights = fusion::profiles::lookup(profile)
        .map(|profile| profile.weights)
        .unwrap_or_default();
    slots
        .iter()
        .map(|slot| (*slot, profile_weights.get(slot).copied().unwrap_or(1.0)))
        .collect()
}

fn weighted_profile(strategy: &FusionStrategy) -> Option<RrfProfile> {
    match strategy {
        FusionStrategy::WeightedRrf { profile } => Some(*profile),
        _ => None,
    }
}

fn stage1_slots(
    strategy: &FusionStrategy,
    query_vectors: &[(SlotId, SlotVector)],
    slots: &[SlotId],
) -> Vec<SlotId> {
    if !matches!(strategy, FusionStrategy::Pipeline) {
        return Vec::new();
    }
    let sparse = query_vectors
        .iter()
        .filter_map(|(slot, vector)| matches!(vector, SlotVector::Sparse { .. }).then_some(*slot))
        .filter(|slot| slots.contains(slot))
        .collect::<Vec<_>>();
    if sparse.is_empty() {
        slots.first().copied().into_iter().collect()
    } else {
        sparse
    }
}

struct SearchOutcome {
    hits: Vec<Hit>,
    guard_tau: Option<f32>,
}

impl SearchOutcome {
    fn empty() -> Self {
        Self {
            hits: Vec::new(),
            guard_tau: None,
        }
    }
}

#[cfg(test)]
pub(super) fn guard_keeps_hit_for_test(
    hit: &Hit,
    docs: &BTreeMap<CxId, Constellation>,
    query_vectors: &[(SlotId, SlotVector)],
) -> bool {
    guard_cosine(hit, docs, query_vectors).is_some_and(|value| value >= GUARD_TAU)
}
