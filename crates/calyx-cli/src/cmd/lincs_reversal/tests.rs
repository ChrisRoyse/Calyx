use super::{MaterializeLincsReversalArgs, parse_materialize_lincs_reversal};
use crate::cmd::Subcommand;

#[test]
fn parses_required_root_and_optional_outputs() {
    let args = [
        "medical-vault",
        "--root",
        "/fsv/lincs",
        "--metadata-root",
        "/fsv/lincs-metadata",
        "--collection",
        "biomed_lincs_cmap_reversal_v1",
        "--report",
        "/fsv/readback.json",
        "--home",
        "/home/calyx",
    ]
    .into_iter()
    .map(str::to_string)
    .collect::<Vec<_>>();

    let parsed = parse_materialize_lincs_reversal(&args).expect("parse command");

    assert_eq!(
        parsed,
        Subcommand::MaterializeLincsReversal(MaterializeLincsReversalArgs {
            vault: "medical-vault".to_string(),
            root: "/fsv/lincs".into(),
            metadata_root: Some("/fsv/lincs-metadata".into()),
            collection: Some("biomed_lincs_cmap_reversal_v1".to_string()),
            report: Some("/fsv/readback.json".into()),
            home: Some("/home/calyx".into()),
        })
    );
}

#[test]
fn rejects_missing_root() {
    let args = ["medical-vault"]
        .into_iter()
        .map(str::to_string)
        .collect::<Vec<_>>();

    let error = parse_materialize_lincs_reversal(&args).expect_err("missing root");

    assert!(error.message().contains("--root"));
}
