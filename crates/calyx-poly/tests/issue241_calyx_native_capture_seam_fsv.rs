//! Issue #241 - CalyxNative forecast artifact registration seam for crypto capture.
//!
//! Source of truth: one persisted CalyxNative forecast artifact, one pending ledger row, and the
//! crypto capture state JSON that carries the artifact hash forward for later first-light audit.

#[path = "live_calyx_native_evidence_support.rs"]
mod evidence_support;
#[path = "fsv_support.rs"]
mod support;

use std::fs;
use std::path::Path;

use calyx_assay::TrustTag;
use calyx_aster::cf::{ColumnFamily, ledger_key};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{Clock, CxId, FixedClock, SystemClock, VaultId, VaultStore};
use calyx_ledger::{EntryKind, decode as decode_ledger};
use calyx_poly::calyx_native::{
    CalyxNativeForecast, CalyxNativeRequest, produce_calyx_native_forecast,
    write_calyx_native_forecast,
};
use calyx_poly::crypto_capture_harness::{
    CryptoCaptureHarnessConfig, CryptoCaptureHarnessRequest, CryptoCaptureRunner,
    CryptoCapturedSnapshotRef, LiveCryptoCaptureRunner, run_crypto_capture_harness_once,
};
use calyx_poly::crypto_forecast_registration::{
    CryptoForecastRegistrationMode, CryptoForecastRegistrationRequest,
    register_crypto_pending_for_mode, register_crypto_pending_from_calyx_native_artifact,
};
use calyx_poly::crypto_ingestor::{
    CRYPTO_INGESTOR_SCHEMA_VERSION, CryptoIngestionRun, CryptoIngestorConfig,
    CryptoSnapshotIngestRecord, ERR_CRYPTO_INGESTOR_PENDING, put_crypto_snapshot,
    register_crypto_pending,
};
use calyx_poly::forecast::{ComponentKind, ForecastComponent};
use calyx_poly::lenses::default_panel;
use calyx_poly::live_calyx_native_evidence::{
    LiveCalyxNativeEvidenceStore, read_latest_live_calyx_native_evidence,
};
use calyx_poly::model::{Book, MarketSnapshot, OracleRiskEvidence};
use calyx_poly::pending_forecast_register::{PendingForecastLedgerStore, PendingForecastRegister};
use calyx_poly::score::ForecastSource;
use calyx_poly::superiority::SuperiorityTiers;
use evidence_support::record_strong_evidence;
use serde_json::{Value, json};
use support::{collect_files, named_fsv_root, reset_dir, write_blake3sums, write_json};

const VAULT_ID: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
const VAULT_SALT: &[u8] = b"poly-issue241-calyx-native-capture";
const CONDITION_ID: &str = "0x0000000000000000000000000000000000000000000000000000000000000241";
const TOKEN_ID: &str =
    "34747254630927017064599589941321309211596494400768268440049646273862919127907";
const TOKEN_ID_NO: &str =
    "11505842631172073248875994305472690177886666199549013462524235701036385693509";

