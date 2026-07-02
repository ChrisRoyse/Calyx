use super::*;

#[test]
fn search_accepts_batch_ingest_ledger_ref_when_payload_names_hit_cx() {
    let root = temp_root("batch-ledger-ref");
    let vault_id = VaultId::from_ulid(Ulid::new());
    let vault_dir = root.join("vault");
    let mut registry = Registry::new();
    let lens = AlgorithmicLens::byte_features("issue918-byte", Modality::Text);
    let contract = lens.contract().clone();
    let lens_id = contract.lens_id();
    let spec = LensSpec {
        name: "issue918-byte".to_string(),
        runtime: LensRuntime::Algorithmic {
            kind: "byte-features".to_string(),
        },
        output: contract.shape(),
        modality: contract.modality(),
        weights_sha256: contract.weights_sha256(),
        corpus_hash: contract.corpus_hash(),
        norm_policy: contract.norm_policy(),
        max_batch: None,
        axis: Some("issue918-byte".to_string()),
        asymmetry: Asymmetry::None,
        quant_default: QuantPolicy::turboquant_default(),
        truncate_dim: None,
        recall_delta: default_recall_delta(),
        retrieval_only: false,
        excluded_from_dedup: false,
    };
    registry
        .register_frozen_with_spec(lens, contract, spec)
        .expect("register lens");
    let panel = panel(lens_id);
    let vault = AsterVault::new_durable(
        &vault_dir,
        vault_id,
        salt(),
        VaultOptions {
            panel: Some(panel.clone()),
            ..VaultOptions::default()
        },
    )
    .expect("open vault");
    persist_vault_panel_state(&vault_dir, &panel, &registry).expect("persist panel");
    let state = VaultPanelState {
        panel,
        registry,
        registry_snapshot: None,
    };
    let first = measure_constellation(
        &vault,
        &state,
        Input::new(Modality::Text, b"alpha".to_vec()),
        1,
    )
    .expect("measure first");
    let second = measure_constellation(
        &vault,
        &state,
        Input::new(Modality::Text, b"omega".to_vec()),
        1,
    )
    .expect("measure second");
    let first_id = first.cx_id;
    let second_id = second.cx_id;

    vault
        .put_batch(vec![first, second])
        .expect("put batch constellations");
    vault.flush().expect("flush vault");
    rebuild_for_vault(&vault_dir, &vault).expect("rebuild search index");
    let first_stored = vault.get(first_id, vault.snapshot()).expect("read first");
    let second_stored = vault.get(second_id, vault.snapshot()).expect("read second");

    assert_eq!(first_stored.provenance, second_stored.provenance);
    assert_ne!(first_id, second_id);

    let outcome = search_outcome(
        &vault,
        &state,
        &vault_dir,
        "omega",
        2,
        FusionChoice::Rrf,
        GuardChoice::Off,
        None,
        false,
    )
    .expect("search succeeds with batch ledger provenance");
    let hit = outcome
        .hits
        .iter()
        .find(|hit| hit.cx_id == second_id)
        .expect("second batch cx appears in hits");
    assert_eq!(hit.provenance, second_stored.provenance);

    maybe_write_fsv_json(
        "shared-search-provenance-batch-ledger-ref.json",
        &json!({
            "source_of_truth": "Aster Base CF rows share one batch Ledger CF row whose payload names both Cx ids",
            "trigger": "put_batch with two measured text constellations, then search for the second input",
            "stored": {
                "first_cx_id": first_id.to_string(),
                "second_cx_id": second_id.to_string(),
                "shared_ledger_ref": first_stored.provenance == second_stored.provenance,
                "ledger_seq": second_stored.provenance.seq,
                "ledger_hash": hex32(&second_stored.provenance.hash),
            },
            "search_hit": {
                "cx_id": hit.cx_id.to_string(),
                "ledger_seq": hit.provenance.seq,
                "ledger_hash": hex32(&hit.provenance.hash),
            },
            "ledger_rows": ledger_rows(&vault_dir),
            "ledger_entries": decoded_ledger_entries(&vault_dir),
        }),
    );
    if calyx_fsv::fsv_root("CALYX_FSV_ROOT").is_none() {
        let _ = fs::remove_dir_all(root);
    }
}

#[test]
fn batch_ingest_subject_mismatch_invalid_payload_fails_actionably() {
    let target = CxId::from_bytes([0x42; 16]);
    let entry = LedgerEntry::new(
        7,
        [0; 32],
        EntryKind::Ingest,
        SubjectId::Query(b"batch-ingest".to_vec()),
        b"{not-json".to_vec(),
        calyx_ledger::ActorId::Service("calyx-search-test".to_string()),
        1,
    );

    let error = entry_covers_cx(&entry, target).unwrap_err();

    assert_eq!(error.code(), "CALYX_LEDGER_CORRUPT");
    assert!(error.message().contains("payload is invalid JSON"));
    assert!(error.message().contains("seq 7"));
    maybe_write_fsv_json(
        "issue979-batch-ledger-invalid-payload-edge.json",
        &json!({
            "source_of_truth": "synthetic valid LedgerEntry decoded by calyx-search provenance verifier",
            "trigger": "EntryKind::Ingest with non-Cx subject and invalid JSON payload",
            "entry": {
                "seq": entry.seq,
                "kind": format!("{:?}", entry.kind),
                "subject": subject_json(&entry.subject),
                "payload_utf8": String::from_utf8_lossy(&entry.payload),
            },
            "error": error_json(&error),
        }),
    );
}
