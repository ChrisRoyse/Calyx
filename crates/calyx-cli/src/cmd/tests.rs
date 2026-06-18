use proptest::prelude::*;

use calyx_core::Modality;

use super::*;

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
        })
    );
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
fn unknown_panel_template_is_usage_error_with_remediation_values() {
    let err = parse(&tokens([
        "create-vault",
        "mydb",
        "--panel-template",
        "unknown-default",
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
        let tokens = command.to_cli_tokens();
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
    ]
}

fn safe_name() -> impl Strategy<Value = String> {
    "[a-z][a-z0-9_-]{0,12}".prop_map(|s| s)
}

fn tokens<const N: usize>(items: [&str; N]) -> Vec<String> {
    items.into_iter().map(str::to_string).collect()
}

impl Subcommand {
    fn to_cli_tokens(&self) -> Vec<String> {
        match self {
            Self::CreateVault(args) => {
                let mut out = vec!["create-vault".to_string(), args.name.clone()];
                if let Some(template) = &args.panel_template {
                    out.extend(["--panel-template".to_string(), template.clone()]);
                }
                out
            }
            Self::AddLens(args) => {
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
            Self::RetireLens(args) => slot_tokens("retire-lens", args),
            Self::ParkLens(args) => slot_tokens("park-lens", args),
            Self::ListPanel(args) => vec!["list-panel".to_string(), args.vault.clone()],
            Self::Ingest(args) => ingest_tokens(args),
            Self::Anchor(args) => anchor_tokens(args),
            Self::Measure(args) => vec![
                "measure".to_string(),
                args.vault.clone(),
                "--text".to_string(),
                args.text.clone(),
            ],
            Self::Search(args) => search::search_tokens(args),
            Self::KernelAnswer(args) => search::kernel_answer_tokens(args),
            Self::Bits(args) => intelligence::bits_tokens(args),
            Self::Kernel(args) => intelligence::kernel_tokens(args),
            Self::Guard(args) => intelligence::guard_tokens(args),
            Self::Abundance(args) => intelligence::abundance_tokens(args),
            Self::ProposeLens(args) => intelligence::propose_lens_tokens(args),
            Self::Provenance(args) => vec![
                "provenance".to_string(),
                args.vault.clone(),
                args.cx_id.clone(),
            ],
            Self::VerifyChain(args) => verify_chain_tokens(args),
            Self::Reproduce(args) => vec![
                "reproduce".to_string(),
                args.vault.clone(),
                args.answer_id.clone(),
            ],
            Self::AnnealStatus(args) => vec!["anneal-status".to_string(), args.vault.clone()],
            Self::ProfileLens(args) => profile_lens_tokens(args),
        }
    }
}

fn verify_chain_tokens(args: &provenance::VerifyChainArgs) -> Vec<String> {
    let mut out = vec!["verify-chain".to_string(), args.vault.clone()];
    if let Some(from) = args.from {
        out.extend(["--from".to_string(), from.to_string()]);
    }
    if let Some(to) = args.to {
        out.extend(["--to".to_string(), to.to_string()]);
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
    out
}

fn modality_name(value: Modality) -> &'static str {
    match value {
        Modality::Audio => "audio",
        Modality::Video => "video",
        _ => "media",
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
