//! Issue #34 - Kalshi external feed capture, encoding, and admission FSV.
//!
//! Source of truth: persisted Kalshi raw HTTP body bytes and parsed market JSON read back from disk.

// calyx-shared-module: path=fsv_support.rs alias=__calyx_shared_fsv_support_rs local=support visibility=private
use crate::__calyx_shared_fsv_support_rs as support;

use std::fs;
use std::path::Path;

use calyx_assay::{EstimateBound, MIN_ASSAY_SAMPLES, TrustTag};
use calyx_core::FixedClock;
use calyx_poly::external_kalshi_feed::{
    DEFAULT_EXTERNAL_SIGNAL_K, ERR_EXTERNAL_SIGNAL_ADMISSION_INVALID, ERR_KALSHI_ENCODE_INVALID,
    ERR_KALSHI_MARKET_INVALID, EXTERNAL_SIGNAL_REFUSED_SINGLE_CLASS,
    EXTERNAL_SIGNAL_REFUSED_UNCALIBRATED_BOUND, EXTERNAL_SIGNAL_REFUSED_UNDERPOWERED,
    ExternalSignalAdmissionReport, ExternalSignalOutcomeObservation, KalshiFeedClient,
    KalshiFeedClientConfig, KalshiMarketsPage, KalshiMarketsRequest, encode_kalshi_market_signal,
    kalshi_lens_candidate_from_admission, kalshi_market_signal_observations,
    measure_external_signal_admission, parse_kalshi_market, parse_kalshi_markets_value,
    persist_kalshi_markets_page, write_external_signal_admission_report,
};
use calyx_poly::lens_autobuild::{
    LENS_AUTOBUILD_MIN_GAIN_BITS, LensAutobuildRequest, LensAutobuildStatus, LensDeficit,
    read_lens_autobuild_report, run_lens_autobuild_report,
};
use serde_json::{Value, json};

use support::{collect_files, named_fsv_root, reset_dir, write_blake3sums, write_json};

