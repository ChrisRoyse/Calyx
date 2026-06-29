use super::*;

#[test]
fn batch_ingest_threads_anchors_into_base_cf_and_anchors_cf() {
    let (root, resolved) = test_vault_with_registered_dense_lens("anchors-at-ingest");
    let jsonl = resolved.path.join("anchored.jsonl");
    fs::write(
        &jsonl,
        concat!(
            r#"{"text":"alpha north signal","metadata":{"source_dataset":"medqa"},"#,
            r#""anchors":[{"kind":"label:answer","value":"B"},{"kind":"test-pass","value":"true"}]}"#,
            "\n",
        ),
    )
    .unwrap();

    ingest_batch_streaming(&resolved, &jsonl).unwrap();

    // FSV: read the anchors back from the stored constellation (base-CF), not from
    // the ingest return value. cx_id is derived from the input bytes + panel version.
    let vault = open_vault(&resolved).unwrap();
    let state = load_vault_panel_state(&resolved.path).unwrap();
    let cx_id = vault.cx_id_for_input("alpha north signal".as_bytes(), state.panel.version);
    let snapshot = vault.snapshot();
    let cx = vault.get(cx_id, snapshot).unwrap();

    assert_eq!(
        cx.anchors.len(),
        2,
        "both anchors persisted on the constellation"
    );
    assert!(cx.anchors.iter().any(|anchor| {
        anchor.kind == AnchorKind::Label("answer".to_string())
            && anchor.value == AnchorValue::Enum("B".to_string())
            && anchor.source == "calyx-ingest"
            && anchor.confidence == 1.0
    }));
    assert!(cx.anchors.iter().any(|anchor| {
        anchor.kind == AnchorKind::TestPass && anchor.value == AnchorValue::Bool(true)
    }));
    // A constellation carrying its own anchor is grounded at distance 0.
    assert!(
        !cx.flags.ungrounded,
        "anchored constellation is not ungrounded"
    );

    // FSV: anchors are physically present in the Anchors CF — the index the kernel's
    // `domain_anchors(kind)` reads to find grounded nodes. One row per (cx, kind).
    let anchor_rows = vault.scan_cf_at(snapshot, ColumnFamily::Anchors).unwrap();
    assert_eq!(anchor_rows.len(), 2, "two anchor rows in the Anchors CF");

    fs::remove_dir_all(root).ok();
}

#[test]
fn batch_reingest_merges_anchors_for_existing_cx() {
    let (root, resolved) = test_vault_with_registered_dense_lens("anchors-backfill");
    let plain = resolved.path.join("plain-backfill.jsonl");
    let anchored = resolved.path.join("anchored-backfill.jsonl");
    fs::write(&plain, "{\"text\":\"alpha north signal\"}\n").unwrap();
    fs::write(
        &anchored,
        concat!(
            r#"{"text":"alpha north signal","#,
            r#""anchors":[{"kind":"label:answer","value":"B"}]}"#,
            "\n",
        ),
    )
    .unwrap();

    ingest_batch_streaming(&resolved, &plain).unwrap();
    let vault_before = open_vault(&resolved).unwrap();
    let state = load_vault_panel_state(&resolved.path).unwrap();
    let cx_id = vault_before.cx_id_for_input("alpha north signal".as_bytes(), state.panel.version);
    let before = vault_before.get(cx_id, vault_before.snapshot()).unwrap();
    assert!(before.anchors.is_empty());
    assert!(before.flags.ungrounded);
    drop(vault_before);

    ingest_batch_streaming(&resolved, &anchored).unwrap();

    let vault_after = open_vault(&resolved).unwrap();
    let snapshot = vault_after.snapshot();
    let after = vault_after.get(cx_id, snapshot).unwrap();
    let anchor_rows = vault_after
        .scan_cf_at(snapshot, ColumnFamily::Anchors)
        .unwrap();
    let ledger_rows = vault_after
        .scan_cf_at(snapshot, ColumnFamily::Ledger)
        .unwrap();

    assert_eq!(after.anchors.len(), 1);
    assert_eq!(
        after.anchors[0].kind,
        AnchorKind::Label("answer".to_string())
    );
    assert_eq!(after.anchors[0].value, AnchorValue::Enum("B".to_string()));
    assert!(!after.flags.ungrounded);
    assert_eq!(anchor_rows.len(), 1);
    assert!(
        ledger_rows.len() >= 3,
        "ingest, idempotent, and anchor ledger rows"
    );

    fs::remove_dir_all(root).ok();
}

