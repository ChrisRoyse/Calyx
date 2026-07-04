use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{
    Constellation, CxFlags, CxId, InputRef, LedgerRef, Modality, VaultId, VaultStore,
};
use calyx_lodestar::{
    AbcHypothesis, CHAIN_WALK_SCHEMA_VERSION, ChainWalkResult, ChainWalkSeed, ChainWalkSeedKind,
    DISCOVERY_CHAIN_SCHEMA_VERSION, DiscoveryChainLog, DiscoveryChainParams, DiscoveryTermination,
};
use ulid::Ulid;

use super::*;
use crate::cmd::vault::vault_salt;

#[test]
fn parse_accepts_positional_vault_and_required_paths() {
    let parsed = parse_assemble_hypothesis_evidence(&[
        "corpus".to_string(),
        "--chain".to_string(),
        "chain.json".to_string(),
        "--out".to_string(),
        "input.json".to_string(),
    ])
    .unwrap();

    assert_eq!(
        parsed,
        Subcommand::AssembleHypothesisEvidence(HypothesisEvidenceArgs {
            vault: "corpus".to_string(),
            chain: "chain.json".into(),
            out: "input.json".into(),
        })
    );
}

#[test]
fn assembles_from_vault_rows_and_readback_matches_base_metadata() {
    let setup = setup_vault(
        "happy",
        &[
            RowSpec::new(1, "source-sha-a", "abstract A"),
            RowSpec::new(2, "source-sha-b", "abstract B"),
            RowSpec::new(3, "source-sha-c", "abstract C"),
        ],
    );
    let chain = setup.home.join("chain.json");
    write_chain_artifact(
        &chain,
        cx(1),
        cx(2),
        cx(3),
        vec![cx(1), cx(2), cx(3), cx(2)],
    );
    let out = setup.home.join("eval-input.json");

    run_assemble_hypothesis_evidence_with_home(
        &setup.home,
        HypothesisEvidenceArgs {
            vault: setup.name.clone(),
            chain,
            out: out.clone(),
        },
    )
    .unwrap();

    let bytes = fs::read(&out).unwrap();
    let input_file: HypothesisEvaluateInputFile = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(input_file.schema_version, 1);
    assert_eq!(input_file.inputs.len(), 1);
    let evidence = &input_file.inputs[0].retrieved_evidence;
    assert_eq!(evidence.len(), 3, "duplicate B must be deduped");
    assert_eq!(
        evidence
            .iter()
            .map(|row| row.source_cx_id)
            .collect::<Vec<_>>(),
        vec![cx(1), cx(2), cx(3)]
    );

    let vault = open_existing_vault(&setup);
    for row in evidence {
        let stored = vault.get(row.source_cx_id, vault.snapshot()).unwrap();
        assert_eq!(
            row.abstract_text, stored.metadata["abstract_text"],
            "evidence text must come from Base metadata"
        );
        let expected_sha = &stored.metadata["source_sha256"];
        assert!(
            row.provenance
                .iter()
                .any(|entry| entry == &format!("source_sha256={expected_sha}")),
            "source_sha256 must be carried through: {:?}",
            row.provenance
        );
    }

    cleanup(setup.home);
}

#[test]
fn missing_base_row_fails_closed_without_output() {
    let setup = setup_vault(
        "missing",
        &[
            RowSpec::new(1, "source-sha-a", "abstract A"),
            RowSpec::new(2, "source-sha-b", "abstract B"),
        ],
    );
    let chain = setup.home.join("chain.json");
    write_chain_artifact(&chain, cx(1), cx(2), cx(3), vec![cx(1), cx(2), cx(3)]);
    let out = setup.home.join("eval-input.json");

    let err = run_assemble_hypothesis_evidence_with_home(
        &setup.home,
        HypothesisEvidenceArgs {
            vault: setup.name.clone(),
            chain,
            out: out.clone(),
        },
    )
    .unwrap_err();

    assert_eq!(err.code(), "CALYX_HYPOTHESIS_EVIDENCE_MISSING_PROVENANCE");
    assert!(!out.exists());
    cleanup(setup.home);
}

#[test]
fn empty_abstract_fails_closed_without_output() {
    let setup = setup_vault(
        "empty-abstract",
        &[
            RowSpec::new(1, "source-sha-a", "abstract A"),
            RowSpec::new(2, "source-sha-b", "abstract B"),
            RowSpec::new(3, "source-sha-c", ""),
        ],
    );
    let chain = setup.home.join("chain.json");
    write_chain_artifact(&chain, cx(1), cx(2), cx(3), vec![cx(1), cx(2), cx(3)]);
    let out = setup.home.join("eval-input.json");

    let err = run_assemble_hypothesis_evidence_with_home(
        &setup.home,
        HypothesisEvidenceArgs {
            vault: setup.name.clone(),
            chain,
            out: out.clone(),
        },
    )
    .unwrap_err();

    assert_eq!(err.code(), "CALYX_HYPOTHESIS_EVIDENCE_EMPTY_ABSTRACT");
    assert!(!out.exists());
    cleanup(setup.home);
}

struct Setup {
    home: PathBuf,
    vault_dir: PathBuf,
    vault_id: VaultId,
    name: String,
}

