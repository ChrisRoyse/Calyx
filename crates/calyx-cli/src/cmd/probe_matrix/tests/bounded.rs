use super::*;

#[test]
fn max_variants_persists_incomplete_matrix_and_progress_source_of_truth() {
    let (home, vault_dir) = seed_home("bounded");
    let out = vault_dir.join("bounded-matrix.json");

    let err = run_probe_matrix_with_home(
        &home,
        ProbeMatrixArgs {
            vault: "bounded".to_string(),
            frontier: "alpha".to_string(),
            slots: vec![SlotId::new(8), SlotId::new(14)],
            weighted_profiles: vec![RrfProfile::Bridge],
            phrasings: vec![ProbePhrasing::Terse],
            lengths: vec![ProbeLength::Entity],
            top_k: 1,
            guard: GuardChoice::Off,
            guard_tau: None,
            out: Some(out.clone()),
            resident_addr: None,
            max_variants: Some(1),
            time_budget_ms: None,
            search_miss_budget_ms: None,
            search_hit_budget_ms: None,
        },
    )
    .unwrap_err();

    assert_eq!(err.code(), "CALYX_PROBE_MATRIX_INCOMPLETE");
    assert!(out.exists());
    let artifact: ProbeMatrixArtifact = serde_json::from_slice(&fs::read(&out).unwrap()).unwrap();
    assert_eq!(artifact.schema_version, 7);
    assert_eq!(artifact.status, ProbeMatrixArtifactStatus::Incomplete);
    assert!(!artifact.run.complete);
    assert_eq!(
        artifact.run.stop_reason.as_deref(),
        Some("variant_budget_exhausted")
    );
    assert_eq!(artifact.run.completed_variant_count, 1);
    assert_eq!(artifact.run.total_variant_count, 6);
    assert_eq!(artifact.run.next_variant_index, Some(1));
    assert_eq!(artifact.run.resume_token.as_deref(), Some("variant:1"));
    assert_eq!(artifact.log.records.len(), 1);
    let cache = &artifact.diagnostics.search_result_cache;
    assert_eq!(
        (
            cache.entry_count,
            cache.lookup_count,
            cache.hit_count,
            cache.miss_count
        ),
        (1, 1, 0, 1)
    );
    assert_eq!(
        artifact.diagnostics.variant_guard_counts[0].search_cache_miss_count,
        1
    );

    let progress_path = PathBuf::from(&artifact.run.progress_artifact);
    let progress: serde_json::Value =
        serde_json::from_slice(&fs::read(&progress_path).unwrap()).unwrap();
    assert_eq!(progress["status"], "incomplete");
    assert_eq!(progress["phase"], "variant_budget_exhausted");
    assert!(progress["events"].as_array().unwrap().len() >= 4);
}

#[test]
fn changed_slot_set_uses_distinct_search_cache_key_source_of_truth() {
    let (home, vault_dir) = seed_home("slot-cache-key");
    let out_one = vault_dir.join("slot-cache-key-one.json");
    let out_two = vault_dir.join("slot-cache-key-two.json");

    for (out, slots) in [
        (&out_one, vec![SlotId::new(8)]),
        (&out_two, vec![SlotId::new(8), SlotId::new(14)]),
    ] {
        let err = run_probe_matrix_with_home(
            &home,
            ProbeMatrixArgs {
                vault: "slot-cache-key".to_string(),
                frontier: "alpha".to_string(),
                slots,
                weighted_profiles: vec![RrfProfile::Bridge],
                phrasings: vec![ProbePhrasing::Terse],
                lengths: vec![ProbeLength::Entity],
                top_k: 1,
                guard: GuardChoice::Off,
                guard_tau: None,
                out: Some(out.clone()),
                resident_addr: None,
                max_variants: Some(1),
                time_budget_ms: None,
                search_miss_budget_ms: None,
                search_hit_budget_ms: None,
            },
        )
        .unwrap_err();
        assert_eq!(err.code(), "CALYX_PROBE_MATRIX_INCOMPLETE");
    }

    let one: ProbeMatrixArtifact = serde_json::from_slice(&fs::read(&out_one).unwrap()).unwrap();
    let two: ProbeMatrixArtifact = serde_json::from_slice(&fs::read(&out_two).unwrap()).unwrap();
    assert_eq!(one.active_slots, vec![SlotId::new(8)]);
    assert_eq!(two.active_slots, vec![SlotId::new(8), SlotId::new(14)]);
    assert_eq!(
        (
            one.diagnostics.search_result_cache.lookup_count,
            one.diagnostics.search_result_cache.miss_count
        ),
        (1, 1)
    );
    assert_eq!(
        (
            two.diagnostics.search_result_cache.lookup_count,
            two.diagnostics.search_result_cache.miss_count
        ),
        (1, 1)
    );
    let one_key = one.diagnostics.variant_guard_counts[0]
        .search_cache_key_sha256
        .as_deref();
    let two_key = two.diagnostics.variant_guard_counts[0]
        .search_cache_key_sha256
        .as_deref();
    assert_ne!(one_key, two_key);
}

