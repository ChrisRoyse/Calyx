//! `calyx-search` — the shared search index + query stack extracted from the CLI
//! (issue #573) so BOTH `calyx` (CLI) and `calyx-web-api` (`/v1/search`) run the
//! exact same Sextant recall → fusion → rerank → provenance path. No mocks, no
//! duplicated logic.
#![deny(warnings)]

pub mod engine;
pub mod error;
pub mod filters;
pub mod persisted;
mod provenance;

pub use engine::{
    FusionChoice, GuardChoice, SearchOutcome, measure_query_vectors, search_outcome,
    search_outcome_with_slots,
};
pub use error::{CliResult, SearchError};
pub use persisted::{PersistedSearchIndexes, load_docs, rebuild_for_vault};
