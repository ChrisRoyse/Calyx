use calyx_core::{AnchorKind, Modality};

use super::super::*;

mod flows;
use flows::{
    chain_walks_tokens, discovery_chain_tokens, graph_collection_generations_tokens,
    graph_collection_state_tokens, probe_matrix_tokens,
};
mod falsification_tokens;
use falsification_tokens::hypothesis_falsification_tokens;
mod validation_tokens;
use validation_tokens::association_validation_tokens;
mod typed_miner_tokens;
use typed_miner_tokens::typed_association_miner_tokens;
mod biomedical_tokens;
use biomedical_tokens::biomedical_blindspot_audit_tokens;

pub(super) fn subcommand_tokens(command: &Subcommand) -> Vec<String> {
    match command {
        Subcommand::CreateVault(args) => {
            let mut out = vec!["create-vault".to_string(), args.name.clone()];
            if let Some(template) = &args.panel_template {
                out.extend(["--panel-template".to_string(), template.clone()]);
            }
            out
        }
        Subcommand::AddLens(args) => {
            let mut out = vec![
                "add-lens".to_string(),
                args.vault.clone(),
                "--name".to_string(),
                args.name.clone(),
                "--runtime".to_string(),
                args.runtime.clone(),
            ];
            push_opt(&mut out, "--endpoint", args.endpoint.as_deref());
            push_opt(
                &mut out,
                "--weights",
                args.weights.as_ref().and_then(|p| p.to_str()),
            );
            push_opt(&mut out, "--shape", args.shape.as_deref());
            push_opt(&mut out, "--modality", args.modality.as_deref());
            out
        }
        Subcommand::RetireLens(args) => slot_tokens("retire-lens", args),
        Subcommand::ParkLens(args) => slot_tokens("park-lens", args),
        Subcommand::RetireVault(args) => {
            let mut out = vec![
                "retire-vault".to_string(),
                args.vault.clone(),
                "--reason".to_string(),
                args.reason.clone(),
            ];
            push_opt(&mut out, "--superseded-by", args.superseded_by.as_deref());
            push_opt(&mut out, "--source-issue", args.source_issue.as_deref());
            push_opt(
                &mut out,
                "--fsv-readback",
                args.fsv_readback.as_ref().and_then(|path| path.to_str()),
            );
            push_opt(&mut out, "--fsv-sha256", args.fsv_sha256.as_deref());
            out
        }
        Subcommand::ListPanel(args) => vec!["list-panel".to_string(), args.vault.clone()],
        Subcommand::Ingest(args) => ingest_tokens(args),
        Subcommand::IngestStatus(args) => vec![
            "ingest-status".to_string(),
            args.vault.clone(),
            "--session".to_string(),
            args.session_id.clone(),
        ],
        Subcommand::Anchor(args) => anchor_tokens(args),
        Subcommand::Measure(args) => vec![
            "measure".to_string(),
            args.vault.clone(),
            "--text".to_string(),
            args.text.clone(),
        ],
        Subcommand::Erase(args) => {
            let mut out = vec![
                "erase".to_string(),
                args.vault.clone(),
                "--cx-id".to_string(),
                args.cx_id.clone(),
            ];
            push_opt(
                &mut out,
                "--fsv-out",
                args.fsv_out.as_ref().and_then(|path| path.to_str()),
            );
            out
        }
        Subcommand::Search(args) => search::search_tokens(args),
        Subcommand::KernelAnswer(args) => search::kernel_answer_tokens(args),
        Subcommand::Bits(args) => intelligence::bits_tokens(args),
        Subcommand::Kernel(args) => intelligence::kernel_tokens(args),
        Subcommand::Guard(args) => intelligence::guard_tokens(args),
        Subcommand::Abundance(args) => intelligence::abundance_tokens(args),
        Subcommand::ProposeLens(args) => intelligence::propose_lens_tokens(args),
        Subcommand::Provenance(args) => vec![
            "provenance".to_string(),
            args.vault.clone(),
            args.cx_id.clone(),
        ],
        Subcommand::VerifyChain(args) => verify_chain_tokens(args),
        Subcommand::Reproduce(args) => vec![
            "reproduce".to_string(),
            args.vault.clone(),
            args.answer_id.clone(),
        ],
        Subcommand::AnnealStatus(args) => vec!["anneal-status".to_string(), args.vault.clone()],
        Subcommand::RebuildSearchIndex(args) => {
            vec!["rebuild-search-index".to_string(), args.vault.clone()]
        }
        Subcommand::KernelBuild(args) => vec![
            "kernel-build".to_string(),
            args.vault.clone(),
            "--held-out-fraction".to_string(),
            args.held_out_fraction.to_string(),
            "--top-k".to_string(),
            args.top_k.to_string(),
            "--min-recall".to_string(),
            args.min_recall.to_string(),
        ],
        Subcommand::WeaveLoom(args) => weave_loom_tokens(args),
        Subcommand::DomainBridges(args) => domain_bridges_tokens(args),
        Subcommand::MaterializeBridgeCorpus(args) => bridge_corpus_tokens(args),
        Subcommand::MaterializeMolecularVault(args) => molecular_vault_tokens(args),
        Subcommand::MaterializeEvidenceSubstrate(args) => evidence_substrate_tokens(args),
        Subcommand::MaterializeLincsReversal(args) => lincs_reversal_tokens(args),
        Subcommand::AssembleHypothesisEvidence(args) => hypothesis_evidence::tokens(args),
        Subcommand::AssociationValidationGates(args) => association_validation_tokens(args),
        Subcommand::TypedAssociationMiner(args) => typed_association_miner_tokens(args),
        Subcommand::HypothesisFalsificationSweep(args) => hypothesis_falsification_tokens(args),
        Subcommand::BiomedicalBlindspotAudit(args) => biomedical_blindspot_audit_tokens(args),
        Subcommand::GraphCollectionGenerations(args) => graph_collection_generations_tokens(args),
        Subcommand::GraphCollectionState(args) => graph_collection_state_tokens(args),
        Subcommand::DiscoveryChain(args) => discovery_chain_tokens(args),
        Subcommand::ChainWalks(args) => chain_walks_tokens(args),
        Subcommand::ProbeMatrix(args) => probe_matrix_tokens(args),
        Subcommand::SpectralCommunities(args) => spectral_communities_tokens(args),
        Subcommand::MaterializeGraphCsr(args) => vec![
            "materialize-graph-csr".to_string(),
            args.vault.clone(),
            "--collection".to_string(),
            args.collection.clone(),
        ],
        Subcommand::ProfileLens(args) => profile_lens_tokens(args),
    }
}

