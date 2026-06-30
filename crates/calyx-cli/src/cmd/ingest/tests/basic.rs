use super::*;

#[test]
fn ingest_same_text_twice_returns_same_cx_and_second_is_not_new() {
    let (root, resolved) = test_vault_with_registered_dense_lens("idem");

    let first = ingest_texts(&resolved, &[String::from("hello")]).unwrap();
    let second = ingest_texts(&resolved, &[String::from("hello")]).unwrap();

    assert_eq!(first[0].cx_id, second[0].cx_id);
    assert!(first[0].new);
    assert!(!second[0].new);
    fs::remove_dir_all(root).ok();
}

#[test]
fn ingest_into_fully_unregistered_panel_fails_loud_not_silently_empty() {
    // Doctrine #1273 rule 3: a vault whose every content lens is unavailable must
    // refuse ingest (loud, named), never silently persist an unsearchable cx.
    let (root, resolved) = test_vault("unbound", panel_with_unregistered_text_slot());
    let before = ingest_cf_state(&resolved);
    println!("issue911_before_cf_state={before}");
    let err = match ingest_texts(&resolved, &[String::from("hello")]) {
        Ok(_) => panic!("ingest into a fully-unregistered panel must fail loud, not Ok"),
        Err(e) => e,
    };
    let after = ingest_cf_state(&resolved);
    println!("issue911_after_cf_state={after}");
    assert_eq!(
        err.code(),
        "CALYX_LENS_UNREACHABLE",
        "got: {}",
        err.to_json()
    );
    assert!(
        err.message().contains("0/") && err.message().contains("content lenses"),
        "message must name the unavailable lenses: {}",
        err.message()
    );
    assert_eq!(before["base_rows"], 0);
    assert_eq!(before["ledger_rows"], 0);
    assert_eq!(before["slot_00_rows"], 0);
    assert_eq!(after["base_rows"], 0);
    assert_eq!(after["ledger_rows"], 0);
    assert_eq!(after["slot_00_rows"], 0);
    assert_eq!(
        before["latest_seq"], after["latest_seq"],
        "failed ingest must not advance the durable sequence"
    );
    write_issue911_fsv(&resolved, &before, &after, err.code(), err.message());
    fs::remove_dir_all(root).ok();
}

#[test]
fn ingest_registered_dense_lens_persists_search_index_files() {
    let (root, resolved) = test_vault_with_registered_dense_lens("persist-index");
    let reports = ingest_texts(
        &resolved,
        &[
            String::from("alpha north signal"),
            String::from("beta south signal"),
            String::from("gamma east signal"),
        ],
    )
    .unwrap();

    let manifest_path = resolved.path.join("idx/search/manifest.json");
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    let slot = &manifest["slots"].as_array().unwrap()[0];

    assert!(reports.iter().all(|report| report.new));
    assert_eq!(manifest["format"], "calyx-search-index-manifest-v1");
    assert_eq!(slot["slot"], 0);
    assert_eq!(slot["dim"], 16);
    assert_eq!(slot["len"], 3);
    match slot["kind"].as_str().unwrap() {
        "diskann" => {
            let graph_path = resolved.path.join(slot["graph_rel"].as_str().unwrap());
            let ids_path = resolved.path.join(slot["id_map_rel"].as_str().unwrap());
            let ids: serde_json::Value =
                serde_json::from_slice(&fs::read(&ids_path).unwrap()).unwrap();
            assert!(graph_path.is_file());
            assert_eq!(ids["format"], "calyx-search-index-idmap-v1");
            assert_eq!(ids["ids"].as_array().unwrap().len(), 3);
        }
        "flat_dense" => {
            let index_path = resolved.path.join(slot["index_rel"].as_str().unwrap());
            assert!(index_path.is_file());
            assert_eq!(slot["sha256"].as_str().unwrap().len(), 64);
        }
        other => panic!("unexpected persisted dense index kind {other}"),
    }
    fs::remove_dir_all(root).ok();
}

#[test]
fn batch_ingest_returns_bounded_summary_verified_by_base_cf() {
    let (root, resolved) = test_vault_with_registered_dense_lens("batch-summary");
    let jsonl = resolved.path.join("summary.jsonl");
    fs::write(
        &jsonl,
        "{\"text\":\"alpha summary signal\"}\n{\"text\":\"beta summary signal\"}\n",
    )
    .unwrap();

    let summary = ingest_batch_streaming(&resolved, &jsonl).unwrap();

    let vault = open_vault(&resolved).unwrap();
    let snapshot = vault.snapshot();
    let base_rows = vault.scan_cf_at(snapshot, ColumnFamily::Base).unwrap();
    assert_eq!(summary.status, "ingested");
    assert_eq!(summary.row_count, 2);
    assert_eq!(summary.new_count, 2);
    assert_eq!(summary.already_count, 0);
    assert_eq!(summary.verified_base_rows, 2);
    assert!(summary.first_cx_id.is_some());
    assert!(summary.last_cx_id.is_some());
    assert_eq!(
        base_rows.len(),
        2,
        "source of truth is Base CF, not ingest return value"
    );

    fs::remove_dir_all(root).ok();
}

