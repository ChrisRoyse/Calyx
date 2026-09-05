use std::net::SocketAddr;
use std::path::PathBuf;

use calyx_aster::vault::IngestPrecondition;
use calyx_core::Modality;

use super::{
    association_validation, biomedical_blindspot_audit, bridge_corpus, chain_walks,
    citation_overlay, discovery_chain, domain_bridges, erase, evidence_substrate, graph_csr,
    graph_lifecycle, hypothesis_evidence, hypothesis_falsification, ingest, intelligence,
    kernel_build, lincs_reversal, molecular_vault, probe_matrix, provenance, search,
    spectral_communities, typed_association_miner, vault_retire, weave, weave_generation_accept,
};

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Subcommand {
    CreateVault(CreateVaultArgs),
    AddLens(AddLensArgs),
    RetireLens(SlotCommandArgs),
    ParkLens(SlotCommandArgs),
    RetireVault(vault_retire::RetireVaultArgs),
    ListPanel(VaultRefArgs),
    ProfileLens(ProfileLensArgs),
    Ingest(IngestArgs),
    IngestStatus(ingest::IngestStatusArgs),
    Anchor(AnchorArgs),
    Measure(MeasureArgs),
    Erase(erase::EraseArgs),
    Search(search::SearchArgs),
    KernelAnswer(search::KernelAnswerArgs),
    Bits(intelligence::BitsArgs),
    Kernel(intelligence::KernelArgs),
    Guard(intelligence::GuardArgs),
    Abundance(intelligence::AbundanceArgs),
    ProposeLens(intelligence::ProposeLensArgs),
    Provenance(provenance::ProvenanceArgs),
    VerifyChain(provenance::VerifyChainArgs),
    Reproduce(provenance::ReproduceArgs),
    AnnealStatus(provenance::AnnealStatusArgs),
    RebuildSearchIndex(VaultRefArgs),
    KernelBuild(kernel_build::KernelBuildArgs),
    WeaveLoom(weave::WeaveLoomArgs),
    DomainBridges(domain_bridges::DomainBridgesArgs),
    MaterializeBridgeCorpus(bridge_corpus::MaterializeBridgeCorpusArgs),
    MaterializeCitationOverlay(citation_overlay::MaterializeCitationOverlayArgs),
    DiscoveryChain(discovery_chain::DiscoveryChainArgs),
    ChainWalks(chain_walks::ChainWalksArgs),
    ProbeMatrix(probe_matrix::ProbeMatrixArgs),
    SpectralCommunities(spectral_communities::SpectralCommunitiesArgs),
    MaterializeGraphCsr(graph_csr::MaterializeGraphCsrArgs),
    GraphCollectionGenerations(graph_lifecycle::GraphCollectionGenerationsArgs),
    GraphCollectionState(graph_lifecycle::GraphCollectionStateArgs),
    GraphGenerationReclaim(graph_lifecycle::GraphGenerationReclaimArgs),
    WeaveGenerationVerifyAccept(weave_generation_accept::WeaveGenerationVerifyAcceptArgs),
    MaterializeMolecularVault(molecular_vault::MaterializeMolecularVaultArgs),
    MaterializeEvidenceSubstrate(evidence_substrate::MaterializeEvidenceSubstrateArgs),
    MaterializeLincsReversal(lincs_reversal::MaterializeLincsReversalArgs),
    AssembleHypothesisEvidence(hypothesis_evidence::HypothesisEvidenceArgs),
    AssociationValidationGates(association_validation::AssociationValidationArgs),
    TypedAssociationMiner(typed_association_miner::TypedAssociationMinerArgs),
    HypothesisFalsificationSweep(hypothesis_falsification::HypothesisFalsificationArgs),
    BiomedicalBlindspotAudit(biomedical_blindspot_audit::BiomedicalBlindspotAuditArgs),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CreateVaultArgs {
    pub name: String,
    pub panel_template: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AddLensArgs {
    pub vault: String,
    pub name: String,
    pub runtime: String,
    pub endpoint: Option<String>,
    pub weights: Option<PathBuf>,
    pub shape: Option<String>,
    pub modality: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SlotCommandArgs {
    pub vault: String,
    pub slot: u16,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct VaultRefArgs {
    pub vault: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProfileLensArgs {
    pub name: Option<String>,
    pub runtime: Option<String>,
    pub endpoint: Option<String>,
    pub weights: Option<PathBuf>,
    pub shape: Option<String>,
    pub modality: Option<String>,
    pub probe: Option<PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct IngestArgs {
    pub vault: String,
    pub text: Option<String>,
    pub batch: Option<PathBuf>,
    pub file: Option<PathBuf>,
    pub modality: Option<Modality>,
    pub idempotent: bool,
    pub output: ingest::IngestOutput,
    pub resident_addr: Option<SocketAddr>,
    pub allow_cold_gpu_workers: bool,
    pub session_id: Option<String>,
    pub precondition: IngestPrecondition,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct AnchorArgs {
    pub vault: String,
    pub cx_id: String,
    pub kind: String,
    pub value: String,
    pub confidence: Option<f32>,
    pub source: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MeasureArgs {
    pub vault: String,
    pub text: String,
    pub record_schema: Option<String>,
    pub record: Option<PathBuf>,
}
