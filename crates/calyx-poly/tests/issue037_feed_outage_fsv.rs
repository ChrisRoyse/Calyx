//! Issue #37 - feed-outage handling as explicit absent/degraded source state.
//!
//! Source of truth: persisted feed observation artifacts, persisted normalized feed-state reports,
//! and forecast-admission decisions derived from readback reports.

use std::path::Path;

use calyx_poly::admission::{AdmissionDecision, AdmissionInputs, AdmissionParams};
use calyx_poly::feed_outage::{
    FEED_OBSERVATION_ARTIFACT_KIND, FEED_OUTAGE_DEGRADED, FEED_OUTAGE_PASSED,
    FEED_OUTAGE_SCHEMA_VERSION, FeedObservation, FeedObservationStatus, FeedSlotStatus,
    FeedStateReport, REFUSE_DEGRADED_FEED, feed_guarded_admission, run_feed_outage_readback,
};
use serde_json::{Value, json};

// calyx-shared-module: path=fsv_support.rs alias=__calyx_shared_fsv_support_rs local=support visibility=private
use crate::__calyx_shared_fsv_support_rs as support;
use support::{
    collect_files, known_healthy_market_integrity, known_healthy_oracle_risk,
    known_healthy_wash_trade, named_fsv_root, reset_dir, write_blake3sums, write_json,
};

const CAPTURED_TS: u64 = 1_785_600_037;

#[test]
fn issue037_feed_outage_fsv() {
    let (root, _keep) = named_fsv_root("POLY_ISSUE037_FSV_ROOT", "poly-issue037-feed-outage");
    reset_dir(&root);

    let happy = happy_healthy_feed_writes_present_slots(&root);
    let empty = edge_empty_response_writes_absent_and_refuses(&root);
    let timeout = edge_timeout_writes_absent_and_refuses(&root);
    let malformed = edge_malformed_payload_writes_absent_and_refuses(&root);

    let mut files = Vec::new();
    collect_files(&root, &mut files);
    let readback = json!({
        "issue": 37,
        "proof_claim": "Poly persists read-only feed observations, reads them back into explicit present/absent source slots, marks empty/timeout/malformed feed states degraded, and refuses forecast admission from degraded feed evidence without trading fallbacks.",
        "minimum_sufficient_corpus": {
            "observations": 4,
            "happy_observations": 1,
            "edge_observations": 3,
            "required_source_fields_per_observation": ["price", "volume_24h"],
            "why_this_is_sufficient": "One healthy observation proves persisted source fields become present slots and can pass admission; one each for empty response, timeout, and malformed payload proves the three required degraded/outage states write Absent slots and refuse admission.",
            "why_smaller_is_insufficient": "Fewer than four observations would omit either the healthy path or one of the required #37 edge cases.",
            "why_larger_is_wasteful": "More feeds or rows would repeat the same observation->readback->slot-state->admission guard path; scale is not the #37 claim."
        },
        "source_of_truth": {
            "root": root.display().to_string(),
            "artifact_types": ["feed observation JSON", "feed state report JSON", "admission decision JSON"]
        },
        "happy_path": happy,
        "edge_cases": {
            "empty_response": empty,
            "timeout": timeout,
            "malformed_payload": malformed
        },
        "physical_files": files
    });
    let readback_path = root.join("issue037_feed_outage_fsv_report.json");
    write_json(&readback_path, &readback);
    write_blake3sums(&root);
    println!("ISSUE037_FEED_OUTAGE_FSV={}", readback_path.display());
}

fn happy_healthy_feed_writes_present_slots(root: &Path) -> Value {
    let run = run_feed_outage_readback(
        &observation(
            "happy-clob-book",
            FeedObservationStatus::Healthy,
            r#"{"price":0.42,"volume_24h":120000.0,"condition_id":"0x37"}"#,
        ),
        &root.join("happy"),
    )
    .expect("healthy feed readback");
    let report = read_report(&run.report_path);
    assert_eq!(report, run.report);
    assert!(!report.degraded);
    assert_eq!(report.status_code, FEED_OUTAGE_PASSED);
    assert_eq!(report.absent_slot_count, 0);
    assert!(
        report
            .slot_states
            .iter()
            .all(|slot| slot.status == FeedSlotStatus::Present)
    );
    let decision = feed_guarded_admission(&report, &AdmissionParams::default(), &admit_inputs());
    assert!(decision.admitted, "{}", decision.reason);
    let evidence = evidence(&report, &decision);
    assert_no_trading_fallback(&evidence);
    evidence
}

