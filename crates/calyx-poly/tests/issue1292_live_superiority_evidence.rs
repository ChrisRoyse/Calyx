#[path = "live_calyx_native_evidence_support.rs"]
mod evidence_support;
#[path = "fsv_support.rs"]
mod support;

use calyx_anneal::{GoodhartViolation, RegressionReport};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{Clock, CxId, FixedClock, SystemClock, VaultId};
use calyx_poly::calyx_native::write_calyx_native_forecast;
use calyx_poly::crypto_forecast_registration::{
    CryptoForecastRegistrationMode, CryptoForecastRegistrationRequest,
    produce_live_calyx_native_forecast, register_crypto_pending_for_mode,
    register_crypto_pending_from_calyx_native_artifact,
};
use calyx_poly::live_calyx_native_evidence::{
    ERR_LIVE_CALYX_NATIVE_EVIDENCE_MISSING, ERR_LIVE_CALYX_NATIVE_EVIDENCE_STALE,
    LIVE_CALYX_NATIVE_EVIDENCE_MAX_AGE_MILLIS, StoredLiveCalyxNativeEvidence,
    read_latest_live_calyx_native_evidence,
};
use calyx_poly::model::{Book, MarketSnapshot, OracleRiskEvidence};
use calyx_poly::pending_forecast_register::PendingForecastRegister;
use calyx_poly::score::ForecastSource;
use evidence_support::{record_evidence_parts, record_strong_evidence, strong_evidence_parts};
use support::{named_fsv_root, reset_dir};

const PANEL_VERSION: u32 = 1_292;
const DOMAIN: &str = "crypto";
const HORIZON: &str = "pre_resolution";
const VAULT_ID: &str = "01ARZ3NDEKTSV4RRFFQ69G5FA1";
const EMPTY_VAULT_ID: &str = "01ARZ3NDEKTSV4RRFFQ69G5FA2";
const STALE_VAULT_ID: &str = "01ARZ3NDEKTSV4RRFFQ69G5FA3";
const SALT: &[u8] = b"issue1292-live-superiority";

#[test]
fn live_calyx_native_requires_measured_aster_evidence() {
    let (root, keep) = named_fsv_root(
        "POLY_ISSUE1292_FSV_ROOT",
        "issue1292-live-superiority-evidence",
    );
    reset_dir(&root);
    let evidence_at_millis = SystemClock.now();
    let forecast_seconds = (evidence_at_millis + 300_000).div_ceil(1_000);
    let forecast_at_millis = forecast_seconds * 1_000;
    let vault = fixed_vault(&root.join("vault"), VAULT_ID, evidence_at_millis);
    let stored = record_strong_evidence(&vault, DOMAIN, HORIZON, PANEL_VERSION, evidence_at_millis);
    let readback = read_latest_live_calyx_native_evidence(
        &vault,
        DOMAIN,
        HORIZON,
        PANEL_VERSION,
        forecast_at_millis,
    )
    .expect("read latest evidence from Aster");
    assert_eq!(readback, stored);

    let snapshot = snapshot(Some(0.02), forecast_seconds);
    let forecast =
        produce_live_calyx_native_forecast(&snapshot, DOMAIN, HORIZON, PANEL_VERSION, &readback)
            .expect("produce measured live forecast");
    assert!(forecast.admissible, "{}", forecast.refusal_reason);
    assert_eq!(forecast.computed_at, forecast_at_millis);
    assert_eq!(forecast.evidence, Some(readback.evidence_ref()));
    assert_eq!(
        forecast.calibration.as_ref(),
        Some(&readback.evidence().calibration().slope)
    );

    let mut register = PendingForecastRegister::default();
    let pending = register_crypto_pending_for_mode(
        &vault,
        &mut register,
        registration_request(&snapshot, &root),
    )
    .expect("register evidence-backed CalyxNative forecast");
    assert_eq!(pending.source, ForecastSource::CalyxNative);
    assert_eq!(register.entries.len(), 1);
    assert_tampered_artifact_fails(&vault, &snapshot, &readback, &root);

    let mut missing_spread = snapshot.clone();
    missing_spread.spread = None;
    let spread_error = produce_live_calyx_native_forecast(
        &missing_spread,
        DOMAIN,
        HORIZON,
        PANEL_VERSION,
        &readback,
    )
    .expect_err("unobserved spread must fail closed");
    assert_eq!(spread_error.code(), "CALYX_POLY_CRYPTO_INGESTOR_PENDING");

    assert_missing_evidence_fails(&root, &snapshot);
    assert_stale_evidence_fails(&root, forecast_at_millis);
    assert_measured_tier_failures(&vault, &root, &snapshot, evidence_at_millis);
    vault.flush().expect("flush issue1292 vault");

    if !keep {
        std::fs::remove_dir_all(root).expect("remove issue1292 test root");
    }
}