#[derive(Clone, Copy)]
struct RowSpec {
    byte: u8,
    source_sha256: &'static str,
    abstract_text: &'static str,
}

impl RowSpec {
    const fn new(byte: u8, source_sha256: &'static str, abstract_text: &'static str) -> Self {
        Self {
            byte,
            source_sha256,
            abstract_text,
        }
    }
}

fn setup_vault(name: &str, rows: &[RowSpec]) -> Setup {
    let home = temp_home(name);
    fs::create_dir_all(home.join("vaults")).unwrap();
    let vault_id = VaultId::from_ulid(Ulid::new());
    let vault_dir = home.join("vaults").join(vault_id.to_string());
    let vault = AsterVault::new_durable(
        &vault_dir,
        vault_id,
        vault_salt(vault_id, name),
        VaultOptions::default(),
    )
    .unwrap();
    for row in rows {
        vault.put(constellation(vault_id, *row)).unwrap();
    }
    vault.flush().unwrap();
    drop(vault);
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
    Setup {
        home,
        vault_dir,
        vault_id,
        name: name.to_string(),
    }
}

fn open_existing_vault(setup: &Setup) -> AsterVault {
    AsterVault::new_durable(
        &setup.vault_dir,
        setup.vault_id,
        vault_salt(setup.vault_id, &setup.name),
        VaultOptions::default(),
    )
    .unwrap()
}

fn constellation(vault_id: VaultId, row: RowSpec) -> Constellation {
    let mut metadata = BTreeMap::new();
    metadata.insert("title".to_string(), format!("Title {}", row.byte));
    metadata.insert("abstract_text".to_string(), row.abstract_text.to_string());
    metadata.insert(
        "source_dataset".to_string(),
        "synthetic-issue1200".to_string(),
    );
    metadata.insert("source_id".to_string(), format!("row-{}", row.byte));
    metadata.insert("source_sha256".to_string(), row.source_sha256.to_string());
    metadata.insert("license".to_string(), "synthetic".to_string());
    metadata.insert(
        "retrieval_ts".to_string(),
        "2026-07-04T00:00:00Z".to_string(),
    );
    metadata.insert("pmid".to_string(), format!("1200{}", row.byte));
    Constellation {
        cx_id: cx(row.byte),
        vault_id,
        panel_version: 1,
        created_at: 1_786_000_000 + u64::from(row.byte),
        input_ref: InputRef {
            hash: [row.byte; 32],
            pointer: Some(format!("synthetic://issue1200/{}", row.byte)),
            redacted: false,
        },
        modality: Modality::Text,
        slots: BTreeMap::new(),
        scalars: BTreeMap::new(),
        metadata,
        anchors: Vec::new(),
        provenance: LedgerRef {
            seq: u64::from(row.byte),
            hash: [row.byte; 32],
        },
        flags: CxFlags::default(),
    }
}

fn write_chain_artifact(path: &PathBuf, a: CxId, b: CxId, c: CxId, terminal_path: Vec<CxId>) {
    let hypothesis = AbcHypothesis {
        seed_id: "operator-centrality-2".to_string(),
        seed_kind: ChainWalkSeedKind::OperatorQuestion,
        a,
        b,
        c,
        terminal_path,
        cross_domain_distance: 2,
        terminal_confidence: 0.72,
        novelty_score: 0.4,
        path_score: 0.8,
        rank_score: 0.76,
        testable_claim: "operator-centrality-2: A -- B -- C".to_string(),
        provenance: vec!["seed_id=operator-centrality-2".to_string()],
    };
    let artifact = ChainWalksArtifactInput {
        node_metadata: BTreeMap::new(),
        report: ChainWalkReport {
            schema_version: CHAIN_WALK_SCHEMA_VERSION,
            seed_count: 1,
            completed_chain_count: 1,
            hypothesis_count: 1,
            results: vec![ChainWalkResult {
                seed: ChainWalkSeed {
                    seed_id: hypothesis.seed_id.clone(),
                    kind: hypothesis.seed_kind,
                    start: a,
                    question: Some("operator question".to_string()),
                    rationale: "test chain".to_string(),
                    provenance: Vec::new(),
                },
                log: DiscoveryChainLog {
                    schema_version: DISCOVERY_CHAIN_SCHEMA_VERSION,
                    starts: vec![a],
                    anchors: Vec::new(),
                    params: DiscoveryChainParams::default(),
                    candidates: Vec::new(),
                    accepted_hops: Vec::new(),
                    gate_pass_count: 0,
                    refused_count: 0,
                    terminated: DiscoveryTermination::FrontierExhausted,
                },
                hypotheses: vec![hypothesis],
            }],
        },
    };
    fs::write(path, serde_json::to_vec_pretty(&artifact).unwrap()).unwrap();
}

fn cx(byte: u8) -> CxId {
    CxId::from_bytes([byte; 16])
}

fn temp_home(name: &str) -> PathBuf {
    let home = std::env::temp_dir().join(format!(
        "calyx-hypothesis-evidence-{name}-{}-{}",
        std::process::id(),
        crate::cmd::vault::now_ms()
    ));
    let _ = fs::remove_dir_all(&home);
    home
}

fn cleanup(path: PathBuf) {
    fs::remove_dir_all(path).unwrap();
}