#[test]
fn issue241_calyx_native_capture_seam_fsv() {
    let (root, _keep) = named_fsv_root(
        "POLY_ISSUE241_FSV_ROOT",
        "issue241-calyx-native-capture-seam",
    );
    assert_c_drive(&root);
    reset_dir(&root);
    let evidence_at_millis = SystemClock.now();
    let capture_ts = (evidence_at_millis + 300_000).div_ceil(1_000);
    let vault_id: VaultId = VAULT_ID.parse().unwrap();
    let vault = AsterVault::new_durable_with_clock(
        root.join("capture-vault"),
        vault_id,
        VAULT_SALT.to_vec(),
        VaultOptions::default(),
        FixedClock::new(capture_ts * 1_000),
    )
    .unwrap();
    record_strong_evidence(&vault, "crypto", "pre_resolution", 241, evidence_at_millis);
    let mut register = PendingForecastRegister::default();
    let config = CryptoCaptureHarnessConfig {
        interval_secs: 60,
        ingestor_config: CryptoIngestorConfig {
            panel_version: 241,
            capture_ws: false,
            forecast_mode: CryptoForecastRegistrationMode::CalyxNative,
            ..CryptoIngestorConfig::default()
        },
    };
    let mut runner = KnownCalyxNativeRunner::default();
    let run = run_crypto_capture_harness_once(
        &vault,
        &mut register,
        CryptoCaptureHarnessRequest {
            vault_id,
            vault_salt: VAULT_SALT,
            output_root: &root,
            config,
            now_ts: capture_ts,
        },
        &mut runner,
    )
    .expect("capture with CalyxNative artifact seam");
    vault.flush().unwrap();
    assert_eq!(runner.calls, 1);
    let artifact_checks = assert_calyx_native_artifacts(&run.state.captures[0].snapshots);
    assert_eq!(artifact_checks.len(), 2);
    let unique_paths = artifact_checks
        .iter()
        .map(|check| check["artifact_path"].as_str().unwrap().to_string())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        unique_paths.len(),
        2,
        "same-condition outcomes need distinct artifacts"
    );
    let captured = &run.state.captures[0].snapshots[0];
    assert_eq!(captured.pending_entry.source, ForecastSource::CalyxNative);
    let ledger = ledger_payload(
        &vault,
        captured.pending_entry.registered_ledger_seq.unwrap(),
    );
    assert_eq!(ledger["event"], json!("poly.pending_forecast_registered"));
    assert_eq!(ledger["forecast"]["source"], json!("CalyxNative"));

    let missing = register_crypto_pending_from_calyx_native_artifact(
        &vault,
        &mut PendingForecastRegister::default(),
        &snapshot(capture_ts),
        CxId::from_bytes([7; 16]),
        "crypto",
        "pre_resolution",
        &root.join("missing-calyx-native.json"),
    )
    .expect_err("missing forecast artifact must fail closed");
    assert_eq!(missing.code(), ERR_CRYPTO_INGESTOR_PENDING);

    let mismatch_dir = root.join("mismatch");
    let mismatch_path = write_calyx_native_forecast(
        &mismatch_dir,
        &forecast(&snapshot(capture_ts), Some("wrong-token")),
    )
    .expect("write mismatched forecast");
    let mismatch = register_crypto_pending_from_calyx_native_artifact(
        &vault,
        &mut PendingForecastRegister::default(),
        &snapshot(capture_ts),
        CxId::from_bytes([8; 16]),
        "crypto",
        "pre_resolution",
        &mismatch_path,
    )
    .expect_err("mismatched artifact must fail closed");
    assert_eq!(mismatch.code(), ERR_CRYPTO_INGESTOR_PENDING);

    let baseline = register_crypto_pending(
        &vault,
        &mut PendingForecastRegister::default(),
        &snapshot(capture_ts),
        CxId::from_bytes([9; 16]),
        "crypto",
        "pre_resolution",
    )
    .expect("baseline path remains explicit");
    assert_eq!(baseline.source, ForecastSource::BaselineMarket);
    assert!(baseline.forecast_artifact_path.is_none());
    assert!(baseline.forecast_artifact_blake3.is_none());

    let mut files = Vec::new();
    collect_files(&root, &mut files);
    let report = json!({
        "issue": 241,
        "schema_version": CRYPTO_INGESTOR_SCHEMA_VERSION,
        "proof_claim": "A crypto capture runner can register a pending forecast only from a persisted/read-back CalyxNative artifact and carry the artifact hash/path into capture state; missing or mismatched artifacts fail closed and baseline registration remains explicitly baseline.",
        "minimum_sufficient_proof_corpus": {
            "known_truth_crypto_snapshots": 2,
            "calyx_native_artifacts": 2,
            "edge_cases": 3,
            "why_this_is_sufficient": "One binary condition with two outcome snapshots proves artifact-backed pending registration, capture-state metadata propagation, and no same-condition artifact overwrite. Missing artifact, mismatched token, and baseline registration cover the false-green edges for this narrow seam.",
            "why_smaller_is_insufficient": "One outcome would not prove same-condition multi-outcome artifact identity; without the failure edges, a missing artifact or baseline relabel could still pass.",
            "why_larger_is_wasteful": "More markets or live sweeps would repeat source capture behavior already covered by #238 and would not add proof for artifact registration correctness."
        },
        "capture_state": run.state,
        "capture_report": run.report,
        "artifact_checks": artifact_checks,
        "edges": {
            "missing_artifact_error": missing.code(),
            "mismatched_artifact_error": mismatch.code(),
            "baseline_source": baseline.source.as_str()
        },
        "physical_files": files,
        "passed": true
    });
    let report_path = root.join("issue241_calyx_native_capture_seam_fsv_report.json");
    write_json(&report_path, &report);
    let readback: Value = serde_json::from_slice(&fs::read(&report_path).unwrap()).unwrap();
    assert_eq!(readback, report);
    write_blake3sums(&root);
}