#[test]
fn batch_reingest_same_anchored_row_is_idempotent_noop() {
    let (root, resolved) = test_vault_with_registered_dense_lens("anchors-idempotent-replay");
    let jsonl = resolved.path.join("anchored-replay.jsonl");
    fs::write(
        &jsonl,
        concat!(
            r#"{"text":"alpha north signal","metadata":{"source_dataset":"medqa"},"#,
            r#""anchors":[{"kind":"label:answer","value":"B"}]}"#,
            "\n",
        ),
    )
    .unwrap();

    ingest_batch_streaming(&resolved, &jsonl).unwrap();
    let vault_before = open_vault(&resolved).unwrap();
    let state = load_vault_panel_state(&resolved.path).unwrap();
    let cx_id = vault_before.cx_id_for_input("alpha north signal".as_bytes(), state.panel.version);
    let snapshot_before = vault_before.snapshot();
    let before = vault_before.get(cx_id, snapshot_before).unwrap();
    let before_base_rows = vault_before
        .scan_cf_at(snapshot_before, ColumnFamily::Base)
        .unwrap();
    let before_anchor_rows = vault_before
        .scan_cf_at(snapshot_before, ColumnFamily::Anchors)
        .unwrap();
    let before_ledger_rows = vault_before
        .scan_cf_at(snapshot_before, ColumnFamily::Ledger)
        .unwrap();
    assert_eq!(before.anchors.len(), 1);
    assert_eq!(before_base_rows.len(), 1);
    assert_eq!(before_anchor_rows.len(), 1);
    drop(vault_before);

    ingest_batch_streaming(&resolved, &jsonl).unwrap();

    let vault_after = open_vault(&resolved).unwrap();
    let snapshot_after = vault_after.snapshot();
    let after = vault_after.get(cx_id, snapshot_after).unwrap();
    let after_base_rows = vault_after
        .scan_cf_at(snapshot_after, ColumnFamily::Base)
        .unwrap();
    let after_anchor_rows = vault_after
        .scan_cf_at(snapshot_after, ColumnFamily::Anchors)
        .unwrap();
    let after_ledger_rows = vault_after
        .scan_cf_at(snapshot_after, ColumnFamily::Ledger)
        .unwrap();

    assert_eq!(after.anchors, before.anchors);
    assert_eq!(after.metadata, before.metadata);
    assert_eq!(
        after_base_rows, before_base_rows,
        "duplicate replay must not rewrite the Base CF row"
    );
    assert_eq!(
        after_anchor_rows, before_anchor_rows,
        "duplicate replay must not duplicate Anchors CF rows"
    );
    assert_eq!(
        after_ledger_rows.len(),
        before_ledger_rows.len() + 1,
        "duplicate replay records exactly one idempotent ingest ledger row"
    );

    fs::remove_dir_all(root).ok();
}

#[test]
fn batch_reingest_same_anchor_changed_metadata_fails_loud() {
    let (root, resolved) = test_vault_with_registered_dense_lens("anchors-metadata-conflict");
    let first_jsonl = resolved.path.join("anchored-first.jsonl");
    let changed_jsonl = resolved.path.join("anchored-changed.jsonl");
    fs::write(
        &first_jsonl,
        concat!(
            r#"{"text":"alpha north signal","metadata":{"source_dataset":"medqa"},"#,
            r#""anchors":[{"kind":"label:answer","value":"B"}]}"#,
            "\n",
        ),
    )
    .unwrap();
    fs::write(
        &changed_jsonl,
        concat!(
            r#"{"text":"alpha north signal","metadata":{"source_dataset":"other"},"#,
            r#""anchors":[{"kind":"label:answer","value":"B"}]}"#,
            "\n",
        ),
    )
    .unwrap();

    ingest_batch_streaming(&resolved, &first_jsonl).unwrap();
    let vault_before = open_vault(&resolved).unwrap();
    let snapshot_before = vault_before.snapshot();
    let before_base_rows = vault_before
        .scan_cf_at(snapshot_before, ColumnFamily::Base)
        .unwrap();
    let before_anchor_rows = vault_before
        .scan_cf_at(snapshot_before, ColumnFamily::Anchors)
        .unwrap();
    drop(vault_before);

    let err = ingest_batch_streaming(&resolved, &changed_jsonl).unwrap_err();
    assert_eq!(err.code(), "CALYX_CLI_USAGE_ERROR");
    assert!(
        err.message().contains("changed stored non-anchor identity"),
        "{}",
        err.message()
    );

    let vault_after = open_vault(&resolved).unwrap();
    let snapshot_after = vault_after.snapshot();
    let after_base_rows = vault_after
        .scan_cf_at(snapshot_after, ColumnFamily::Base)
        .unwrap();
    let after_anchor_rows = vault_after
        .scan_cf_at(snapshot_after, ColumnFamily::Anchors)
        .unwrap();
    assert_eq!(after_base_rows, before_base_rows);
    assert_eq!(after_anchor_rows, before_anchor_rows);

    fs::remove_dir_all(root).ok();
}