fn bridge_corpus_tokens(args: &bridge_corpus::MaterializeBridgeCorpusArgs) -> Vec<String> {
    let mut out = vec![
        "materialize-bridge-corpus".to_string(),
        args.name.clone(),
        "--rows".to_string(),
        args.rows.to_string_lossy().into_owned(),
    ];
    push_opt(
        &mut out,
        "--home",
        args.home.as_ref().and_then(|p| p.to_str()),
    );
    out
}

fn molecular_vault_tokens(args: &molecular_vault::MaterializeMolecularVaultArgs) -> Vec<String> {
    let mut out = vec![
        "materialize-molecular-vault".to_string(),
        args.vault.clone(),
        "--rows".to_string(),
        args.rows.to_string_lossy().into_owned(),
    ];
    push_opt(
        &mut out,
        "--home",
        args.home.as_ref().and_then(|p| p.to_str()),
    );
    out
}

fn evidence_substrate_tokens(
    args: &evidence_substrate::MaterializeEvidenceSubstrateArgs,
) -> Vec<String> {
    let mut out = vec![
        "materialize-evidence-substrate".to_string(),
        args.vault.clone(),
        "--pubtator-root".to_string(),
        args.pubtator_root.to_string_lossy().into_owned(),
        "--clinicaltrials-root".to_string(),
        args.clinicaltrials_root.to_string_lossy().into_owned(),
        "--dgidb-root".to_string(),
        args.dgidb_root.to_string_lossy().into_owned(),
    ];
    push_opt(&mut out, "--collection", args.collection.as_deref());
    push_opt(
        &mut out,
        "--report",
        args.report.as_ref().and_then(|p| p.to_str()),
    );
    push_opt(
        &mut out,
        "--home",
        args.home.as_ref().and_then(|p| p.to_str()),
    );
    out
}

fn lincs_reversal_tokens(args: &lincs_reversal::MaterializeLincsReversalArgs) -> Vec<String> {
    let mut out = vec![
        "materialize-lincs-reversal".to_string(),
        args.vault.clone(),
        "--root".to_string(),
        args.root.to_string_lossy().into_owned(),
    ];
    push_opt(
        &mut out,
        "--metadata-root",
        args.metadata_root.as_ref().and_then(|p| p.to_str()),
    );
    push_opt(&mut out, "--collection", args.collection.as_deref());
    push_opt(
        &mut out,
        "--report",
        args.report.as_ref().and_then(|p| p.to_str()),
    );
    push_opt(
        &mut out,
        "--home",
        args.home.as_ref().and_then(|p| p.to_str()),
    );
    out
}

#[test]
fn lincs_reversal_round_trips_through_tokens() {
    let command =
        Subcommand::MaterializeLincsReversal(lincs_reversal::MaterializeLincsReversalArgs {
            vault: "corpus".to_string(),
            root: "/fsv/lincs".into(),
            metadata_root: Some("/fsv/lincs-metadata".into()),
            collection: Some("biomed_lincs_cmap_reversal_v1".to_string()),
            report: Some("/fsv/readback.json".into()),
            home: Some("/home/calyx".into()),
        });
    let tokens = subcommand_tokens(&command);
    assert_eq!(parse(&tokens).unwrap(), command);
}