#[test]
fn issue034_kalshi_external_feed_fsv() {
    let (root, _keep) = named_fsv_root("POLY_ISSUE034_FSV_ROOT", "issue034-kalshi-feed");
    assert_not_d_drive(&root);
    reset_dir(&root);
    let clock = FixedClock::new(1_788_652_800);

    let page = KalshiMarketsPage::from_raw(
        "https://external-api.kalshi.com/trade-api/v2/markets?status=settled&limit=2".to_string(),
        200,
        kalshi_fixture_body(),
    )
    .expect("parse raw fixture");
    assert_eq!(page.markets.len(), 2);
    let persisted = persist_kalshi_markets_page(&root, "known-truth-settled", &page)
        .expect("persist raw and parsed Kalshi page");

    let first_signal = encode_kalshi_market_signal(&page.markets[0]).expect("encode first market");
    assert_eq!(first_signal.feature_names.len(), 5);
    assert!((first_signal.values[0] - 0.43).abs() < 0.0001);
    let settled_wide_spread =
        encode_kalshi_market_signal(&page.markets[1]).expect("encode settled market");
    assert!(
        (settled_wide_spread.values[0] - 0.72).abs() < 0.0001,
        "wide 0/1 settled quote must fall back to last_price_dollars"
    );

    let fixture_observations =
        kalshi_market_signal_observations(&page.markets).expect("fixture observations");
    assert_eq!(fixture_observations.len(), 2);

    let observations = known_truth_observations(MIN_ASSAY_SAMPLES);
    let admission = measure_external_signal_admission(
        "kalshi",
        "yes_price_signal",
        &observations,
        &clock,
        DEFAULT_EXTERNAL_SIGNAL_K,
    )
    .expect("admission measurement");
    assert_eq!(
        serde_json::to_value(&admission).expect("admission JSON")["estimate_bound"],
        json!("point"),
        "external-signal evidence must retain the mixed estimator's Point contract"
    );
    assert_eq!(admission.estimate_bound, Some(EstimateBound::Point));
    assert_eq!(admission.code, EXTERNAL_SIGNAL_REFUSED_UNCALIBRATED_BOUND);
    assert!(!admission.admitted);
    let mut legacy_admission = serde_json::to_value(&admission).expect("legacy admission JSON");
    legacy_admission
        .as_object_mut()
        .expect("admission object")
        .remove("estimate_bound");
    let legacy_admission: ExternalSignalAdmissionReport =
        serde_json::from_value(legacy_admission).expect("legacy admission without bound");
    assert_eq!(legacy_admission.estimate_bound, None);
    assert!(
        admission.bits >= 0.05,
        "known-truth Kalshi signal must clear the 0.05-bit floor"
    );
    let admission_path =
        write_external_signal_admission_report(&root, "known_truth_admission", &admission)
            .expect("write admission report");
    let lens_autobuild =
        lens_autobuild_handoff(&root, &admission, &admission_path.display().to_string());

    let edge_cases = edge_cases(&root, &clock);
    let mut files = Vec::new();
    collect_files(&root, &mut files);
    let report = json!({
        "issue": 34,
        "proof_claim": "A read-only Kalshi external feed page is captured as raw bytes, parsed from disk readback into typed markets, encoded into finite numeric signal vectors, and measured by the real Assay mixed KSG estimator. Point-bound evidence is preserved and refused by admission and lens autobuild instead of being treated as a calibrated lower bound.",
        "minimum_sufficient_proof_corpus": {
            "raw_kalshi_market_rows": page.markets.len(),
            "known_outcome_admission_rows": observations.len(),
            "lens_autobuild_candidates": 1,
            "edge_cases": 9,
            "why_this_is_sufficient": "Two market rows are the smallest fixture that proves both tight bid/ask midpoint encoding and settled wide-spread last-price fallback while exercising yes/no labels. Exactly 50 known-outcome rows is the calyx-assay KSG sample floor, so it is the smallest corpus that can prove a real mixed-estimator measurement. One Point-bound lens-autobuild candidate is the smallest #110 handoff that proves fail-closed rejection. Nine edges cover missing markets array, missing price signal, missing spread, missing liquidity/volume/open-interest evidence, non-finite signal refusal, single-class refusal, and below-floor refusal.",
            "why_smaller_is_insufficient": "One market row would not prove both encoding branches or both outcome labels. Fewer than 50 known-outcome rows cannot satisfy the Assay KSG sample floor. Removing the edge cases would not prove fail-closed behavior.",
            "why_larger_is_wasteful": "More market rows or more than 50 known-outcome rows repeat the same raw readback, parser, encoder, mixed-estimator, and bound-refusal paths without adding a #34 invariant."
        },
        "source_of_truth": "Kalshi-compatible raw body bytes persisted under this FSV root and parsed only after disk readback",
        "persisted_feed": persisted,
        "first_encoded_signal": first_signal,
        "fixture_observation_count": fixture_observations.len(),
        "admission": admission,
        "admission_path": admission_path.display().to_string(),
        "lens_autobuild": lens_autobuild,
        "edge_cases": edge_cases,
        "physical_files": files,
        "passed": true
    });
    let report_path = root.join("issue034_kalshi_external_feed_fsv_report.json");
    write_json(&report_path, &report);
    let readback: Value =
        serde_json::from_slice(&fs::read(&report_path).expect("read report")).expect("decode");
    assert_eq!(readback["issue"], json!(34));
    assert_eq!(readback["passed"], json!(true));
    assert_eq!(
        readback["admission"]["code"],
        json!(EXTERNAL_SIGNAL_REFUSED_UNCALIBRATED_BOUND)
    );
    assert_eq!(readback["persisted_feed"], report["persisted_feed"]);
    write_blake3sums(&root);
    println!(
        "ISSUE034_KALSHI_EXTERNAL_FEED_FSV={}",
        report_path.display()
    );
}

