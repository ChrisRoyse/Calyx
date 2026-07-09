use super::*;
use proptest::prelude::*;

mod ingest_flag_parse;
mod ingest_session_parse;
mod token_roundtrip;

#[test]
fn parse_create_vault_without_template() {
    let parsed = parse(&tokens(["create-vault", "mydb"])).unwrap();
    assert_eq!(
        parsed,
        Subcommand::CreateVault(CreateVaultArgs {
            name: "mydb".to_string(),
            panel_template: None,
        })
    );
}

#[test]
fn parse_add_lens_populates_required_and_optional_fields() {
    let parsed = parse(&tokens([
        "add-lens",
        "mydb",
        "--name",
        "gte",
        "--runtime",
        "tei-http",
        "--endpoint",
        "http://localhost:8088",
        "--shape",
        "Dense(768)",
    ]))
    .unwrap();
    assert_eq!(
        parsed,
        Subcommand::AddLens(AddLensArgs {
            vault: "mydb".to_string(),
            name: "gte".to_string(),
            runtime: "tei-http".to_string(),
            endpoint: Some("http://localhost:8088".to_string()),
            weights: None,
            shape: Some("Dense(768)".to_string()),
            modality: None,
        })
    );
}

#[test]
fn retire_lens_missing_slot_is_usage_error() {
    let err = parse(&tokens(["retire-lens", "mydb"])).unwrap_err();
    assert_eq!(err.code(), "CALYX_CLI_USAGE_ERROR");
}

#[test]
fn parse_anchor_label_command() {
    let parsed = parse(&tokens([
        "anchor",
        "mydb",
        "00000000000000000000000000000000",
        "--kind",
        "label:positive",
        "--value",
        "positive",
        "--confidence",
        "0.75",
        "--source",
        "unit",
    ]))
    .unwrap();
    assert_eq!(
        parsed,
        Subcommand::Anchor(AnchorArgs {
            vault: "mydb".to_string(),
            cx_id: "00000000000000000000000000000000".to_string(),
            kind: "label:positive".to_string(),
            value: "positive".to_string(),
            confidence: Some(0.75),
            source: Some("unit".to_string()),
        })
    );
}

#[test]
fn parse_measure_rejects_empty_text() {
    let err = parse(&tokens(["measure", "mydb", "--text", ""])).unwrap_err();
    assert_eq!(err.code(), "CALYX_CLI_USAGE_ERROR");
}

#[test]
fn parse_provenance_ops_commands() {
    assert_eq!(
        parse(&tokens([
            "provenance",
            "mydb",
            "00000000000000000000000000000000",
        ]))
        .unwrap(),
        Subcommand::Provenance(provenance::ProvenanceArgs {
            vault: "mydb".to_string(),
            cx_id: "00000000000000000000000000000000".to_string(),
        })
    );
    assert_eq!(
        parse(&tokens([
            "verify-chain",
            "mydb",
            "--from",
            "1",
            "--to",
            "3",
        ]))
        .unwrap(),
        Subcommand::VerifyChain(provenance::VerifyChainArgs {
            vault: "mydb".to_string(),
            from: Some(1),
            to: Some(3),
            progress_jsonl: None,
            time_budget_ms: None,
            batch_size: 8192,
        })
    );
    assert_eq!(
        parse(&tokens(["reproduce", "--record", "mydb", "answer-1"])).unwrap(),
        Subcommand::Reproduce(provenance::ReproduceArgs {
            vault: "mydb".to_string(),
            answer_id: "answer-1".to_string(),
            record: true,
        })
    );
    assert_eq!(
        parse(&tokens(["reproduce", "mydb", "answer-1"])).unwrap(),
        Subcommand::Reproduce(provenance::ReproduceArgs {
            vault: "mydb".to_string(),
            answer_id: "answer-1".to_string(),
            record: false,
        })
    );
}

#[test]
fn known_subcommand_help_bypasses_required_arg_validation() {
    let result = try_run(&tokens(["probe-matrix", "--help"]))
        .expect("known command should be handled by cmd dispatcher");
    assert!(result.is_ok(), "{result:?}");

    let result =
        try_run(&tokens(["probe-matrix", "-h"])).expect("short help flag should be handled");
    assert!(result.is_ok(), "{result:?}");
}