fn assert_missing_evidence_fails(root: &std::path::Path, snapshot: &MarketSnapshot) {
    let vault = fixed_vault(
        &root.join("empty-vault"),
        EMPTY_VAULT_ID,
        snapshot.snapshot_ts * 1_000,
    );
    let error = register_crypto_pending_for_mode(
        &vault,
        &mut PendingForecastRegister::default(),
        registration_request(snapshot, root),
    )
    .expect_err("missing evidence must refuse CalyxNative registration");
    assert_eq!(error.code(), ERR_LIVE_CALYX_NATIVE_EVIDENCE_MISSING);
}

fn assert_stale_evidence_fails(root: &std::path::Path, forecast_at_millis: u64) {
    let old_millis = forecast_at_millis - LIVE_CALYX_NATIVE_EVIDENCE_MAX_AGE_MILLIS - 1;
    let vault = fixed_vault(&root.join("stale-vault"), STALE_VAULT_ID, old_millis);
    record_strong_evidence(&vault, DOMAIN, HORIZON, PANEL_VERSION, old_millis);
    let error = read_latest_live_calyx_native_evidence(
        &vault,
        DOMAIN,
        HORIZON,
        PANEL_VERSION,
        forecast_at_millis,
    )
    .expect_err("stale evidence must fail closed");
    assert_eq!(error.code(), ERR_LIVE_CALYX_NATIVE_EVIDENCE_STALE);
    vault.flush().expect("flush stale evidence vault");
}

fn assert_tampered_artifact_fails(
    vault: &AsterVault<FixedClock>,
    snapshot: &MarketSnapshot,
    evidence: &StoredLiveCalyxNativeEvidence,
    root: &std::path::Path,
) {
    let mut forecast =
        produce_live_calyx_native_forecast(snapshot, DOMAIN, HORIZON, PANEL_VERSION, evidence)
            .expect("produce forecast for tamper regression");
    forecast.p_model = (forecast.p_model + 0.01).min(1.0);
    let path = write_calyx_native_forecast(&root.join("tampered"), &forecast)
        .expect("write tampered forecast artifact");
    let error = register_crypto_pending_from_calyx_native_artifact(
        vault,
        &mut PendingForecastRegister::default(),
        snapshot,
        CxId::from_bytes([13; 16]),
        DOMAIN,
        HORIZON,
        &path,
    )
    .expect_err("tampered forecast must not become an Aster pending row");
    assert_eq!(error.code(), "CALYX_POLY_CRYPTO_INGESTOR_PENDING");
}

