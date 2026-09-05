//! `calyx-search` — the shared search index + query stack extracted from the CLI
//! (issue #573) so BOTH `calyx` (CLI) and `calyx-web-api` (`/v1/search`) run the
//! exact same Sextant recall → fusion → rerank → provenance path. No mocks, no
//! duplicated logic.
#![deny(warnings)]

/// Whether this exact `calyx-search` crate instance compiled its CUDA-gated
/// search paths. Consumers must inspect this owner-level capability instead of
/// inferring it from the transitive Sextant feature.
pub const CUDA_COMPILED: bool = cfg!(feature = "cuda");

pub mod engine;
mod engine_fusion;
mod engine_measure;
mod engine_slot_cache;
mod engine_slot_fanout;
mod engine_trace;
pub mod error;
pub mod filters;
mod metadata_exact;
pub mod persisted;
mod provenance;

pub use engine::{
    DEFAULT_IN_REGION_GUARD_TAU, FusionChoice, GuardChoice, RERANK_CANDIDATE_FLOOR, SearchBudget,
    SearchFreshness, SearchOutcome, SearchRerankReport, SearchSlotCache, SearchSlotCacheDiagnostic,
    SearchTraceEvent, measure_query_vectors, measure_query_vectors_with_slots,
    rerank_search_outcome, search_outcome, search_outcome_with_freshness,
    search_outcome_with_query_vectors, search_outcome_with_query_vectors_freshness,
    search_outcome_with_query_vectors_freshness_cached,
    search_outcome_with_query_vectors_freshness_cached_ledger_view,
    search_outcome_with_query_vectors_freshness_cached_ledger_view_exact_metadata,
    search_outcome_with_slots, search_outcome_with_slots_traced,
};
pub use error::{CliResult, SearchError};
pub use persisted::{
    MarkerClearOutcome, PersistedSearchGeneration, PersistedSearchIndexes, PersistedSearchSlot,
    REBUILD_REQUIRED_REMEDIATION, REBUILD_REQUIRED_SCHEMA, RebuildProgress, RebuildRequiredMarker,
    SEARCH_PIN_BUDGET_EXCEEDED_CODE, SEARCH_PIN_BUDGET_EXCEEDED_REMEDIATION,
    clear_rebuild_required_marker, clear_rebuild_required_marker_if_owned, load_docs,
    read_rebuild_required_marker, rebuild_for_vault, rebuild_for_vault_with_fallible_progress,
    rebuild_for_vault_with_panel_state, rebuild_for_vault_with_panel_state_fallible_progress,
    rebuild_for_vault_with_panel_state_progress, rebuild_for_vault_with_progress,
    rebuild_for_vault_with_stored_panel_state_fallible_progress, rebuild_required_marker_path,
    replace_rebuild_required_marker_if_exact, validate_rebuild_config,
    write_rebuild_required_marker,
};
