use super::*;

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
    fs::write(&jsonl, "{\"text\":\"alpha temporal sidecar signal\"}\n").unwrap();

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
    fs::write(&invalid, "{\"text\":\"ok\"}\nnot-json\n").unwrap();
    let preflight_err = validate_batch_file(&invalid).unwrap_err();
    assert_eq!(preflight_err.code(), "CALYX_CLI_IO_ERROR");
    assert!(preflight_err.message().contains("line 2"));
    let err = read_batch_texts(&invalid).unwrap_err();
    assert_eq!(err.code(), "CALYX_CLI_IO_ERROR");
    assert!(err.message().contains("line 2"));
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