fn assert_measured_tier_failures(
    vault: &AsterVault<FixedClock>,
    root: &std::path::Path,
    snapshot: &MarketSnapshot,
    evidence_at_millis: u64,
) {
    let mut kernel = strong_evidence_parts(DOMAIN, HORIZON, PANEL_VERSION, evidence_at_millis);
    kernel.kernel_recall.measured_ratio = 0.80;
    kernel.kernel_recall.recall.ratio = 0.80;
    kernel.kernel_recall.recall.kernel_only = 0.80 * kernel.kernel_recall.recall.full;
    kernel.kernel_recall.gate_passed = false;
    let kernel_evidence = record_evidence_parts(vault, &kernel);
    assert_failing_tier(snapshot, &kernel_evidence, "kernel");
    let error = register_crypto_pending_for_mode(
        vault,
        &mut PendingForecastRegister::default(),
        registration_request(snapshot, root),
    )
    .expect_err("non-admissible measured kernel must not register");
    assert_eq!(error.code(), "CALYX_POLY_CRYPTO_INGESTOR_PENDING");

    let mut panel = strong_evidence_parts(DOMAIN, HORIZON, PANEL_VERSION, evidence_at_millis);
    panel.panel.sufficient = false;
    panel.panel.assay_card.sufficient = false;
    let panel_evidence = record_evidence_parts(vault, &panel);
    assert_failing_tier(snapshot, &panel_evidence, "sufficient");

    let mut goodhart = strong_evidence_parts(DOMAIN, HORIZON, PANEL_VERSION, evidence_at_millis);
    goodhart.goodhart.passed = false;
    goodhart
        .goodhart
        .violations
        .push(GoodhartViolation::GtauViolation {
            in_region_frac: 0.50,
            threshold: 0.95,
        });
    goodhart.goodhart.in_region_frac = Some(0.50);
    let goodhart_evidence = record_evidence_parts(vault, &goodhart);
    assert_failing_tier(snapshot, &goodhart_evidence, "goodhart");

    let mut mistakes = strong_evidence_parts(DOMAIN, HORIZON, PANEL_VERSION, evidence_at_millis);
    mistakes.mistakes.results[0].recurred = true;
    mistakes.mistakes = RegressionReport::new(mistakes.mistakes.results.clone());
    let mistake_evidence = record_evidence_parts(vault, &mistakes);
    assert_failing_tier(snapshot, &mistake_evidence, "mistake_closed");
}

fn assert_failing_tier(
    snapshot: &MarketSnapshot,
    evidence: &StoredLiveCalyxNativeEvidence,
    tier: &str,
) {
    let forecast =
        produce_live_calyx_native_forecast(snapshot, DOMAIN, HORIZON, PANEL_VERSION, evidence)
            .expect("produce measured non-admissible forecast");
    assert!(!forecast.admissible);
    assert!(
        forecast
            .superiority
            .failing_tiers
            .iter()
            .any(|name| name == tier)
    );
}

fn fixed_vault(
    path: &std::path::Path,
    vault_id: &str,
    clock_millis: u64,
) -> AsterVault<FixedClock> {
    AsterVault::new_durable_with_clock(
        path,
        vault_id.parse::<VaultId>().expect("parse vault id"),
        SALT.to_vec(),
        VaultOptions::default(),
        FixedClock::new(clock_millis),
    )
    .expect("open fixed-clock Aster vault")
}

fn registration_request<'a>(
    snapshot: &'a MarketSnapshot,
    output_root: &'a std::path::Path,
) -> CryptoForecastRegistrationRequest<'a> {
    CryptoForecastRegistrationRequest {
        snapshot,
        cx_id: CxId::from_bytes([12; 16]),
        domain: DOMAIN,
        horizon_bucket: HORIZON,
        output_root,
        mode: CryptoForecastRegistrationMode::CalyxNative,
        panel_version: PANEL_VERSION,
    }
}

fn snapshot(spread: Option<f64>, snapshot_ts: u64) -> MarketSnapshot {
    MarketSnapshot {
        token_id: "issue1292-token".to_string(),
        condition_id: "issue1292-condition".to_string(),
        outcome_index: 0,
        slug: "issue1292-live-superiority".to_string(),
        question: Some("Will measured evidence gate this forecast?".to_string()),
        event_id: Some("issue1292".to_string()),
        category: Some(DOMAIN.to_string()),
        region: Some("global".to_string()),
        tags: vec![DOMAIN.to_string()],
        resolution_source: Some("uma".to_string()),
        neg_risk: false,
        snapshot_ts,
        price: Some(0.61),
        mid: Some(0.61),
        best_bid: Some(0.60),
        best_ask: Some(0.62),
        spread,
        tick_size: Some(0.01),
        volume_24h: Some(1_000.0),
        liquidity: Some(500.0),
        one_hour_change: Some(0.01),
        one_day_change: Some(0.03),
        ofi: Some(0.20),
        yes_no_residual: Some(0.0),
        secs_to_resolution: Some(3_600.0),
        holders: Vec::new(),
        makers: Vec::new(),
        counterparty_volumes: Vec::new(),
        onchain_fills: Vec::new(),
        temporal_reference_ts: Some(snapshot_ts),
        sequence_position: Some(1),
        sequence_total: Some(1),
        oracle_risk: OracleRiskEvidence::default(),
        book: Book::default(),
    }
}
