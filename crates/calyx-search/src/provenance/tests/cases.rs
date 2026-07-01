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
