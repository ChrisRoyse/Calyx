use super::*;

#[test]
fn single_text_ingest_retains_hash_verified_source_bytes() {
    let (root, resolved) = test_vault_with_registered_dense_lens("retained-single");
    let text = "issue1423 single retained input";

    let reports = ingest_texts(&resolved, &[text.to_string()]).unwrap();
    let cx_id = reports[0].cx_id.parse::<CxId>().unwrap();
    let vault = open_vault(&resolved).unwrap();
    let stored = vault.get(cx_id, vault.snapshot()).unwrap();
    let replay = calyx_aster::retained_input::input_from_ref(
        &resolved.path,
        Modality::Text,
        &stored.input_ref,
    )
    .unwrap();
    let expected_pointer =
        calyx_aster::retained_input::canonical_text_pointer(&stored.input_ref.hash);

    assert_eq!(replay.bytes, text.as_bytes());
    assert_eq!(
        stored.input_ref.hash,
        *blake3::hash(text.as_bytes()).as_bytes()
    );
    assert_eq!(
        stored.input_ref.pointer.as_deref(),
        Some(expected_pointer.as_str())
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn exact_single_replay_backfills_pointerless_base_with_migrate_ledger() {
    let (root, resolved) = test_vault_with_registered_dense_lens("retained-migrate");
    let text = "issue1423 legacy pointerless input";
    let cx_id = put_pointerless(&resolved, text);

    let reports = ingest_texts(&resolved, &[text.to_string()]).unwrap();
    let vault = open_vault(&resolved).unwrap();
    let stored = vault.get(cx_id, vault.snapshot()).unwrap();
    let migration = ledger_entries(&vault)
        .into_iter()
        .find(|entry| entry.kind == calyx_ledger::EntryKind::Migrate)
        .expect("migration ledger entry");

    assert!(!reports[0].new);
    assert!(stored.input_ref.pointer.is_some());
    assert_eq!(stored.provenance.seq, migration.seq);
    assert_eq!(stored.provenance.hash, migration.entry_hash);
    assert_eq!(
        calyx_aster::retained_input::input_from_ref(
            &resolved.path,
            Modality::Text,
            &stored.input_ref,
        )
        .unwrap()
        .bytes,
        text.as_bytes()
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn conflicting_existing_pointer_rejects_replay_without_base_mutation() {
    let (root, resolved) = test_vault_with_registered_dense_lens("retained-conflict");
    let text = "issue1423 conflicting pointer";
    let cx_id = put_with_pointer(
        &resolved,
        text,
        "calyx-vault://inputs/noncanonical-existing.bin",
    );
    let before = {
        let vault = open_vault(&resolved).unwrap();
        vault.get(cx_id, vault.snapshot()).unwrap()
    };

    let error = match ingest_texts(&resolved, &[text.to_string()]) {
        Ok(_) => panic!("conflicting pointer replay unexpectedly succeeded"),
        Err(error) => error,
    };
    let vault = open_vault(&resolved).unwrap();
    let after = vault.get(cx_id, vault.snapshot()).unwrap();

    assert!(error.message().contains("input_ref fields=pointer"));
    assert_eq!(after.input_ref, before.input_ref);
    assert_eq!(after.provenance, before.provenance);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn batch_ingest_retains_new_rows_and_migrates_pointerless_rows() {
    let (root, resolved) = test_vault_with_registered_dense_lens("retained-batch");
    let legacy = "issue1423 batch legacy";
    let fresh = "issue1423 batch fresh";
    let metadata = BTreeMap::from([
        (
            "source_dataset".to_string(),
            "issue1423-retention-test".to_string(),
        ),
        (
            "source_sha256".to_string(),
            "sha256-issue1423-retention-test".to_string(),
        ),
        (
            "source_url".to_string(),
            "https://example.test/issue1423-retention-test".to_string(),
        ),
        ("license".to_string(), "CC-BY-4.0".to_string()),
        (
            "retrieval_ts".to_string(),
            "2026-07-10T00:00:00Z".to_string(),
        ),
    ]);
    let legacy_id = put_pointerless_with_metadata(&resolved, legacy, metadata.clone());
    let batch = root.join("retained.jsonl");
    fs::write(
        &batch,
        format!(
            "{}\n{}\n",
            json!({"text": legacy, "metadata": metadata.clone()}),
            json!({"text": fresh, "metadata": metadata})
        ),
    )
    .unwrap();

    let summary = ingest_batch_streaming(&resolved, &batch).unwrap();
    let vault = open_vault(&resolved).unwrap();
    let legacy_stored = vault.get(legacy_id, vault.snapshot()).unwrap();
    let fresh_id = vault.cx_id_for_input(fresh.as_bytes(), 1);
    let fresh_stored = vault.get(fresh_id, vault.snapshot()).unwrap();

    assert_eq!(summary.new_count, 1);
    assert_eq!(summary.already_count, 1);
    for (stored, expected) in [(&legacy_stored, legacy), (&fresh_stored, fresh)] {
        assert!(stored.input_ref.pointer.is_some());
        assert_eq!(
            calyx_aster::retained_input::input_from_ref(
                &resolved.path,
                Modality::Text,
                &stored.input_ref,
            )
            .unwrap()
            .bytes,
            expected.as_bytes()
        );
    }
    assert!(
        ledger_entries(&vault)
            .iter()
            .any(|entry| entry.kind == calyx_ledger::EntryKind::Migrate)
    );
    fs::remove_dir_all(root).unwrap();
}

fn put_pointerless(resolved: &ResolvedVault, text: &str) -> CxId {
    put_measured(resolved, text, None, BTreeMap::new())
}

fn put_pointerless_with_metadata(
    resolved: &ResolvedVault,
    text: &str,
    metadata: BTreeMap<String, String>,
) -> CxId {
    put_measured(resolved, text, None, metadata)
}

fn put_with_pointer(resolved: &ResolvedVault, text: &str, pointer: &str) -> CxId {
    put_measured(resolved, text, Some(pointer), BTreeMap::new())
}

fn put_measured(
    resolved: &ResolvedVault,
    text: &str,
    pointer: Option<&str>,
    metadata: BTreeMap<String, String>,
) -> CxId {
    let vault = open_vault(resolved).unwrap();
    let state = load_vault_panel_state(&resolved.path).unwrap();
    let mut input = text_input(text.to_string());
    input.pointer = pointer.map(str::to_string);
    let mut cx = measure_constellation(&vault, &state, input, now_ms()).unwrap();
    cx.metadata = metadata;
    let cx_id = cx.cx_id;
    vault.put(cx).unwrap();
    vault.flush().unwrap();
    cx_id
}

fn ledger_entries(vault: &AsterVault) -> Vec<calyx_ledger::LedgerEntry> {
    vault
        .scan_cf_at(vault.snapshot(), ColumnFamily::Ledger)
        .unwrap()
        .into_iter()
        .map(|(_, bytes)| calyx_ledger::decode(&bytes).unwrap())
        .collect()
}
