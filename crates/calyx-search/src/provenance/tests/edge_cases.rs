use super::*;

#[test]
fn repeated_search_reuses_snapshot_keyed_hydration_and_ledger_verification() {
    let fixture = Fixture::new_with_inputs(
        "hydration-cache-reuse",
        &[b"alpha" as &[u8], b"alphabet" as &[u8]],
    );
    let vault = fixture.open_vault();
    let state = load_vault_panel_state(&fixture.vault_dir).unwrap();
    let run = |events: &mut Vec<crate::engine::SearchTraceEvent>| {
        let mut trace_sink = |event: crate::engine::SearchTraceEvent| {
            events.push(event);
        };
        search_outcome_with_slots_traced(
            &vault,
            &state,
            &fixture.vault_dir,
            "alpha",
            2,
            FusionChoice::Rrf,
            GuardChoice::Off,
            None,
            false,
            None,
            SearchFreshness::Fresh,
            Some(&mut trace_sink),
        )
        .expect("search succeeds")
    };
    let mut first_events = Vec::new();
    let first = run(&mut first_events);
    let mut second_events = Vec::new();
    let second = run(&mut second_events);

    assert_eq!(first.hits, second.hits);
    assert_eq!(first.docs, second.docs);
    let hydrate_done = |events: &[crate::engine::SearchTraceEvent], cached: bool| {
        events
            .iter()
            .filter(|event| event.phase == "hit_doc.hydrate.done")
            .filter(|event| {
                event
                    .detail
                    .as_deref()
                    .is_some_and(|detail| detail.contains(&format!("cached={cached}")))
            })
            .count()
    };
    let pins = |events: &[crate::engine::SearchTraceEvent]| {
        events
            .iter()
            .filter(|event| {
                event.phase == "snapshot.pin.done"
                    && event
                        .detail
                        .as_deref()
                        .is_some_and(|detail| detail.contains("phase=hit_doc_hydration"))
            })
            .count()
    };
    // First run reads every doc from the vault; the repeat run serves each
    // from the MVCC-snapshot-keyed cache while still pinning a reader lease
    // (and re-verifying index freshness) per hit.
    assert_eq!(hydrate_done(&first_events, false), first.hits.len());
    assert_eq!(hydrate_done(&second_events, true), second.hits.len());
    assert_eq!(pins(&second_events), second.hits.len());
    fixture.cleanup();
}

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
