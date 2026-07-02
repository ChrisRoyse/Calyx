use super::*;

#[test]
fn search_attaches_provenance_only_after_ledger_readback() {
    let fixture = Fixture::new("happy");
    let vault = fixture.open_vault();
    let state = load_vault_panel_state(&fixture.vault_dir).unwrap();

    let outcome = search_outcome(
        &vault,
        &state,
        &fixture.vault_dir,
        "alpha",
        1,
        FusionChoice::Rrf,
        GuardChoice::Off,
        None,
        false,
    )
    .expect("search succeeds");
    let hit = outcome.hits.first().expect("hit");

    assert_eq!(hit.cx_id, fixture.cx_id);
    assert_eq!(hit.provenance, fixture.ledger_ref);
    maybe_write_fsv_json(
        "shared-search-provenance-happy-path.json",
        &json!({
            "source_of_truth": "Aster Base CF row, Aster Ledger CF row, and persisted search index idmap",
            "before": fixture.readback(),
            "index_candidates": fixture.index_candidates(&state),
            "search_hit": {
                "cx_id": hit.cx_id.to_string(),
                "ledger_seq": hit.provenance.seq,
                "ledger_hash": hex32(&hit.provenance.hash),
                "provenance_matches_base": hit.provenance == fixture.ledger_ref,
            }
        }),
    );
    fixture.cleanup();
}

#[test]
fn stale_ok_search_tags_hits_with_manifest_lag() {
    let fixture = Fixture::new("stale-ok");
    let vault = fixture.open_vault();
    let state = load_vault_panel_state(&fixture.vault_dir).unwrap();
    let before = fixture.readback();
    let extra = measure_constellation(
        &vault,
        &state,
        Input::new(Modality::Text, b"zeta".to_vec()),
        1,
    )
    .expect("measure extra row");
    vault.put(extra).expect("write row after index rebuild");
    vault.flush().expect("flush stale-producing write");
    let after = fixture.readback();

    let fresh_error = match search_outcome(
        &vault,
        &state,
        &fixture.vault_dir,
        "alpha",
        1,
        FusionChoice::Rrf,
        GuardChoice::Off,
        None,
        false,
    ) {
        Ok(_) => panic!("fresh search must reject stale manifest"),
        Err(error) => error,
    };
    let stale_outcome = search_outcome_with_freshness(
        &vault,
        &state,
        &fixture.vault_dir,
        "alpha",
        1,
        FusionChoice::Rrf,
        GuardChoice::Off,
        None,
        false,
        SearchFreshness::StaleOk,
    )
    .expect("stale-ok search succeeds with explicit freshness tag");
    let hit = stale_outcome.hits.first().expect("stale-ok hit");

    assert_eq!(fresh_error.code(), "CALYX_STALE_DERIVED");
    assert_eq!(hit.cx_id, fixture.cx_id);
    assert_eq!(hit.freshness.policy, "stale_ok");
    assert_eq!(
        hit.freshness.built_at_seq,
        before["manifest"]["base_seq"].as_u64().unwrap()
    );
    assert_eq!(
        hit.freshness.base_seq,
        after["vault_manifest"]["durable_seq"].as_u64().unwrap()
    );
    assert!(hit.freshness.stale_by > 0);
    maybe_write_fsv_json(
        "issue1036-stale-ok-freshness-readback.json",
        &json!({
            "source_of_truth": "idx/search/manifest.json base_seq, vault MANIFEST durable_seq, and search hit freshness tag",
            "trigger": "write and flush an extra real measured constellation after rebuilding the search index",
            "before": before,
            "after": after,
            "fresh_error": error_json(&fresh_error),
            "stale_hit": {
                "cx_id": hit.cx_id.to_string(),
                "freshness": hit.freshness,
                "ledger_seq": hit.provenance.seq,
                "ledger_hash": hex32(&hit.provenance.hash),
            }
        }),
    );
    fixture.cleanup();
}

