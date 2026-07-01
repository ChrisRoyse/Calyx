use super::*;

use std::path::PathBuf;

const VAULT_ID: &str = "01KW0000000000000000000000";

#[test]
fn retire_vault_removes_active_entry_and_persists_record() {
    let root = setup_home("happy", true);
    let before = fs::read_to_string(root.join("vaults/index.json")).unwrap();

    run_with_home(
        &root,
        RetireVaultArgs {
            vault: VAULT_ID.to_string(),
            reason: "issue1072 synthetic registry snapshot absent".to_string(),
        },
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
    let error = run_with_home(
        &root,
        RetireVaultArgs {
            vault: VAULT_ID.to_string(),
            reason: "edge missing quarantine".to_string(),
        },
    )
    .unwrap_err();

    assert_eq!(error.code(), NOT_QUARANTINED_CODE);
    assert_eq!(fs::read(root.join("vaults/index.json")).unwrap(), before);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn retire_vault_fails_closed_on_duplicate_retirement() {
    let root = setup_home("duplicate", true);
    run_with_home(
        &root,
        RetireVaultArgs {
            vault: VAULT_ID.to_string(),
            reason: "first retirement".to_string(),
        },
    )
    .unwrap();
    let before = fs::read(root.join("vaults/index.json")).unwrap();
    let error = run_with_home(
        &root,
        RetireVaultArgs {
            vault: VAULT_ID.to_string(),
            reason: "duplicate retirement".to_string(),
        },
    )
    .unwrap_err();

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

    let error = run_with_home(
        &root,
        RetireVaultArgs {
            vault: VAULT_ID.to_string(),
            reason: "edge invalid quarantine".to_string(),
        },
    )
    .unwrap_err();

    assert_eq!(error.code(), QUARANTINE_INVALID_CODE);
    assert_eq!(fs::read(root.join("vaults/index.json")).unwrap(), before);
    fs::remove_dir_all(root).unwrap();
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

fn quarantine_json() -> &'static [u8] {
    br#"{"schema":"calyx.fsv.vault_quarantine.v1","source_of_truth":"physical marker","vault_id":"01KW0000000000000000000000","vault_name":"happy","vault_dir":"vaults/01KW0000000000000000000000","written_at_unix_ms":1,"failed_checks":[{"name":"registry_snapshot_ref","code":"CALYX_ASTER_CORRUPT_SHARD","message":"missing registry","remediation":"restore"}]}"#
}
