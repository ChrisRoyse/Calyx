use super::{MaterializeEvidenceSubstrateArgs, parse_materialize_evidence_substrate};
use crate::cmd::Subcommand;

#[test]
fn parses_required_roots_and_optional_outputs() {
    let args = [
        "medical-vault",
        "--pubtator-root",
        "/fsv/pubtator",
        "--clinicaltrials-root",
        "/fsv/clinical",
        "--dgidb-root",
        "/fsv/dgidb",
        "--collection",
        "biomed_evidence_substrate",
        "--report",
        "/fsv/readback.json",
        "--home",
        "/home/calyx",
    ]
    .into_iter()
    .map(str::to_string)
    .collect::<Vec<_>>();

    let parsed = parse_materialize_evidence_substrate(&args).expect("parse command");

    assert_eq!(
        parsed,
        Subcommand::MaterializeEvidenceSubstrate(MaterializeEvidenceSubstrateArgs {
            vault: "medical-vault".to_string(),
            pubtator_root: "/fsv/pubtator".into(),
            clinicaltrials_root: "/fsv/clinical".into(),
            dgidb_root: "/fsv/dgidb".into(),
            collection: Some("biomed_evidence_substrate".to_string()),
            report: Some("/fsv/readback.json".into()),
            home: Some("/home/calyx".into()),
        })
    );
}

#[test]
fn rejects_missing_source_root() {
    let args = ["medical-vault"]
        .into_iter()
        .map(str::to_string)
        .collect::<Vec<_>>();

    let error = parse_materialize_evidence_substrate(&args).expect_err("missing roots");

    assert!(error.message().contains("--pubtator-root"));
}