#[test]
fn batch_ingest_measure_window_persists_all_rows_to_physical_cfs() {
    let (root, resolved) = test_vault_with_registered_dense_lens("issue999-window");
    let jsonl = resolved.path.join("window.jsonl");
    fs::write(
        &jsonl,
        [
            "{\"text\":\"issue999 row 01\"}",
            "{\"text\":\"issue999 row 02\"}",
            "{\"text\":\"issue999 row 03\"}",
            "{\"text\":\"issue999 row 04\"}",
            "{\"text\":\"issue999 row 05\"}",
            "{\"text\":\"issue999 row 06\"}",
            "",
        ]
        .join("\n"),
    )
    .unwrap();
    let before = ingest_cf_state(&resolved);
    println!("issue999_before_cf_state={before}");

    let summary = ingest_batch_streaming(&resolved, &jsonl).unwrap();

    let after = ingest_cf_state(&resolved);
    println!("issue999_after_cf_state={after}");
    assert_eq!(summary.status, "ingested");
    assert_eq!(summary.row_count, 6);
    assert_eq!(summary.new_count, 6);
    assert_eq!(summary.already_count, 0);
    assert_eq!(summary.verified_base_rows, 6);
    assert_eq!(before["base_rows"], 0);
    assert_eq!(before["ledger_rows"], 0);
    assert_eq!(before["slot_00_rows"], 0);
    assert_eq!(after["base_rows"], 6);
    assert_eq!(after["ledger_rows"], 1);
    assert_eq!(after["slot_00_rows"], 6);

    fs::remove_dir_all(root).ok();
}

#[test]
fn batch_reingest_existing_rows_skips_measurement_and_uses_base_cf_source_of_truth() {
    let (root, resolved) = test_vault_with_registered_dense_lens("existing-fast-path");
    let jsonl = resolved.path.join("existing.jsonl");
    fs::write(
        &jsonl,
        "{\"text\":\"existing row alpha\"}\n{\"text\":\"existing row beta\"}\n",
    )
    .unwrap();

    let first = ingest_batch_streaming(&resolved, &jsonl).unwrap();
    assert_eq!(first.new_count, 2);
    let before = ingest_cf_state(&resolved);
    println!("existing_fast_path_before_cf_state={before}");

    let panel = load_vault_panel_state(&resolved.path).unwrap().panel;
    persist_vault_panel_state(&resolved.path, &panel, &Registry::new()).unwrap();
    let state_without_runtime = load_vault_panel_state(&resolved.path).unwrap();
    assert!(
        state_without_runtime
            .registry_snapshot
            .as_ref()
            .unwrap()
            .lenses
            .is_empty(),
        "test precondition: persisted registry has no measurable runtime"
    );

    let replay = ingest_batch_streaming(&resolved, &jsonl).unwrap();

    let after = ingest_cf_state(&resolved);
    println!("existing_fast_path_after_cf_state={after}");
    assert_eq!(replay.status, "ingested");
    assert_eq!(replay.row_count, 2);
    assert_eq!(replay.new_count, 0);
    assert_eq!(replay.already_count, 2);
    assert_eq!(replay.verified_base_rows, 2);
    assert_eq!(after["base_rows"], before["base_rows"]);
    assert_eq!(after["slot_00_rows"], before["slot_00_rows"]);
    assert_eq!(
        after["ledger_rows"].as_u64().unwrap(),
        before["ledger_rows"].as_u64().unwrap() + 1,
        "one idempotent replay batch ledger row is physically present"
    );

    fs::remove_dir_all(root).ok();
}

