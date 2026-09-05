use crate::error::{CliError, CliResult};

use super::command_types::{CreateVaultArgs, SlotCommandArgs, Subcommand, VaultRefArgs};
use super::lens_parse::{parse_add_lens, parse_profile_lens};
use super::parse_helpers::{validate_panel_template_name, validate_vault_name, value};
use super::{
    association_validation, biomedical_blindspot_audit, bridge_corpus, chain_walks,
    citation_overlay, discovery_chain, domain_bridges, erase, evidence_substrate, graph_csr,
    graph_lifecycle, hypothesis_evidence, hypothesis_falsification, ingest, intelligence,
    kernel_build, lincs_reversal, molecular_vault, probe_matrix, provenance, search,
    spectral_communities, typed_association_miner, vault_retire, weave, weave_generation_accept,
};

pub(crate) fn parse(args: &[String]) -> CliResult<Subcommand> {
    let (command, rest) = args
        .split_first()
        .ok_or_else(|| CliError::usage("missing command"))?;
    match command.as_str() {
        "create-vault" => parse_create_vault(rest),
        "add-lens" => parse_add_lens(rest),
        "retire-lens" => parse_slot_command(rest).map(Subcommand::RetireLens),
        "park-lens" => parse_slot_command(rest).map(Subcommand::ParkLens),
        "retire-vault" => vault_retire::parse_retire_vault(rest),
        "list-panel" => parse_vault_ref(rest).map(Subcommand::ListPanel),
        "profile-lens" => parse_profile_lens(rest),
        "ingest" => ingest::parse_ingest(rest),
        "ingest-status" => ingest::parse_ingest_status(rest),
        "anchor" => ingest::parse_anchor(rest),
        "measure" => ingest::parse_measure(rest),
        "erase" => erase::parse_erase(rest),
        "search" => search::parse_search(rest),
        "kernel-answer" => search::parse_kernel_answer(rest),
        "bits" => intelligence::parse_bits(rest),
        "kernel" => intelligence::parse_kernel(rest),
        "guard" => intelligence::parse_guard(rest),
        "abundance" => intelligence::parse_abundance(rest),
        "propose-lens" => intelligence::parse_propose_lens(rest),
        "provenance" => provenance::parse_provenance(rest),
        "verify-chain" => provenance::parse_verify_chain(rest),
        "reproduce" => provenance::parse_reproduce(rest),
        "anneal-status" => provenance::parse_anneal_status(rest),
        "rebuild-search-index" => parse_vault_ref(rest).map(Subcommand::RebuildSearchIndex),
        "kernel-build" => kernel_build::parse_kernel_build(rest),
        "weave-loom" => weave::parse_weave_loom(rest),
        "domain-bridges" => domain_bridges::parse_domain_bridges(rest),
        "materialize-bridge-corpus" => bridge_corpus::parse_materialize_bridge_corpus(rest),
        "materialize-citation-overlay" => {
            citation_overlay::parse_materialize_citation_overlay(rest)
        }
        "discovery-chain" => discovery_chain::parse_discovery_chain(rest),
        "chain-walks" => chain_walks::parse_chain_walks(rest),
        "probe-matrix" => probe_matrix::parse_probe_matrix(rest),
        "spectral-communities" => spectral_communities::parse_spectral_communities(rest),
        "materialize-graph-csr" => graph_csr::parse_materialize_graph_csr(rest),
        "graph-collection-generations" => graph_lifecycle::parse_graph_collection_generations(rest),
        "graph-collection-state" => graph_lifecycle::parse_graph_collection_state(rest),
        "graph-generation-reclaim" => graph_lifecycle::parse_graph_generation_reclaim(rest),
        "weave-generation-verify-accept" => {
            weave_generation_accept::parse_weave_generation_verify_accept(rest)
        }
        "materialize-molecular-vault" => molecular_vault::parse_materialize_molecular_vault(rest),
        "materialize-evidence-substrate" => {
            evidence_substrate::parse_materialize_evidence_substrate(rest)
        }
        "materialize-lincs-reversal" => lincs_reversal::parse_materialize_lincs_reversal(rest),
        "assemble-hypothesis-evidence" => {
            hypothesis_evidence::parse_assemble_hypothesis_evidence(rest)
        }
        "association-validation-gates" => {
            association_validation::parse_association_validation_gates(rest)
        }
        "typed-association-miner" => typed_association_miner::parse_typed_association_miner(rest),
        "hypothesis-falsification-sweep" => {
            hypothesis_falsification::parse_hypothesis_falsification_sweep(rest)
        }
        "biomedical-blindspot-audit" => {
            biomedical_blindspot_audit::parse_biomedical_blindspot_audit(rest)
        }
        other => Err(CliError::usage(format!("unknown PH62 command {other}"))),
    }
}

fn parse_create_vault(rest: &[String]) -> CliResult<Subcommand> {
    let name = rest
        .first()
        .ok_or_else(|| CliError::usage("create-vault requires <name>"))?
        .clone();
    validate_vault_name(&name)?;
    let mut panel_template = None;
    let mut idx = 1;
    while idx < rest.len() {
        match rest[idx].as_str() {
            "--panel-template" => {
                idx += 1;
                let value = value(rest, idx, "--panel-template")?;
                validate_panel_template_name(value)?;
                panel_template = Some(value.to_string());
            }
            other => {
                return Err(CliError::usage(format!(
                    "unexpected create-vault flag {other}"
                )));
            }
        }
        idx += 1;
    }
    Ok(Subcommand::CreateVault(CreateVaultArgs {
        name,
        panel_template,
    }))
}

fn parse_slot_command(rest: &[String]) -> CliResult<SlotCommandArgs> {
    let vault = rest
        .first()
        .ok_or_else(|| CliError::usage("lens lifecycle command requires <vault>"))?
        .clone();
    let mut slot = None;
    let mut idx = 1;
    while idx < rest.len() {
        match rest[idx].as_str() {
            "--slot" => {
                idx += 1;
                let raw = value(rest, idx, "--slot")?;
                slot = Some(
                    raw.parse::<u16>()
                        .map_err(|err| CliError::usage(format!("parse --slot {raw}: {err}")))?,
                );
            }
            other => {
                return Err(CliError::usage(format!(
                    "unexpected lifecycle flag {other}"
                )));
            }
        }
        idx += 1;
    }
    Ok(SlotCommandArgs {
        vault,
        slot: slot
            .ok_or_else(|| CliError::usage("lens lifecycle command requires --slot <u16>"))?,
    })
}

fn parse_vault_ref(rest: &[String]) -> CliResult<VaultRefArgs> {
    match rest {
        [vault] => Ok(VaultRefArgs {
            vault: vault.clone(),
        }),
        _ => Err(CliError::usage("list-panel requires exactly <vault>")),
    }
}
