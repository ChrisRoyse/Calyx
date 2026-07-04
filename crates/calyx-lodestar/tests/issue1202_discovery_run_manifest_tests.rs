use std::fs;

use calyx_core::FixedClock;
use calyx_ledger::{
    LedgerAppender, LedgerCfStore, MemoryLedgerStore, VerifyResult, decode, verify_chain,
};
use calyx_lodestar::{
    DiscoveryRunManifest, DiscoveryRunStage, ObservedStageOutput, build_discovery_run_manifest,
    manifest_sha256, reproduce_discovery_run_manifest, seal_discovery_run_manifest,
};
use serde_json::json;

const TEST_TS: u64 = 1_783_142_000;

#[test]
fn seals_five_stage_manifest_and_verify_chain_covers_entry() {
    let manifest = manifest().unwrap();
    let expected_hash = manifest_sha256(&manifest).unwrap();
    let mut appender = ledger();

    let seal = seal_discovery_run_manifest(&mut appender, manifest.clone()).unwrap();
    assert_eq!(seal.manifest_sha256, expected_hash);
    assert_eq!(seal.ledger_ref.seq, 0);

    let store = appender.into_store();
    assert_eq!(
        verify_chain(&store, 0..1).unwrap(),
        VerifyResult::Intact { count: 1 }
    );
    let row = store.read_seq(0).unwrap().unwrap();
    let entry = decode(&row.bytes).unwrap();
    let payload: serde_json::Value = serde_json::from_slice(&entry.payload).unwrap();
    assert_eq!(payload["run_id"], "issue1202-run");
    assert_eq!(payload["manifest_sha256"], expected_hash);
    assert_eq!(payload["stage_count"], 5);
}

#[test]
fn stale_stage_input_breaks_manifest_chain() {
    let mut stages = stages();
    stages[2].input_sha256 = sha('9');

    let err = build_discovery_run_manifest("issue1202-run", "clinical-vault", sha('a'), stages)
        .unwrap_err();

    assert_eq!(err.code(), "CALYX_DISCOVERY_RUN_MANIFEST_CHAIN_BROKEN");
}

#[test]
fn missing_upstream_stage_fails_closed() {
    let mut stages = stages();
    stages[2].upstream_stage_id = Some("missing-stage".to_string());

    let err = build_discovery_run_manifest("issue1202-run", "clinical-vault", sha('a'), stages)
        .unwrap_err();

    assert_eq!(err.code(), "CALYX_DISCOVERY_RUN_MANIFEST_MISSING_UPSTREAM");
}

#[test]
fn reproduce_detects_output_drift() {
    let manifest = manifest().unwrap();
    let mut observed = observed_outputs(&manifest);
    observed[4].output_sha256 = sha('f');

    let err = reproduce_discovery_run_manifest(&manifest, &observed).unwrap_err();

    assert_eq!(err.code(), "CALYX_DISCOVERY_RUN_MANIFEST_DRIFT");
}

#[test]
fn reproduce_matches_when_observed_hashes_equal_manifest() {
    let manifest = manifest().unwrap();
    let observed = observed_outputs(&manifest);

    let report = reproduce_discovery_run_manifest(&manifest, &observed).unwrap();

    assert_eq!(report.run_id, "issue1202-run");
    assert_eq!(report.stage_count, 5);
    assert_eq!(report.manifest_sha256, manifest_sha256(&manifest).unwrap());
}

#[test]
fn writes_fsv_readback_when_root_is_set() {
    let Some(root) = calyx_fsv::fsv_root("CALYX_FSV_ROOT") else {
        return;
    };
    let manifest = manifest().unwrap();
    let mut appender = ledger();
    let seal = seal_discovery_run_manifest(&mut appender, manifest.clone()).unwrap();
    let store = appender.into_store();
    let verify = verify_chain(&store, 0..1).unwrap();
    let row = store.read_seq(0).unwrap().unwrap();
    let entry = decode(&row.bytes).unwrap();
    let payload: serde_json::Value = serde_json::from_slice(&entry.payload).unwrap();
    let value = json!({
        "issue": 1202,
        "run_id": manifest.run_id,
        "stage_count": manifest.stages.len(),
        "manifest_sha256": seal.manifest_sha256,
        "ledger_ref": {
            "seq": seal.ledger_ref.seq,
            "hash": hex(&seal.ledger_ref.hash),
        },
        "verify_chain": format!("{verify:?}"),
        "ledger_payload": payload,
        "manifest": manifest,
    });
    fs::create_dir_all(&root).unwrap();
    let path = root.join("issue1202_discovery_run_manifest_readback.json");
    fs::write(&path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
    let bytes = fs::read(&path).unwrap();
    let readback: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(readback["stage_count"], 5);
    assert_eq!(readback["ledger_payload"]["stage_count"], 5);
    assert_eq!(readback["verify_chain"], "Intact { count: 1 }");
    println!(
        "issue1202_fsv_path={} bytes={}",
        path.display(),
        bytes.len()
    );
}

fn manifest() -> calyx_lodestar::Result<DiscoveryRunManifest> {
    build_discovery_run_manifest("issue1202-run", "clinical-vault", sha('a'), stages())
}

fn stages() -> Vec<DiscoveryRunStage> {
    vec![
        stage(
            "typed-association-miner",
            "calyx typed-association-miner",
            None,
            sha('0'),
            sha('1'),
        ),
        stage(
            "hypothesis-falsification",
            "calyx hypothesis-falsification-sweep",
            None,
            sha('1'),
            sha('2'),
        ),
        stage(
            "association-validation",
            "calyx association-validation-gates",
            Some("hypothesis-falsification"),
            sha('2'),
            sha('3'),
        ),
        stage(
            "hypothesis-evaluate",
            "calyx hypothesis-evaluate",
            None,
            sha('3'),
            sha('4'),
        ),
        stage(
            "hypothesis-rank",
            "calyx hypothesis-rank",
            None,
            sha('4'),
            sha('5'),
        ),
    ]
}

fn stage(
    stage_id: &str,
    command: &str,
    upstream_stage_id: Option<&str>,
    input_sha256: String,
    output_sha256: String,
) -> DiscoveryRunStage {
    DiscoveryRunStage {
        stage_id: stage_id.to_string(),
        command: command.to_string(),
        args: vec!["--synthetic".to_string()],
        upstream_stage_id: upstream_stage_id.map(ToString::to_string),
        input_sha256,
        output_sha256,
        git_sha: "0a7fd1d2".to_string(),
    }
}

fn observed_outputs(manifest: &DiscoveryRunManifest) -> Vec<ObservedStageOutput> {
    manifest
        .stages
        .iter()
        .map(|stage| ObservedStageOutput {
            stage_id: stage.stage_id.clone(),
            output_sha256: stage.output_sha256.clone(),
        })
        .collect()
}

fn ledger() -> LedgerAppender<MemoryLedgerStore, FixedClock> {
    LedgerAppender::open(MemoryLedgerStore::default(), FixedClock::new(TEST_TS)).unwrap()
}

fn sha(ch: char) -> String {
    std::iter::repeat_n(ch, 64).collect()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