#[test]
fn probe_matrix_guard_tau_round_trips_through_tokens() {
    let command = Subcommand::ProbeMatrix(probe_matrix::ProbeMatrixArgs {
        vault: "corpus".to_string(),
        frontier: "guard tau roundtrip".to_string(),
        guard: calyx_search::GuardChoice::InRegion,
        guard_tau: Some(0.72),
        ..probe_matrix::ProbeMatrixArgs::default()
    });
    let tokens = subcommand_tokens(&command);
    let tau_flag = tokens.iter().position(|token| token == "--guard-tau");
    assert!(
        tau_flag.is_some(),
        "tokens must carry --guard-tau: {tokens:?}"
    );
    assert_eq!(tokens[tau_flag.unwrap() + 1], "0.72");
    assert_eq!(parse(&tokens).unwrap(), command);
}

#[test]
fn chain_walks_round_trips_through_tokens() {
    let command = Subcommand::ChainWalks(chain_walks::ChainWalksArgs {
        vault: "corpus".to_string(),
        seed_file: "seeds.json".into(),
        anchors: vec!["00000000000000000000000000000000".parse().unwrap()],
        anchor_files: vec!["anchors.txt".into()],
        max_hops: 4,
        branch_width: 8,
        probe_width: 16,
        max_groundedness_distance: 2,
        min_gate_confidence: 0.5,
        novelty_weight: 0.25,
        max_hypotheses_per_seed: 3,
        min_terminal_confidence: 0.5,
        assay_domain: "issue1205".to_string(),
        assay_anchor: AnchorKind::Label("known-outcome".to_string()),
        out: Some("target/chain-walks.json".into()),
    });
    let tokens = subcommand_tokens(&command);
    assert!(tokens.iter().any(|token| token == "--seed-file"));
    assert_eq!(parse(&tokens).unwrap(), command);
}

fn spectral_communities_tokens(
    args: &spectral_communities::SpectralCommunitiesArgs,
) -> Vec<String> {
    let mut out = vec!["spectral-communities".to_string(), args.vault.clone()];
    out.extend([
        "--eigen-k".to_string(),
        args.eigen_k.to_string(),
        "--eigen-max-iter".to_string(),
        args.eigen_max_iter.to_string(),
        "--centrality-max-iter".to_string(),
        args.centrality_max_iter.to_string(),
        "--centrality-tol".to_string(),
        args.centrality_tol.to_string(),
        "--max-bridge-candidates".to_string(),
        args.max_bridge_candidates.to_string(),
        "--max-centrality-candidates".to_string(),
        args.max_centrality_candidates.to_string(),
    ]);
    push_opt(
        &mut out,
        "--out",
        args.out.as_ref().and_then(|p| p.to_str()),
    );
    out
}

fn domain_bridges_tokens(args: &domain_bridges::DomainBridgesArgs) -> Vec<String> {
    let mut out = vec!["domain-bridges".to_string(), args.vault.clone()];
    for (left, right) in &args.pairs {
        out.extend(["--pair".to_string(), left.clone(), right.clone()]);
    }
    if let Some(kind) = &args.anchor_kind {
        out.extend(["--anchor-kind".to_string(), anchor_kind_name(kind)]);
    }
    out.extend([
        "--min-gate-confidence".to_string(),
        args.min_gate_confidence.to_string(),
        "--max-per-pair".to_string(),
        args.max_per_pair.to_string(),
        "--max-evidence-hops".to_string(),
        args.max_evidence_hops.to_string(),
        "--scope-radius".to_string(),
        args.scope_radius.to_string(),
        "--kernel-target-fraction".to_string(),
        args.kernel_target_fraction.to_string(),
    ]);
    push_opt(
        &mut out,
        "--out",
        args.out.as_ref().and_then(|p| p.to_str()),
    );
    out
}

fn weave_loom_tokens(args: &weave::WeaveLoomArgs) -> Vec<String> {
    let mut out = vec!["weave-loom".to_string(), args.vault.clone()];
    if let Some(slot) = args.content_slot {
        out.extend(["--content-slot".to_string(), slot.to_string()]);
    }
    out.extend(["--knn".to_string(), args.knn.to_string()]);
    out.extend([
        "--edge-cos-threshold".to_string(),
        args.edge_cos_threshold.to_string(),
    ]);
    out.extend([
        "--max-groundedness-distance".to_string(),
        args.max_groundedness_distance.to_string(),
    ]);
    out.extend(["--batch".to_string(), args.batch.to_string()]);
    out.extend(["--limit".to_string(), args.limit.to_string()]);
    out.extend([
        "--candidate-selection".to_string(),
        args.candidate_selection.as_str().to_string(),
    ]);
    if args.coverage_only {
        out.push("--coverage-only".to_string());
    }
    if let Some(time_budget_ms) = args.time_budget_ms {
        out.extend(["--time-budget-ms".to_string(), time_budget_ms.to_string()]);
    }
    out
}

