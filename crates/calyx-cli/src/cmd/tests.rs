use proptest::prelude::*;

use calyx_core::Modality;

use super::*;

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
fn parse_ingest_text_command() {
    let parsed = parse(&tokens(["ingest", "mydb", "--text", "hello"])).unwrap();
    assert_eq!(
        parsed,
        Subcommand::Ingest(IngestArgs {
            vault: "mydb".to_string(),
            text: Some("hello".to_string()),
            batch: None,
            file: None,
            modality: None,
            idempotent: true,
            output: IngestOutput::Summary,
        })
    );
}

#[test]
fn parse_ingest_video_file_command() {
    let parsed = parse(&tokens([
        "ingest",
        "media",
        "--file",
        "clip.webm",
        "--modality",
        "video",
    ]))
    .unwrap();
    assert_eq!(
        parsed,
        Subcommand::Ingest(IngestArgs {
            vault: "media".to_string(),
            text: None,
            batch: None,
            file: Some("clip.webm".into()),
            modality: Some(Modality::Video),
            idempotent: true,
            output: IngestOutput::Summary,
        })
    );
}

#[test]
fn parse_ingest_rows_output_command() {
    let parsed = parse(&tokens([
        "ingest",
        "mydb",
        "--batch",
        "batch.jsonl",
        "--output",
        "rows",
    ]))
    .unwrap();
    assert_eq!(
        parsed,
        Subcommand::Ingest(IngestArgs {
            vault: "mydb".to_string(),
            text: None,
            batch: Some("batch.jsonl".into()),
            file: None,
            modality: None,
            idempotent: true,
            output: IngestOutput::Rows,
        })
    );
}

#[test]
fn parse_ingest_rejects_unknown_output_mode() {
    let err = parse(&tokens([
        "ingest",
        "mydb",
        "--batch",
        "batch.jsonl",
        "--output",
        "verbose",
    ]))
    .unwrap_err();

    assert_eq!(err.code(), "CALYX_CLI_USAGE_ERROR");
    assert!(err.message().contains("summary or rows"));
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
        })
    );
    assert!(try_run(&tokens(["verify-chain", "--vault", "legacy"])).is_none());
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
        safe_name().prop_map(|vault| Subcommand::Ingest(IngestArgs {
            vault,
            text: Some("roundtrip".to_string()),
            batch: None,
            file: None,
            modality: None,
            idempotent: true,
            output: IngestOutput::Summary,
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
            })
        ),
        safe_name().prop_map(|vault| Subcommand::Reproduce(provenance::ReproduceArgs {
            vault,
            answer_id: "answer-1".to_string(),
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
                guard: calyx_search::GuardChoice::Off,
                out: None,
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