#[test]
fn batch_mixed_new_then_changed_metadata_fails_before_partial_write() {
    let (root, resolved) = test_vault_with_registered_dense_lens("mixed-preflight-conflict");
    let first_jsonl = resolved.path.join("anchored-first.jsonl");
    let mixed_jsonl = resolved.path.join("anchored-mixed-conflict.jsonl");
    fs::write(
        &first_jsonl,
        concat!(
            r#"{"text":"alpha north signal","metadata":{"source_dataset":"medqa"},"#,
            r#""anchors":[{"kind":"label:answer","value":"B"}]}"#,
            "\n",
        ),
    )
    .unwrap();
    fs::write(
        &mixed_jsonl,
        concat!(
            r#"{"text":"new row before conflict","metadata":{"source_dataset":"medqa"},"#,
            r#""anchors":[{"kind":"label:answer","value":"A"}]}"#,
            "\n",
            r#"{"text":"alpha north signal","metadata":{"source_dataset":"other"},"#,
            r#""anchors":[{"kind":"label:answer","value":"B"}]}"#,
            "\n",
        ),
    )
    .unwrap();

    ingest_batch_streaming(&resolved, &first_jsonl).unwrap();
    let vault_before = open_vault(&resolved).unwrap();
    let snapshot_before = vault_before.snapshot();
    let before_base_rows = vault_before
        .scan_cf_at(snapshot_before, ColumnFamily::Base)
        .unwrap();
    let before_anchor_rows = vault_before
        .scan_cf_at(snapshot_before, ColumnFamily::Anchors)
        .unwrap();
    let before_ledger_rows = vault_before
        .scan_cf_at(snapshot_before, ColumnFamily::Ledger)
        .unwrap();
    drop(vault_before);

    let err = ingest_batch_streaming(&resolved, &mixed_jsonl).unwrap_err();
    assert_eq!(err.code(), "CALYX_CLI_USAGE_ERROR");
    assert!(
        err.message()
            .contains("changed stored non-anchor identity: metadata"),
        "{}",
        err.message()
    );

    let vault_after = open_vault(&resolved).unwrap();
    let snapshot_after = vault_after.snapshot();
    let after_base_rows = vault_after
        .scan_cf_at(snapshot_after, ColumnFamily::Base)
        .unwrap();
    let after_anchor_rows = vault_after
        .scan_cf_at(snapshot_after, ColumnFamily::Anchors)
        .unwrap();
    let after_ledger_rows = vault_after
        .scan_cf_at(snapshot_after, ColumnFamily::Ledger)
        .unwrap();
    assert_eq!(after_base_rows, before_base_rows);
    assert_eq!(after_anchor_rows, before_anchor_rows);
    assert_eq!(after_ledger_rows, before_ledger_rows);

    fs::remove_dir_all(root).ok();
}

#[test]
fn batch_reingest_same_anchor_changed_value_fails_loud() {
    let (root, resolved) = test_vault_with_registered_dense_lens("anchors-value-conflict");
    let first_jsonl = resolved.path.join("anchored-first.jsonl");
    let changed_jsonl = resolved.path.join("anchored-value-conflict.jsonl");
    fs::write(
        &first_jsonl,
        concat!(
            r#"{"text":"alpha north signal","metadata":{"source_dataset":"medqa"},"#,
            r#""anchors":[{"kind":"label:answer","value":"B"}]}"#,
            "\n",
        ),
    )
    .unwrap();
    fs::write(
        &changed_jsonl,
        concat!(
            r#"{"text":"alpha north signal","metadata":{"source_dataset":"medqa"},"#,
            r#""anchors":[{"kind":"label:answer","value":"C"}]}"#,
            "\n",
        ),
    )
    .unwrap();

    ingest_batch_streaming(&resolved, &first_jsonl).unwrap();
    let before = ingest_cf_state(&resolved);
    println!("anchor_value_conflict_before_cf_state={before}");
    let vault_before = open_vault(&resolved).unwrap();
    let snapshot_before = vault_before.snapshot();
    let before_base_rows = vault_before
        .scan_cf_at(snapshot_before, ColumnFamily::Base)
        .unwrap();
    let before_anchor_rows = vault_before
        .scan_cf_at(snapshot_before, ColumnFamily::Anchors)
        .unwrap();
    let before_ledger_rows = vault_before
        .scan_cf_at(snapshot_before, ColumnFamily::Ledger)
        .unwrap();
    drop(vault_before);

    let err = ingest_batch_streaming(&resolved, &changed_jsonl).unwrap_err();
    assert_eq!(err.code(), "CALYX_ASTER_CORRUPT_SHARD");
    assert!(err.message().contains("conflicting"), "{}", err.message());

    let vault_after = open_vault(&resolved).unwrap();
    let snapshot_after = vault_after.snapshot();
    let after_base_rows = vault_after
        .scan_cf_at(snapshot_after, ColumnFamily::Base)
        .unwrap();
    let after_anchor_rows = vault_after
        .scan_cf_at(snapshot_after, ColumnFamily::Anchors)
        .unwrap();
    let after_ledger_rows = vault_after
        .scan_cf_at(snapshot_after, ColumnFamily::Ledger)
        .unwrap();
    let after = ingest_cf_state(&resolved);
    println!("anchor_value_conflict_after_cf_state={after}");
    assert_eq!(after_base_rows, before_base_rows);
    assert_eq!(after_anchor_rows, before_anchor_rows);
    assert_eq!(
        after_ledger_rows, before_ledger_rows,
        "conflicting anchor replay must not append ledger rows"
    );

    fs::remove_dir_all(root).ok();
}

