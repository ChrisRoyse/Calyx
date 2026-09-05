use super::super::session::{BatchIngestSession, read_session_status, reconcile_session_status};
use super::*;
use crate::cmd::IngestOutput;
use crate::cmd::ingest::route::IngestGpuRoute;
use crate::error::CliError;
use sha2::Digest;

#[test]
fn batch_ingest_writes_durable_session_status_readback() {
    let (root, resolved) = test_vault_with_registered_dense_lens("issue1065-session-ok");
    let jsonl = resolved.path.join("issue1065-ok.jsonl");
    fs::write(
        &jsonl,
        format!(
            "{}\n{}\n",
            batch_line("issue1065 durable session alpha"),
            batch_line("issue1065 durable session beta")
        ),
    )
    .unwrap();
    let session_id = "issue1065-session-ok";
    let status_path = resolved
        .path
        .join("idx/ingest/runs")
        .join(session_id)
        .join("status.json");
    assert!(!status_path.exists(), "session status must not pre-exist");

    let validation = validate_batch_file(&jsonl).unwrap();
    let identity = BatchSnapshotIdentity::capture(&jsonl, validation.row_count).unwrap();
    let mut session =
        BatchIngestSession::start(&resolved, &jsonl, &validation, &identity, Some(session_id))
            .unwrap();
    ingest_validated_batch_streaming_with_output(
        &resolved,
        &jsonl,
        IngestOutput::Summary,
        &identity,
        IngestGpuRoute::cold_workers_allowed(),
        None,
        Some(&mut session),
    )
    .unwrap();

    let status_bytes = fs::read(&status_path).unwrap();
    let status: serde_json::Value = serde_json::from_slice(&status_bytes).unwrap();
    println!("issue1065_status_after={status}");
    assert_eq!(status["schema_version"], 3);
    assert_eq!(status["process_identity"]["process_id"], std::process::id());
    assert!(
        status["process_identity"]["process_start"]
            .as_u64()
            .unwrap()
            > 0
    );
    assert!(
        !status["process_identity"]["executable"]
            .as_str()
            .unwrap()
            .is_empty()
    );
    assert_eq!(status["session_id"], session_id);
    assert_eq!(status["status"], "complete");
    assert_eq!(status["phase"], "complete");
    assert_eq!(status["planned_row_count"], 2);
    assert_eq!(status["rows_started"], 2);
    assert_eq!(status["rows_committed"], 2);
    assert_eq!(status["pending_rows"], 0);
    assert_eq!(status["uncommitted_started_rows"], 0);
    assert_eq!(status["committed_new_rows"], 2);
    assert_eq!(status["already_idempotent_rows"], 0);
    assert_eq!(status["failed_rows"], 0);
    assert_eq!(status["index_rebuild_phase"], "complete");
    assert_eq!(status["batch_sha256"].as_str().unwrap().len(), 64);
    assert!(status["final_chain_seq"].as_u64().unwrap() >= 1);

    let readback = read_session_status(&resolved.path, session_id).unwrap();
    assert_eq!(readback.status, "complete");

    let vault = open_vault(&resolved).unwrap();
    let base_rows = vault
        .scan_cf_at(vault.snapshot(), ColumnFamily::Base)
        .unwrap();
    assert_eq!(base_rows.len(), 2, "Base CF is the ingest source of truth");
    fs::remove_dir_all(root).ok();
}

#[test]
fn batch_ingest_session_fails_closed_on_reused_session_id() {
    let (root, resolved) = test_vault_with_registered_dense_lens("issue1065-session-reuse");
    let jsonl = resolved.path.join("issue1065-reuse.jsonl");
    fs::write(
        &jsonl,
        format!("{}\n", batch_line("issue1065 session reuse alpha")),
    )
    .unwrap();
    let session_id = "issue1065-reused-session";
    let validation = validate_batch_file(&jsonl).unwrap();
    let identity = BatchSnapshotIdentity::capture(&jsonl, validation.row_count).unwrap();
    let mut session =
        BatchIngestSession::start(&resolved, &jsonl, &validation, &identity, Some(session_id))
            .unwrap();
    ingest_validated_batch_streaming_with_output(
        &resolved,
        &jsonl,
        IngestOutput::Summary,
        &identity,
        IngestGpuRoute::cold_workers_allowed(),
        None,
        Some(&mut session),
    )
    .unwrap();
    let status_file = resolved
        .path
        .join("idx/ingest/runs")
        .join(session_id)
        .join("status.json");
    let before = fs::read(&status_file).unwrap();
    let err =
        BatchIngestSession::start(&resolved, &jsonl, &validation, &identity, Some(session_id))
            .unwrap_err();
    let after = fs::read(&status_file).unwrap();
    assert_eq!(err.code(), "CALYX_INGEST_SESSION_EXISTS");
    assert_eq!(before, after, "reused session must not overwrite prior SoT");
    fs::remove_dir_all(root).ok();
}