#[test]
#[ignore = "requires live public Gamma/CLOB/Data API"]
fn issue241_live_calyx_native_capture_mode_fsv() {
    let (root, _keep) = named_fsv_root(
        "POLY_ISSUE241_LIVE_FSV_ROOT",
        "issue241-live-calyx-native-capture",
    );
    assert_c_drive(&root);
    reset_dir(&root);
    let vault_id: VaultId = VAULT_ID.parse().unwrap();
    let vault = AsterVault::new_durable(
        root.join("live-capture-vault"),
        vault_id,
        VAULT_SALT.to_vec(),
        VaultOptions::default(),
    )
    .unwrap();
    let mut register = PendingForecastRegister::default();
    let config = CryptoCaptureHarnessConfig {
        interval_secs: 60,
        ingestor_config: CryptoIngestorConfig {
            market_limit: 500,
            holder_limit: 25,
            trade_limit: 10,
            panel_version: 241,
            capture_ws: false,
            forecast_mode: CryptoForecastRegistrationMode::CalyxNative,
            ..CryptoIngestorConfig::default()
        },
    };
    let run = run_crypto_capture_harness_once(
        &vault,
        &mut register,
        CryptoCaptureHarnessRequest {
            vault_id,
            vault_salt: VAULT_SALT,
            output_root: &root,
            config,
            now_ts: live_now(),
        },
        &mut LiveCryptoCaptureRunner,
    )
    .expect("live CalyxNative capture mode");
    vault.flush().unwrap();
    assert!(!run.state.captures[0].snapshots.is_empty());
    let artifact_checks = assert_calyx_native_artifacts(&run.state.captures[0].snapshots);
    let mut files = Vec::new();
    collect_files(&root, &mut files);
    let report = json!({
        "issue": 241,
        "proof_claim": "The real live crypto capture runner can run in CalyxNative mode against one public/read-only market and persist/read back forecast artifacts linked into pending capture state.",
        "minimum_sufficient_proof_corpus": {
            "live_due_passes": 1,
            "selected_market_count": 1,
            "why_this_is_sufficient": "One live due pass proves the default live runner composes public Gamma/CLOB/Data capture with CalyxNative artifact registration.",
            "why_smaller_is_insufficient": "Zero live passes would not prove the real runner wiring.",
            "why_larger_is_wasteful": "More live markets repeat the same source and artifact-registration path without adding #241 proof."
        },
        "state": run.state,
        "report": run.report,
        "artifact_checks": artifact_checks,
        "physical_files": files,
        "passed": true
    });
    let report_path = root.join("issue241_live_calyx_native_capture_mode_fsv_report.json");
    write_json(&report_path, &report);
    let readback: Value = serde_json::from_slice(&fs::read(&report_path).unwrap()).unwrap();
    assert_eq!(readback, report);
    write_blake3sums(&root);
}

#[derive(Default)]
struct KnownCalyxNativeRunner {
    calls: usize,
}

impl<S> CryptoCaptureRunner<S> for KnownCalyxNativeRunner
where
    S: VaultStore + PendingForecastLedgerStore + LiveCalyxNativeEvidenceStore,
{
    fn run_capture_cycle(
        &mut self,
        store: &S,
        register: &mut PendingForecastRegister,
        vault_id: VaultId,
        vault_salt: &[u8],
        output_root: &Path,
        config: CryptoIngestorConfig,
    ) -> calyx_poly::Result<CryptoIngestionRun> {
        self.calls += 1;
        let _ = read_latest_live_calyx_native_evidence(
            store,
            &config.domain,
            &config.horizon_bucket,
            config.panel_version,
            config.captured_ts * 1_000,
        )?;
        let panel = default_panel(config.panel_version, config.region_vocab.clone());
        let mut records = Vec::new();
        for snapshot in [
            snapshot(config.captured_ts),
            snapshot_no(config.captured_ts),
        ] {
            let put = put_crypto_snapshot(store, &panel, &snapshot, vault_id, vault_salt)?;
            let cx_id: CxId = put.cx_id.parse().expect("parse cx id");
            let pending = register_crypto_pending_for_mode(
                store,
                register,
                CryptoForecastRegistrationRequest {
                    snapshot: &snapshot,
                    cx_id,
                    domain: &config.domain,
                    horizon_bucket: &config.horizon_bucket,
                    output_root,
                    mode: config.forecast_mode,
                    panel_version: config.panel_version,
                },
            )?;
            records.push(CryptoSnapshotIngestRecord { put, pending });
        }
        Ok(CryptoIngestionRun {
            schema_version: CRYPTO_INGESTOR_SCHEMA_VERSION.to_string(),
            domain: config.domain,
            captured_ts: config.captured_ts,
            market_id: "241".to_string(),
            condition_id: CONDITION_ID.to_string(),
            token_count: records.len(),
            snapshots: records,
            ws_report: None,
        })
    }
}

