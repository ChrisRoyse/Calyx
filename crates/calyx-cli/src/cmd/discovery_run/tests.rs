use std::fs;
use std::path::{Path, PathBuf};

use calyx_lodestar::{DiscoveryRunManifest, DiscoveryRunStage, ObservedStageOutput};

use super::*;

#[test]
fn seal_verify_and_reproduce_round_trip_disk_ledger() {
    let root = temp_root("round-trip");
    let manifest_path = root.join("manifest.json");
    let observed_path = root.join("observed.json");
    let ledger = root.join("ledger");
    let seal_out = root.join("seal.json");
    let verify_out = root.join("verify.json");
    let reproduce_out = root.join("reproduce.json");
    write_manifest(&manifest_path, &manifest());
    write_observed(&observed_path, &observed());

    run_seal(SealArgs {
        manifest: manifest_path.clone(),
        ledger: ledger.clone(),
        out: seal_out.clone(),
    })
    .unwrap();
    run_verify(VerifyArgs {
        manifest: manifest_path.clone(),
        ledger,
        seq: 0,
        out: verify_out.clone(),
    })
    .unwrap();
    run_reproduce(ReproduceArgs {
        manifest: manifest_path,
        observed: observed_path,
        out: reproduce_out.clone(),
    })
    .unwrap();

    let seal: SealArtifact = read_json(&seal_out).unwrap();
    assert_eq!(seal.verify_chain, "Intact { count: 1 }");
    assert_eq!(seal.ledger_ref_seq, 0);
    assert_eq!(seal.ledger_payload["stage_count"], 2);
    let verify: VerifyArtifact = read_json(&verify_out).unwrap();
    assert_eq!(verify.manifest_sha256, seal.manifest_sha256);
    let reproduce: ReproduceArtifact = read_json(&reproduce_out).unwrap();
    assert_eq!(reproduce.report.stage_count, 2);
    cleanup(root);
}

#[test]
fn seal_accepts_long_benign_manifest_provenance_tokens() {
    let root = temp_root("long-benign-manifest");
    let manifest_path = root.join("manifest.json");
    let ledger = root.join("ledger");
    let seal_out = root.join("seal.json");
    write_manifest(&manifest_path, &long_benign_manifest());

    run_seal(SealArgs {
        manifest: manifest_path,
        ledger,
        out: seal_out.clone(),
    })
    .unwrap();

    let seal: SealArtifact = read_json(&seal_out).unwrap();
    assert_eq!(seal.verify_chain, "Intact { count: 1 }");
    assert_eq!(
        seal.ledger_payload["run_id"],
        "issue1221-biomedical-discovery-run-20260704T065206Z"
    );
    assert_eq!(
        seal.ledger_payload["corpus_vault_id"],
        "issue1221-biomedical-discovery-fsv-vault"
    );
    assert_eq!(
        seal.ledger_payload["stages"][0]["git_sha"],
        "faac5d1a6b391c584ab8264c33fcbee4892bd53a"
    );
    cleanup(root);
}

#[test]
fn seal_rejects_secret_like_manifest_token_without_artifact() {
    let root = temp_root("secret-manifest");
    let manifest_path = root.join("manifest.json");
    let ledger = root.join("ledger");
    let out = root.join("seal.json");
    let mut manifest = manifest();
    manifest.stages[0].stage_id = "mF9zK4sQ7xP2nT8vB3cD6eG1hJ5lR0uW9yA2bC4dE6".to_string();
    write_manifest(&manifest_path, &manifest);

    let err = run_seal(SealArgs {
        manifest: manifest_path,
        ledger,
        out: out.clone(),
    })
    .unwrap_err();

    assert_eq!(err.code(), "CALYX_LEDGER_SECRET_IN_PAYLOAD");
    assert!(!out.exists());
    cleanup(root);
}

#[test]
fn reproduce_drift_fails_without_artifact() {
    let root = temp_root("drift");
    let manifest_path = root.join("manifest.json");
    let observed_path = root.join("observed.json");
    let out = root.join("reproduce.json");
    write_manifest(&manifest_path, &manifest());
    let mut observed = observed();
    observed[1].output_sha256 = sha('f');
    write_observed(&observed_path, &observed);

    let err = run_reproduce(ReproduceArgs {
        manifest: manifest_path,
        observed: observed_path,
        out: out.clone(),
    })
    .unwrap_err();

    assert_eq!(err.code(), "CALYX_DISCOVERY_RUN_MANIFEST_DRIFT");
    assert!(!out.exists());
    cleanup(root);
}

#[test]
fn chain_broken_seal_fails_without_artifact() {
    let root = temp_root("broken");
    let manifest_path = root.join("manifest.json");
    let ledger = root.join("ledger");
    let out = root.join("seal.json");
    let mut manifest = manifest();
    manifest.stages[1].input_sha256 = sha('9');
    write_manifest(&manifest_path, &manifest);

    let err = run_seal(SealArgs {
        manifest: manifest_path,
        ledger: ledger.clone(),
        out: out.clone(),
    })
    .unwrap_err();

    assert_eq!(err.code(), "CALYX_DISCOVERY_RUN_MANIFEST_CHAIN_BROKEN");
    assert!(!out.exists());
    assert!(!ledger.exists());
    cleanup(root);
}

