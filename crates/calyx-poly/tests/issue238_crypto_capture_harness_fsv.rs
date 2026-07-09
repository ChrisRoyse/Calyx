//! Issue #238 - continuous live pre-resolution capture harness.
//!
//! Source of truth: durable AsterVault rows plus the harness state/corpus JSON read back from disk.

#[path = "fsv_support.rs"]
mod support;

use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use calyx_aster::cf::{ColumnFamily, ledger_key};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{Clock, CxId, VaultId, VaultStore};
use calyx_ledger::{EntryKind, decode as decode_ledger};
use calyx_poly::crypto_capture_harness::{
    CRYPTO_CAPTURE_HARNESS_SCHEMA_VERSION, CryptoCaptureDecisionKind, CryptoCaptureHarnessConfig,
    CryptoCaptureHarnessRequest, CryptoCaptureRunner, ERR_CRYPTO_CAPTURE_LOOKAHEAD,
    LiveCryptoCaptureRunner, join_crypto_capture_resolution, read_crypto_capture_state,
    run_crypto_capture_harness_once,
};
use calyx_poly::crypto_ingestor::{
    CRYPTO_INGESTOR_SCHEMA_VERSION, CryptoIngestionRun, CryptoIngestorConfig, CryptoMarketInputs,
    CryptoSnapshotIngestRecord, ERR_CRYPTO_INGESTOR_NOT_PRE_RESOLUTION,
    build_crypto_market_snapshots, put_crypto_snapshot, register_crypto_pending,
};
use calyx_poly::lenses::default_panel;
use calyx_poly::{
    ClobBookStatus, ClobOrderBook, DataApiTradeRecord, DataApiTradeSide, GammaJoinKey,
    GammaMarketRecord, GammaOutcomeShape, HolderShare, PendingForecastRegister, PublicBookLevel,
    Resolution,
};
use serde_json::{Value, json};

use support::{collect_files, named_fsv_root, reset_dir, write_blake3sums, write_json};

const VAULT_ID: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
const VAULT_SALT: &[u8] = b"poly-issue238-crypto-capture";
const CAPTURE_TS: u64 = 1_785_500_238;
const CONDITION: &str = "0x0000000000000000000000000000000000000000000000000000000000000238";