#[test]
fn batch_reingest_existing_anchored_rows_uses_base_anchor_cfs_source_of_truth() {
    let (root, resolved) = test_vault_with_registered_dense_lens("anchored-existing-fast-path");
    let jsonl = resolved.path.join("anchored-existing.jsonl");
    fs::write(
        &jsonl,
        concat!(
            r#"{"text":"anchored existing row alpha","metadata":{"source_dataset":"issue999"},"#,
            r#""anchors":[{"kind":"label:campaign","value":"calyx15000"},{"kind":"label:source_type","value":"test"}]}"#,
            "\n",
            r#"{"text":"anchored existing row beta","metadata":{"source_dataset":"issue999"},"#,
            r#""anchors":[{"kind":"label:campaign","value":"calyx15000"},{"kind":"label:source_type","value":"test"}]}"#,
            "\n",
        ),
    )
    .unwrap();

    let first = ingest_batch_streaming(&resolved, &jsonl).unwrap();
    assert_eq!(first.new_count, 2);
    let before = ingest_cf_state(&resolved);
    println!("anchored_existing_fast_path_before_cf_state={before}");
    assert_eq!(before["base_rows"], 2);
    assert_eq!(before["anchors_rows"], 4);

    let panel = load_vault_panel_state(&resolved.path).unwrap().panel;
    persist_vault_panel_state(&resolved.path, &panel, &Registry::new()).unwrap();
    let state_without_runtime = load_vault_panel_state(&resolved.path).unwrap();
    assert!(
        state_without_runtime
            .registry_snapshot
            .as_ref()
            .unwrap()
            .lenses
            .is_empty(),
        "test precondition: persisted registry has no measurable runtime"
    );

    let replay = ingest_batch_streaming(&resolved, &jsonl).unwrap();

    let after = ingest_cf_state(&resolved);
    println!("anchored_existing_fast_path_after_cf_state={after}");
    assert_eq!(replay.status, "ingested");
    assert_eq!(replay.row_count, 2);
    assert_eq!(replay.new_count, 0);
    assert_eq!(replay.already_count, 2);
    assert_eq!(replay.verified_base_rows, 2);
    assert_eq!(
        after["base_rows"], before["base_rows"],
        "duplicate anchored replay must not rewrite Base CF rows"
    );
    assert_eq!(
        after["anchors_rows"], before["anchors_rows"],
        "duplicate anchored replay must not duplicate Anchors CF rows"
    );
    assert_eq!(after["slot_00_rows"], before["slot_00_rows"]);
    assert_eq!(
        after["ledger_rows"].as_u64().unwrap(),
        before["ledger_rows"].as_u64().unwrap() + 1,
        "one idempotent replay batch ledger row is physically present"
    );

    fs::remove_dir_all(root).ok();
}

#[test]
fn batch_mixed_existing_and_new_rows_still_fails_loud_when_runtime_is_missing() {
    let (root, resolved) = test_vault_with_registered_dense_lens("mixed-fast-path-reject");
    let first_jsonl = resolved.path.join("first.jsonl");
    let mixed_jsonl = resolved.path.join("mixed.jsonl");
    fs::write(&first_jsonl, "{\"text\":\"existing row alpha\"}\n").unwrap();
    fs::write(
        &mixed_jsonl,
        "{\"text\":\"existing row alpha\"}\n{\"text\":\"new row beta\"}\n",
    )
    .unwrap();

    ingest_batch_streaming(&resolved, &first_jsonl).unwrap();
    let panel = load_vault_panel_state(&resolved.path).unwrap().panel;
    persist_vault_panel_state(&resolved.path, &panel, &Registry::new()).unwrap();
    let before = ingest_cf_state(&resolved);
    println!("mixed_fast_path_reject_before_cf_state={before}");

    let error = ingest_batch_streaming(&resolved, &mixed_jsonl).unwrap_err();

    let after = ingest_cf_state(&resolved);
    println!("mixed_fast_path_reject_after_cf_state={after}");
    assert_eq!(error.code(), "CALYX_LENS_UNREACHABLE");
    assert!(
        error.message().contains("0/"),
        "error must name the unavailable content floor: {}",
        error.message()
    );
    assert_eq!(after["base_rows"], before["base_rows"]);
    assert_eq!(after["ledger_rows"], before["ledger_rows"]);
    assert_eq!(after["slot_00_rows"], before["slot_00_rows"]);

    fs::remove_dir_all(root).ok();
}

#[test]
fn anchor_label_kind_round_trips() {
    let kind = parse_anchor_kind("label:positive").unwrap();
    assert_eq!(kind, AnchorKind::Label("positive".to_string()));
    let anchor = Anchor {
        kind,
        value: AnchorValue::Enum("positive".to_string()),
        source: "unit".to_string(),
        observed_at: 7,
        confidence: 0.75,
    };
    let decoded: Anchor = serde_json::from_str(&serde_json::to_string(&anchor).unwrap()).unwrap();
    assert_eq!(decoded, anchor);
}

#[test]
fn measure_outputs_absent_not_zero_filled_and_does_not_store() {
    let (root, resolved) = test_vault("measure", panel_with_unregistered_text_slot());
    let vault = open_vault(&resolved).unwrap();
    let state = load_vault_panel_state(&resolved.path).unwrap();

    let cx = measure_constellation(&vault, &state, text_input("hello".to_string()), 1).unwrap();

    assert!(matches!(
        cx.slots.get(&SlotId::new(0)),
        Some(SlotVector::Absent {
            reason: AbsentReason::LensUnavailable
        })
    ));
    assert!(
        cx.flags.degraded,
        "missing applicable content lens degrades"
    );
    assert_eq!(
        vault
            .scan_cf_at(vault.snapshot(), ColumnFamily::Base)
            .unwrap()
            .len(),
        0
    );
    fs::remove_dir_all(root).ok();
}
