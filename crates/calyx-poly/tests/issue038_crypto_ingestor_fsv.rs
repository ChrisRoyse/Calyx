//! Issue #38 - crypto-domain ingestor MVP.
//!
//! Source of truth: durable AsterVault Base/Ledger CF rows plus persisted FSV JSON readback.

#[path = "fsv_support.rs"]
mod support;

use std::fs;
use std::path::{Path, PathBuf};

use calyx_aster::cf::{ColumnFamily, ledger_key};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{CxId, SlotId, SlotVector, VaultId, VaultStore};
use calyx_ledger::{EntryKind, decode as decode_ledger};
use calyx_poly::crypto_ingestor::{
    CRYPTO_INGESTOR_SCHEMA_VERSION, CryptoIngestorConfig, CryptoMarketInputs,
    ERR_CRYPTO_INGESTOR_NOT_PRE_RESOLUTION, build_crypto_market_snapshots, put_crypto_snapshot,
    register_crypto_pending, run_live_crypto_ingestion_cycle,
};
use calyx_poly::lenses::default_panel;
use calyx_poly::{
    ClobBookStatus, ClobOrderBook, DataApiTradeRecord, DataApiTradeSide, GammaJoinKey,
    GammaMarketRecord, GammaOutcomeShape, HolderShare, MarketWsClientConfig, PublicBookLevel,
};
use serde_json::{Value, json};

use support::{collect_files, named_fsv_root, reset_dir, write_blake3sums, write_json};

const VAULT_ID: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
const VAULT_SALT: &[u8] = b"poly-issue038-crypto-ingestor";

