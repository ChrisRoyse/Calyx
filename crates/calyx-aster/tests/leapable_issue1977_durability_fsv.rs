//! Gate G1 durability FSV for Leapable issue #1977.
//!
//! Source of truth is the durable vault directory: WAL segment bytes,
//! manifest bytes, and Base CF SST rows are decoded directly.

use std::fs;
use std::process::{Command, Stdio};
use std::time::Duration;

use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_aster::verify_restore::verify_restore;
use calyx_core::VaultStore;
use serde_json::json;

mod leapable_issue1977_support;

use leapable_issue1977_support::*;
#[test]
fn issue1977_kill_recover_wal_tail_and_torn_tail_fsv() {
    if std::env::var_os(CHILD_ENV).is_some() {
        child_writer();
        return;
    }

    let fixture = FixtureRoot::new("kill-recover");
    let vault_dir = fixture.root.join("vault");
    fs::create_dir_all(&vault_dir).expect("create vault dir");

    let mut child = Command::new(std::env::current_exe().expect("current test exe"))
        .arg("--exact")
        .arg("issue1977_kill_recover_wal_tail_and_torn_tail_fsv")
        .arg("--nocapture")
        .env(CHILD_ENV, "1")
        .env(FSV_ROOT_ENV, &fixture.root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn child writer");

    let ready_path = fixture.root.join("ready-to-kill.json");
    wait_for_path(&ready_path, Duration::from_secs(15));
    let expected = read_committed(&fixture.root);
    assert_eq!(expected.len(), EXPECTED_TOTAL);
    assert_eq!(
        expected
            .iter()
            .filter(|record| record.checkpoint_flushed)
            .count(),
        EXPECTED_FLUSHED
    );

    let before_kill = collect_vault_state(&vault_dir).expect("collect before-kill state");
    assert_eq!(before_kill.wal_base_rows.len(), EXPECTED_TOTAL);
    assert_eq!(before_kill.sst_base_rows.len(), EXPECTED_FLUSHED);
    println!(
        "ISSUE1977 BEFORE KILL {}",
        serde_json::to_string_pretty(&before_kill).unwrap()
    );

    child.kill().expect("kill child writer");
    let status = child.wait().expect("wait child writer");
    assert!(
        !status.success(),
        "child must be externally terminated, got {status}"
    );

    let after_kill = collect_vault_state(&vault_dir).expect("collect after-kill state");
    assert_eq!(
        before_kill.wal_files, after_kill.wal_files,
        "TerminateProcess/kill must not rewrite committed WAL segment bytes"
    );

    let recovered = AsterVault::open(&vault_dir, vault_id(), VAULT_SALT, VaultOptions::default())
        .expect("cold-open killed vault");
    let recovery_report = recovered.recovery_report().clone();
    assert_eq!(recovery_report.last_recovered_seq, EXPECTED_TOTAL as u64);
    assert_eq!(recovery_report.torn_tail, None);

    for record in &expected {
        let cx_id = parse_cx_id(&record.cx_id);
        let got = recovered
            .get(cx_id, recovered.snapshot())
            .expect("read recovered constellation");
        assert_eq!(got.metadata_value(TEXT_KEY), Some(record.text.as_str()));
        assert_eq!(got.metadata_value(CASE_KEY), Some(record.case.as_str()));
    }

    recovered
        .flush()
        .expect("checkpoint recovered WAL tail into SST bytes");
    drop(recovered);

    let after_recovery = collect_vault_state(&vault_dir).expect("collect after-recovery state");
    assert_eq!(after_recovery.sst_base_rows.len(), EXPECTED_TOTAL);
    for record in &expected {
        let wal = before_kill
            .wal_base_rows
            .get(&record.cx_id)
            .expect("expected WAL base row");
        let sst = after_recovery
            .sst_base_rows
            .get(&record.cx_id)
            .expect("expected SST base row after recovery");
        assert_eq!(sst.text, record.text);
        assert_eq!(
            sst.value_sha256, wal.value_sha256,
            "Base CF SST bytes must match the committed WAL base row for {}",
            record.cx_id
        );
    }

    let restore = verify_restore(&vault_dir).expect("verify restored killed vault");
    assert!(
        restore.success(),
        "verify_restore failed: {:?}",
        restore.failure_reasons()
    );

    let torn =
        append_torn_tail_and_recover(&vault_dir, &expected).expect("torn tail recovery edge case");
    let final_state = collect_vault_state(&vault_dir).expect("collect final state");
    println!(
        "ISSUE1977 AFTER RECOVERY {}",
        serde_json::to_string_pretty(&json!({
            "recovery_report": {
                "last_recovered_seq": recovery_report.last_recovered_seq,
                "torn_tail": recovery_report.torn_tail.as_ref().map(torn_tail_json),
            },
            "verify_restore": restore,
            "torn_tail_edge": torn,
            "final_state": final_state,
        }))
        .unwrap()
    );

    write_json(
        &fixture.root.join("issue1977-kill-recover-readback.json"),
        &json!({
            "source_of_truth": "WAL segment bytes plus Base CF SST bytes decoded from the durable vault directory",
            "before_kill": before_kill,
            "after_kill": after_kill,
            "after_recovery": after_recovery,
            "verify_restore": restore,
            "torn_tail_edge": torn,
            "expected_records": expected,
        }),
    );
}

#[test]
fn issue1977_parent_dir_sync_through_reparse_root_fsv() {
    let fixture = FixtureRoot::new("reparse-parent-sync");
    let target = fixture.root.join("target");
    let link = fixture.root.join("link-root");
    fs::create_dir_all(&target).expect("create reparse target");
    let link_kind = create_dir_link(&target, &link).expect("create reparse/symlink root");

    let vault_via_link = link.join("vault");
    let vault = AsterVault::new_durable(
        &vault_via_link,
        vault_id(),
        VAULT_SALT,
        VaultOptions::default(),
    )
    .expect("open vault through reparse root");
    let expected = sample_record(31, true);
    let expected_cx = parse_cx_id(&expected.cx_id);
    vault
        .put(sample_constellation(31, "reparse-parent-sync"))
        .expect("put through reparse root");
    vault.flush().expect("flush through reparse root");
    drop(vault);

    let vault_via_target = target.join("vault");
    let link_state = collect_vault_state(&vault_via_link).expect("collect link state");
    let target_state = collect_vault_state(&vault_via_target).expect("collect target state");
    assert_base_rows_match_ignoring_paths(&link_state.sst_base_rows, &target_state.sst_base_rows);

    let reopened = AsterVault::open(
        &vault_via_target,
        vault_id(),
        VAULT_SALT,
        VaultOptions::default(),
    )
    .expect("open target side after link write");
    let got = reopened
        .get(expected_cx, reopened.snapshot())
        .expect("read through target side");
    assert_eq!(got.metadata_value(TEXT_KEY), Some(expected.text.as_str()));
    drop(reopened);

    let restore = verify_restore(&vault_via_target).expect("verify target-side restored vault");
    assert!(
        restore.success(),
        "verify_restore failed: {:?}",
        restore.failure_reasons()
    );
    println!(
        "ISSUE1977 REPARSE PARENT SYNC {}",
        serde_json::to_string_pretty(&json!({
            "link_kind": link_kind,
            "link_path": link,
            "target_path": target,
            "link_state": link_state,
            "target_state": target_state,
            "verify_restore": restore,
        }))
        .unwrap()
    );

    write_json(
        &fixture
            .root
            .join("issue1977-reparse-parent-sync-readback.json"),
        &json!({
            "source_of_truth": "same durable vault bytes read through reparse/symlink root and target root",
            "link_kind": link_kind,
            "link_state": link_state,
            "target_state": target_state,
            "verify_restore": restore,
        }),
    );
}
