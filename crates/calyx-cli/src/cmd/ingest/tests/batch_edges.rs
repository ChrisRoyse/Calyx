use super::*;
use calyx_ledger::EntryKind;

#[test]
fn microbatch_rejects_mixed_modalities_before_measurement() {
    let (root, resolved) = test_vault_with_registered_dense_lens("mixed-modality");
    let vault = open_vault(&resolved).unwrap();
    let state = load_vault_panel_state(&resolved.path).unwrap();
    let before = vault
        .scan_cf_at(vault.snapshot(), ColumnFamily::Base)
        .unwrap();

    let err = measure_constellation_microbatch(
        &vault,
        &state,
        &[
            text_input("known text input".to_string()),
            Input::new(Modality::Structured, br#"{"k":"v"}"#.to_vec()),
        ],
        1,
    )
    .unwrap_err();

    let after = vault
        .scan_cf_at(vault.snapshot(), ColumnFamily::Base)
        .unwrap();
    assert_eq!(err.code(), "CALYX_LENS_DIM_MISMATCH");
    assert_eq!(
        before, after,
        "failed mixed-modality measurement must not write to Base CF"
    );
    fs::remove_dir_all(root).ok();
}

#[test]
fn retrieval_only_temporal_absence_does_not_degrade_content_ingest() {
    let (root, resolved) =
        test_vault_with_registered_dense_lens_and_temporal_sidecar("temporal-sidecar-degraded");
    let jsonl = resolved.path.join("plain.jsonl");
    fs::write(
        &jsonl,
        format!("{}\n", batch_line("alpha temporal sidecar signal")),
    )
    .unwrap();

    ingest_batch_streaming(&resolved, &jsonl).unwrap();

    let vault = open_vault(&resolved).unwrap();
    let state = load_vault_panel_state(&resolved.path).unwrap();
    let cx_id = vault.cx_id_for_input(
        "alpha temporal sidecar signal".as_bytes(),
        state.panel.version,
    );
    let snapshot = vault.snapshot();
    let cx = vault.get(cx_id, snapshot).unwrap();

    assert!(
        !cx.flags.degraded,
        "expected temporal sidecar absence must not mark content degraded"
    );
    assert!(matches!(
        cx.slots.get(&SlotId::new(0)),
        Some(SlotVector::Dense { dim: 16, .. })
    ));
    assert!(matches!(
        cx.slots.get(&SlotId::new(1)),
        Some(SlotVector::Absent {
            reason: AbsentReason::NotApplicable
        })
    ));

    fs::remove_dir_all(root).ok();
}

#[test]
fn batch_jsonl_empty_and_invalid_edges() {
    let root = temp_root("jsonl");
    fs::create_dir_all(&root).unwrap();
    let empty = root.join("empty.jsonl");
    fs::write(&empty, "").unwrap();
    assert_eq!(validate_batch_file(&empty).unwrap().row_count, 0);
    assert!(read_batch_texts(&empty).unwrap().is_empty());

    let invalid = root.join("bad.jsonl");
    fs::write(&invalid, format!("{}\nnot-json\n", batch_line("ok"))).unwrap();
    let preflight_err = validate_batch_file(&invalid).unwrap_err();
    assert_eq!(preflight_err.code(), "CALYX_CLI_IO_ERROR");
    assert!(preflight_err.message().contains("line 2"));
    let err = read_batch_texts(&invalid).unwrap_err();
    assert_eq!(err.code(), "CALYX_CLI_IO_ERROR");
    assert!(err.message().contains("line 2"));
    fs::remove_dir_all(root).ok();
}

#[test]
fn batch_ingest_requires_and_persists_source_provenance() {
    let (root, resolved) = test_vault_with_registered_dense_lens("provenance-required");
    let text = "provenance persisted row";
    let jsonl = resolved.path.join("provenance.jsonl");
    fs::write(&jsonl, format!("{}\n", batch_line(text))).unwrap();

    ingest_batch_streaming(&resolved, &jsonl).unwrap();

    let vault = open_vault(&resolved).unwrap();
    let state = load_vault_panel_state(&resolved.path).unwrap();
    let cx_id = vault.cx_id_for_input(text.as_bytes(), state.panel.version);
    let snapshot = vault.snapshot();
    let cx = vault.get(cx_id, snapshot).unwrap();
    for key in [
        "source_dataset",
        "source_sha256",
        "source_url",
        "license",
        "retrieval_ts",
    ] {
        assert!(
            cx.metadata.get(key).is_some_and(|value| !value.is_empty()),
            "stored Base CF metadata missing {key}: {:?}",
            cx.metadata
        );
    }
    write_issue1211_valid_fsv(&resolved, snapshot, &cx_id.to_string(), &cx.metadata);
    fs::remove_dir_all(root).ok();
}

#[test]
fn batch_provenance_edge_cases_fail_in_parser_preflight() {
    let root = temp_root("provenance-edge-cases");
    fs::create_dir_all(&root).unwrap();
    let mut observed_cases = Vec::new();
    let cases = [
        (
            "missing-metadata",
            r#"{"text":"missing metadata"}"#,
            "metadata requires source_dataset",
        ),
        (
            "empty-source-dataset",
            r#"{"text":"empty dataset","metadata":{"source_dataset":"","source_sha256":"sha","source_url":"https://example.test/source","license":"CC-BY-4.0","retrieval_ts":"2026-07-04T00:00:00Z"}}"#,
            "metadata requires source_dataset",
        ),
        (
            "missing-source-sha",
            r#"{"text":"missing sha","metadata":{"source_dataset":"dataset","source_url":"https://example.test/source","license":"CC-BY-4.0","retrieval_ts":"2026-07-04T00:00:00Z"}}"#,
            "metadata requires source_sha256",
        ),
        (
            "missing-locator",
            r#"{"text":"missing locator","metadata":{"source_dataset":"dataset","source_sha256":"sha","license":"CC-BY-4.0","retrieval_ts":"2026-07-04T00:00:00Z"}}"#,
            "metadata requires one source locator",
        ),
        (
            "missing-license",
            r#"{"text":"missing license","metadata":{"source_dataset":"dataset","source_sha256":"sha","source_url":"https://example.test/source","retrieval_ts":"2026-07-04T00:00:00Z"}}"#,
            "metadata requires license",
        ),
        (
            "missing-retrieval-ts",
            r#"{"text":"missing retrieval","metadata":{"source_dataset":"dataset","source_sha256":"sha","source_url":"https://example.test/source","license":"CC-BY-4.0"}}"#,
            "metadata requires retrieval_ts",
        ),
    ];
    for (name, line, expected) in cases {
        let path = root.join(format!("{name}.jsonl"));
        fs::write(&path, format!("{line}\n")).unwrap();
        let err = validate_batch_file(&path).unwrap_err();
        assert_eq!(err.code(), "CALYX_CLI_USAGE_ERROR");
        assert!(err.message().contains("line 1"), "{}", err.message());
        assert!(
            err.message().contains(expected),
            "expected {expected:?}, got {}",
            err.message()
        );
        observed_cases.push(json!({
            "case": name,
            "expected_message_fragment": expected,
            "observed_code": err.code(),
            "observed_message": err.message(),
        }));
    }
    write_issue1211_edge_cases_fsv(&observed_cases);
    fs::remove_dir_all(root).ok();
}

#[test]
fn invalid_batch_jsonl_fails_before_vault_open() {
    let root = temp_root("jsonl-preflight-before-vault");
    fs::create_dir_all(&root).unwrap();
    let invalid = root.join("bad.jsonl");
    fs::write(&invalid, "not-json\n").unwrap();
    let missing_vault = root.join("missing-vault");
    let resolved = ResolvedVault {
        path: missing_vault.clone(),
        name: "missing".to_string(),
        vault_id: VaultId::from_ulid(Ulid::new()),
    };

    let err = ingest_batch_streaming(&resolved, &invalid).unwrap_err();

    assert_eq!(err.code(), "CALYX_CLI_IO_ERROR");
    assert!(err.message().contains("batch JSONL line 1 is invalid"));
    assert!(
        !missing_vault.exists(),
        "invalid JSONL must fail before opening or creating vault state"
    );
    fs::remove_dir_all(root).ok();
}

#[test]
fn missing_batch_provenance_fails_before_vault_open() {
    let root = temp_root("provenance-preflight-before-vault");
    fs::create_dir_all(&root).unwrap();
    let invalid = root.join("bad-provenance.jsonl");
    fs::write(&invalid, "{\"text\":\"ungrounded row\"}\n").unwrap();
    let missing_vault = root.join("missing-vault");
    let resolved = ResolvedVault {
        path: missing_vault.clone(),
        name: "missing".to_string(),
        vault_id: VaultId::from_ulid(Ulid::new()),
    };

    let err = ingest_batch_streaming(&resolved, &invalid).unwrap_err();

    assert_eq!(err.code(), "CALYX_CLI_USAGE_ERROR");
    assert!(err.message().contains("metadata requires source_dataset"));
    assert!(
        !missing_vault.exists(),
        "invalid provenance must fail before opening or creating vault state"
    );
    write_issue1211_preflight_fsv(&json!({
        "invalid_batch": invalid,
        "missing_vault": missing_vault,
        "observed_code": err.code(),
        "observed_message": err.message(),
        "vault_path_exists_after_error": missing_vault.exists(),
    }));
    fs::remove_dir_all(root).ok();
}

fn write_issue1211_valid_fsv(
    resolved: &ResolvedVault,
    snapshot: u64,
    cx_id: &str,
    metadata: &BTreeMap<String, String>,
) {
    let Some(root) = issue1211_fsv_root() else {
        return;
    };
    fs::create_dir_all(&root).unwrap();
    let artifact = json!({
        "issue": 1211,
        "source_of_truth": "Aster Base CF readback via vault.get(cx_id, snapshot) after batch ingest flush",
        "vault": {
            "name": resolved.name,
            "vault_id": resolved.vault_id.to_string(),
            "path": resolved.path,
        },
        "snapshot": snapshot,
        "cx_id": cx_id,
        "required_metadata_keys": [
            "source_dataset",
            "source_sha256",
            "source_url",
            "license",
            "retrieval_ts"
        ],
        "stored_metadata": metadata,
    });
    fs::write(
        root.join("valid-row-base-cf-provenance-readback.json"),
        serde_json::to_vec_pretty(&artifact).unwrap(),
    )
    .unwrap();
}

fn write_issue1211_edge_cases_fsv(observed_cases: &[serde_json::Value]) {
    let Some(root) = issue1211_fsv_root() else {
        return;
    };
    fs::create_dir_all(&root).unwrap();
    let artifact = json!({
        "issue": 1211,
        "source_of_truth": "validate_batch_file parser preflight; no vault handle is available in this path",
        "expected": "every malformed provenance case fails closed with CALYX_CLI_USAGE_ERROR and line 1",
        "observed_cases": observed_cases,
    });
    fs::write(
        root.join("parser-preflight-provenance-edge-cases.json"),
        serde_json::to_vec_pretty(&artifact).unwrap(),
    )
    .unwrap();
}

fn write_issue1211_preflight_fsv(observed: &serde_json::Value) {
    let Some(root) = issue1211_fsv_root() else {
        return;
    };
    fs::create_dir_all(&root).unwrap();
    let artifact = json!({
        "issue": 1211,
        "source_of_truth": "missing-vault path existence checked after rejected ingest_batch_streaming call",
        "expected": "invalid provenance fails before vault open, Base CF writes, or Ledger seals",
        "observed": observed,
    });
    fs::write(
        root.join("missing-provenance-fails-before-vault-open.json"),
        serde_json::to_vec_pretty(&artifact).unwrap(),
    )
    .unwrap();
}

fn issue1211_fsv_root() -> Option<std::path::PathBuf> {
    calyx_fsv::fsv_root("CALYX_FSV_ROOT").map(|root| root.join("issue1211-batch-provenance-gate"))
}

#[test]
fn ingest_open_uses_latest_only_router_readback_for_checkpointed_rows() {
    let (root, resolved) = test_vault_with_registered_dense_lens("ingest-latest-only-open");
    let first_text = "first latest-only ingest row";
    let first = resolved.path.join("first.jsonl");
    fs::write(&first, format!("{}\n", batch_line(first_text))).unwrap();

    ingest_batch_streaming(&resolved, &first).unwrap();

    let state = load_vault_panel_state(&resolved.path).unwrap();
    let reopened = open_vault(&resolved).unwrap();
    let first_id = reopened.cx_id_for_input(first_text.as_bytes(), state.panel.version);
    let snapshot = reopened.snapshot();
    let first_row = reopened.get(first_id, snapshot).unwrap();
    assert_eq!(first_row.cx_id, first_id);
    assert_eq!(first_row.panel_version, state.panel.version);

    let seq_error = reopened
        .seq_for_key(ColumnFamily::Base, &base_key(first_id))
        .unwrap_err();
    assert_eq!(
        seq_error.code, "CALYX_ASTER_LATEST_ONLY_HISTORY_UNAVAILABLE",
        "checkpointed rows must be served from router latest readback, not restored into MVCC"
    );

    let second_text = "second latest-only ingest row";
    let second = resolved.path.join("second.jsonl");
    fs::write(&second, format!("{}\n", batch_line(second_text))).unwrap();
    ingest_batch_streaming(&resolved, &second).unwrap();

    let after = open_vault(&resolved).unwrap();
    let after_snapshot = after.snapshot();
    let rows = after
        .scan_cf_at(after_snapshot, ColumnFamily::Base)
        .unwrap();
    assert_eq!(rows.len(), 2);

    fs::remove_dir_all(root).ok();
}

#[test]
fn batch_ingest_rebuilds_stale_base_page_index_for_physical_readback() {
    let (root, resolved) = test_vault_with_registered_dense_lens("ingest-stale-base-page-index");
    let first_text = "first stale base page index row";
    let first = resolved.path.join("first.jsonl");
    fs::write(&first, format!("{}\n", batch_line(first_text))).unwrap();
    ingest_batch_streaming(&resolved, &first).unwrap();

    let built =
        calyx_aster::base_page_index::build_base_page_index(&resolved.path, 2, |_| Ok(())).unwrap();
    assert_eq!(built.live_entries, 1);

    let state = load_vault_panel_state(&resolved.path).unwrap();
    let ledger_only_head = {
        let vault = open_vault(&resolved).unwrap();
        let first_id = vault.cx_id_for_input(first_text.as_bytes(), state.panel.version);
        super::super::ledger::append_cli_ledger(
            &vault,
            EntryKind::Ingest,
            first_id,
            "test-ledger-only-stale-base-page-index",
        )
        .unwrap();
        vault.flush().unwrap();
        vault.snapshot()
    };
    assert!(
        ledger_only_head > built.ledger_head_height,
        "ledger-only write must make the Base page index head stale"
    );

    let second_text = "second stale base page index row";
    let second = resolved.path.join("second.jsonl");
    fs::write(&second, format!("{}\n", batch_line(second_text))).unwrap();
    let summary = ingest_batch_streaming(&resolved, &second).unwrap();
    assert_eq!(summary.new_count, 1);
    assert_eq!(summary.verified_base_rows, 1);

    let after = open_vault(&resolved).unwrap();
    let after_snapshot = after.snapshot();
    let rows = after
        .scan_cf_at(after_snapshot, ColumnFamily::Base)
        .unwrap();
    assert_eq!(rows.len(), 2);

    let rebuilt =
        calyx_aster::base_page_index::read_base_page_index_manifest(&resolved.path).unwrap();
    assert_eq!(
        rebuilt.ledger_head_height, after_snapshot,
        "rebuilt index must be sealed to the post-ingest ledger head"
    );
    assert_eq!(rebuilt.live_entries, 2);

    fs::remove_dir_all(root).ok();
}
