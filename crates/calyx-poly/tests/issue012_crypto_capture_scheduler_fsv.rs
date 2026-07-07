//! Issue #12 - scheduler wiring to continuous crypto capture.
//!
//! Source of truth: scheduler state/report JSON plus delegated #238 harness state read back from
//! disk, with vault ledger rows for captured pending forecasts.

#[path = "fsv_support.rs"]
mod support;

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use calyx_aster::cf::{ColumnFamily, ledger_key};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{Clock, CxId, VaultId, VaultStore};
use calyx_ledger::{EntryKind, decode as decode_ledger};
use calyx_poly::crypto_capture_harness::{
    CryptoCaptureHarnessConfig, CryptoCaptureRunner, LiveCryptoCaptureRunner,
};
use calyx_poly::crypto_capture_scheduler::{
    CRYPTO_CAPTURE_SCHEDULER_SCHEMA_VERSION, CryptoCaptureSchedulerConfig,
    CryptoCaptureSchedulerDecision, CryptoCaptureSchedulerRequest,
    ERR_CRYPTO_CAPTURE_SCHEDULER_INVALID_CONFIG, read_crypto_capture_scheduler_state,
    run_crypto_capture_scheduler_tick,
};
use calyx_poly::crypto_ingestor::{
    CRYPTO_INGESTOR_SCHEMA_VERSION, CryptoIngestionRun, CryptoIngestorConfig, CryptoMarketInputs,
    CryptoSnapshotIngestRecord, build_crypto_market_snapshots, put_crypto_snapshot,
    register_crypto_pending,
};
use calyx_poly::lenses::default_panel;
use calyx_poly::{
    ClobBookStatus, ClobOrderBook, DataApiTradeRecord, DataApiTradeSide, GammaJoinKey,
    GammaMarketRecord, GammaOutcomeShape, HolderShare, LocalOnlyPolicy, PendingForecastRegister,
    PublicBookLevel,
};
use serde_json::{Value, json};

use support::{collect_files, named_fsv_root, reset_dir, write_blake3sums, write_json};

const VAULT_ID: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
const VAULT_SALT: &[u8] = b"poly-issue012-crypto-scheduler";
const CAPTURE_TS: u64 = 1_785_500_120;
const CONDITION: &str = "0x0000000000000000000000000000000000000000000000000000000000000012";

