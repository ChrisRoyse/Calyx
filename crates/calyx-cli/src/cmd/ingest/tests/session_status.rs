use super::super::session::{BatchIngestSession, read_session_status};
use super::*;
use crate::cmd::IngestOutput;
use crate::cmd::ingest::route::IngestGpuRoute;

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
    let mut session =
        BatchIngestSession::start(&resolved, &jsonl, &validation, Some(session_id)).unwrap();
    ingest_validated_batch_streaming_with_output(
        &resolved,
        &jsonl,
        IngestOutput::Summary,
        validation.row_count,
        IngestGpuRoute::cold_workers_allowed(),
        None,
        Some(&mut session),
    )
    .unwrap();

    let status_bytes = fs::read(&status_path).unwrap();
    let status: serde_json::Value = serde_json::from_slice(&status_bytes).unwrap();
    println!("issue1065_status_after={status}");
    assert_eq!(status["schema_version"], 1);
    assert_eq!(status["session_id"], session_id);
    assert_eq!(status["status"], "complete");
    assert_eq!(status["phase"], "complete");
    assert_eq!(status["planned_row_count"], 2);
    assert_eq!(status["rows_started"], 2);
    assert_eq!(status["rows_committed"], 2);
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
    let mut session =
        BatchIngestSession::start(&resolved, &jsonl, &validation, Some(session_id)).unwrap();
    ingest_validated_batch_streaming_with_output(
        &resolved,
        &jsonl,
        IngestOutput::Summary,
        validation.row_count,
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
        BatchIngestSession::start(&resolved, &jsonl, &validation, Some(session_id)).unwrap_err();
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
    let mut session =
        BatchIngestSession::start(&resolved, &jsonl, &validation, Some(session_id)).unwrap();
    let err = ingest_validated_batch_streaming_with_output(
        &resolved,
        &jsonl,
        IngestOutput::Summary,
        validation.row_count,
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
