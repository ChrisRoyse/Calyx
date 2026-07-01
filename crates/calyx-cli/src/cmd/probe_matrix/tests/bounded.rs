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
    assert_eq!(artifact.schema_version, 3);
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