#[test]
#[ignore = "requires live public Kalshi market data endpoint"]
fn issue034_live_kalshi_external_feed_fsv() {
    let (root, _keep) = named_fsv_root("POLY_ISSUE034_LIVE_FSV_ROOT", "issue034-live-kalshi-feed");
    assert_not_d_drive(&root);
    reset_dir(&root);
    let clock = FixedClock::new(1_788_652_801);
    let client = KalshiFeedClient::new(KalshiFeedClientConfig::default()).expect("Kalshi client");
    let page = client
        .fetch_markets(&KalshiMarketsRequest::settled(MIN_ASSAY_SAMPLES))
        .expect("fetch live settled Kalshi markets");
    assert_eq!(page.status_code, 200);
    assert!(!page.markets.is_empty());
    let persisted =
        persist_kalshi_markets_page(&root, "live-settled", &page).expect("persist live feed");
    let first_signal = encode_kalshi_market_signal(&page.markets[0]).expect("encode live market");
    assert!(first_signal.values.iter().all(|value| value.is_finite()));

    let observations = kalshi_market_signal_observations(&page.markets).expect("live observations");
    let admission = measure_external_signal_admission(
        "kalshi",
        "yes_price_signal_live_settled",
        &observations,
        &clock,
        DEFAULT_EXTERNAL_SIGNAL_K,
    )
    .expect("live admission/refusal report");
    let admission_path =
        write_external_signal_admission_report(&root, "live_settled_admission", &admission)
            .expect("write live admission report");

    let mut files = Vec::new();
    collect_files(&root, &mut files);
    let report = json!({
        "issue": 34,
        "proof_claim": "The live unauthenticated Kalshi /markets feed returns settled external market rows, persists raw and parsed source-of-truth artifacts, encodes a finite signal, extracts yes/no outcomes where available, and logs the real Assay admission or refusal report.",
        "official_docs": [
            "https://docs.kalshi.com/getting_started/quick_start_market_data",
            "https://docs.kalshi.com/api-reference/market/get-markets"
        ],
        "minimum_sufficient_proof_corpus": {
            "requested_live_settled_rows": MIN_ASSAY_SAMPLES,
            "parsed_live_rows": page.markets.len(),
            "outcome_labeled_rows": observations.len(),
            "why_this_is_sufficient": "Requesting exactly 50 settled rows matches the Assay KSG sample floor and is the smallest live corpus that can possibly admit a Kalshi signal. If live labels are fewer or single-class, the persisted refusal is the intended fail-closed result.",
            "why_smaller_is_insufficient": "Fewer than 50 live rows cannot prove the real KSG admission path, though they could still prove raw parsing.",
            "why_larger_is_wasteful": "More than 50 live rows would repeat the same endpoint, readback, parser, encoder, and admission/refusal paths without adding a #34 invariant."
        },
        "source_of_truth": "live public Kalshi HTTP response body persisted under this FSV root and parsed only after disk readback",
        "persisted_feed": persisted,
        "first_encoded_signal": first_signal,
        "admission": admission,
        "admission_path": admission_path.display().to_string(),
        "physical_files": files,
        "passed": true
    });
    let report_path = root.join("issue034_live_kalshi_external_feed_fsv_report.json");
    write_json(&report_path, &report);
    let readback: Value =
        serde_json::from_slice(&fs::read(&report_path).expect("read report")).expect("decode");
    assert_eq!(readback["issue"], json!(34));
    assert_eq!(readback["passed"], json!(true));
    assert_eq!(readback["persisted_feed"], report["persisted_feed"]);
    assert_eq!(
        readback["admission"]["code"],
        serde_json::to_value(&admission.code).expect("admission code JSON")
    );
    write_blake3sums(&root);
    println!(
        "ISSUE034_LIVE_KALSHI_EXTERNAL_FEED_FSV={}",
        report_path.display()
    );
}

fn lens_autobuild_handoff(
    root: &Path,
    admission: &calyx_poly::external_kalshi_feed::ExternalSignalAdmissionReport,
    evidence_artifact: &str,
) -> Value {
    let candidate =
        kalshi_lens_candidate_from_admission(admission, evidence_artifact, TrustTag::Trusted)
            .expect("Kalshi lens candidate");
    let deficit = LensDeficit {
        domain: "external".to_string(),
        panel_id: "poly_v1".to_string(),
        panel_version: 1,
        source_artifact: evidence_artifact.to_string(),
        proposal_action: "propose_lens".to_string(),
        deficit_bits: 0.20,
        weakest_slots: vec![16],
        reason: "known external Kalshi signal deficit fixture".to_string(),
        trust: TrustTag::Trusted,
    };
    let request = LensAutobuildRequest {
        domain: deficit.domain.clone(),
        panel_id: deficit.panel_id.clone(),
        panel_version: deficit.panel_version,
        existing_lens_keys: Vec::new(),
        deficits: vec![deficit],
        candidates: vec![candidate],
        min_gain_bits: LENS_AUTOBUILD_MIN_GAIN_BITS,
    };
    let run = run_lens_autobuild_report(&request, &root.join("lens-autobuild"))
        .expect("lens autobuild handoff");
    let readback = read_lens_autobuild_report(&run.report_path).expect("read lens autobuild");
    assert_eq!(readback, run.report);
    assert_eq!(run.report.status, LensAutobuildStatus::Rejected);
    assert_eq!(run.report.admitted_count, 0);
    assert_eq!(run.report.rejected_count, 1);
    let rejection = &run.report.rejected[0];
    assert_eq!(rejection.lens_key, "external_kalshi_yes_price_signal");
    assert_eq!(rejection.code, "uncalibrated_estimate_bound");
    json!({
        "report_path": run.report_path.display().to_string(),
        "status": run.report.status,
        "admitted_count": run.report.admitted_count,
        "rejected_count": run.report.rejected_count,
        "lens_key": rejection.lens_key,
        "rejection_code": rejection.code,
        "estimate_bound": admission.estimate_bound,
        "decision_hash": run.report.decision_hash
    })
}