fn forecast(snapshot: &MarketSnapshot, token_override: Option<&str>) -> CalyxNativeForecast {
    let req = CalyxNativeRequest {
        domain: "crypto".to_string(),
        condition_id: snapshot.condition_id.clone(),
        token_id: token_override.unwrap_or(&snapshot.token_id).to_string(),
        horizon_bucket: "pre_resolution".to_string(),
        components: vec![
            component(ComponentKind::KnnBaseRate, 0.62, 0.8),
            component(ComponentKind::BitsVote, 0.67, 0.9),
        ],
        calibration: None,
        raw_confidence: 0.8,
        oracle_flakiness: 0.05,
        oracle_validity: 0.95,
        panel_bits: 1.0,
        anchor_entropy_bits: 1.0,
        superiority_tiers: SuperiorityTiers {
            oracle_self_consistency: 0.9,
            panel_sufficient: true,
            kernel_recall_ratio: 0.97,
            min_kernel_recall_ratio: 0.95,
            calibrated: true,
            goodhart_defended: true,
            mistake_closed: true,
        },
        evidence: None,
    };
    produce_calyx_native_forecast(&req, &FixedClock::new(snapshot.snapshot_ts * 1_000))
        .expect("produce forecast")
}

fn component(kind: ComponentKind, p: f64, reliability: f64) -> ForecastComponent {
    ForecastComponent::new(kind, p, reliability, 100, TrustTag::Trusted, "issue241").unwrap()
}

fn snapshot(snapshot_ts: u64) -> MarketSnapshot {
    snapshot_for(snapshot_ts, 0, TOKEN_ID, 0.61)
}

fn snapshot_no(snapshot_ts: u64) -> MarketSnapshot {
    snapshot_for(snapshot_ts, 1, TOKEN_ID_NO, 0.39)
}

fn snapshot_for(
    snapshot_ts: u64,
    outcome_index: u32,
    token_id: &str,
    price: f64,
) -> MarketSnapshot {
    MarketSnapshot {
        token_id: token_id.to_string(),
        condition_id: CONDITION_ID.to_string(),
        outcome_index,
        slug: "issue241-calyx-native-seam".to_string(),
        question: Some("Issue 241 known-truth crypto forecast seam?".to_string()),
        event_id: Some("issue241".to_string()),
        category: Some("crypto".to_string()),
        region: Some("global".to_string()),
        tags: vec!["crypto".to_string()],
        resolution_source: Some("uma".to_string()),
        neg_risk: false,
        snapshot_ts,
        price: Some(price),
        mid: Some(price),
        best_bid: Some((price - 0.01).max(0.0)),
        best_ask: Some((price + 0.01).min(1.0)),
        spread: Some(0.02),
        tick_size: Some(0.01),
        volume_24h: Some(1000.0),
        liquidity: Some(500.0),
        one_hour_change: Some(0.01),
        one_day_change: Some(0.03),
        ofi: Some(0.2),
        yes_no_residual: Some(0.0),
        secs_to_resolution: Some(3600.0),
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

fn assert_calyx_native_artifacts(snapshots: &[CryptoCapturedSnapshotRef]) -> Vec<Value> {
    snapshots
        .iter()
        .map(|snapshot| {
            assert_eq!(snapshot.pending_entry.source, ForecastSource::CalyxNative);
            let path = snapshot
                .forecast_artifact_path
                .as_ref()
                .expect("artifact path")
                .clone();
            let hash = snapshot
                .forecast_artifact_blake3
                .as_ref()
                .expect("artifact hash")
                .clone();
            let bytes = fs::read(&path).expect("read forecast artifact");
            assert_eq!(blake3::hash(&bytes).to_hex().to_string(), hash);
            json!({
                "forecast_id": snapshot.forecast_id,
                "token_id": snapshot.token_id,
                "source": snapshot.pending_entry.source.as_str(),
                "artifact_path": path,
                "artifact_blake3": hash,
                "p_model": snapshot.pending_entry.p_model,
                "confidence": snapshot.pending_entry.confidence
            })
        })
        .collect()
}

fn ledger_payload<C: Clock>(vault: &AsterVault<C>, seq: u64) -> Value {
    let row = vault
        .read_cf_at(vault.snapshot(), ColumnFamily::Ledger, &ledger_key(seq))
        .unwrap()
        .expect("ledger row");
    let ledger = decode_ledger(&row).expect("decode ledger");
    assert_eq!(ledger.kind, EntryKind::Measure);
    serde_json::from_slice(&ledger.payload).expect("decode payload")
}

fn assert_c_drive(path: &Path) {
    support::assert_host_fsv_root(path, "FSV root");
}

fn live_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}