#[test]
fn issue238_crypto_capture_harness_known_truth_fsv() {
    let (root, _keep) = named_fsv_root("POLY_ISSUE238_FSV_ROOT", "issue238-crypto-capture");
    assert_c_drive(&root);
    reset_dir(&root);
    let vault_id: VaultId = VAULT_ID.parse().unwrap();
    let vault = AsterVault::new_durable(
        root.join("capture-vault"),
        vault_id,
        VAULT_SALT.to_vec(),
        VaultOptions::default(),
    )
    .unwrap();
    let mut register = PendingForecastRegister::default();
    let config = CryptoCaptureHarnessConfig {
        interval_secs: 60,
        ingestor_config: CryptoIngestorConfig {
            panel_version: 238,
            capture_ws: false,
            ..CryptoIngestorConfig::default()
        },
    };

    let mut runner = KnownRunner::active();
    let captured = run_crypto_capture_harness_once(
        &vault,
        &mut register,
        harness_request(&root, vault_id, config.clone(), CAPTURE_TS),
        &mut runner,
    )
    .expect("scheduled capture");
    assert_eq!(
        captured.report.decision,
        CryptoCaptureDecisionKind::Captured
    );
    assert_eq!(captured.state.captures.len(), 1);
    assert_eq!(captured.state.captures[0].snapshots.len(), 2);
    assert_eq!(runner.calls, 1);
    let state_after_capture = fs::read(&captured.state_path).unwrap();
    let state_readback = read_crypto_capture_state(&captured.state_path).unwrap();
    assert_eq!(state_readback, captured.state);

    let duplicate = run_crypto_capture_harness_once(
        &vault,
        &mut register,
        harness_request(&root, vault_id, config.clone(), CAPTURE_TS),
        &mut runner,
    )
    .expect("duplicate interval skip");
    assert_eq!(
        duplicate.report.decision,
        CryptoCaptureDecisionKind::SkippedDuplicateInterval
    );
    assert_eq!(runner.calls, 1, "duplicate interval must not fetch/capture");
    assert_eq!(
        fs::read(&duplicate.state_path).unwrap(),
        state_after_capture
    );

    let mut restarted_register = PendingForecastRegister::default();
    let resolved = resolution(CAPTURE_TS + 120);
    let matured =
        join_crypto_capture_resolution(&vault, &mut restarted_register, &root, &resolved, false)
            .expect("mature pending pairs after restart");
    assert_eq!(matured.record.pairs.len(), 2);
    assert!(matured.record.pairs.iter().any(|pair| pair.actual_win));
    assert!(matured.record.pairs.iter().any(|pair| !pair.actual_win));
    let corpus: Vec<Value> =
        serde_json::from_slice(&fs::read(&matured.corpus_path).unwrap()).expect("decode corpus");
    assert_eq!(corpus.len(), 2);
    let join_payload = ledger_payload(&vault, matured.join.ledger_seq.unwrap());
    assert_eq!(
        join_payload["event"],
        json!("poly.pending_forecast_resolution_join")
    );
    assert_eq!(join_payload["work_items"].as_array().unwrap().len(), 2);

    let mut second_runner = KnownRunner::active();
    run_crypto_capture_harness_once(
        &vault,
        &mut restarted_register,
        harness_request(&root, vault_id, config.clone(), CAPTURE_TS + 180),
        &mut second_runner,
    )
    .expect("second due capture for lookahead edge");
    let lookahead = join_crypto_capture_resolution(
        &vault,
        &mut restarted_register,
        &root,
        &resolution(CAPTURE_TS + 180),
        false,
    )
    .expect_err("resolution at capture time must fail closed");
    assert_eq!(lookahead.code(), ERR_CRYPTO_CAPTURE_LOOKAHEAD);

    let mut terminal_runner = KnownRunner::terminal();
    let terminal = run_crypto_capture_harness_once(
        &vault,
        &mut restarted_register,
        harness_request(&root, vault_id, config.clone(), CAPTURE_TS + 240),
        &mut terminal_runner,
    )
    .expect_err("terminal market cannot be captured as pre-resolution");
    assert_eq!(terminal.code(), ERR_CRYPTO_INGESTOR_NOT_PRE_RESOLUTION);

    vault.flush().unwrap();
    let final_state = read_crypto_capture_state(&root.join("crypto-capture-harness-state.json"))
        .expect("read final state");
    assert_eq!(final_state.captures.len(), 2);
    assert_eq!(final_state.matured_resolutions.len(), 1);
    let first_registered = final_state.captures[0].snapshots[0]
        .pending_entry
        .registered_ledger_seq
        .unwrap();
    assert_eq!(
        ledger_payload(&vault, first_registered)["event"],
        json!("poly.pending_forecast_registered")
    );

    let mut files = Vec::new();
    collect_files(&root, &mut files);
    let report = json!({
        "issue": 238,
        "schema_version": CRYPTO_CAPTURE_HARNESS_SCHEMA_VERSION,
        "ingestor_schema_version": CRYPTO_INGESTOR_SCHEMA_VERSION,
        "proof_claim": "The crypto capture harness captures one scheduled active binary crypto market, writes and reads back harness state, skips duplicate interval capture before source fetch, hydrates pending entries after restart, emits matured pre-resolution pairs only after valid settlement timing, and fails closed on required edges.",
        "minimum_sufficient_proof_corpus": {
            "known_truth_markets": 1,
            "outcome_snapshots_per_capture": 2,
            "scheduled_due_evaluations": 2,
            "resolution_joins": 1,
            "edge_cases": 3,
            "why_this_is_sufficient": "One binary crypto market with two outcome tokens is the smallest corpus that proves #38 token capture, harness schedule state, restart hydration, and pending->resolved pair emission. Duplicate, terminal, and look-ahead edges cover the #238 fail-closed invariants.",
            "why_smaller_is_insufficient": "One token would not prove binary outcome-pair corpus emission; one schedule evaluation would not prove duplicate/resume behavior; no resolution join would not prove matured pair output.",
            "why_larger_is_wasteful": "More markets or longer loops repeat the same state, vault, pending-register, and corpus readback paths without proving a new harness invariant."
        },
        "source_of_truth": "durable AsterVault Base/Ledger rows plus crypto-capture-harness-state.json and crypto-pre-resolution-corpus.json read back from disk",
        "capture_report": captured.report,
        "duplicate_report": duplicate.report,
        "matured_record": matured.record,
        "final_state": final_state,
        "edges": {
            "duplicate_interval_fetch_count_after_two_runs": runner.calls,
            "terminal_market_error": terminal.code(),
            "lookahead_resolution_error": lookahead.code()
        },
        "physical_files": files,
        "passed": true
    });
    let report_path = root.join("issue238_crypto_capture_harness_fsv_report.json");
    write_json(&report_path, &report);
    let readback: Value = serde_json::from_slice(&fs::read(&report_path).unwrap()).unwrap();
    assert_eq!(readback, report);
    write_blake3sums(&root);
}