/// Regression for #1103: a content-neutral commit (an idempotency-ledger
/// append) advances the pinned vault seq past the search manifest's base_seq
/// but does NOT change derived-search inputs. Fresh search must still succeed
/// and return real hits — it must not self-stale by one on the raw seq gap.
///
/// Before #1106 the freshness gate compared `manifest.base_seq` against the
/// raw pinned seq with exact equality, so any Ledger/TimeIndex-only commit
/// after the rebuild made `search --fresh` fail closed with
/// `CALYX_STALE_DERIVED` even though the corpus was unchanged. This test pins
/// that the watermark gate (`derived_content_seq <= base_seq <= pinned_seq`)
/// keeps Fresh available across a content-neutral seq advance.
#[test]
fn fresh_search_survives_content_neutral_seq_advance() {
    use calyx_ledger::{ActorId, EntryKind, SubjectId};

    let fixture = Fixture::new("content-neutral-seq-advance");
    let vault = fixture.open_vault();
    let state = load_vault_panel_state(&fixture.vault_dir).unwrap();

    // Manifest was rebuilt at the ingest seq. Capture the source-of-truth
    // watermark state before the content-neutral commit.
    let manifest_base_seq = fixture.readback()["manifest"]["base_seq"]
        .as_u64()
        .expect("manifest base_seq");
    let seq_before = vault.latest_seq();
    let derived_before = vault.derived_content_seq();
    assert_eq!(
        manifest_base_seq, derived_before,
        "precondition: manifest covers the derived-content watermark"
    );

    // Append a Ledger-only entry. Ledger CF does not feed derived search
    // content, so this advances the raw vault seq WITHOUT advancing the
    // derived-content watermark — the exact #1103 trigger.
    vault
        .append_ledger_entry(
            EntryKind::Ingest,
            SubjectId::Cx(fixture.cx_id),
            serde_json::to_vec(&json!({ "mode": "issue1103-idempotent-replay" })).unwrap(),
            ActorId::Service("issue1103-regression".to_string()),
        )
        .expect("append content-neutral ledger entry");
    vault.flush().expect("flush ledger append");

    let seq_after = vault.latest_seq();
    let derived_after = vault.derived_content_seq();
    assert!(
        seq_after > seq_before,
        "ledger append must advance the raw vault seq ({seq_before} -> {seq_after})"
    );
    assert_eq!(
        derived_after, derived_before,
        "content-neutral commit must not advance the derived-content watermark"
    );
    assert!(
        seq_after > manifest_base_seq,
        "the manifest base_seq {manifest_base_seq} must now trail the pinned vault seq {seq_after}"
    );

    // The decisive assertion: Fresh search across the seq gap returns real
    // hits instead of failing closed with CALYX_STALE_DERIVED.
    let outcome = search_outcome(
        &vault,
        &state,
        &fixture.vault_dir,
        "alpha",
        1,
        FusionChoice::Rrf,
        GuardChoice::Off,
        None,
        false,
    )
    .expect("fresh search must not self-stale across a content-neutral seq advance");
    let hit = outcome.hits.first().expect("fresh hit");
    assert_eq!(hit.cx_id, fixture.cx_id);
    assert_eq!(
        hit.freshness.policy, "fresh_derived",
        "hit must carry the fresh-derived freshness policy, not a stale tag"
    );
    assert_eq!(
        hit.freshness.stale_by, 0,
        "a content-neutral seq gap must not count as staleness"
    );

    maybe_write_fsv_json(
        "issue1103-content-neutral-seq-advance.json",
        &json!({
            "source_of_truth": "AsterVault latest_seq/derived_content_seq, idx/search/manifest.json base_seq, and the fresh search hit freshness tag",
            "trigger": "append one Ledger-only (idempotency replay) entry after the search index rebuild",
            "manifest_base_seq": manifest_base_seq,
            "raw_seq_before": seq_before,
            "raw_seq_after": seq_after,
            "derived_content_seq_before": derived_before,
            "derived_content_seq_after": derived_after,
            "fresh_hit": {
                "cx_id": hit.cx_id.to_string(),
                "freshness": hit.freshness,
            }
        }),
    );
    fixture.cleanup();
}