fn edge_cases(root: &Path, clock: &FixedClock) -> Value {
    let missing = parse_kalshi_markets_value(&json!({"cursor": ""}))
        .expect_err("missing markets array fails closed");
    assert_eq!(missing.code(), ERR_KALSHI_MARKET_INVALID);

    let no_price = parse_kalshi_market(&json!({
        "ticker": "KXNO-PRICE",
        "title": "No price row",
        "status": "settled",
        "result": "yes"
    }))
    .expect("minimal market parses");
    let no_price_err =
        encode_kalshi_market_signal(&no_price).expect_err("missing price fails closed");
    assert_eq!(no_price_err.code(), ERR_KALSHI_ENCODE_INVALID);

    let no_spread = parse_kalshi_market(&json!({
        "ticker": "KXNO-SPREAD",
        "title": "No spread row",
        "status": "settled",
        "result": "yes",
        "last_price_dollars": "0.4200",
        "liquidity_dollars": "10.0",
        "volume_fp": "11.0",
        "open_interest_fp": "12.0"
    }))
    .expect("missing-spread market parses");
    let no_spread_err =
        encode_kalshi_market_signal(&no_spread).expect_err("missing spread fails closed");
    assert_eq!(no_spread_err.code(), ERR_KALSHI_ENCODE_INVALID);

    let missing_required = [
        (
            "liquidity_dollars",
            json!({
                "ticker": "KXNO-LIQ",
                "title": "No liquidity row",
                "status": "settled",
                "result": "yes",
                "yes_bid_dollars": "0.4000",
                "yes_ask_dollars": "0.4400",
                "volume_fp": "11.0",
                "open_interest_fp": "12.0"
            }),
        ),
        (
            "volume_fp",
            json!({
                "ticker": "KXNO-VOL",
                "title": "No volume row",
                "status": "settled",
                "result": "yes",
                "yes_bid_dollars": "0.4000",
                "yes_ask_dollars": "0.4400",
                "liquidity_dollars": "10.0",
                "open_interest_fp": "12.0"
            }),
        ),
        (
            "open_interest_fp",
            json!({
                "ticker": "KXNO-OI",
                "title": "No open interest row",
                "status": "settled",
                "result": "yes",
                "yes_bid_dollars": "0.4000",
                "yes_ask_dollars": "0.4400",
                "liquidity_dollars": "10.0",
                "volume_fp": "11.0"
            }),
        ),
    ];
    let mut missing_required_report = Vec::new();
    for (field, payload) in missing_required {
        let market = parse_kalshi_market(&payload).expect("required-field market parses");
        let err = encode_kalshi_market_signal(&market).expect_err("missing field fails closed");
        assert_eq!(err.code(), ERR_KALSHI_ENCODE_INVALID);
        assert!(err.message().contains(field));
        missing_required_report.push(json!({"field": field, "message": err.message()}));
    }

    let mut nonfinite = known_truth_observations(MIN_ASSAY_SAMPLES);
    nonfinite[0].signal_value = f32::NAN;
    let nonfinite_err = measure_external_signal_admission(
        "kalshi",
        "bad_signal",
        &nonfinite,
        clock,
        DEFAULT_EXTERNAL_SIGNAL_K,
    )
    .expect_err("non-finite signal fails closed");
    assert_eq!(nonfinite_err.code(), ERR_EXTERNAL_SIGNAL_ADMISSION_INVALID);

    let single_class: Vec<_> = (0..MIN_ASSAY_SAMPLES)
        .map(|i| ExternalSignalOutcomeObservation {
            signal_value: 0.2 + i as f32 * 0.001,
            outcome: true,
        })
        .collect();
    let single_report = measure_external_signal_admission(
        "kalshi",
        "single_class",
        &single_class,
        clock,
        DEFAULT_EXTERNAL_SIGNAL_K,
    )
    .expect("single-class refusal report");
    assert_eq!(single_report.code, EXTERNAL_SIGNAL_REFUSED_SINGLE_CLASS);
    assert!(!single_report.admitted);

    let below_floor = known_truth_observations(MIN_ASSAY_SAMPLES - 1);
    let below_report = measure_external_signal_admission(
        "kalshi",
        "below_floor",
        &below_floor,
        clock,
        DEFAULT_EXTERNAL_SIGNAL_K,
    )
    .expect("below-floor refusal report");
    assert_eq!(below_report.code, EXTERNAL_SIGNAL_REFUSED_UNDERPOWERED);
    assert!(!below_report.admitted);

    let edge_report = json!({
        "missing_markets_array": {"code": missing.code(), "message": missing.message()},
        "missing_price_signal": {"code": no_price_err.code(), "message": no_price_err.message()},
        "missing_spread": {"code": no_spread_err.code(), "message": no_spread_err.message()},
        "missing_required_features": missing_required_report,
        "nonfinite_signal": {"code": nonfinite_err.code(), "message": nonfinite_err.message()},
        "single_class_refusal": single_report,
        "below_floor_refusal": below_report
    });
    write_json(&root.join("edge_cases.json"), &edge_report);
    edge_report
}