#[test]
fn writes_fsv_readback_when_root_is_set() {
    let Some(root) = calyx_fsv::fsv_root("CALYX_FSV_ROOT") else {
        return;
    };
    let manifest_path = root.join("manifest.json");
    let observed_path = root.join("observed.json");
    let ledger = root.join("ledger");
    let seal_out = root.join("seal.json");
    let verify_out = root.join("verify.json");
    let reproduce_out = root.join("reproduce.json");
    write_manifest(&manifest_path, &manifest());
    write_observed(&observed_path, &observed());
    run_seal(SealArgs {
        manifest: manifest_path.clone(),
        ledger: ledger.clone(),
        out: seal_out.clone(),
    })
    .unwrap();
    run_verify(VerifyArgs {
        manifest: manifest_path.clone(),
        ledger,
        seq: 0,
        out: verify_out.clone(),
    })
    .unwrap();
    run_reproduce(ReproduceArgs {
        manifest: manifest_path,
        observed: observed_path,
        out: reproduce_out.clone(),
    })
    .unwrap();
    let seal: SealArtifact = read_json(&seal_out).unwrap();
    let verify: VerifyArtifact = read_json(&verify_out).unwrap();
    let reproduce: ReproduceArtifact = read_json(&reproduce_out).unwrap();
    let summary = json!({
        "issue": 1217,
        "seal_sha256": file_sha256(&seal_out),
        "verify_sha256": file_sha256(&verify_out),
        "reproduce_sha256": file_sha256(&reproduce_out),
        "manifest_sha256": seal.manifest_sha256,
        "ledger_ref_seq": seal.ledger_ref_seq,
        "seal_verify_chain": seal.verify_chain,
        "verify_chain": verify.verify_chain,
        "reproduce_stage_count": reproduce.report.stage_count,
    });
    let summary_path = root.join("issue1217_discovery_run_cli_readback.json");
    fs::write(&summary_path, serde_json::to_vec_pretty(&summary).unwrap()).unwrap();
    let readback: serde_json::Value =
        serde_json::from_slice(&fs::read(&summary_path).unwrap()).unwrap();
    assert_eq!(readback["ledger_ref_seq"], 0);
    assert_eq!(readback["seal_verify_chain"], "Intact { count: 1 }");
    println!(
        "issue1217_fsv_path={} bytes={}",
        summary_path.display(),
        fs::metadata(&summary_path).unwrap().len()
    );
}

fn manifest() -> DiscoveryRunManifest {
    DiscoveryRunManifest {
        schema_version: 1,
        run_id: "issue1217-run".to_string(),
        corpus_vault_id: "clinical-vault".to_string(),
        panel_manifest_sha256: sha('a'),
        stages: vec![
            stage(
                "typed-association-miner",
                "calyx typed-association-miner",
                None,
                sha('0'),
                sha('1'),
            ),
            stage(
                "hypothesis-rank",
                "calyx hypothesis-rank",
                None,
                sha('1'),
                sha('2'),
            ),
        ],
    }
}

fn long_benign_manifest() -> DiscoveryRunManifest {
    DiscoveryRunManifest {
        schema_version: 1,
        run_id: "issue1221-biomedical-discovery-run-20260704T065206Z".to_string(),
        corpus_vault_id: "issue1221-biomedical-discovery-fsv-vault".to_string(),
        panel_manifest_sha256: sha('a'),
        stages: vec![
            stage(
                "bridge-falsification-to-evaluate",
                "calyx-discovery-run/bridge-falsification-to-evaluate",
                None,
                sha('0'),
                sha('1'),
            ),
            stage(
                "hypothesis-rank",
                "calyx hypothesis-rank",
                Some("bridge-falsification-to-evaluate"),
                sha('1'),
                sha('2'),
            ),
        ],
    }
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
        git_sha: "faac5d1a6b391c584ab8264c33fcbee4892bd53a".to_string(),
    }
}

fn observed() -> Vec<ObservedStageOutput> {
    manifest()
        .stages
        .into_iter()
        .map(|stage| ObservedStageOutput {
            stage_id: stage.stage_id,
            output_sha256: stage.output_sha256,
        })
        .collect()
}

fn write_manifest(path: &Path, manifest: &DiscoveryRunManifest) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, serde_json::to_vec_pretty(manifest).unwrap()).unwrap();
}

fn write_observed(path: &Path, observed: &[ObservedStageOutput]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, serde_json::to_vec_pretty(observed).unwrap()).unwrap();
}

fn temp_root(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "calyx-discovery-run-{name}-{}-{}",
        std::process::id(),
        crate::cmd::vault::now_ms()
    ));
    let _ = fs::remove_dir_all(&root);
    root
}

fn cleanup(path: PathBuf) {
    fs::remove_dir_all(path).unwrap();
}

fn sha(ch: char) -> String {
    std::iter::repeat_n(ch, 64).collect()
}

fn file_sha256(path: &Path) -> String {
    sha256_hex(&fs::read(path).unwrap())
}