fn verify_chain_tokens(args: &provenance::VerifyChainArgs) -> Vec<String> {
    let mut out = vec!["verify-chain".to_string(), args.vault.clone()];
    if let Some(from) = args.from {
        out.extend(["--from".to_string(), from.to_string()]);
    }
    if let Some(to) = args.to {
        out.extend(["--to".to_string(), to.to_string()]);
    }
    if let Some(progress_jsonl) = &args.progress_jsonl {
        out.extend(["--progress-jsonl".to_string(), progress_jsonl.clone()]);
    }
    if let Some(time_budget_ms) = args.time_budget_ms {
        out.extend(["--time-budget-ms".to_string(), time_budget_ms.to_string()]);
    }
    if args.batch_size != 8192 {
        out.extend(["--batch-size".to_string(), args.batch_size.to_string()]);
    }
    out
}

fn ingest_tokens(args: &IngestArgs) -> Vec<String> {
    let mut out = vec!["ingest".to_string(), args.vault.clone()];
    push_opt(&mut out, "--text", args.text.as_deref());
    push_opt(
        &mut out,
        "--batch",
        args.batch.as_ref().and_then(|path| path.to_str()),
    );
    push_opt(
        &mut out,
        "--file",
        args.file.as_ref().and_then(|path| path.to_str()),
    );
    push_opt(&mut out, "--modality", args.modality.map(modality_name));
    if args.idempotent {
        out.push("--idempotent".to_string());
    }
    match args.output {
        IngestOutput::Summary => {}
        IngestOutput::Rows => out.extend(["--output".to_string(), "rows".to_string()]),
    }
    if let Some(addr) = args.resident_addr {
        out.extend(["--resident-addr".to_string(), addr.to_string()]);
    }
    if args.allow_cold_gpu_workers {
        out.push("--allow-cold-gpu-workers".to_string());
    }
    push_opt(&mut out, "--session-id", args.session_id.as_deref());
    out
}

fn modality_name(value: Modality) -> &'static str {
    match value {
        Modality::Audio => "audio",
        Modality::Video => "video",
        _ => "media",
    }
}

fn anchor_kind_name(kind: &AnchorKind) -> String {
    match kind {
        AnchorKind::Label(value) => format!("label:{value}"),
        AnchorKind::TestPass => "test-pass".to_string(),
        AnchorKind::TieFormed => "tie-formed".to_string(),
        AnchorKind::Thumbs => "thumbs-up".to_string(),
        AnchorKind::Reward => "reward".to_string(),
        AnchorKind::SpeakerMatch => "speaker-match".to_string(),
        AnchorKind::StyleHold => "style-hold".to_string(),
        AnchorKind::Recurrence => "recurrence".to_string(),
    }
}

fn anchor_tokens(args: &AnchorArgs) -> Vec<String> {
    let mut out = vec![
        "anchor".to_string(),
        args.vault.clone(),
        args.cx_id.clone(),
        "--kind".to_string(),
        args.kind.clone(),
        "--value".to_string(),
        args.value.clone(),
    ];
    if let Some(confidence) = args.confidence {
        out.extend(["--confidence".to_string(), confidence.to_string()]);
    }
    push_opt(&mut out, "--source", args.source.as_deref());
    out
}

fn profile_lens_tokens(args: &ProfileLensArgs) -> Vec<String> {
    let mut out = vec!["profile-lens".to_string()];
    push_opt(&mut out, "--name", args.name.as_deref());
    push_opt(&mut out, "--runtime", args.runtime.as_deref());
    push_opt(&mut out, "--endpoint", args.endpoint.as_deref());
    push_opt(
        &mut out,
        "--weights",
        args.weights.as_ref().and_then(|p| p.to_str()),
    );
    push_opt(&mut out, "--shape", args.shape.as_deref());
    push_opt(&mut out, "--modality", args.modality.as_deref());
    push_opt(
        &mut out,
        "--probe",
        args.probe.as_ref().and_then(|p| p.to_str()),
    );
    out
}

fn push_opt(out: &mut Vec<String>, flag: &str, value: Option<&str>) {
    if let Some(value) = value {
        out.extend([flag.to_string(), value.to_string()]);
    }
}

fn slot_tokens(command: &str, args: &SlotCommandArgs) -> Vec<String> {
    vec![
        command.to_string(),
        args.vault.clone(),
        "--slot".to_string(),
        args.slot.to_string(),
    ]
}