#[test]
fn batch_ingest_session_records_post_commit_failure() {
    let (root, resolved) = test_vault_with_registered_dense_lens("issue1065-session-failed");
    let jsonl = resolved.path.join("issue1065-failed.jsonl");
    fs::write(
        &jsonl,
        format!("{}\n", batch_line("issue1065 session failure alpha")),
    )
    .unwrap();
    let manifest_path = resolved.path.join("idx/search/manifest.json");
    fs::create_dir_all(manifest_path.parent().unwrap()).unwrap();
    fs::write(&manifest_path, b"{not-json").unwrap();
    let session_id = "issue1065-session-failed";

    let validation = validate_batch_file(&jsonl).unwrap();
    let identity = BatchSnapshotIdentity::capture(&jsonl, validation.row_count).unwrap();
    let mut session =
        BatchIngestSession::start(&resolved, &jsonl, &validation, &identity, Some(session_id))
            .unwrap();
    let err = ingest_validated_batch_streaming_with_output(
        &resolved,
        &jsonl,
        IngestOutput::Summary,
        &identity,
        IngestGpuRoute::cold_workers_allowed(),
        None,
        Some(&mut session),
    )
    .unwrap_err();
    session.fail_with_error(&err).unwrap();
    assert_eq!(err.code(), "CALYX_INGEST_INDEX_REBUILD_FAILED");

    let status_path = resolved
        .path
        .join("idx/ingest/runs")
        .join(session_id)
        .join("status.json");
    let status: serde_json::Value =
        serde_json::from_slice(&fs::read(&status_path).unwrap()).unwrap();
    println!("issue1065_failed_status_after={status}");
    assert_eq!(status["status"], "failed");
    assert_eq!(status["rows_committed"], 1);
    assert_eq!(status["committed_new_rows"], 1);
    assert_eq!(status["pending_rows"], 0);
    assert_eq!(status["uncommitted_started_rows"], 0);
    assert_eq!(status["failed_rows"], 0);
    assert_eq!(status["error"]["code"], "CALYX_INGEST_INDEX_REBUILD_FAILED");

    let readback = read_session_status(&resolved.path, session_id).unwrap();
    assert_eq!(readback.status, "failed");

    let vault = open_vault(&resolved).unwrap();
    let base_rows = vault
        .scan_cf_at(vault.snapshot(), ColumnFamily::Base)
        .unwrap();
    assert_eq!(
        base_rows.len(),
        1,
        "failed session still records committed SoT rows"
    );
    fs::remove_dir_all(root).ok();
}

#[test]
fn running_and_failed_status_keep_pending_uncommitted_and_failed_rows_distinct() {
    let (root, resolved) = test_vault_with_registered_dense_lens("issue1684-counters");
    let jsonl = resolved.path.join("issue1684-counters.jsonl");
    fs::write(
        &jsonl,
        format!(
            "{}\n{}\n",
            batch_line("issue1684 first row"),
            batch_line("issue1684 second row")
        ),
    )
    .unwrap();
    let validation = validate_batch_file(&jsonl).unwrap();
    let identity = BatchSnapshotIdentity::capture(&jsonl, validation.row_count).unwrap();
    let mut session = BatchIngestSession::start(
        &resolved,
        &jsonl,
        &validation,
        &identity,
        Some("issue1684-counter-state"),
    )
    .unwrap();
    let created = read_session_status(&resolved.path, session.session_id()).unwrap();
    assert_eq!(created.pending_rows, 2);
    assert_eq!(created.uncommitted_started_rows, 0);
    assert_eq!(created.failed_rows, 0);

    session.record_rows_started(1, "batch_flush_start").unwrap();
    let running = read_session_status(&resolved.path, session.session_id()).unwrap();
    assert_eq!(running.pending_rows, 1);
    assert_eq!(running.uncommitted_started_rows, 1);
    assert_eq!(running.failed_rows, 0);

    let cause = CliError::runtime("real fail-fast session error");
    session.fail_with_error(&cause).unwrap();
    let failed = read_session_status(&resolved.path, session.session_id()).unwrap();
    assert_eq!(failed.status, "failed");
    assert_eq!(failed.pending_rows, 1);
    assert_eq!(failed.uncommitted_started_rows, 1);
    assert_eq!(failed.failed_rows, 0);
    assert_eq!(
        failed.error.unwrap().message,
        "real fail-fast session error"
    );

    let status_path = resolved
        .path
        .join("idx/ingest/runs/issue1684-counter-state/status.json");
    let mut tampered: serde_json::Value =
        serde_json::from_slice(&fs::read(&status_path).unwrap()).unwrap();
    tampered["failed_rows"] = serde_json::json!(1);
    fs::write(&status_path, serde_json::to_vec_pretty(&tampered).unwrap()).unwrap();
    let before_invalid = fs::read(&status_path).unwrap();
    let error = reconcile_session_status(&resolved.path, session.session_id()).unwrap_err();
    assert_eq!(error.code(), "CALYX_INGEST_SESSION_INVALID");
    assert!(error.message().contains("counter invariants"));
    assert_eq!(
        fs::read(&status_path).unwrap(),
        before_invalid,
        "malformed source of truth must fail closed without archival or mutation"
    );
    fs::remove_dir_all(root).ok();
}