fn known_truth_observations(n: usize) -> Vec<ExternalSignalOutcomeObservation> {
    let split = n / 2;
    (0..n)
        .map(|i| {
            let outcome = i >= split;
            let offset = (i % split.max(1)) as f32 * 0.001;
            ExternalSignalOutcomeObservation {
                signal_value: if outcome {
                    0.72 + offset
                } else {
                    0.18 + offset
                },
                outcome,
            }
        })
        .collect()
}

fn kalshi_fixture_body() -> Vec<u8> {
    serde_json::to_vec_pretty(&json!({
        "cursor": "",
        "markets": [
            {
                "ticker": "KXISSUE34YES",
                "event_ticker": "KXISSUE34",
                "title": "Will the issue 34 known-truth yes market resolve yes?",
                "subtitle": "Known-truth yes row",
                "status": "settled",
                "market_type": "binary",
                "close_time": "2026-07-01T00:00:00Z",
                "expiration_time": "2026-07-01T01:00:00Z",
                "settlement_ts": "2026-07-01T02:00:00Z",
                "result": "yes",
                "expiration_value": "",
                "yes_bid_dollars": "0.4100",
                "yes_ask_dollars": "0.4500",
                "no_bid_dollars": "0.5500",
                "no_ask_dollars": "0.5900",
                "last_price_dollars": "0.4300",
                "previous_price_dollars": "0.4200",
                "settlement_value_dollars": "1.0000",
                "liquidity_dollars": "12345.67",
                "volume_fp": "1000.25",
                "volume_24h_fp": "125.50",
                "open_interest_fp": "50.00"
            },
            {
                "ticker": "KXISSUE34NO",
                "event_ticker": "KXISSUE34",
                "title": "Will the issue 34 known-truth no market resolve yes?",
                "subtitle": "Known-truth no row",
                "status": "finalized",
                "market_type": "binary",
                "result": "no",
                "expiration_value": "",
                "yes_bid_dollars": "0.0000",
                "yes_ask_dollars": "1.0000",
                "last_price_dollars": "0.7200",
                "settlement_value_dollars": "0.0000",
                "liquidity_dollars": "0.0000",
                "volume_fp": "2272.72",
                "volume_24h_fp": "12.00",
                "open_interest_fp": "2272.72"
            }
        ]
    }))
    .expect("encode fixture body")
}

fn assert_not_d_drive(path: &Path) {
    #[cfg(not(windows))]
    let _ = path;
    #[cfg(windows)]
    {
        use std::path::{Component, Prefix};
        if let Some(Component::Prefix(prefix)) = path.components().next() {
            match prefix.kind() {
                Prefix::Disk(b'D')
                | Prefix::Disk(b'd')
                | Prefix::VerbatimDisk(b'D')
                | Prefix::VerbatimDisk(b'd') => panic!("FSV root must not use D: drive"),
                _ => {}
            }
        }
    }
}
