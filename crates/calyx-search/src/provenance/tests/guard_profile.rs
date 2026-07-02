//! Profile-backed in-region guard tests (#1094): `--guard in-region` without
//! an operator tau must load the calibrated Ward guard profile from the Guard
//! CF, fail closed (`CALYX_GUARD_PROVISIONAL`) when it is absent, uncalibrated
//! or panel-mismatched, apply per-slot conformal taus (MCP parity), and error
//! `CALYX_GUARD_OOD` when the guard blocks every candidate. The explicit
//! operator tau keeps the flat single-tau path with its provenance recorded.

use std::collections::BTreeMap as WardBTreeMap;
use std::str::FromStr;

use calyx_core::SlotId;
use calyx_ward::{
    CalibrationMeta, GuardId, GuardPolicy, GuardProfile, NoveltyAction, SlotCalibrationMeta,
    SlotKind,
};

use super::*;
use crate::engine::search_outcome_with_query_vectors_freshness_cached;

fn calibrated_profile(panel_version: u64, tau: f32) -> GuardProfile {
    let slot = SlotId::new(0);
    let mut per_slot = WardBTreeMap::new();
    per_slot.insert(
        slot,
        SlotCalibrationMeta {
            corpus_hash: [9; 32],
            estimator: "synthetic-fsv".to_string(),
            far: 0.0,
            frr: 0.0,
            confidence: 0.99,
            ts: 1_785_400_000,
            slot_kind: Some(SlotKind::Content),
        },
    );
    let mut tau_by_slot = WardBTreeMap::new();
    tau_by_slot.insert(slot, tau);
    GuardProfile {
        guard_id: GuardId::from_str("018f48a4-9a79-74d2-8a5c-9ad7f6b8c101").expect("guard id"),
        panel_version,
        domain: "default".to_string(),
        tau: tau_by_slot,
        required_slots: vec![slot],
        policy: GuardPolicy::AllRequired,
        calibration: Some(CalibrationMeta {
            corpus_hash: [9; 32],
            estimator: "synthetic-fsv".to_string(),
            far: 0.0,
            frr: 0.0,
            confidence: 0.99,
            ts: 1_785_400_000,
            per_slot,
        }),
        novelty_action: NoveltyAction::RejectClosed,
    }
}

fn write_default_guard_profile(vault: &AsterVault, profile: &GuardProfile) {
    let bytes = serde_json::to_vec(profile).expect("serialize guard profile");
    vault
        .write_cf(ColumnFamily::Guard, b"profile\0default".to_vec(), bytes)
        .expect("write guard profile");
    vault.flush().expect("flush guard profile");
}

/// Rebuild the search index after the Guard CF write advanced the vault seq
/// (Guard feeds derived content, so Fresh search would otherwise correctly
/// refuse the stale manifest).
fn rebuild_index(fixture: &Fixture, vault: &AsterVault) {
    rebuild_for_vault(&fixture.vault_dir, vault).expect("rebuild search index");
}

#[test]
fn in_region_without_profile_fails_closed_before_slot_search() {
    let fixture = Fixture::new("guard-no-profile");
    let vault = fixture.open_vault();
    let state = load_vault_panel_state(&fixture.vault_dir).unwrap();

    let mut events = Vec::new();
    let mut sink = |event: crate::engine::SearchTraceEvent| events.push(event);
    let error = match search_outcome_with_slots_traced(
        &vault,
        &state,
        &fixture.vault_dir,
        "alpha",
        1,
        FusionChoice::Rrf,
        GuardChoice::InRegion,
        None,
        false,
        None,
        SearchFreshness::Fresh,
        Some(&mut sink),
    ) {
        Ok(_) => panic!("in-region search without a calibrated profile must fail closed"),
        Err(error) => error,
    };

    assert_eq!(error.code(), "CALYX_GUARD_PROVISIONAL");
    assert!(
        error.message().contains("calibrated default guard profile"),
        "unexpected message: {}",
        error.message()
    );
    // Fail-fast contract: the profile gate runs before any recall work.
    assert!(
        !events
            .iter()
            .any(|event| event.phase.starts_with("search_slots")
                || event.phase.starts_with("indexes.open")),
        "guard resolution must fail before index open / slot search; got {:?}",
        events.iter().map(|event| event.phase).collect::<Vec<_>>()
    );
    fixture.cleanup();
}

