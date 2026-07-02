use super::*;

#[test]
fn refused_probe_persists_diagnostic_matrix_before_fail_closed_exit() {
    let (home, vault_dir) = seed_home_without_anchors("refused");

    let err = run_probe_matrix_with_home(
        &home,
        ProbeMatrixArgs {
            vault: "refused".to_string(),
            frontier: "alpha".to_string(),
            slots: vec![SlotId::new(8), SlotId::new(14)],
            weighted_profiles: vec![RrfProfile::Bridge],
            phrasings: vec![ProbePhrasing::Terse],
            lengths: vec![ProbeLength::Entity],
            top_k: 1,
            guard: GuardChoice::Off,
            guard_tau: None,
            out: None,
            resident_addr: None,
            max_variants: None,
            time_budget_ms: None,
            search_miss_budget_ms: None,
            search_hit_budget_ms: None,
        },
    )
    .unwrap_err();

    assert_eq!(err.code(), "CALYX_PROBE_NO_GROUNDED_CANDIDATES");
    assert!(
        err.message()
            .contains("diagnostic matrix artifact persisted")
    );
    let matrix_path = only_progress(&vault_dir).with_file_name("matrix.json");
    let readback_bytes = fs::read(&matrix_path).unwrap();
    let artifact: ProbeMatrixArtifact = serde_json::from_slice(&readback_bytes).unwrap();
    let grounding = artifact
        .diagnostics
        .grounding_preflight
        .as_ref()
        .expect("grounding preflight persisted");

    assert_eq!(artifact.status, ProbeMatrixArtifactStatus::Incomplete);
    assert_eq!(
        artifact.run.stop_reason.as_deref(),
        Some("grounding_preflight_failed")
    );
    assert!(artifact.log.records.is_empty());
    assert_eq!(grounding.base_row_count, 3);
    assert_eq!(grounding.anchors_cf_row_count, 0);
    assert_eq!(grounding.accepted_eligible_active_slot_row_count, 0);
    assert_eq!(grounding.active_slots.len(), 2);
}
