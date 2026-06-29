use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use calyx_aster::plain_graph::PlainGraph;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{AnchorKind, CxId, VaultId};
use calyx_lodestar::{
    AsterAssocNodeProps, DEFAULT_ASTER_ASSOC_COLLECTION, DomainBridgeReport,
    encode_assoc_node_props,
};
use serde_json::json;

use super::*;
use crate::cmd::vault::vault_salt;

fn toks(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| s.to_string()).collect()
}

fn parse(parts: &[&str]) -> CliResult<DomainBridgesArgs> {
    match super::parse_domain_bridges(&toks(parts))? {
        Subcommand::DomainBridges(args) => Ok(args),
        _ => unreachable!("parse_domain_bridges must return DomainBridges"),
    }
}

#[test]
fn parses_required_pair_and_grounding_flags() {
    let args = parse(&[
        "corpus",
        "--pair",
        "metadata:domain=clinical",
        "metadata:domain=molecular",
        "--anchor-kind",
        "label:answer",
        "--scope-radius",
        "3",
        "--max-evidence-hops",
        "4",
        "--kernel-target-fraction",
        "1.0",
    ])
    .unwrap();

    assert_eq!(args.vault, "corpus");
    assert_eq!(args.pairs.len(), 1);
    assert_eq!(
        args.anchor_kind,
        Some(AnchorKind::Label("answer".to_string()))
    );
    assert_eq!(args.scope_radius, 3);
    assert_eq!(args.max_evidence_hops, 4);
    assert_eq!(args.kernel_target_fraction, 1.0);
}

#[test]
fn missing_pair_fails_closed() {
    let err = parse(&["corpus", "--anchor-kind", "label:answer"]).unwrap_err();

    assert_eq!(err.code(), "CALYX_CLI_USAGE_ERROR");
    assert!(err.message().contains("at least one --pair"));
}

#[test]
fn run_persists_report_then_reads_back_source_of_truth() {
    let (home, vault_dir) = seed_home("happy", SeedShape::Bridge);

    run_domain_bridges_with_home(
        &home,
        DomainBridgesArgs {
            vault: "happy".to_string(),
            pairs: vec![(
                "metadata:domain=clinical".to_string(),
                "metadata:domain=molecular".to_string(),
            )],
            anchor_kind: Some(AnchorKind::Label("answer".to_string())),
            kernel_target_fraction: 1.0,
            ..DomainBridgesArgs::default()
        },
    )
    .unwrap();

    let report_path = only_report(&vault_dir);
    let readback_bytes = fs::read(&report_path).unwrap();
    let report: DomainBridgeReport = serde_json::from_slice(&readback_bytes).unwrap();

    assert_eq!(report.schema_version, 1);
    assert_eq!(report.pair_reports.len(), 1);
    assert_eq!(report.pair_reports[0].candidate_count, 1);
    assert_eq!(report.pair_reports[0].candidates[0].text, "shared cytokine");
    assert_eq!(
        report.pair_reports[0].candidates[0].cross_domain_distance,
        Some(2)
    );
    assert!(
        report.pair_reports[0].candidates[0]
            .gate
            .evidence
            .iter()
            .any(|item| item.starts_with("left_kernel_id="))
    );
}

#[test]
fn unknown_metadata_scope_fails_before_artifact_write() {
    let (home, vault_dir) = seed_home("missing-scope", SeedShape::Bridge);

    let err = run_domain_bridges_with_home(
        &home,
        DomainBridgesArgs {
            vault: "missing-scope".to_string(),
            pairs: vec![(
                "metadata:domain=clinical".to_string(),
                "metadata:domain=finance".to_string(),
            )],
            anchor_kind: Some(AnchorKind::Label("answer".to_string())),
            kernel_target_fraction: 1.0,
            ..DomainBridgesArgs::default()
        },
    )
    .unwrap_err();

    assert_eq!(err.code(), "CALYX_KERNEL_INVALID_PARAMS");
    assert!(err.message().contains("no source-of-truth root nodes"));
    assert!(!vault_dir.join("idx").join("domain_bridges").exists());
}

#[test]
fn strict_gate_refuses_without_persisting_artifact() {
    let (home, vault_dir) = seed_home("strict-gate", SeedShape::Bridge);

    let err = run_domain_bridges_with_home(
        &home,
        DomainBridgesArgs {
            vault: "strict-gate".to_string(),
            pairs: vec![(
                "metadata:domain=clinical".to_string(),
                "metadata:domain=molecular".to_string(),
            )],
            anchor_kind: Some(AnchorKind::Label("answer".to_string())),
            min_gate_confidence: 0.95,
            kernel_target_fraction: 1.0,
            ..DomainBridgesArgs::default()
        },
    )
    .unwrap_err();

    assert_eq!(err.code(), "CALYX_KERNEL_INVALID_PARAMS");
    assert!(err.message().contains("had only refused"));
    assert!(!vault_dir.join("idx").join("domain_bridges").exists());
}

#[derive(Clone, Copy)]
enum SeedShape {
    Bridge,
}

fn seed_home(name: &str, _shape: SeedShape) -> (PathBuf, PathBuf) {
    let home = std::env::temp_dir().join(format!(
        "calyx-domain-bridges-{name}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&home);
    fs::create_dir_all(home.join("vaults")).unwrap();
    let vault_id = vault_id();
    let vault_dir = home.join("vaults").join(vault_id.to_string());
    fs::write(
        home.join("vaults").join("index.json"),
        serde_json::to_vec_pretty(&json!({
            "vaults": [{
                "name": name,
                "vault_id": vault_id.to_string(),
                "path": format!("vaults/{vault_id}"),
                "panel_template": "text-default"
            }]
        }))
        .unwrap(),
    )
    .unwrap();

    let vault = AsterVault::new_durable(
        &vault_dir,
        vault_id,
        vault_salt(vault_id, name),
        VaultOptions::default(),
    )
    .unwrap();
    let graph = PlainGraph::new(&vault, DEFAULT_ASTER_ASSOC_COLLECTION).unwrap();
    for (seed, domain, term) in [
        (1, "clinical", "clinical seed one"),
        (2, "clinical", "clinical seed two"),
        (3, "molecular", "molecular seed one"),
        (4, "molecular", "molecular seed two"),
        (9, "bridge", "shared cytokine"),
    ] {
        let props = AsterAssocNodeProps {
            embedding: Some(vec![seed as f32, (10 - seed) as f32]),
            anchors: if domain == "bridge" {
                Vec::new()
            } else {
                vec![AnchorKind::Label("answer".to_string())]
            },
            metadata: BTreeMap::from([
                ("domain".to_string(), domain.to_string()),
                ("term".to_string(), term.to_string()),
                ("source_id".to_string(), format!("row-{seed}")),
            ]),
            ..Default::default()
        };
        graph
            .put_node(cx(seed), &encode_assoc_node_props(&props).unwrap())
            .unwrap();
    }
    for (src, dst) in [
        (1, 9),
        (9, 1),
        (2, 9),
        (9, 2),
        (3, 9),
        (9, 3),
        (4, 9),
        (9, 4),
    ] {
        graph.put_edge(cx(src), "assoc", cx(dst), b"1").unwrap();
    }
    vault.flush().unwrap();
    (home, vault_dir)
}

fn only_report(vault_dir: &Path) -> PathBuf {
    let root = vault_dir.join("idx").join("domain_bridges");
    let dirs = fs::read_dir(&root)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(dirs.len(), 1);
    dirs[0].path().join("report.json")
}

fn cx(seed: u8) -> CxId {
    CxId::from_bytes([seed; 16])
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}