#[test]
fn gpu_slot_without_resident_persists_incomplete_matrix_source_of_truth() {
    let (home, vault_dir) = seed_home("resident-required");
    let out = vault_dir.join("resident-required-matrix.json");
    let mut state = load_vault_panel_state(&vault_dir).unwrap();
    state
        .panel
        .slots
        .iter_mut()
        .find(|slot| slot.slot_id == SlotId::new(8))
        .unwrap()
        .resource
        .placement = calyx_core::Placement::Gpu;
    persist_vault_panel_state(&vault_dir, &state.panel, &state.registry).unwrap();

    let err = run_probe_matrix_with_home(
        &home,
        ProbeMatrixArgs {
            vault: "resident-required".to_string(),
            frontier: "alpha".to_string(),
            slots: vec![SlotId::new(8)],
            weighted_profiles: vec![RrfProfile::Bridge],
            phrasings: vec![ProbePhrasing::Terse],
            lengths: vec![ProbeLength::Entity],
            top_k: 1,
            guard: GuardChoice::Off,
            guard_tau: None,
            out: Some(out.clone()),
            resident_addr: None,
            max_variants: None,
            time_budget_ms: None,
            search_miss_budget_ms: None,
            search_hit_budget_ms: None,
        },
    )
    .unwrap_err();

    assert_eq!(err.code(), "CALYX_PROBE_MATRIX_RESIDENT_REQUIRED");
    let artifact: ProbeMatrixArtifact = serde_json::from_slice(&fs::read(&out).unwrap()).unwrap();
    assert_eq!(artifact.status, ProbeMatrixArtifactStatus::Incomplete);
    assert_eq!(
        artifact.run.stop_reason.as_deref(),
        Some("resident_required")
    );
    assert_eq!(artifact.run.completed_variant_count, 0);
    assert_eq!(artifact.log.records.len(), 0);

    let progress: serde_json::Value =
        serde_json::from_slice(&fs::read(&artifact.run.progress_artifact).unwrap()).unwrap();
    assert_eq!(progress["status"], "failed");
    assert_eq!(progress["phase"], "resident_required");
}

#[test]
fn stale_manifest_fails_closed_with_incomplete_cache_state_source_of_truth() {
    let (home, vault_dir) = seed_home("stale-manifest-cache");
    let out = vault_dir.join("stale-manifest-cache-matrix.json");
    let manifest_path = vault_dir.join("idx").join("search").join("manifest.json");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    let base_seq = manifest["base_seq"].as_u64().unwrap();
    manifest["base_seq"] = serde_json::json!(base_seq - 1);
    fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();

    let err = run_probe_matrix_with_home(
        &home,
        ProbeMatrixArgs {
            vault: "stale-manifest-cache".to_string(),
            frontier: "alpha".to_string(),
            slots: vec![SlotId::new(8), SlotId::new(14)],
            weighted_profiles: vec![RrfProfile::Bridge],
            phrasings: vec![ProbePhrasing::Terse],
            lengths: vec![ProbeLength::Entity],
            top_k: 1,
            guard: GuardChoice::Off,
            guard_tau: None,
            out: Some(out.clone()),
            resident_addr: None,
            max_variants: None,
            time_budget_ms: None,
            search_miss_budget_ms: None,
            search_hit_budget_ms: None,
        },
    )
    .unwrap_err();

    assert_eq!(err.code(), "CALYX_STALE_DERIVED");
    let artifact: ProbeMatrixArtifact = serde_json::from_slice(&fs::read(&out).unwrap()).unwrap();
    assert_eq!(artifact.status, ProbeMatrixArtifactStatus::Incomplete);
    assert_eq!(artifact.run.stop_reason.as_deref(), Some("variant_error"));
    assert_eq!(artifact.run.completed_variant_count, 0);
    assert_eq!(artifact.log.records.len(), 0);
    assert_eq!(artifact.diagnostics.search_result_cache.lookup_count, 1);
    assert_eq!(artifact.diagnostics.search_result_cache.miss_count, 1);

    let progress: serde_json::Value =
        serde_json::from_slice(&fs::read(&artifact.run.progress_artifact).unwrap()).unwrap();
    assert_eq!(progress["status"], "failed");
    assert_eq!(progress["phase"], "variant_error");
}