#[test]
fn in_region_applies_calibrated_per_slot_tau_and_reports_no_flat_tau() {
    let fixture = Fixture::new("guard-profile-pass");
    let vault = fixture.open_vault();
    let state = load_vault_panel_state(&fixture.vault_dir).unwrap();
    // tau=0.0: every candidate passes; the point is the profile PATH is used.
    write_default_guard_profile(
        &vault,
        &calibrated_profile(u64::from(state.panel.version), 0.0),
    );
    rebuild_index(&fixture, &vault);

    let outcome = search_outcome(
        &vault,
        &state,
        &fixture.vault_dir,
        "alpha",
        1,
        FusionChoice::Rrf,
        GuardChoice::InRegion,
        None,
        false,
    )
    .expect("profile-backed guarded search succeeds");
    let hit = outcome.hits.first().expect("guarded hit");

    assert_eq!(hit.cx_id, fixture.cx_id);
    let evidence = hit.guard.as_ref().expect("hit carries guard evidence");
    assert!(
        evidence.verdict.overall_pass,
        "calibrated tau 0.0 must pass the aligned hit"
    );
    assert!(!evidence.verdict.provisional);
    assert_eq!(
        outcome.guard_tau, None,
        "profile mode has no flat tau to report; evidence lives on the hits"
    );
    fixture.cleanup();
}

#[test]
fn in_region_profile_blocking_all_candidates_is_guard_ood() {
    let fixture = Fixture::new("guard-profile-ood");
    let vault = fixture.open_vault();
    let state = load_vault_panel_state(&fixture.vault_dir).unwrap();
    // A near-1.0 calibrated tau: an off-corpus query cannot reach it.
    write_default_guard_profile(
        &vault,
        &calibrated_profile(u64::from(state.panel.version), 0.999_999),
    );
    rebuild_index(&fixture, &vault);

    let error = match search_outcome(
        &vault,
        &state,
        &fixture.vault_dir,
        "zzzzzz",
        1,
        FusionChoice::Rrf,
        GuardChoice::InRegion,
        None,
        false,
    ) {
        Ok(outcome) => panic!(
            "guard must block the off-corpus query; got {} hits",
            outcome.hits.len()
        ),
        Err(error) => error,
    };
    assert_eq!(error.code(), "CALYX_GUARD_OOD");
    fixture.cleanup();
}

#[test]
fn in_region_profile_panel_version_mismatch_fails_closed() {
    let fixture = Fixture::new("guard-profile-panel-mismatch");
    let vault = fixture.open_vault();
    let state = load_vault_panel_state(&fixture.vault_dir).unwrap();
    write_default_guard_profile(
        &vault,
        &calibrated_profile(u64::from(state.panel.version) + 1, 0.0),
    );
    rebuild_index(&fixture, &vault);

    let error = match search_outcome(
        &vault,
        &state,
        &fixture.vault_dir,
        "alpha",
        1,
        FusionChoice::Rrf,
        GuardChoice::InRegion,
        None,
        false,
    ) {
        Ok(_) => panic!("panel-version-mismatched profile must fail closed"),
        Err(error) => error,
    };
    assert_eq!(error.code(), "CALYX_GUARD_PROVISIONAL");
    assert!(
        error.message().contains("panel_version"),
        "unexpected message: {}",
        error.message()
    );
    fixture.cleanup();
}

#[test]
fn operator_tau_keeps_flat_gate_and_reports_its_provenance() {
    let fixture = Fixture::new("guard-operator-tau");
    let vault = fixture.open_vault();
    let state = load_vault_panel_state(&fixture.vault_dir).unwrap();
    // Deliberately NO profile in the Guard CF: the operator override must not
    // require one (the #1088 calibration path stays available).
    let (slot, query) = measure_query_vectors(&state, "alpha")
        .expect("measure query")
        .into_iter()
        .next()
        .expect("query vector");

    let outcome = search_outcome_with_query_vectors_freshness_cached(
        &vault,
        &fixture.vault_dir,
        &[(slot, query)],
        1,
        FusionChoice::Rrf,
        GuardChoice::InRegion,
        Some(0.25),
        Some(u64::from(state.panel.version)),
        None,
        false,
        SearchFreshness::Fresh,
        crate::engine::SearchBudget::disabled(),
        None,
        None,
    )
    .expect("operator-tau guarded search succeeds without a profile");

    assert_eq!(
        outcome.guard_tau,
        Some(0.25),
        "operator tau is recorded verbatim in the outcome"
    );
    assert!(!outcome.hits.is_empty());
    fixture.cleanup();
}
