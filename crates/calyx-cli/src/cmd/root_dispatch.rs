use crate::error::CliResult;

use super::command_types::Subcommand;
use super::known_commands::is_cmd;
use super::{
    association_validation, biomedical_blindspot_audit, bridge_corpus, build_info, chain_walks,
    citation_overlay, citation_part_attribution, courtlistener_locator_audit, discovery_bridge,
    discovery_chain, discovery_run, domain_bridges, erase, evidence_substrate, graph_csr,
    graph_lifecycle, healthcheck, hypothesis_evaluate, hypothesis_evaluator, hypothesis_evidence,
    hypothesis_falsification, hypothesis_rank, ingest, intelligence, judge_association_overlay,
    kernel_build, lincs_reversal, molecular_vault, novelty_split, opinion_alias_overlay,
    probe_matrix, provenance, readback, search, spectral_communities, summary_attribution,
    typed_association_miner, vault, vault_rebuild_preflight, weave, weave_generation_accept,
};

pub(crate) fn try_run(args: &[String]) -> Option<CliResult> {
    for direct in [
        crate::grounding_batch_command::try_run,
        readback::try_run,
        healthcheck::try_run,
        build_info::try_run,
        discovery_run::try_run,
        discovery_bridge::try_run,
        novelty_split::try_run,
        hypothesis_evaluate::try_run,
        hypothesis_evaluator::try_run,
        hypothesis_rank::try_run,
        opinion_alias_overlay::try_run,
        judge_association_overlay::try_run,
        citation_part_attribution::try_run,
        summary_attribution::try_run,
        courtlistener_locator_audit::try_run,
        vault_rebuild_preflight::try_run,
    ] {
        if let Some(result) = direct(args) {
            return Some(result);
        }
    }
    if !args.first().is_some_and(|command| is_cmd(command)) {
        return None;
    }
    if let [command, flag] = args
        && matches!(flag.as_str(), "--help" | "-h")
    {
        return Some(crate::usage::print_command_usage(command));
    }
    let verify_chain = args
        .first()
        .is_some_and(|command| command == "verify-chain");
    let ledger_form = args.get(1).is_some_and(|arg| arg == "--ledger");
    let vault_range_form =
        args.get(1).is_some_and(|arg| arg == "--vault") && args.iter().any(|arg| arg == "--range");
    let verify_chain_legacy = verify_chain && (ledger_form || vault_range_form);
    if verify_chain_legacy {
        return None;
    }
    Some(super::parse(args).and_then(run))
}

fn run(command: Subcommand) -> CliResult {
    match command {
        Subcommand::CreateVault(_)
        | Subcommand::AddLens(_)
        | Subcommand::RetireLens(_)
        | Subcommand::ParkLens(_)
        | Subcommand::RetireVault(_)
        | Subcommand::ListPanel(_)
        | Subcommand::ProfileLens(_) => vault::run(command),
        Subcommand::Ingest(_)
        | Subcommand::IngestStatus(_)
        | Subcommand::Anchor(_)
        | Subcommand::Measure(_) => ingest::run(command),
        Subcommand::Erase(_) => erase::run(command),
        Subcommand::Search(_) | Subcommand::KernelAnswer(_) | Subcommand::RebuildSearchIndex(_) => {
            search::run(command)
        }
        Subcommand::Bits(_)
        | Subcommand::Kernel(_)
        | Subcommand::Guard(_)
        | Subcommand::Abundance(_)
        | Subcommand::ProposeLens(_) => intelligence::run(command),
        Subcommand::Provenance(_)
        | Subcommand::VerifyChain(_)
        | Subcommand::Reproduce(_)
        | Subcommand::AnnealStatus(_) => provenance::run(command),
        Subcommand::KernelBuild(_) => kernel_build::run(command),
        Subcommand::WeaveLoom(_) => weave::run(command),
        Subcommand::DomainBridges(_) => domain_bridges::run(command),
        Subcommand::MaterializeBridgeCorpus(_) => bridge_corpus::run(command),
        Subcommand::MaterializeCitationOverlay(_) => citation_overlay::run(command),
        Subcommand::DiscoveryChain(_) => discovery_chain::run(command),
        Subcommand::ChainWalks(_) => chain_walks::run(command),
        Subcommand::ProbeMatrix(_) => probe_matrix::run(command),
        Subcommand::SpectralCommunities(_) => spectral_communities::run(command),
        Subcommand::MaterializeGraphCsr(_) => graph_csr::run(command),
        Subcommand::GraphCollectionGenerations(_)
        | Subcommand::GraphCollectionState(_)
        | Subcommand::GraphGenerationReclaim(_) => graph_lifecycle::run(command),
        Subcommand::WeaveGenerationVerifyAccept(_) => weave_generation_accept::run(command),
        Subcommand::MaterializeMolecularVault(_) => molecular_vault::run(command),
        Subcommand::MaterializeEvidenceSubstrate(_) => evidence_substrate::run(command),
        Subcommand::MaterializeLincsReversal(_) => lincs_reversal::run(command),
        Subcommand::AssembleHypothesisEvidence(_) => hypothesis_evidence::run(command),
        Subcommand::AssociationValidationGates(_) => association_validation::run(command),
        Subcommand::TypedAssociationMiner(_) => typed_association_miner::run(command),
        Subcommand::HypothesisFalsificationSweep(_) => hypothesis_falsification::run(command),
        Subcommand::BiomedicalBlindspotAudit(_) => biomedical_blindspot_audit::run(command),
    }
}