#[test]
fn search_hydrates_each_hit_with_bounded_reader_lease_readback() {
    let fixture = Fixture::new_with_inputs(
        "bounded-hit-hydration",
        &[b"alpha" as &[u8], b"alphabet" as &[u8]],
    );
    let vault = fixture.open_vault();
    let state = load_vault_panel_state(&fixture.vault_dir).unwrap();
    let before = fixture.readback();
    let mut events = Vec::new();
    let mut trace_sink = |event: crate::engine::SearchTraceEvent| {
        events.push(event);
    };

    let outcome = search_outcome_with_slots_traced(
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
    .expect("search succeeds");

    assert!(
        outcome.hits.len() >= 2,
        "fixture should produce at least two physical hits"
    );
    let hit_hydrate_starts = events
        .iter()
        .filter(|event| event.phase == "hit_doc.hydrate.start")
        .count();
    let snapshot_pin_details = events
        .iter()
        .filter(|event| {
            event.phase == "snapshot.pin.done"
                && event
                    .detail
                    .as_deref()
                    .is_some_and(|detail| detail.contains("phase=hit_doc_hydration"))
        })
        .count();

    assert_eq!(hit_hydrate_starts, outcome.hits.len());
    assert_eq!(snapshot_pin_details, outcome.hits.len());
    maybe_write_fsv_json(
        "issue1070-bounded-hit-hydration-happy-path.json",
        &json!({
            "source_of_truth": "Aster Base/Ledger CF rows plus persisted search index manifest and emitted search trace",
            "trigger": "search a two-row physical vault and hydrate every hit with a separate reader lease",
            "before": before,
            "fixture_cx_ids": fixture
                .all_cx_ids
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            "hit_count": outcome.hits.len(),
            "hit_hydrate_start_count": hit_hydrate_starts,
            "snapshot_pin_done_count": snapshot_pin_details,
            "events": events
                .iter()
                .map(trace_event_json)
                .collect::<Vec<_>>(),
        }),
    );
    fixture.cleanup();
}

#[test]
fn search_fails_closed_when_vault_advances_between_hit_hydration_snapshots() {
    let fixture = Fixture::new_with_inputs(
        "hydration-seq-advance",
        &[b"alpha" as &[u8], b"alphabet" as &[u8]],
    );
    let vault = fixture.open_vault();
    let state = load_vault_panel_state(&fixture.vault_dir).unwrap();
    let before = fixture.readback();
    let mut events = Vec::new();
    let mut advanced = false;
    let mut trace_sink = |event: crate::engine::SearchTraceEvent| {
        if event.phase == "hit_doc.hydrate.done" && !advanced {
            let extra = measure_constellation(
                &vault,
                &state,
                Input::new(Modality::Text, b"row inserted during hydration".to_vec()),
                1,
            )
            .expect("measure hydration-advance row");
            vault.put(extra).expect("advance vault during hydration");
            vault.flush().expect("flush hydration-advance row");
            advanced = true;
        }
        events.push(event);
    };

    let error = match search_outcome_with_slots_traced(
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
    ) {
        Ok(_) => panic!("search must fail closed when Base advances during hydration"),
        Err(error) => error,
    };
    let after = fixture.readback();

    assert!(
        advanced,
        "test must advance the real vault during hydration"
    );
    assert_eq!(error.code(), "CALYX_STALE_DERIVED");
    assert!(
        error
            .message()
            .contains("vault advanced during search hit hydration"),
        "error should name the hydration snapshot consistency failure: {error}"
    );
    assert!(
        after["vault_manifest"]["durable_seq"].as_u64().unwrap()
            > before["vault_manifest"]["durable_seq"].as_u64().unwrap()
    );
    maybe_write_fsv_json(
        "issue1070-hydration-seq-advance-fail-closed.json",
        &json!({
            "source_of_truth": "Aster MANIFEST durable_seq and persisted search index manifest base_seq after a real write during hydration",
            "trigger": "insert and flush a real measured constellation after the first hydrated hit but before the second hydration snapshot",
            "before": before,
            "after": after,
            "error": error_json(&error),
            "events": events
                .iter()
                .map(trace_event_json)
                .collect::<Vec<_>>(),
        }),
    );
    fixture.cleanup();
}

#[test]
fn search_budget_fails_closed_during_hit_hydration() {
    let fixture = Fixture::new_with_inputs(
        "hydration-budget",
        &[b"alpha" as &[u8], b"alphabet" as &[u8]],
    );
    let vault = fixture.open_vault();
    let state = load_vault_panel_state(&fixture.vault_dir).unwrap();
    let query_vectors = measure_query_vectors(&state, "alpha").expect("measure query");
    let mut phases = Vec::new();
    let mut budget = |phase: &'static str, processed: usize| {
        phases.push((phase, processed));
        if phase == "before_hit_doc_hydration" {
            return Err(calyx_core::CalyxError {
                code: "CALYX_CLI_TIMEOUT",
                message: format!("test budget exceeded during {phase} after {processed}"),
                remediation: "inspect the emitted progress phase",
            }
            .into());
        }
        Ok(())
    };

    let error = match search_outcome_with_query_vectors_freshness(
        &vault,
        &fixture.vault_dir,
        &query_vectors,
        2,
        FusionChoice::Rrf,
        GuardChoice::Off,
        None,
        None,
        false,
        SearchFreshness::Fresh,
        SearchBudget::new(&mut budget),
        None,
    ) {
        Ok(_) => panic!("budgeted search must fail closed during hit hydration"),
        Err(error) => error,
    };

    assert_eq!(error.code(), "CALYX_CLI_TIMEOUT");
    assert!(
        phases
            .iter()
            .any(|(phase, _)| *phase == "before_hit_doc_hydration")
    );
    fixture.cleanup();
}