#[test]
fn in_region_guard_filtering_all_candidates_fails_closed_with_specific_error() {
    let (home, vault_dir) = seed_home("guard-filtered-all");
    let out = vault_dir.join("guard-filtered-all-matrix.json");

    // Frontier "beta" retrieves candidates (HNSW always returns nearest
    // neighbours), but its byte-feature vector is not parallel to any stored
    // slot-8 vector, so no candidate can reach cosine 1.0: the operator-supplied
    // tau=1.0 guard must filter every retrieved candidate and the run must fail
    // closed with the specific guard-filtered-all diagnosis, not a generic
    // empty-benchmark error (#1088).
    let err = run_probe_matrix_with_home(
        &home,
        ProbeMatrixArgs {
            vault: "guard-filtered-all".to_string(),
            frontier: "beta".to_string(),
            slots: vec![SlotId::new(8), SlotId::new(14)],
            weighted_profiles: vec![RrfProfile::Bridge],
            phrasings: vec![ProbePhrasing::Terse],
            lengths: vec![ProbeLength::Entity],
            top_k: 1,
            guard: GuardChoice::InRegion,
            guard_tau: Some(1.0),
            out: Some(out.clone()),
            resident_addr: None,
            max_variants: Some(1),
            time_budget_ms: None,
            search_miss_budget_ms: None,
            search_hit_budget_ms: None,
        },
    )
    .unwrap_err();

    assert_eq!(err.code(), "CALYX_PROBE_MATRIX_GUARD_FILTERED_ALL");
    assert!(
        err.message().contains("operator-supplied"),
        "error must name the supplied tau provenance: {}",
        err.message()
    );
    assert!(
        err.message().contains("tau=1.000000"),
        "error must name the applied tau: {}",
        err.message()
    );

    let artifact: ProbeMatrixArtifact = serde_json::from_slice(&fs::read(&out).unwrap()).unwrap();
    assert_eq!(artifact.status, ProbeMatrixArtifactStatus::Incomplete);
    assert_eq!(artifact.diagnostics.variant_guard_counts.len(), 1);
    let row = &artifact.diagnostics.variant_guard_counts[0];
    assert!(row.guard_prefilter_input_count.unwrap() > 0);
    assert_eq!(row.guard_prefilter_output_count, Some(0));
    assert_eq!(
        row.guard_zero_hit_reason.as_deref(),
        Some("in_region_guard_prefilter_rejected_all_candidates")
    );
    assert_eq!(row.guard_tau.as_deref(), Some("1.000000"));
}

#[test]
fn operator_calibrated_guard_tau_threads_to_engine_and_keeps_in_region_hits() {
    let (home, vault_dir) = seed_home("guard-tau-calibrated");
    let out = vault_dir.join("guard-tau-calibrated-matrix.json");

    // With a calibrated (permissive) tau the same in-region guard keeps the
    // aligned candidate: the run exits through the plain variant-budget path
    // and the persisted diagnostics show the operator tau was applied verbatim.
    let err = run_probe_matrix_with_home(
        &home,
        ProbeMatrixArgs {
            vault: "guard-tau-calibrated".to_string(),
            frontier: "alpha".to_string(),
            slots: vec![SlotId::new(8), SlotId::new(14)],
            weighted_profiles: vec![RrfProfile::Bridge],
            phrasings: vec![ProbePhrasing::Terse],
            lengths: vec![ProbeLength::Entity],
            top_k: 1,
            guard: GuardChoice::InRegion,
            guard_tau: Some(0.1),
            out: Some(out.clone()),
            resident_addr: None,
            max_variants: Some(1),
            time_budget_ms: None,
            search_miss_budget_ms: None,
            search_hit_budget_ms: None,
        },
    )
    .unwrap_err();

    assert_eq!(
        err.code(),
        "CALYX_PROBE_MATRIX_INCOMPLETE",
        "guard kept hits, so the exit must be the plain variant budget stop"
    );

    let artifact: ProbeMatrixArtifact = serde_json::from_slice(&fs::read(&out).unwrap()).unwrap();
    assert_eq!(artifact.log.records.len(), 1);
    assert!(artifact.log.records[0].accepted_hit_count > 0);
    let row = &artifact.diagnostics.variant_guard_counts[0];
    assert_eq!(row.guard_tau.as_deref(), Some("0.100000"));
    assert!(row.post_guard_hit_count.unwrap() > 0);
    assert!(row.guard_zero_hit_reason.is_none());
}