#[test]
fn exact_pid_reuse_reconciliation_archives_prior_bytes_and_preserves_counters() {
    let (root, resolved) = test_vault_with_registered_dense_lens("issue1698-pid-reuse");
    let jsonl = resolved.path.join("issue1698-pid-reuse.jsonl");
    fs::write(
        &jsonl,
        format!(
            "{}\n{}\n",
            batch_line("issue1698 pid reuse first"),
            batch_line("issue1698 pid reuse second")
        ),
    )
    .unwrap();
    let validation = validate_batch_file(&jsonl).unwrap();
    let identity = BatchSnapshotIdentity::capture(&jsonl, validation.row_count).unwrap();
    let session_id = "issue1698-pid-reuse";
    let mut session =
        BatchIngestSession::start(&resolved, &jsonl, &validation, &identity, Some(session_id))
            .unwrap();
    session.record_rows_started(1, "batch_flush_start").unwrap();
    let status_path = resolved
        .path
        .join("idx/ingest/runs")
        .join(session_id)
        .join("status.json");
    let mut synthetic_reuse: serde_json::Value =
        serde_json::from_slice(&fs::read(&status_path).unwrap()).unwrap();
    let exact_start = synthetic_reuse["process_identity"]["process_start"]
        .as_u64()
        .unwrap();
    synthetic_reuse["process_identity"]["process_start"] = serde_json::json!(exact_start + 1);
    let mut before = serde_json::to_vec_pretty(&synthetic_reuse).unwrap();
    before.push(b'\n');
    fs::write(&status_path, &before).unwrap();
    let before_sha = format!("{:x}", sha2::Sha256::digest(&before));

    let (after, owner_unknown) = reconcile_session_status(&resolved.path, session_id).unwrap();

    assert_eq!(owner_unknown, None);
    assert_eq!(after.status, "abandoned");
    assert_eq!(after.phase, "batch_flush_start");
    assert_eq!(after.rows_started, 1);
    assert_eq!(after.rows_committed, 0);
    assert_eq!(after.pending_rows, 1);
    assert_eq!(after.uncommitted_started_rows, 1);
    assert_eq!(
        after.terminal.as_ref().unwrap().previous_status_sha256,
        before_sha
    );
    let archive_path = status_path
        .parent()
        .unwrap()
        .join("history")
        .join(format!("status-{before_sha}.json"));
    assert_eq!(
        fs::read(&archive_path).unwrap(),
        before,
        "history source of truth must retain the exact pre-reconciliation bytes"
    );
    let physical = read_session_status(&resolved.path, session_id).unwrap();
    assert_eq!(physical, after);
    println!(
        "ISSUE1698_PID_REUSE_FSV before_sha={} after_status={} phase={} started={} committed={} pending={} uncommitted={} archive={}",
        before_sha,
        physical.status,
        physical.phase,
        physical.rows_started,
        physical.rows_committed,
        physical.pending_rows,
        physical.uncommitted_started_rows,
        archive_path.display()
    );
    fs::remove_dir_all(root).ok();
}

