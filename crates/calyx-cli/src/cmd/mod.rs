mod artifact_hash;
mod association_validation;
mod biomedical_blindspot_audit;
mod bridge_corpus;
mod build_info;
mod chain_walks;
mod citation_overlay;
mod citation_part_attribution;
mod command_types;
mod courtlistener_locator_audit;
mod discovery_bridge;
mod discovery_chain;
mod discovery_gate;
mod discovery_run;
pub(crate) mod discovery_run_preflight;
mod domain_bridges;
mod erase;
mod evidence_substrate;
mod graph_csr;
mod graph_lifecycle;
mod healthcheck;
mod hypothesis_evaluate;
mod hypothesis_evaluator;
mod hypothesis_evidence;
mod hypothesis_falsification;
mod hypothesis_rank;
mod ingest;
mod intelligence;
mod judge_association_overlay;
mod kernel_build;
pub(crate) mod kernel_generation;
mod kernel_scope;
mod known_commands;
mod lens;
mod lens_parse;
mod lincs_reversal;
pub(crate) mod mechanistic_direction;
mod molecular_vault;
mod novelty_split;
mod opinion_alias_overlay;
mod panel_templates;
mod parse_helpers;
mod probe_matrix;
mod provenance;
mod readback;
mod root_dispatch;
mod root_parser;
mod search;
mod spectral_communities;
mod summary_attribution;
mod typed_association_miner;
pub(crate) mod vault;
mod vault_rebuild_preflight;
mod vault_retire;
mod weave;
mod weave_generation_accept;

#[cfg(test)]
use calyx_aster::vault::IngestPrecondition;
pub(crate) use command_types::{
    AddLensArgs, AnchorArgs, CreateVaultArgs, IngestArgs, MeasureArgs, ProfileLensArgs,
    SlotCommandArgs, Subcommand, VaultRefArgs,
};
pub(crate) use ingest::run_lens_worker as run_ingest_lens_worker;
#[cfg(test)]
use ingest::{IngestOutput, IngestStatusArgs};
pub(crate) use ingest::{
    measure_constellation as measure_ingest_constellation, text_input as ingest_text_input,
};
pub(crate) use panel_templates::PANEL_TEMPLATES;
pub(crate) use parse_helpers::{validate_panel_template_name, validate_vault_name, value};
pub(crate) use root_dispatch::try_run;
pub(crate) use root_parser::parse;
pub(crate) use search::{
    PersistedSearchIndexes, load_docs as load_search_docs, measure_text_query_vectors,
    rebuild_persistent_indexes,
};
pub(crate) use search::{open_vault as open_search_vault, search_read_cfs};

#[cfg(test)]
mod tests;