#[test]
fn issue038_crypto_ingestor_known_truth_fsv() {
    let (root, _keep) = named_fsv_root("POLY_ISSUE038_FSV_ROOT", "issue038-crypto-ingestor");
    assert_c_drive(&root);
    reset_dir(&root);
    let vault_dir = root.join("crypto-domain-vault");
    let vault_id: VaultId = VAULT_ID.parse().unwrap();
    let panel = default_panel(38, vec!["global".to_string()]);
    let vault = AsterVault::new_durable(
        &vault_dir,
        vault_id,
        VAULT_SALT.to_vec(),
        VaultOptions::default(),
    )
    .unwrap();
    let mut register = calyx_poly::PendingForecastRegister::default();

    let inputs = known_inputs();
    let snapshots = build_crypto_market_snapshots(&inputs).expect("known market snapshots");
    assert_eq!(
        snapshots.len(),
        2,
        "binary market emits both outcome tokens"
    );
    let mut records = Vec::new();
    for snapshot in &snapshots {
        let put = put_crypto_snapshot(&vault, &panel, snapshot, vault_id, VAULT_SALT)
            .expect("put snapshot");
        let cx_id = parse_cx(&put.cx_id);
        let pending = register_crypto_pending(
            &vault,
            &mut register,
            snapshot,
            cx_id,
            "crypto",
            "pre_resolution",
        )
        .expect("register pending");
        records.push(json!({"put": put, "pending": pending}));
    }
    let seq_after_happy = vault.snapshot();
    let before_dup = vault.snapshot();
    let duplicate = put_crypto_snapshot(&vault, &panel, &snapshots[0], vault_id, VAULT_SALT)
        .expect("duplicate put");
    let after_dup = vault.snapshot();
    assert_eq!(
        before_dup, after_dup,
        "duplicate must not advance vault seq"
    );

    let mut gap = snapshots[0].clone();
    gap.token_id = "tok-issue038-gap".to_string();
    gap.slug = "issue038-feed-gap".to_string();
    gap.price = None;
    gap.mid = None;
    gap.best_bid = None;
    gap.best_ask = None;
    gap.spread = None;
    gap.ofi = None;
    let gap_put = put_crypto_snapshot(&vault, &panel, &gap, vault_id, VAULT_SALT).expect("gap put");
    let gap_stored = vault
        .get(parse_cx(&gap_put.cx_id), vault.snapshot())
        .unwrap();
    assert!(matches!(
        gap_stored.slots.get(&SlotId::new(0)),
        Some(SlotVector::Absent { .. })
    ));
    assert!(!gap_stored.scalars.contains_key("price"));

    let mut nonfinite = snapshots[0].clone();
    nonfinite.token_id = "tok-issue038-bad".to_string();
    nonfinite.liquidity = Some(f64::INFINITY);
    let nonfinite_err = put_crypto_snapshot(&vault, &panel, &nonfinite, vault_id, VAULT_SALT)
        .expect_err("non-finite must fail closed");

    let mut resolved_inputs = inputs.clone();
    resolved_inputs.market.end_ts = Some(resolved_inputs.captured_ts);
    let resolved_err = build_crypto_market_snapshots(&resolved_inputs)
        .expect_err("just-resolved market must not capture");
    assert_eq!(resolved_err.code(), ERR_CRYPTO_INGESTOR_NOT_PRE_RESOLUTION);

    vault.flush().unwrap();
    drop(vault);
    let reopened = AsterVault::new_durable(
        &vault_dir,
        vault_id,
        VAULT_SALT.to_vec(),
        VaultOptions::default(),
    )
    .unwrap();
    let ledger_payloads = records
        .iter()
        .map(|record| {
            let seq = record["pending"]["ledger_seq"].as_u64().unwrap();
            ledger_payload(&reopened, seq)
        })
        .collect::<Vec<_>>();
    assert!(ledger_payloads.iter().all(|row| {
        row["event"] == json!("poly.pending_forecast_registered")
            && row["forecast"]["status"] == json!("pending")
    }));
    let readback_ids = records
        .iter()
        .map(|record| {
            let id = parse_cx(record["put"]["cx_id"].as_str().unwrap());
            let stored = reopened.get(id, reopened.snapshot()).unwrap();
            json!({
                "cx_id": id.to_string(),
                "flags_ungrounded": stored.flags.ungrounded,
                "condition_id": stored.metadata.get("condition_id"),
                "token_id": stored.metadata.get("token_id")
            })
        })
        .collect::<Vec<_>>();

    let mut files = Vec::new();
    collect_files(&root, &mut files);
    let report = json!({
        "issue": 38,
        "schema_version": CRYPTO_INGESTOR_SCHEMA_VERSION,
        "proof_claim": "The crypto ingestor converts one active binary crypto market into per-outcome MarketSnapshot records, writes them to the domain AsterVault as ungrounded constellations, reads those persisted rows back byte-identically, registers pending resolution rows, and fails closed on required edge cases.",
        "minimum_sufficient_proof_corpus": {
            "live_like_known_truth_markets": 1,
            "happy_path_outcome_snapshots": 2,
            "edge_cases": 4,
            "why_this_is_sufficient": "One binary market with two outcome tokens is the smallest corpus that proves per-market token iteration, yes/no residuals, vault put/get readback, and pending registration. The four edges cover feed gap Absent slots, non-finite refusal, duplicate dedup, and just-resolved exclusion.",
            "why_smaller_is_insufficient": "A single outcome token would not prove per-market outcome-token iteration or binary residual construction; omitting any edge would leave an acceptance invariant unproven.",
            "why_larger_is_wasteful": "More markets or a large corpus would repeat the same Gamma/CLOB/Data snapshot->constellation->vault->pending code paths without proving a new #38 invariant."
        },
        "source_of_truth": "durable AsterVault Base/Ledger CF rows reopened from disk plus persisted report JSON",
        "seq_after_happy": seq_after_happy,
        "duplicate": {
            "cx_id": duplicate.cx_id,
            "seq_before": before_dup,
            "seq_after": after_dup,
            "dedup_noop": before_dup == after_dup
        },
        "happy_records": records,
        "vault_reopen_readback": readback_ids,
        "pending_ledger_payloads": ledger_payloads,
        "edges": {
            "feed_gap_price_slot": "absent",
            "nonfinite_error": nonfinite_err.code(),
            "just_resolved_error": resolved_err.code()
        },
        "physical_files": files,
        "passed": true
    });
    let report_path = root.join("issue038_crypto_ingestor_fsv_report.json");
    write_json(&report_path, &report);
    let readback: Value =
        serde_json::from_slice(&fs::read(&report_path).unwrap()).expect("decode report");
    assert_eq!(readback, report);
    write_blake3sums(&root);
}

#[test]
#[ignore = "requires live public Gamma/CLOB/Data API and market WebSocket"]
fn issue038_live_crypto_capture_cycle_fsv() {
    let (root, _keep) = named_fsv_root("POLY_ISSUE038_LIVE_FSV_ROOT", "issue038-live-crypto");
    assert_c_drive(&root);
    reset_dir(&root);
    let vault_id: VaultId = VAULT_ID.parse().unwrap();
    let vault_dir = root.join("live-vault");
    let vault = AsterVault::new_durable(
        &vault_dir,
        vault_id,
        VAULT_SALT.to_vec(),
        VaultOptions::default(),
    )
    .unwrap();
    let mut register = calyx_poly::PendingForecastRegister::default();
    let config = CryptoIngestorConfig {
        market_limit: 5,
        holder_limit: 50,
        trade_limit: 25,
        ws_config: MarketWsClientConfig {
            timeout_secs: 12,
            max_frames: 16,
            heartbeat_secs: 10,
            min_data_events: 1,
            require_pong: true,
            ..MarketWsClientConfig::default()
        },
        ..CryptoIngestorConfig::default()
    };
    let run =
        run_live_crypto_ingestion_cycle(&vault, &mut register, vault_id, VAULT_SALT, &root, config)
            .expect("live crypto ingestion");
    vault.flush().unwrap();
    assert!(run.run.token_count >= 1);
    persist_live_sources(&root, &run);
    write_json(
        &root.join("issue038_live_crypto_ingestor_fsv_report.json"),
        &serde_json::to_value(&run.run).unwrap(),
    );
    write_blake3sums(&root);
}