#[test]
fn issue012_crypto_capture_scheduler_known_truth_fsv() {
    let (root, _keep) = named_fsv_root(
        "POLY_ISSUE012_FSV_ROOT",
        "issue012-crypto-capture-scheduler",
    );
    assert_c_drive(&root);
    reset_dir(&root);
    let vault_id: VaultId = VAULT_ID.parse().unwrap();
    let vault = AsterVault::new_durable(
        root.join("scheduler-vault"),
        vault_id,
        VAULT_SALT.to_vec(),
        VaultOptions::default(),
    )
    .unwrap();
    let mut register = PendingForecastRegister::default();
    let config = scheduler_config();
    let mut runner = KnownRunner::default();
    let first = run_crypto_capture_scheduler_tick(
        &vault,
        &mut register,
        scheduler_request(&root, vault_id, config.clone(), CAPTURE_TS),
        &mut runner,
    )
    .expect("first scheduler tick captures");
    assert_eq!(
        first.report.decision,
        CryptoCaptureSchedulerDecision::Captured
    );
    assert_eq!(runner.calls, 1);
    assert_eq!(first.state.capture_invocation_count, 1);
    assert!(first.report.harness_report.is_some());

    let duplicate = run_crypto_capture_scheduler_tick(
        &vault,
        &mut register,
        scheduler_request(&root, vault_id, config.clone(), CAPTURE_TS),
        &mut runner,
    )
    .expect("duplicate scheduler tick skips before harness fetch");
    assert_eq!(
        duplicate.report.decision,
        CryptoCaptureSchedulerDecision::SchedulerSkippedAlreadyRan
    );
    assert_eq!(runner.calls, 1, "duplicate tick must not invoke capture");
    assert!(duplicate.harness_run.is_none());
    assert_eq!(duplicate.state.capture_invocation_count, 1);
    let duplicate_calls_after = runner.calls;

    let second = run_crypto_capture_scheduler_tick(
        &vault,
        &mut register,
        scheduler_request(&root, vault_id, config.clone(), CAPTURE_TS + 60),
        &mut runner,
    )
    .expect("next due slot captures again");
    assert_eq!(
        second.report.decision,
        CryptoCaptureSchedulerDecision::Captured
    );
    assert_eq!(runner.calls, 2);
    assert_eq!(second.state.tick_count, 3);
    assert_eq!(second.state.capture_invocation_count, 2);

    let invalid_cadence = run_crypto_capture_scheduler_tick(
        &vault,
        &mut register,
        scheduler_request(
            &root.join("edge-zero-cadence"),
            vault_id,
            CryptoCaptureSchedulerConfig {
                cadence_secs: 0,
                ..config.clone()
            },
            CAPTURE_TS,
        ),
        &mut runner,
    )
    .expect_err("zero cadence fails closed");
    assert_eq!(
        invalid_cadence.code(),
        ERR_CRYPTO_CAPTURE_SCHEDULER_INVALID_CONFIG
    );

    let mismatch = run_crypto_capture_scheduler_tick(
        &vault,
        &mut register,
        scheduler_request(
            &root,
            vault_id,
            CryptoCaptureSchedulerConfig {
                job_id: "different-capture-job".to_string(),
                ..config
            },
            CAPTURE_TS + 120,
        ),
        &mut runner,
    )
    .expect_err("existing scheduler job id mismatch fails closed");
    assert_eq!(mismatch.code(), ERR_CRYPTO_CAPTURE_SCHEDULER_INVALID_CONFIG);

    vault.flush().unwrap();
    let scheduler_state =
        read_crypto_capture_scheduler_state(&root.join("crypto-capture-scheduler-state.json"))
            .expect("scheduler state readback");
    assert_eq!(scheduler_state.capture_invocation_count, 2);
    let harness_state: Value = serde_json::from_slice(
        &fs::read(root.join("crypto-capture-harness/crypto-capture-harness-state.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(harness_state["captures"].as_array().unwrap().len(), 2);
    let ledger = ledger_payload(
        &vault,
        harness_state["captures"][0]["snapshots"][0]["pending_entry"]["registered_ledger_seq"]
            .as_u64()
            .unwrap(),
    );
    assert_eq!(ledger["event"], json!("poly.pending_forecast_registered"));

    let mut files = Vec::new();
    collect_files(&root, &mut files);
    let report = json!({
        "issue": 12,
        "schema_version": CRYPTO_CAPTURE_SCHEDULER_SCHEMA_VERSION,
        "ingestor_schema_version": CRYPTO_INGESTOR_SCHEMA_VERSION,
        "proof_claim": "A local scheduler tick drives the #238 crypto capture harness, persists and reads back scheduler state/report artifacts, skips duplicate due slots before any capture fetch, invokes the next due slot, and fails closed on malformed scheduler config.",
        "minimum_sufficient_proof_corpus": {
            "known_truth_markets": 1,
            "outcome_snapshots_per_capture": 2,
            "scheduler_ticks": 3,
            "edge_cases": 3,
            "why_this_is_sufficient": "One binary crypto market with two outcomes is the smallest corpus that proves scheduler-to-harness capture and vault pending registration. Three ticks prove first due capture, duplicate pre-fetch skip, and next-slot capture. The zero-cadence and state-mismatch edges prove fail-closed scheduler config handling.",
            "why_smaller_is_insufficient": "One tick would not prove duplicate skip or next-slot scheduling; one token would not prove binary capture; no malformed config would leave #12 fail-closed scheduler behavior unproven.",
            "why_larger_is_wasteful": "More markets or longer schedules would repeat the same scheduler, harness, vault, and readback paths without proving another invariant."
        },
        "source_of_truth": "crypto-capture-scheduler-state.json, crypto-capture-scheduler-report.json, delegated harness state JSON, and AsterVault ledger rows read back from disk",
        "first_tick": first.report,
        "duplicate_tick": duplicate.report,
        "second_tick": second.report,
        "final_scheduler_state": scheduler_state,
        "final_harness_state": harness_state,
        "edges": {
            "duplicate_tick_capture_invocations": duplicate_calls_after,
            "zero_cadence_error": invalid_cadence.code(),
            "state_mismatch_error": mismatch.code()
        },
        "physical_files": files,
        "passed": true
    });
    let report_path = root.join("issue012_crypto_capture_scheduler_fsv_report.json");
    write_json(&report_path, &report);
    let readback: Value = serde_json::from_slice(&fs::read(&report_path).unwrap()).unwrap();
    assert_eq!(readback, report);
    write_blake3sums(&root);
}

#[test]
#[ignore = "requires live public Gamma/CLOB/Data API"]
fn issue012_live_crypto_capture_scheduler_tick_fsv() {
    let (root, _keep) = named_fsv_root(
        "POLY_ISSUE012_LIVE_FSV_ROOT",
        "issue012-live-crypto-capture-scheduler",
    );
    assert_c_drive(&root);
    reset_dir(&root);
    let vault_id: VaultId = VAULT_ID.parse().unwrap();
    let vault = AsterVault::new_durable(
        root.join("live-scheduler-vault"),
        vault_id,
        VAULT_SALT.to_vec(),
        VaultOptions::default(),
    )
    .unwrap();
    let now_ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let mut register = PendingForecastRegister::default();
    let run = run_crypto_capture_scheduler_tick(
        &vault,
        &mut register,
        scheduler_request(&root, vault_id, live_scheduler_config(), now_ts),
        &mut LiveCryptoCaptureRunner,
    )
    .expect("live scheduler tick captures through #238 harness");
    vault.flush().unwrap();
    assert_eq!(
        run.report.decision,
        CryptoCaptureSchedulerDecision::Captured
    );
    assert!(run.report.harness_report.is_some());
    assert_eq!(
        read_crypto_capture_scheduler_state(&run.state_path).unwrap(),
        run.state
    );
    let report = json!({
        "issue": 12,
        "schema_version": CRYPTO_CAPTURE_SCHEDULER_SCHEMA_VERSION,
        "proof_claim": "The local scheduler can drive one real public/read-only crypto capture tick through the #238 live harness and read back scheduler plus harness state from disk.",
        "minimum_sufficient_proof_corpus": {
            "live_scheduler_ticks": 1,
            "selected_market_count": 1,
            "why_this_is_sufficient": "One live due tick proves the scheduler composes the real public-source #238 capture path instead of only synthetic runner wiring.",
            "why_smaller_is_insufficient": "Zero live ticks would not prove scheduler-to-live-harness behavior.",
            "why_larger_is_wasteful": "More live ticks before market resolution would repeat the same source and state paths without proving maturity."
        },
        "source_of_truth": "scheduler state/report JSON, delegated harness state/report JSON, and AsterVault rows under this C-drive FSV root",
        "state": run.state,
        "report": run.report,
        "passed": true
    });
    write_json(
        &root.join("issue012_live_crypto_capture_scheduler_fsv_report.json"),
        &report,
    );
    write_blake3sums(&root);
}

fn scheduler_request<'a>(
    root: &'a Path,
    vault_id: VaultId,
    config: CryptoCaptureSchedulerConfig,
    now_ts: u64,
) -> CryptoCaptureSchedulerRequest<'a> {
    CryptoCaptureSchedulerRequest {
        vault_id,
        vault_salt: VAULT_SALT,
        output_root: root,
        config,
        now_ts,
        policy: LocalOnlyPolicy::default(),
    }
}

fn scheduler_config() -> CryptoCaptureSchedulerConfig {
    CryptoCaptureSchedulerConfig {
        job_id: "crypto-capture-minute".to_string(),
        cadence_secs: 60,
        harness_config: CryptoCaptureHarnessConfig {
            interval_secs: 60,
            ingestor_config: CryptoIngestorConfig {
                panel_version: 12,
                capture_ws: false,
                ..CryptoIngestorConfig::default()
            },
        },
    }
}

fn live_scheduler_config() -> CryptoCaptureSchedulerConfig {
    let mut config = scheduler_config();
    config.harness_config.ingestor_config = CryptoIngestorConfig {
        market_limit: 500,
        holder_limit: 25,
        trade_limit: 10,
        panel_version: 12,
        capture_ws: false,
        ..CryptoIngestorConfig::default()
    };
    config
}

#[derive(Default)]
struct KnownRunner {
    calls: usize,
}

impl<C> CryptoCaptureRunner<AsterVault<C>> for KnownRunner
where
    C: Clock,
{
    fn run_capture_cycle(
        &mut self,
        store: &AsterVault<C>,
        register: &mut PendingForecastRegister,
        vault_id: VaultId,
        vault_salt: &[u8],
        _output_root: &Path,
        config: CryptoIngestorConfig,
    ) -> calyx_poly::Result<CryptoIngestionRun> {
        self.calls += 1;
        let inputs = known_inputs(config.captured_ts);
        let panel = default_panel(config.panel_version, config.region_vocab.clone());
        let snapshots = build_crypto_market_snapshots(&inputs)?;
        let mut records = Vec::new();
        for snapshot in &snapshots {
            let put = put_crypto_snapshot(store, &panel, snapshot, vault_id, vault_salt)?;
            let cx_id = CxId::from_input(
                &snapshot.canonical_input_bytes()?,
                config.panel_version,
                vault_salt,
            );
            let pending = register_crypto_pending(
                store,
                register,
                snapshot,
                cx_id,
                &config.domain,
                &config.horizon_bucket,
            )?;
            records.push(CryptoSnapshotIngestRecord { put, pending });
        }
        Ok(CryptoIngestionRun {
            schema_version: CRYPTO_INGESTOR_SCHEMA_VERSION.to_string(),
            domain: config.domain,
            captured_ts: config.captured_ts,
            market_id: inputs.market.market_id,
            condition_id: inputs.market.condition_id,
            token_count: records.len(),
            snapshots: records,
            ws_report: None,
        })
    }
}

fn known_inputs(captured_ts: u64) -> CryptoMarketInputs {
    CryptoMarketInputs {
        market: GammaMarketRecord {
            market_id: "m012".to_string(),
            condition_id: CONDITION.to_string(),
            slug: Some("issue012-btc-known-truth".to_string()),
            question: Some("Will BTC close above the scheduler threshold?".to_string()),
            event_id: Some("evt012".to_string()),
            event_slug: Some("evt012-slug".to_string()),
            active: true,
            closed: false,
            neg_risk: false,
            enable_order_book: Some(true),
            outcomes: vec!["Yes".to_string(), "No".to_string()],
            outcome_prices: vec![0.61, 0.39],
            clob_token_ids: vec!["tok-yes-012".to_string(), "tok-no-012".to_string()],
            outcome_shape: GammaOutcomeShape::Binary,
            category: Some("crypto".to_string()),
            resolution_source: Some("uma".to_string()),
            volume_24h: Some(7_000.0),
            liquidity: Some(13_000.0),
            best_bid: Some(0.60),
            best_ask: Some(0.62),
            spread: Some(0.02),
            last_trade_price: Some(0.61),
            end_ts: Some(captured_ts + 86_400),
            join_key: GammaJoinKey {
                market_id: "m012".to_string(),
                condition_id: CONDITION.to_string(),
                token_ids: vec!["tok-yes-012".to_string(), "tok-no-012".to_string()],
                event_id: Some("evt012".to_string()),
            },
        },
        books: vec![
            book("tok-yes-012", 0.60, 0.62),
            book("tok-no-012", 0.38, 0.40),
        ],
        holders: vec![holder("0xholdera", 55.0, 0), holder("0xholderb", 45.0, 1)],
        trades: vec![trade(
            "0xwalleta",
            DataApiTradeSide::Buy,
            "tok-yes-012",
            80.0,
            0.61,
            0,
        )],
        captured_ts,
    }
}

fn book(token_id: &str, bid: f64, ask: f64) -> ClobOrderBook {
    ClobOrderBook {
        condition_id: CONDITION.to_string(),
        token_id: token_id.to_string(),
        timestamp_ms: CAPTURE_TS * 1000,
        hash: Some(format!("hash-{token_id}")),
        bids: vec![PublicBookLevel {
            price: bid,
            size: 90.0,
        }],
        asks: vec![PublicBookLevel {
            price: ask,
            size: 80.0,
        }],
        min_order_size: Some(5.0),
        tick_size: Some(0.01),
        neg_risk: Some(false),
        last_trade_price: Some((bid + ask) / 2.0),
        best_bid: Some(bid),
        best_ask: Some(ask),
        midpoint: Some((bid + ask) / 2.0),
        spread: Some(ask - bid),
        status: ClobBookStatus::Ready,
    }
}

fn holder(wallet: &str, amount: f64, outcome_index: u32) -> HolderShare {
    HolderShare {
        wallet: wallet.to_string(),
        amount,
        outcome_index,
    }
}

fn trade(
    wallet: &str,
    side: DataApiTradeSide,
    asset: &str,
    size: f64,
    price: f64,
    outcome_index: u32,
) -> DataApiTradeRecord {
    DataApiTradeRecord {
        proxy_wallet: wallet.to_string(),
        side,
        asset: asset.to_string(),
        condition_id: CONDITION.to_string(),
        size,
        price,
        timestamp: CAPTURE_TS,
        outcome_index,
        transaction_hash: Some(format!("0xtx{outcome_index}")),
    }
}

fn ledger_payload(vault: &AsterVault<impl Clock>, seq: u64) -> Value {
    let row = vault
        .read_cf_at(vault.snapshot(), ColumnFamily::Ledger, &ledger_key(seq))
        .unwrap()
        .expect("ledger row");
    let ledger = decode_ledger(&row).expect("decode ledger");
    assert_eq!(ledger.kind, EntryKind::Measure);
    serde_json::from_slice(&ledger.payload).expect("decode ledger payload")
}

fn assert_c_drive(path: &Path) {
    let text = PathBuf::from(path).display().to_string().replace('/', "\\");
    assert!(
        text.to_ascii_lowercase().starts_with("c:\\"),
        "FSV root must stay on C:, got {text}"
    );
}