fn edge_empty_response_writes_absent_and_refuses(root: &Path) -> Value {
    degraded_case(
        root,
        "edge-empty",
        FeedObservationStatus::EmptyResponse,
        "",
        "empty_response",
    )
}

fn edge_timeout_writes_absent_and_refuses(root: &Path) -> Value {
    degraded_case(
        root,
        "edge-timeout",
        FeedObservationStatus::Timeout,
        "",
        "timeout",
    )
}

fn edge_malformed_payload_writes_absent_and_refuses(root: &Path) -> Value {
    degraded_case(
        root,
        "edge-malformed",
        FeedObservationStatus::MalformedPayload,
        r#"{"price":0.42,"#,
        "malformed_payload",
    )
}

fn degraded_case(
    root: &Path,
    source_id: &str,
    status: FeedObservationStatus,
    payload_text: &str,
    absent_reason: &str,
) -> Value {
    let run = run_feed_outage_readback(
        &observation(source_id, status, payload_text),
        &root.join(source_id),
    )
    .expect("degraded feed readback");
    let report = read_report(&run.report_path);
    assert_eq!(report, run.report);
    assert!(report.degraded);
    assert_eq!(report.status_code, FEED_OUTAGE_DEGRADED);
    assert_eq!(report.absent_slot_count, 2);
    assert!(report.slot_states.iter().all(|slot| {
        slot.status == FeedSlotStatus::Absent
            && slot.absent_reason.as_deref() == Some(absent_reason)
            && slot.value_sha256.is_none()
    }));
    let decision = feed_guarded_admission(&report, &AdmissionParams::default(), &admit_inputs());
    assert!(!decision.admitted);
    assert_eq!(decision.code, REFUSE_DEGRADED_FEED);
    let evidence = evidence(&report, &decision);
    assert_no_trading_fallback(&evidence);
    evidence
}

fn observation(
    source_id: &str,
    status: FeedObservationStatus,
    payload_text: &str,
) -> FeedObservation {
    FeedObservation {
        schema_version: FEED_OUTAGE_SCHEMA_VERSION.to_string(),
        artifact_kind: FEED_OBSERVATION_ARTIFACT_KIND.to_string(),
        source_id: source_id.to_string(),
        source_kind: "clob_market_data".to_string(),
        source_url: "https://clob.polymarket.com/book?token_id=issue037".to_string(),
        captured_ts: CAPTURED_TS,
        status,
        required_fields: vec!["price".to_string(), "volume_24h".to_string()],
        payload_text: payload_text.to_string(),
    }
}

fn read_report(path: &Path) -> FeedStateReport {
    let bytes = std::fs::read(path).expect("read feed report");
    serde_json::from_slice(&bytes).expect("decode feed report")
}

fn evidence(report: &FeedStateReport, decision: &AdmissionDecision) -> Value {
    json!({
        "source_id": report.source_id,
        "status_code": report.status_code,
        "degraded": report.degraded,
        "absent_slot_count": report.absent_slot_count,
        "slot_states": report.slot_states,
        "payload_sha256": report.payload_sha256,
        "raw_observation_sha256": report.raw_observation_sha256,
        "admission": decision
    })
}

fn admit_inputs() -> AdmissionInputs {
    AdmissionInputs {
        p_win: 0.94,
        confidence: 0.80,
        sufficiency_ok: true,
        evidence_count: 3,
        source_derived_evidence_count: 3,
        stale_evidence_count: 0,
        circular_evidence_count: 0,
        super_intel_pass: true,
        guard_calibrated: true,
        grounding_anchor_count: 50,
        guard_pass: true,
        liquidity_ok: true,
        market_integrity: known_healthy_market_integrity(),
        oracle_risk: known_healthy_oracle_risk(),
        wash_trade: known_healthy_wash_trade(),
        kill_switch_active: false,
        daily_error_score: 0.0,
    }
}

fn assert_no_trading_fallback(value: &Value) {
    let text = value.to_string().to_ascii_lowercase();
    for forbidden in ["order", "sign", "bankroll", "stake", "bet"] {
        assert!(
            !text.contains(forbidden),
            "feed-outage evidence must not include trading fallback term {forbidden}"
        );
    }
}