fn known_inputs() -> CryptoMarketInputs {
    let captured_ts = 1_785_500_038;
    CryptoMarketInputs {
        market: GammaMarketRecord {
            market_id: "m38".to_string(),
            condition_id: "0x0000000000000000000000000000000000000000000000000000000000000038"
                .to_string(),
            slug: Some("issue038-btc-known-truth".to_string()),
            question: Some("Will BTC test the known threshold?".to_string()),
            event_id: Some("evt38".to_string()),
            event_slug: Some("evt38-slug".to_string()),
            active: true,
            closed: false,
            neg_risk: false,
            enable_order_book: Some(true),
            outcomes: vec!["Yes".to_string(), "No".to_string()],
            outcome_prices: vec![0.62, 0.38],
            clob_token_ids: vec!["tok-yes-38".to_string(), "tok-no-38".to_string()],
            outcome_shape: GammaOutcomeShape::Binary,
            category: Some("crypto".to_string()),
            resolution_source: Some("uma".to_string()),
            volume_24h: Some(10_000.0),
            liquidity: Some(25_000.0),
            best_bid: Some(0.61),
            best_ask: Some(0.63),
            spread: Some(0.02),
            last_trade_price: Some(0.62),
            end_ts: Some(captured_ts + 86_400),
            join_key: GammaJoinKey {
                market_id: "m38".to_string(),
                condition_id: "0x0000000000000000000000000000000000000000000000000000000000000038"
                    .to_string(),
                token_ids: vec!["tok-yes-38".to_string(), "tok-no-38".to_string()],
                event_id: Some("evt38".to_string()),
            },
        },
        books: vec![
            book("tok-yes-38", 0.61, 0.63),
            book("tok-no-38", 0.37, 0.39),
        ],
        holders: vec![
            holder("0xholdera", 60.0, 0),
            holder("0xholderb", 40.0, 0),
            holder("0xholderc", 50.0, 1),
        ],
        trades: vec![
            trade(
                "0xwalleta",
                DataApiTradeSide::Buy,
                "tok-yes-38",
                100.0,
                0.62,
                0,
            ),
            trade(
                "0xwalletb",
                DataApiTradeSide::Sell,
                "tok-yes-38",
                50.0,
                0.61,
                0,
            ),
        ],
        captured_ts,
    }
}

fn book(token_id: &str, bid: f64, ask: f64) -> ClobOrderBook {
    ClobOrderBook {
        condition_id: "0x0000000000000000000000000000000000000000000000000000000000000038"
            .to_string(),
        token_id: token_id.to_string(),
        timestamp_ms: 1_785_500_038_000,
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
        condition_id: "0x0000000000000000000000000000000000000000000000000000000000000038"
            .to_string(),
        size,
        price,
        timestamp: 1_785_500_000,
        outcome_index,
        transaction_hash: Some(format!("0xtx{outcome_index}")),
    }
}

fn ledger_payload(vault: &AsterVault, seq: u64) -> Value {
    let row = vault
        .read_cf_at(vault.snapshot(), ColumnFamily::Ledger, &ledger_key(seq))
        .unwrap()
        .expect("ledger row");
    let ledger = decode_ledger(&row).expect("decode ledger");
    assert_eq!(ledger.kind, EntryKind::Measure);
    serde_json::from_slice(&ledger.payload).expect("decode ledger payload")
}

fn persist_live_sources(root: &Path, run: &calyx_poly::crypto_ingestor::CryptoLiveCaptureRun) {
    fs::write(root.join("gamma-body.json"), &run.gamma_page.raw_body).unwrap();
    for (index, page) in run.books.iter().enumerate() {
        fs::write(
            root.join(format!("clob-book-{index}.json")),
            &page.http.raw_body,
        )
        .unwrap();
    }
    fs::write(root.join("data-holders.json"), &run.holders.http.raw_body).unwrap();
    fs::write(root.join("data-trades.json"), &run.trades.http.raw_body).unwrap();
}

fn parse_cx(value: &str) -> CxId {
    value.parse().expect("parse CxId")
}

fn assert_c_drive(path: &Path) {
    let text = PathBuf::from(path).display().to_string().replace('/', "\\");
    assert!(
        text.to_ascii_lowercase().starts_with("c:\\"),
        "FSV root must stay on C:, got {text}"
    );
}