#[test]
fn probe_matrix_missing_frontier_still_fails_closed_through_dispatcher() {
    let result = try_run(&tokens(["probe-matrix", "corpus", "--top-k", "3"]))
        .expect("known command should be handled by cmd dispatcher");
    let err = result.unwrap_err();

    assert_eq!(err.code(), "CALYX_CLI_USAGE_ERROR");
    assert!(err.message().contains("--frontier"), "{}", err.message());
}

#[test]
fn parse_erase_command() {
    assert_eq!(
        parse(&tokens([
            "erase",
            "mydb",
            "--cx-id",
            "00000000000000000000000000000000",
            "--fsv-out",
            "target/fsv/erase.json",
        ]))
        .unwrap(),
        Subcommand::Erase(erase::EraseArgs {
            vault: "mydb".to_string(),
            cx_id: "00000000000000000000000000000000".to_string(),
            fsv_out: Some("target/fsv/erase.json".into()),
        })
    );
}

#[test]
fn unsafe_panel_template_selector_is_usage_error_with_remediation_values() {
    let err = parse(&tokens([
        "create-vault",
        "mydb",
        "--panel-template",
        "../unknown-default",
    ]))
    .unwrap_err();
    assert_eq!(err.code(), "CALYX_CLI_USAGE_ERROR");
    assert!(err.message().contains("text-default"));
    assert!(err.message().contains("media-default"));
}

#[test]
fn vault_name_with_spaces_is_usage_error() {
    let err = parse(&tokens(["create-vault", "my db"])).unwrap_err();
    assert_eq!(err.code(), "CALYX_CLI_USAGE_ERROR");
}

proptest! {
    #[test]
fn vault_subcommands_round_trip(command in arb_subcommand()) {
        let tokens = token_roundtrip::subcommand_tokens(&command);
        prop_assert_eq!(parse(&tokens).unwrap(), command);
    }
}