#[test]
#[ignore = "requires live public Gamma/CLOB/Data API"]
fn issue238_live_crypto_capture_harness_cycle_fsv() {
    let (root, _keep) = named_fsv_root(
        "POLY_ISSUE238_LIVE_FSV_ROOT",
        "issue238-live-crypto-capture",
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
    let now_ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let mut register = PendingForecastRegister::default();
    let config = CryptoCaptureHarnessConfig {
        interval_secs: 60,
        ingestor_config: CryptoIngestorConfig {
            market_limit: 500,
            holder_limit: 25,
            trade_limit: 10,
            panel_version: 238,
            capture_ws: false,
            ..CryptoIngestorConfig::default()
        },
    };
    let run = run_crypto_capture_harness_once(
        &vault,
        &mut register,
        harness_request(&root, vault_id, config, now_ts),
        &mut LiveCryptoCaptureRunner,
    )
    .expect("live harness capture");
    vault.flush().unwrap();
    assert_eq!(run.report.decision, CryptoCaptureDecisionKind::Captured);
    assert!(run.state.captures[0].token_count >= 1);
    let state = read_crypto_capture_state(&run.state_path).expect("live state readback");
    assert_eq!(state, run.state);
    let report = json!({
        "issue": 238,
        "schema_version": CRYPTO_CAPTURE_HARNESS_SCHEMA_VERSION,
        "proof_claim": "The live crypto capture harness can take one real public/read-only active crypto market through scheduled capture, vault writes, pending registration, and harness state readback.",
        "minimum_sufficient_proof_corpus": {
            "live_due_passes": 1,
            "selected_market_count": 1,
            "why_this_is_sufficient": "One live due pass proves the harness composes the #38 real public Gamma/CLOB/Data source path and persists scheduler state after vault readback.",
            "why_smaller_is_insufficient": "Zero live passes would not prove public-source capture through the harness.",
            "why_larger_is_wasteful": "More live markets or repeated cycles would repeat the same public-source and state readback path without adding correctness proof for this slice."
        },
        "source_of_truth": "AsterVault rows and crypto-capture-harness-state.json under the live FSV root",
        "state": state,
        "report": run.report,
        "passed": true
    });
    write_json(
        &root.join("issue238_live_crypto_capture_harness_fsv_report.json"),
        &report,
    );
    write_blake3sums(&root);
}

fn harness_request<'a>(
    root: &'a Path,
    vault_id: VaultId,
    config: CryptoCaptureHarnessConfig,
    now_ts: u64,
) -> CryptoCaptureHarnessRequest<'a> {
    CryptoCaptureHarnessRequest {
        vault_id,
        vault_salt: VAULT_SALT,
        output_root: root,
        config,
        now_ts,
    }
}

struct KnownRunner {
    terminal: bool,
    calls: usize,
}

impl KnownRunner {
    fn active() -> Self {
        Self {
            terminal: false,
            calls: 0,
        }
    }

    fn terminal() -> Self {
        Self {
            terminal: true,
            calls: 0,
        }
    }
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
        let mut inputs = known_inputs(config.captured_ts);
        if self.terminal {
            inputs.market.closed = true;
            inputs.market.end_ts = Some(config.captured_ts);
        }
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
            market_id: "m238".to_string(),
            condition_id: CONDITION.to_string(),
            slug: Some("issue238-btc-known-truth".to_string()),
            question: Some("Will BTC close above the known threshold?".to_string()),
            event_id: Some("evt238".to_string()),
            event_slug: Some("evt238-slug".to_string()),
            active: true,
            closed: false,
            neg_risk: false,
            enable_order_book: Some(true),
            outcomes: vec!["Yes".to_string(), "No".to_string()],
            outcome_prices: vec![0.64, 0.36],
            clob_token_ids: vec!["tok-yes-238".to_string(), "tok-no-238".to_string()],
            outcome_shape: GammaOutcomeShape::Binary,
            category: Some("crypto".to_string()),
            resolution_source: Some("uma".to_string()),
            volume_24h: Some(11_000.0),
            liquidity: Some(27_000.0),
            best_bid: Some(0.63),
            best_ask: Some(0.65),
            spread: Some(0.02),
            last_trade_price: Some(0.64),
            end_ts: Some(captured_ts + 86_400),
            join_key: GammaJoinKey {
                market_id: "m238".to_string(),
                condition_id: CONDITION.to_string(),
                token_ids: vec!["tok-yes-238".to_string(), "tok-no-238".to_string()],
                event_id: Some("evt238".to_string()),
            },
        },
        books: vec![
            book("tok-yes-238", 0.63, 0.65),
            book("tok-no-238", 0.35, 0.37),
        ],
        holders: vec![holder("0xholdera", 60.0, 0), holder("0xholderb", 40.0, 1)],
        trades: vec![trade(
            "0xwalleta",
            DataApiTradeSide::Buy,
            "tok-yes-238",
            100.0,
            0.64,
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
            size: 100.0,
        }],
        asks: vec![PublicBookLevel {
            price: ask,
            size: 90.0,
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

fn resolution(resolved_ts: u64) -> Resolution {
    Resolution {
        condition_id: CONDITION.to_string(),
        winning_outcome_index: 0,
        winning_label: "Yes".to_string(),
        resolved_ts,
        source: "uma".to_string(),
        disputed: false,
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
    support::assert_host_fsv_root(path, "FSV root");
}
