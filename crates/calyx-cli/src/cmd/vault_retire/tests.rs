use super::*;

use std::path::PathBuf;

const VAULT_ID: &str = "01KW0000000000000000000000";
const REPLACEMENT_ID: &str = "01KW0000000000000000000001";

#[test]
fn retire_vault_removes_active_entry_and_persists_record() {
    let root = setup_home("happy", true);
    let before = fs::read_to_string(root.join("vaults/index.json")).unwrap();

    run_with_home(
        &root,
        retire_args(VAULT_ID, "issue1072 synthetic registry snapshot absent"),
    )
    .unwrap();

    let after: Value =
        serde_json::from_slice(&fs::read(root.join("vaults/index.json")).unwrap()).unwrap();
    assert!(!before.contains("retired_vaults"));
    assert_eq!(active_vault_count(&after).unwrap(), 0);
    assert_eq!(retired_vault_count(&after).unwrap(), 1);
    let record = retired_record(&after, VAULT_ID).unwrap().unwrap();
    assert_eq!(record["manifest_registry_ref"], Value::Null);
    assert_eq!(record["registry_snapshot_file_count"], 0);
    assert!(record["quarantine_marker"]["bytes"].as_u64().unwrap() > 0);
    assert!(
        root.join("vaults")
            .join(VAULT_ID)
            .join(QUARANTINE_FILE)
            .exists()
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn retire_vault_fails_closed_without_quarantine_marker() {
    let root = setup_home("missing-quarantine", false);
    let before = fs::read(root.join("vaults/index.json")).unwrap();
    let error = run_with_home(&root, retire_args(VAULT_ID, "edge missing quarantine")).unwrap_err();

    assert_eq!(error.code(), NOT_QUARANTINED_CODE);
    assert_eq!(fs::read(root.join("vaults/index.json")).unwrap(), before);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn retire_vault_fails_closed_on_duplicate_retirement() {
    let root = setup_home("duplicate", true);
    run_with_home(&root, retire_args(VAULT_ID, "first retirement")).unwrap();
    let before = fs::read(root.join("vaults/index.json")).unwrap();
    let error = run_with_home(&root, retire_args(VAULT_ID, "duplicate retirement")).unwrap_err();

    assert_eq!(error.code(), ALREADY_RETIRED_CODE);
    assert_eq!(fs::read(root.join("vaults/index.json")).unwrap(), before);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn retire_vault_fails_closed_on_invalid_quarantine_schema() {
    let root = setup_home("bad-quarantine", true);
    fs::write(
            root.join("vaults").join(VAULT_ID).join(QUARANTINE_FILE),
            br#"{"schema":"wrong","vault_id":"01KW0000000000000000000000","failed_checks":[{"name":"x"}]}"#,
        )
        .unwrap();
    let before = fs::read(root.join("vaults/index.json")).unwrap();

    let error = run_with_home(&root, retire_args(VAULT_ID, "edge invalid quarantine")).unwrap_err();

    assert_eq!(error.code(), QUARANTINE_INVALID_CODE);
    assert_eq!(fs::read(root.join("vaults/index.json")).unwrap(), before);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn supersede_vault_removes_old_keeps_replacement_and_persists_record() {
    let (root, fsv_path, fsv_sha) = setup_supersession_home("supersede-happy");

    run_with_home(
        &root,
        supersede_args(VAULT_ID, REPLACEMENT_ID, &fsv_path, &fsv_sha),
    )
    .unwrap();

    let after: Value =
        serde_json::from_slice(&fs::read(root.join("vaults/index.json")).unwrap()).unwrap();
    assert_eq!(active_vault_count(&after).unwrap(), 1);
    assert_eq!(active_position(&after, VAULT_ID).unwrap(), None);
    assert!(active_position(&after, REPLACEMENT_ID).unwrap().is_some());
    let records = after["superseded_vaults"].as_array().unwrap();
    assert_eq!(records.len(), 1);
    let record = &records[0];
    assert_eq!(record["superseded_vault_id"], VAULT_ID);
    assert_eq!(record["replacement_vault_id"], REPLACEMENT_ID);
    assert_eq!(record["source_issue"], "1230");
    assert_eq!(record["fsv_readback_artifact"]["sha256"], fsv_sha);
    assert_eq!(record["fsv_matched_replacement_vault_id"], true);
    assert!(
        root.join("vaults").join(VAULT_ID).exists(),
        "supersession must not delete superseded vault bytes"
    );
    assert!(root.join("vaults").join(REPLACEMENT_ID).exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn supersede_vault_fails_closed_when_replacement_not_active() {
    let (root, fsv_path, fsv_sha) = setup_supersession_home("missing-replacement");
    let before = fs::read(root.join("vaults/index.json")).unwrap();

    let error = run_with_home(
        &root,
        supersede_args(VAULT_ID, "not-active", &fsv_path, &fsv_sha),
    )
    .unwrap_err();

    assert_eq!(error.code(), supersession::REPLACEMENT_NOT_ACTIVE_CODE);
    assert_eq!(fs::read(root.join("vaults/index.json")).unwrap(), before);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn supersede_vault_fails_closed_on_fsv_hash_mismatch() {
    let (root, fsv_path, _) = setup_supersession_home("bad-fsv-hash");
    let before = fs::read(root.join("vaults/index.json")).unwrap();

    let error = run_with_home(
        &root,
        supersede_args(
            VAULT_ID,
            REPLACEMENT_ID,
            &fsv_path,
            "0000000000000000000000000000000000000000000000000000000000000000",
        ),
    )
    .unwrap_err();

    assert_eq!(error.code(), supersession::FSV_MISMATCH_CODE);
    assert_eq!(fs::read(root.join("vaults/index.json")).unwrap(), before);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn parse_retire_vault_supersession_flags() {
    let tokens = [
        "trial-corpus",
        "--reason",
        "corrected FSV run replaces generated evidence vault",
        "--superseded-by",
        "corrected-corpus",
        "--source-issue",
        "1230",
        "--fsv-readback",
        "/fsv/readback.json",
        "--fsv-sha256",
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
    ]
    .into_iter()
    .map(str::to_string)
    .collect::<Vec<_>>();

    let command = parse_retire_vault(&tokens).unwrap();

    match command {
        Subcommand::RetireVault(args) => {
            assert_eq!(args.vault, "trial-corpus");
            assert_eq!(args.superseded_by.as_deref(), Some("corrected-corpus"));
            assert_eq!(args.source_issue.as_deref(), Some("1230"));
            assert_eq!(
                args.fsv_readback.unwrap(),
                std::path::PathBuf::from("/fsv/readback.json")
            );
            assert_eq!(
                args.fsv_sha256.as_deref(),
                Some("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")
            );
        }
        other => panic!("expected RetireVault command, got {other:?}"),
    }
}

fn setup_home(name: &str, quarantine: bool) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "calyx-retire-vault-{name}-{}-{}",
        std::process::id(),
        now_ms().expect("test clock should be after UNIX_EPOCH")
    ));
    let vault = root.join("vaults").join(VAULT_ID);
    fs::create_dir_all(&vault).unwrap();
    fs::write(
        vault.join("CURRENT"),
        "manifest-00000000000000000001.json\n",
    )
    .unwrap();
    fs::write(
        vault.join("manifest-00000000000000000001.json"),
        br#"{"manifest_seq":1,"durable_seq":0,"registry_ref":null}"#,
    )
    .unwrap();
    if quarantine {
        fs::write(vault.join(QUARANTINE_FILE), quarantine_json()).unwrap();
    }
    fs::create_dir_all(root.join("vaults")).unwrap();
    fs::write(
            root.join("vaults/index.json"),
            format!(
                "{{\n  \"vaults\": [{{\n    \"name\": \"{name}\",\n    \"vault_id\": \"{VAULT_ID}\",\n    \"path\": \"vaults/{VAULT_ID}\",\n    \"panel_template\": \"text-default\"\n  }}]\n}}\n"
            ),
        )
        .unwrap();
    root
}

fn setup_supersession_home(name: &str) -> (PathBuf, PathBuf, String) {
    let root = std::env::temp_dir().join(format!(
        "calyx-supersede-vault-{name}-{}-{}",
        std::process::id(),
        now_ms().expect("test clock should be after UNIX_EPOCH")
    ));
    write_vault(&root, VAULT_ID);
    write_vault(&root, REPLACEMENT_ID);
    fs::create_dir_all(root.join("vaults")).unwrap();
    fs::write(
        root.join("vaults/index.json"),
        format!(
            "{{\n  \"vaults\": [{{\n    \"name\": \"trial-corpus\",\n    \"vault_id\": \"{VAULT_ID}\",\n    \"path\": \"vaults/{VAULT_ID}\",\n    \"panel_template\": \"text-default\"\n  }}, {{\n    \"name\": \"corrected-corpus\",\n    \"vault_id\": \"{REPLACEMENT_ID}\",\n    \"path\": \"vaults/{REPLACEMENT_ID}\",\n    \"panel_template\": \"text-default\"\n  }}]\n}}\n"
        ),
    )
    .unwrap();
    let fsv_dir = root.join("fsv");
    fs::create_dir_all(&fsv_dir).unwrap();
    let fsv_path = fsv_dir.join("readback.json");
    fs::write(
        &fsv_path,
        format!(
            "{{\"schema\":\"test.fsv\",\"replacement_vault_id\":\"{REPLACEMENT_ID}\",\"replacement_name\":\"corrected-corpus\",\"active_index_hits\":[{{\"vault_id\":\"{REPLACEMENT_ID}\",\"name\":\"corrected-corpus\"}}]}}"
        ),
    )
    .unwrap();
    let fsv_sha = sha256_hex(&fs::read(&fsv_path).unwrap());
    (root, fsv_path, fsv_sha)
}

fn write_vault(root: &std::path::Path, vault_id: &str) {
    let vault = root.join("vaults").join(vault_id);
    fs::create_dir_all(&vault).unwrap();
    fs::write(
        vault.join("CURRENT"),
        "manifest-00000000000000000001.json\n",
    )
    .unwrap();
    fs::write(
        vault.join("manifest-00000000000000000001.json"),
        br#"{"manifest_seq":1,"durable_seq":0,"registry_ref":null}"#,
    )
    .unwrap();
}

fn retire_args(vault: &str, reason: &str) -> RetireVaultArgs {
    RetireVaultArgs {
        vault: vault.to_string(),
        reason: reason.to_string(),
        superseded_by: None,
        source_issue: None,
        fsv_readback: None,
        fsv_sha256: None,
    }
}

fn supersede_args(
    vault: &str,
    replacement: &str,
    fsv_path: &std::path::Path,
    fsv_sha: &str,
) -> RetireVaultArgs {
    RetireVaultArgs {
        vault: vault.to_string(),
        reason: "corrected FSV run replaces generated evidence vault".to_string(),
        superseded_by: Some(replacement.to_string()),
        source_issue: Some("1230".to_string()),
        fsv_readback: Some(fsv_path.to_path_buf()),
        fsv_sha256: Some(fsv_sha.to_string()),
    }
}

fn quarantine_json() -> &'static [u8] {
    br#"{"schema":"calyx.fsv.vault_quarantine.v1","source_of_truth":"physical marker","vault_id":"01KW0000000000000000000000","vault_name":"happy","vault_dir":"vaults/01KW0000000000000000000000","written_at_unix_ms":1,"failed_checks":[{"name":"registry_snapshot_ref","code":"CALYX_ASTER_CORRUPT_SHARD","message":"missing registry","remediation":"restore"}]}"#
}