fn arb_subcommand() -> impl Strategy<Value = Subcommand> {
    prop_oneof![
        safe_name().prop_map(|name| {
            Subcommand::CreateVault(CreateVaultArgs {
                name,
                panel_template: None,
            })
        }),
        (safe_name(), safe_name()).prop_map(|(vault, name)| {
            Subcommand::AddLens(AddLensArgs {
                vault,
                name,
                runtime: "algorithmic".to_string(),
                endpoint: None,
                weights: None,
                shape: Some("Dense(16)".to_string()),
                modality: Some("text".to_string()),
            })
        }),
        (safe_name(), 0u16..128u16)
            .prop_map(|(vault, slot)| { Subcommand::RetireLens(SlotCommandArgs { vault, slot }) }),
        (safe_name(), 0u16..128u16)
            .prop_map(|(vault, slot)| { Subcommand::ParkLens(SlotCommandArgs { vault, slot }) }),
        safe_name().prop_map(|vault| Subcommand::ListPanel(VaultRefArgs { vault })),
        safe_name().prop_map(|name| {
            Subcommand::MaterializeBridgeCorpus(bridge_corpus::MaterializeBridgeCorpusArgs {
                name,
                rows: "rows.jsonl".into(),
                home: Some("target/home".into()),
            })
        }),
        safe_name().prop_map(|vault| Subcommand::Ingest(IngestArgs {
            vault,
            text: Some("roundtrip".to_string()),
            batch: None,
            file: None,
            modality: None,
            idempotent: true,
            output: IngestOutput::Summary,
            resident_addr: None,
            allow_cold_gpu_workers: false,
            session_id: None,
        })),
        safe_name().prop_map(|vault| Subcommand::Measure(MeasureArgs {
            vault,
            text: "roundtrip".to_string(),
        })),
        safe_name().prop_map(|vault| Subcommand::Anchor(AnchorArgs {
            vault,
            cx_id: "00000000000000000000000000000000".to_string(),
            kind: "label:roundtrip".to_string(),
            value: "roundtrip".to_string(),
            confidence: None,
            source: None,
        })),
        safe_name().prop_map(|vault| Subcommand::Search(search::SearchArgs {
            vault,
            query: "roundtrip".to_string(),
            k: 5,
            fusion: search::SearchFusionArg::Rrf,
            guard: search::SearchGuardArg::Off,
            explain: false,
            provenance: true,
            freshness: search::SearchFreshnessArg::Fresh,
            filter: None,
            resident_addr: None,
        })),
        safe_name().prop_map(|vault| Subcommand::KernelAnswer(search::KernelAnswerArgs {
            vault,
            query: "roundtrip".to_string(),
            anchor: None,
            explain: false,
        })),
        safe_name().prop_map(|vault| Subcommand::Bits(intelligence::BitsArgs {
            vault,
            anchor_kind: "test-pass".to_string(),
            explain: true,
        })),
        safe_name().prop_map(|vault| Subcommand::Kernel(intelligence::KernelArgs {
            vault,
            anchor: Some("test-pass".to_string()),
            rebuild: true,
        })),
        safe_name().prop_map(|vault| Subcommand::Abundance(intelligence::AbundanceArgs { vault })),
        safe_name().prop_map(
            |vault| Subcommand::ProposeLens(intelligence::ProposeLensArgs {
                vault,
                anchor: "test-pass".to_string(),
            })
        ),
        safe_name().prop_map(|vault| Subcommand::Provenance(provenance::ProvenanceArgs {
            vault,
            cx_id: "00000000000000000000000000000000".to_string(),
        })),
        safe_name().prop_map(
            |vault| Subcommand::VerifyChain(provenance::VerifyChainArgs {
                vault,
                from: Some(0),
                to: Some(1),
                progress_jsonl: None,
                time_budget_ms: None,
                batch_size: 8192,
            })
        ),
        safe_name().prop_map(|vault| Subcommand::Reproduce(provenance::ReproduceArgs {
            vault,
            answer_id: "answer-1".to_string(),
            record: false,
        })),
        safe_name()
            .prop_map(|vault| Subcommand::AnnealStatus(provenance::AnnealStatusArgs { vault })),
        safe_name().prop_map(|vault| Subcommand::Guard(intelligence::GuardArgs {
            vault,
            command: intelligence::GuardCommand::Check {
                cx_id: "00000000000000000000000000000000".to_string(),
                identity_cx: None,
            },
        })),
        safe_name().prop_map(|vault| {
            Subcommand::AssembleHypothesisEvidence(hypothesis_evidence::HypothesisEvidenceArgs {
                vault,
                chain: "chain.json".into(),
                out: "eval-input.json".into(),
            })
        }),
        Just(Subcommand::ProfileLens(ProfileLensArgs {
            name: Some("probe".to_string()),
            runtime: Some("algorithmic".to_string()),
            endpoint: None,
            weights: None,
            shape: Some("Dense(16)".to_string()),
            modality: Some("text".to_string()),
            probe: None,
        })),
        (safe_name(), 1usize..64).prop_map(|(vault, top_k)| {
            Subcommand::KernelBuild(kernel_build::KernelBuildArgs {
                vault,
                held_out_fraction: 0.5,
                top_k,
                min_recall: 0.95,
            })
        }),
        (safe_name(), 1usize..16).prop_map(|(vault, top_k)| {
            Subcommand::ProbeMatrix(probe_matrix::ProbeMatrixArgs {
                vault,
                frontier: "roundtrip frontier".to_string(),
                slots: vec![calyx_core::SlotId::new(8)],
                weighted_profiles: vec![calyx_sextant::RrfProfile::Bridge],
                phrasings: vec![calyx_lodestar::ProbePhrasing::Clinical],
                lengths: vec![calyx_lodestar::ProbeLength::Phrase],
                top_k,
                ..probe_matrix::ProbeMatrixArgs::default()
            })
        }),
        (
            safe_name(),
            1usize..64,
            1usize..8,
            1usize..1024,
            0usize..1000,
        )
            .prop_map(|(vault, knn, max_groundedness_distance, batch, limit)| {
                Subcommand::WeaveLoom(weave::WeaveLoomArgs {
                    vault,
                    content_slot: None,
                    knn,
                    edge_cos_threshold: 0.5,
                    max_groundedness_distance,
                    batch,
                    limit,
                    candidate_selection: weave::CandidateSelectionMode::Covered,
                    coverage_only: false,
                    time_budget_ms: None,
                })
            }),
    ]
}

fn safe_name() -> impl Strategy<Value = String> {
    "[a-z][a-z0-9_-]{0,12}".prop_map(|s| s)
}

fn tokens<const N: usize>(items: [&str; N]) -> Vec<String> {
    items.into_iter().map(str::to_string).collect()
}