#[test]
fn in_region_guard_diagnostics_persist_hydration_state_source_of_truth() {
    let (home, vault_dir) = seed_home("guard-diagnostics");
    let out = vault_dir.join("guard-diagnostics-matrix.json");

    // Explicit operator tau: since #1094 the flat single-tau gate (whose
    // prefilter diagnostics are asserted below) runs ONLY under an
    // operator-supplied tau; `guard_tau: None` is profile-backed and fails
    // closed on this uncalibrated fixture (see
    // `in_region_without_operator_tau_requires_calibrated_profile`).
    let err = run_probe_matrix_with_home(
        &home,
        ProbeMatrixArgs {
            vault: "guard-diagnostics".to_string(),
            frontier: "alpha".to_string(),
            slots: vec![SlotId::new(8), SlotId::new(14)],
            weighted_profiles: vec![RrfProfile::Bridge],
            phrasings: vec![ProbePhrasing::Terse],
            lengths: vec![ProbeLength::Entity],
            top_k: 1,
            guard: GuardChoice::InRegion,
            guard_tau: Some(0.999),
            out: Some(out.clone()),
            resident_addr: None,
            max_variants: Some(1),
            time_budget_ms: None,
            search_miss_budget_ms: None,
            search_hit_budget_ms: None,
        },
    )
    .unwrap_err();

    assert_eq!(err.code(), "CALYX_PROBE_MATRIX_INCOMPLETE");
    let artifact: ProbeMatrixArtifact = serde_json::from_slice(&fs::read(&out).unwrap()).unwrap();
    assert_eq!(artifact.schema_version, 7);
    assert_eq!(artifact.diagnostics.variant_guard_counts.len(), 1);
    let row = &artifact.diagnostics.variant_guard_counts[0];
    assert_eq!(row.guard_prefilter_input_count, Some(3));
    assert_eq!(row.guard_prefilter_output_count, Some(1));
    assert_eq!(row.guard_prefilter_filtered_count, Some(2));
    assert!(row.guard_prefilter_elapsed_ms.is_some());
    assert_eq!(row.hit_hydration_candidate_count, Some(1));
    assert_eq!(row.hit_hydration_doc_count, Some(1));
    assert!(row.hit_hydration_elapsed_ms.is_some());
    assert!(row.per_hit_hydrate_start_count >= row.hit_hydration_doc_count.unwrap());
    assert!(row.per_hit_hydrate_done_count >= row.hit_hydration_doc_count.unwrap());
    assert_eq!(row.pre_guard_hit_count, Some(1));
    assert_eq!(row.post_guard_hit_count, Some(1));
    assert_eq!(row.guard_filtered_hit_count, Some(0));
    assert_eq!(row.guard_tau.as_deref(), Some("0.999000"));
    assert_eq!(row.guard_best_cosine_min.as_deref(), Some("1.000000"));
    assert_eq!(row.guard_best_cosine_max.as_deref(), Some("1.000000"));
    assert_eq!(row.guard_missing_cosine_count, Some(0));
    assert!(row.guard_start_elapsed_ms.is_some());
    assert!(row.guard_done_elapsed_ms.is_some());
    assert_eq!(row.search_cache_miss_count, 1);
    assert_eq!(row.search_cache_hit_count, 0);
    assert!(row.search_cache_key_sha256.is_some());
    assert!(row.search_done_elapsed_ms.is_some());
    assert_eq!(row.last_search_phase.as_deref(), Some("search.done"));
    assert!(row.guard_zero_hit_reason.is_none());

    let progress: serde_json::Value =
        serde_json::from_slice(&fs::read(&artifact.run.progress_artifact).unwrap()).unwrap();
    assert_eq!(progress["status"], "incomplete");
    assert_eq!(progress["phase"], "variant_budget_exhausted");
}

/// #1094: `--guard in-region` without an operator tau is profile-backed.
/// On a vault without a calibrated Ward guard profile it must fail closed
/// with `CALYX_GUARD_PROVISIONAL` — never fall back to a silent flat default.
#[test]
fn in_region_without_operator_tau_requires_calibrated_profile() {
    let (home, vault_dir) = seed_home("guard-profile-required");
    let out = vault_dir.join("guard-profile-required-matrix.json");

    let err = run_probe_matrix_with_home(
        &home,
        ProbeMatrixArgs {
            vault: "guard-profile-required".to_string(),
            frontier: "alpha".to_string(),
            slots: vec![SlotId::new(8), SlotId::new(14)],
            weighted_profiles: vec![RrfProfile::Bridge],
            phrasings: vec![ProbePhrasing::Terse],
            lengths: vec![ProbeLength::Entity],
            top_k: 1,
            guard: GuardChoice::InRegion,
            guard_tau: None,
            out: Some(out),
            resident_addr: None,
            max_variants: Some(1),
            time_budget_ms: None,
            search_miss_budget_ms: None,
            search_hit_budget_ms: None,
        },
    )
    .unwrap_err();

    assert!(
        err.code() == "CALYX_GUARD_PROVISIONAL"
            || err.message().contains("CALYX_GUARD_PROVISIONAL"),
        "uncalibrated in-region probe must fail closed with CALYX_GUARD_PROVISIONAL; got code={} message={}",
        err.code(),
        err.message()
    );
}
