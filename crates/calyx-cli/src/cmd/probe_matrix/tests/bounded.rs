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
            out: Some(out.clone()),
            resident_addr: None,
            max_variants: Some(1),
            time_budget_ms: None,
        },
    )
    .unwrap_err();

    assert_eq!(err.code(), "CALYX_PROBE_MATRIX_INCOMPLETE");
    assert!(out.exists());
    let artifact: ProbeMatrixArtifact = serde_json::from_slice(&fs::read(&out).unwrap()).unwrap();
    assert_eq!(artifact.schema_version, 5);
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

    let progress_path = PathBuf::from(&artifact.run.progress_artifact);
    let progress: serde_json::Value =
        serde_json::from_slice(&fs::read(&progress_path).unwrap()).unwrap();
    assert_eq!(progress["status"], "incomplete");
    assert_eq!(progress["phase"], "variant_budget_exhausted");
    assert!(progress["events"].as_array().unwrap().len() >= 4);
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
            out: Some(out.clone()),
            resident_addr: None,
            max_variants: None,
            time_budget_ms: None,
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
fn in_region_guard_diagnostics_persist_hydration_state_source_of_truth() {
    let (home, vault_dir) = seed_home("guard-diagnostics");
    let out = vault_dir.join("guard-diagnostics-matrix.json");

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
            out: Some(out.clone()),
            resident_addr: None,
            max_variants: Some(1),
            time_budget_ms: None,
        },
    )
    .unwrap_err();

    assert_eq!(err.code(), "CALYX_PROBE_MATRIX_INCOMPLETE");
    let artifact: ProbeMatrixArtifact = serde_json::from_slice(&fs::read(&out).unwrap()).unwrap();
    assert_eq!(artifact.schema_version, 5);
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
    assert!(row.search_done_elapsed_ms.is_some());
    assert_eq!(row.last_search_phase.as_deref(), Some("search.done"));
    assert!(row.guard_zero_hit_reason.is_none());

    let progress: serde_json::Value =
        serde_json::from_slice(&fs::read(&artifact.run.progress_artifact).unwrap()).unwrap();
    assert_eq!(progress["status"], "incomplete");
    assert_eq!(progress["phase"], "variant_budget_exhausted");
}
