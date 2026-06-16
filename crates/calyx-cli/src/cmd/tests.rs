use proptest::prelude::*;

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