#[test]
fn remote_owner_remains_running_and_status_bytes_are_unchanged() {
    let (root, resolved) = test_vault_with_registered_dense_lens("issue1698-remote-owner");
    let jsonl = resolved.path.join("issue1698-remote-owner.jsonl");
    fs::write(
        &jsonl,
        format!("{}\n", batch_line("issue1698 remote owner")),
    )
    .unwrap();
    let validation = validate_batch_file(&jsonl).unwrap();
    let identity = BatchSnapshotIdentity::capture(&jsonl, validation.row_count).unwrap();
    let session_id = "issue1698-remote-owner";
    BatchIngestSession::start(&resolved, &jsonl, &validation, &identity, Some(session_id)).unwrap();
    let status_path = resolved
        .path
        .join("idx/ingest/runs")
        .join(session_id)
        .join("status.json");
    let mut remote: serde_json::Value =
        serde_json::from_slice(&fs::read(&status_path).unwrap()).unwrap();
    let host = remote["process_identity"]["host_name"]
        .as_str()
        .unwrap()
        .to_string();
    remote["process_identity"]["host_name"] = serde_json::json!(format!("{host}-remote"));
    let mut before = serde_json::to_vec_pretty(&remote).unwrap();
    before.push(b'\n');
    fs::write(&status_path, &before).unwrap();

    let (status, owner_unknown) = reconcile_session_status(&resolved.path, session_id).unwrap();

    assert_eq!(status.status, "running");
    assert!(owner_unknown.unwrap().contains("differs from local host"));
    assert_eq!(fs::read(&status_path).unwrap(), before);
    assert!(!status_path.parent().unwrap().join("history").exists());
    fs::remove_dir_all(root).ok();
}

#[test]
fn legacy_v1_dead_process_derives_absent_partial_counters_without_inferring_commit() {
    let (root, resolved) = test_vault_with_registered_dense_lens("issue1698-v1-dead-owner");
    let jsonl = resolved.path.join("issue1698-v1-dead-owner.jsonl");
    fs::write(
        &jsonl,
        format!(
            "{}\n{}\n{}\n",
            batch_line("issue1698 v1 first"),
            batch_line("issue1698 v1 second"),
            batch_line("issue1698 v1 third")
        ),
    )
    .unwrap();
    let validation = validate_batch_file(&jsonl).unwrap();
    let identity = BatchSnapshotIdentity::capture(&jsonl, validation.row_count).unwrap();
    let session_id = "issue1698-v1-dead-owner";
    let mut session =
        BatchIngestSession::start(&resolved, &jsonl, &validation, &identity, Some(session_id))
            .unwrap();
    session.record_rows_started(2, "batch_flush_start").unwrap();
    let dead_pid = exited_child_pid();
    let status_path = resolved
        .path
        .join("idx/ingest/runs")
        .join(session_id)
        .join("status.json");
    let mut legacy: serde_json::Value =
        serde_json::from_slice(&fs::read(&status_path).unwrap()).unwrap();
    legacy["schema_version"] = serde_json::json!(1);
    legacy["process_id"] = serde_json::json!(dead_pid);
    legacy.as_object_mut().unwrap().remove("process_identity");
    legacy.as_object_mut().unwrap().remove("pending_rows");
    legacy
        .as_object_mut()
        .unwrap()
        .remove("uncommitted_started_rows");
    legacy.as_object_mut().unwrap().remove("terminal");
    let mut before = serde_json::to_vec_pretty(&legacy).unwrap();
    before.push(b'\n');
    fs::write(&status_path, &before).unwrap();

    let (after, owner_unknown) = reconcile_session_status(&resolved.path, session_id).unwrap();

    assert_eq!(owner_unknown, None);
    assert_eq!(after.status, "abandoned");
    assert_eq!(after.planned_row_count, 3);
    assert_eq!(after.rows_started, 2);
    assert_eq!(after.rows_committed, 0);
    assert_eq!(after.pending_rows, 1);
    assert_eq!(after.uncommitted_started_rows, 2);
    assert_eq!(after.committed_new_rows, 0);
    assert!(
        after
            .error
            .unwrap()
            .message
            .contains("no commit success was inferred")
    );
    fs::remove_dir_all(root).ok();
}

#[cfg(windows)]
fn exited_child_pid() -> u32 {
    let mut child = std::process::Command::new("cmd")
        .args(["/C", "exit", "0"])
        .spawn()
        .expect("spawn real Windows child");
    let pid = child.id();
    assert!(child.wait().expect("wait for real Windows child").success());
    pid
}

#[cfg(unix)]
fn exited_child_pid() -> u32 {
    let mut child = std::process::Command::new("/bin/sh")
        .args(["-c", "exit 0"])
        .spawn()
        .expect("spawn real Unix child");
    let pid = child.id();
    assert!(child.wait().expect("wait for real Unix child").success());
    pid
}