#[test]
fn batch_staging_predicate_requires_new_cx_or_new_anchor_kind() {
    assert!(should_stage_batch_constellation(true, &[]));
    assert!(should_stage_batch_constellation(
        false,
        &[AnchorKind::Label("answer".to_string())]
    ));
    assert!(!should_stage_batch_constellation(false, &[]));
}

#[test]
fn batch_ingest_without_anchors_stays_ungrounded() {
    let (root, resolved) = test_vault_with_registered_dense_lens("no-anchors-at-ingest");
    let jsonl = resolved.path.join("plain.jsonl");
    fs::write(&jsonl, "{\"text\":\"beta south signal\"}\n").unwrap();

    ingest_batch_streaming(&resolved, &jsonl).unwrap();

    let vault = open_vault(&resolved).unwrap();
    let state = load_vault_panel_state(&resolved.path).unwrap();
    let cx_id = vault.cx_id_for_input("beta south signal".as_bytes(), state.panel.version);
    let snapshot = vault.snapshot();
    let cx = vault.get(cx_id, snapshot).unwrap();

    assert!(cx.anchors.is_empty());
    assert!(cx.flags.ungrounded, "no anchors => ungrounded stays true");
    assert!(
        vault
            .scan_cf_at(snapshot, ColumnFamily::Anchors)
            .unwrap()
            .is_empty()
    );

    fs::remove_dir_all(root).ok();
}

#[test]
fn batch_jsonl_malformed_anchor_is_loud_usage_error() {
    // Unknown anchor kind must fail loudly (no silent drop of a grounding truth).
    let bad_kind = parse_batch_line(
        0,
        "{\"text\":\"x\",\"anchors\":[{\"kind\":\"bogus\",\"value\":\"y\"}]}",
    )
    .unwrap_err();
    assert_eq!(bad_kind.code(), "CALYX_CLI_USAGE_ERROR");
    assert!(bad_kind.message().contains("line 1"));

    // Out-of-range confidence is rejected at parse time.
    let bad_conf = parse_batch_line(
        4,
        "{\"text\":\"x\",\"anchors\":[{\"kind\":\"label:a\",\"value\":\"v\",\"confidence\":1.5}]}",
    )
    .unwrap_err();
    assert_eq!(bad_conf.code(), "CALYX_CLI_USAGE_ERROR");
    assert!(bad_conf.message().contains("line 5"));
}

#[test]
fn empty_text_and_bad_confidence_are_usage_errors() {
    assert_eq!(
        validate_text("").unwrap_err().code(),
        "CALYX_CLI_USAGE_ERROR"
    );
    assert_eq!(
        parse_anchor(&tokens([
            "v",
            "00000000000000000000000000000000",
            "--kind",
            "label:x",
            "--value",
            "x",
            "--confidence",
            "1.5",
        ]))
        .unwrap_err()
        .code(),
        "CALYX_CLI_USAGE_ERROR"
    );
}

#[test]
fn anchor_unknown_cx_fails_as_vault_access_denied() {
    let (root, resolved) = test_vault("anchor-miss", panel_with_unregistered_text_slot());
    let vault = open_vault(&resolved).unwrap();
    let err = ensure_base_exists(&vault, CxId::from_bytes([9; 16])).unwrap_err();
    assert_eq!(err.code(), "CALYX_VAULT_ACCESS_DENIED");
    fs::remove_dir_all(root).ok();
}

proptest! {
    #[test]
    fn cx_id_derivation_is_deterministic(input in ".*") {
        let salt = b"cli-ingest-salt";
        let left = CxId::from_input(input.as_bytes(), 17, salt);
        let right = CxId::from_input(input.as_bytes(), 17, salt);
        prop_assert_eq!(left, right);
    }
}
