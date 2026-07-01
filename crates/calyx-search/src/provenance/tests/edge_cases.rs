use super::*;

#[test]
fn search_fails_closed_when_index_hit_lacks_stored_constellation() {
    let fixture = Fixture::new("missing-base");
    let state = load_vault_panel_state(&fixture.vault_dir).unwrap();
    let before = fixture.readback();
    let index_candidates = fixture.index_candidates(&state);
    remove_cf_row(
        &fixture.vault_dir,
        ColumnFamily::Base,
        &base_key(fixture.cx_id),
    );
    let after = fixture.readback();

    let error = fixture.search_error(&state);

    assert_eq!(error.code(), CALYX_SEXTANT_PROVENANCE_MISSING);
    assert_eq!(index_candidates, vec![fixture.cx_id.to_string()]);
    assert!(!after["base_exists"].as_bool().unwrap());
    maybe_write_fsv_json(
        "shared-search-provenance-missing-base-fail-closed.json",
        &json!({
            "source_of_truth": "Persisted search index idmap still contains candidate while Aster Base CF lacks the row",
            "trigger": "remove Base CF row after building search index",
            "before": before,
            "after": after,
            "index_candidates": index_candidates,
            "error": error_json(&error),
        }),
    );
    fixture.cleanup();
}

#[test]
fn search_fails_closed_when_hit_ledger_row_is_missing() {
    let fixture = Fixture::new("missing-ledger");
    let state = load_vault_panel_state(&fixture.vault_dir).unwrap();
    let before = fixture.readback();
    let index_candidates = fixture.index_candidates(&state);
    let vault = fixture.open_vault();
    remove_cf_row(
        &fixture.vault_dir,
        ColumnFamily::Ledger,
        &ledger_key(fixture.ledger_ref.seq),
    );
    let after = fixture.readback();

    let error = fixture.search_error_with_vault(&vault, &state);

    assert_eq!(error.code(), CALYX_SEXTANT_PROVENANCE_MISSING);
    assert_eq!(index_candidates, vec![fixture.cx_id.to_string()]);
    assert!(after["base_exists"].as_bool().unwrap());
    assert_eq!(after["ledger_rows"].as_array().unwrap().len(), 0);
    maybe_write_fsv_json(
        "shared-search-provenance-missing-ledger-fail-closed.json",
        &json!({
            "source_of_truth": "Aster Base CF references a ledger seq that is absent from Aster Ledger CF",
            "trigger": "remove Ledger CF row after building search index",
            "before": before,
            "after": after,
            "index_candidates": index_candidates,
            "error": error_json(&error),
        }),
    );
    fixture.cleanup();
}

#[test]
fn search_fails_closed_when_hit_ledger_row_is_corrupt() {
    let fixture = Fixture::new("corrupt-ledger");
    let state = load_vault_panel_state(&fixture.vault_dir).unwrap();
    let before = fixture.readback();
    let index_candidates = fixture.index_candidates(&state);
    let vault = fixture.open_vault();
    corrupt_cf_row(
        &fixture.vault_dir,
        ColumnFamily::Ledger,
        &ledger_key(fixture.ledger_ref.seq),
    );
    let after = fixture.readback();

    let error = fixture.search_error_with_vault(&vault, &state);

    assert_eq!(error.code(), "CALYX_LEDGER_CHAIN_BROKEN");
    assert_eq!(index_candidates, vec![fixture.cx_id.to_string()]);
    maybe_write_fsv_json(
        "shared-search-provenance-corrupt-ledger-fail-closed.json",
        &json!({
            "source_of_truth": "Aster Ledger CF row bytes are present but hash-chain verification rejects them",
            "trigger": "flip one byte in the Ledger CF row after building search index",
            "before": before,
            "after": after,
            "index_candidates": index_candidates,
            "error": error_json(&error),
        }),
    );
    fixture.cleanup();
}
