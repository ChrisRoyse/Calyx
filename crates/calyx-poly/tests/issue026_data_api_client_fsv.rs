//! Issue #26 - Data API typed client FSV.

// calyx-shared-module: path=fsv_support.rs alias=__calyx_shared_fsv_support_rs local=support visibility=private
use crate::__calyx_shared_fsv_support_rs as support;

use std::fs;
use std::path::Path;

use calyx_poly::{
    DataApiActivityPage, DataApiBoundedWindowPage, DataApiClient, DataApiClientConfig,
    DataApiEvidenceStatus, DataApiHoldersPage, DataApiJsonPage, DataApiMarketPositionsPage,
    DataApiOpenInterestPage, DataApiPositionsPage, DataApiTradesPage, ERR_DATA_API_BOUNDED_WINDOW,
    ERR_DATA_API_MAKER_EVIDENCE_UNAVAILABLE, ERR_DATA_API_ROW_INVALID, GammaClient,
    GammaClientConfig, GammaMarketRecord, GammaMarketsRequest, build_data_api_concentration_inputs,
    parse_data_api_activity_value, parse_data_api_holders_value,
    parse_data_api_market_positions_value, parse_data_api_open_interest_value,
    parse_data_api_positions_value, parse_data_api_trades_value,
};
use serde_json::{Value, json};

use support::{collect_files, named_fsv_root, reset_dir, write_blake3sums, write_json};

#[test]
fn issue026_data_api_parser_known_truth_edges() {
    let (holder_groups, holder_status) =
        parse_data_api_holders_value(&holders_fixture()).expect("holders parse");
    assert_eq!(holder_status, DataApiEvidenceStatus::Ready);
    assert_eq!(holder_groups[0].holders[0].amount, 12.5);

    let (empty_holders, empty_status) =
        parse_data_api_holders_value(&Value::Null).expect("null holders is absent");
    assert!(empty_holders.is_empty());
    assert_eq!(empty_status, DataApiEvidenceStatus::Absent);

    let trades = parse_data_api_trades_value(&trades_fixture()).expect("trades parse");
    let trade_page = DataApiTradesPage {
        http: dummy_page(),
        trades,
        bounded_window: true,
    };
    let volumes = trade_page.counterparty_volumes();
    assert_eq!(volumes.len(), 1);
    assert_eq!(
        volumes[0].counterparty,
        "0x1111111111111111111111111111111111111111"
    );
    assert_eq!(volumes[0].volume, 7.5);

    let malformed = parse_data_api_trades_value(&json!([{
        "proxyWallet": "0x1111111111111111111111111111111111111111",
        "side": "HOLD",
        "asset": "tok",
        "conditionId": "0xabc",
        "size": 1,
        "price": 0.5,
        "timestamp": 1,
        "outcomeIndex": 0
    }]))
    .expect_err("malformed trade side fails closed");
    assert_eq!(malformed.code(), ERR_DATA_API_ROW_INVALID);

    let client = DataApiClient::new(DataApiClientConfig::default()).expect("client");
    let maker = client
        .require_true_maker_evidence()
        .expect_err("Data API cannot fabricate maker evidence");
    assert_eq!(maker.code(), ERR_DATA_API_MAKER_EVIDENCE_UNAVAILABLE);
}

#[test]
#[ignore = "requires live public Gamma and Data APIs"]
fn issue026_data_api_client_live_fsv() {
    let (root, _keep) = named_fsv_root("POLY_ISSUE026_FSV_ROOT", "issue026-data-api-client");
    reset_dir(&root);
    let gamma = GammaClient::new(GammaClientConfig::default()).expect("Gamma client");
    let data = DataApiClient::new(DataApiClientConfig::default()).expect("Data API client");
    let (market, trades) = select_live_market_with_trades(&gamma, &data);
    let wallet = trades.trades[0].proxy_wallet.clone();

    let holders = data
        .fetch_holders(&market.condition_id, 5)
        .expect("fetch holders");
    let market_positions = data
        .fetch_market_positions(&market.condition_id, 5)
        .expect("fetch market positions");
    let oi = data
        .fetch_open_interest(&market.condition_id)
        .expect("fetch open interest");
    let user_positions = data.fetch_positions(&wallet, 5).expect("fetch positions");
    let user_activity = data.fetch_activity(&wallet, 5).expect("fetch activity");
    let user_trades = data
        .fetch_trades_by_user(&wallet, 5, 0)
        .expect("fetch user trades");
    let offset_cap = data
        .probe_trades_offset_cap()
        .expect("probe trades offset cap");
    assert!(offset_cap.bounded);
    assert_eq!(offset_cap.http.status_code, 400);
    let offset_guard = data
        .fetch_trades_by_market(&market.condition_id, 1, 10_000)
        .expect_err("client refuses complete-history trade window");
    assert_eq!(offset_guard.code(), ERR_DATA_API_BOUNDED_WINDOW);

    let holder_shares = holders.holder_shares();
    let counterparty_volumes = trades.counterparty_volumes();
    assert!(!holder_shares.is_empty());
    assert!(!counterparty_volumes.is_empty());
    let concentration = build_data_api_concentration_inputs(
        market.condition_id.clone(),
        holder_shares,
        counterparty_volumes,
    );
    assert_eq!(
        concentration.maker_evidence_status,
        DataApiEvidenceStatus::Absent
    );
    assert!(concentration.maker_shares.is_empty());

    let persisted = json!({
        "trades_by_market": persist_trades(&root, "trades-by-market", &trades),
        "holders": persist_holders(&root, "holders-by-market", &holders),
        "market_positions": persist_market_positions(&root, "market-positions", &market_positions),
        "open_interest": persist_oi(&root, "open-interest", &oi),
        "positions_by_user": persist_positions(&root, "positions-by-user", &user_positions),
        "activity_by_user": persist_activity(&root, "activity-by-user", &user_activity),
        "trades_by_user": persist_trades(&root, "trades-by-user", &user_trades),
        "offset_cap": persist_bounded_window(&root, "trades-offset-cap", &offset_cap)
    });
    let edge_cases = json!({
        "empty_holders_null": {
            "status": format!("{:?}", parse_data_api_holders_value(&Value::Null).expect("null holders").1)
        },
        "malformed_row": {
            "code": parse_data_api_trades_value(&json!([{"side": "HOLD"}])).expect_err("bad row").code()
        },
        "offset_cap_guard": {
            "code": offset_guard.code(),
            "live_status": offset_cap.http.status_code
        },
        "maker_evidence_unavailable": {
            "code": data.require_true_maker_evidence().expect_err("maker unavailable").code()
        }
    });

    let mut files = Vec::new();
    collect_files(&root, &mut files);
    let report = json!({
        "issue": 26,
        "proof_claim": "The read-only Data API client fetches trades, holders, market positions, user positions, user activity, and open interest for one live Gamma-derived crypto condition and one observed public wallet; persists exact raw body bytes; reads them back; parses holder/counterparty projections; and fails closed on bounded trade-history and unavailable maker evidence.",
        "selected_market": {
            "market_id": market.market_id,
            "condition_id": market.condition_id,
            "event_id": market.event_id,
            "wallet": wallet
        },
        "minimum_sufficient_proof_corpus": {
            "live_conditions": 1,
            "live_wallets": 1,
            "live_endpoint_cases": 8,
            "synthetic_edge_cases": 4,
            "why_this_is_sufficient": "One live condition proves market-scoped trades, holders, market positions, and OI. One wallet observed from that market's trades proves user-scoped positions, activity, and user trades without expanding the corpus. Synthetic edges prove absent/null holders, malformed fail-closed rows, bounded offset refusal, and maker-evidence unavailability.",
            "why_smaller_is_insufficient": "Without a live condition, market-scoped Data API wire shapes and condition joins are unproven. Without a live wallet, user-scoped positions/activity are unproven. Without synthetic edges, absent and malformed states are not guaranteed by the live API.",
            "why_larger_is_wasteful": "More conditions or wallets would repeat the same Data API client, parser, raw readback, and projection paths. Larger trade windows or all-history captures belong to #27/#198 on-chain backfill, not #26 client correctness."
        },
        "source_of_truth": "live public Data API HTTP response bodies persisted under this FSV root and parsed only after disk readback",
        "persisted_readbacks": persisted,
        "concentration_summary": {
            "holder_count": concentration.holder_shares.len(),
            "counterparty_count": concentration.counterparty_volumes.len(),
            "maker_evidence_status": format!("{:?}", concentration.maker_evidence_status),
            "maker_evidence_reason": concentration.maker_evidence_reason
        },
        "edge_cases": edge_cases,
        "physical_files": files,
        "passed": true
    });
    let report_path = root.join("issue026_data_api_client_fsv_report.json");
    write_json(&report_path, &report);
    let readback: Value =
        serde_json::from_slice(&fs::read(&report_path).expect("read report")).expect("decode");
    assert_eq!(readback, report);
    write_blake3sums(&root);
    println!("ISSUE026_DATA_API_CLIENT_FSV={}", report_path.display());
}

fn select_live_market_with_trades(
    gamma: &GammaClient,
    data: &DataApiClient,
) -> (GammaMarketRecord, DataApiTradesPage) {
    let page = gamma
        .fetch_markets(&GammaMarketsRequest::crypto_active(10))
        .expect("fetch active crypto Gamma markets");
    for market in page.markets {
        if let Ok(trades) = data.fetch_trades_by_market(&market.condition_id, 5, 0)
            && !trades.trades.is_empty()
        {
            return (market, trades);
        }
    }
    panic!("no active crypto Gamma condition produced Data API trades");
}

fn persist_trades(root: &Path, name: &str, page: &DataApiTradesPage) -> Value {
    persist_case(root, name, &page.http, |value| {
        Ok(serde_json::to_value(parse_data_api_trades_value(value)?).expect("trades JSON"))
    })
}

fn persist_holders(root: &Path, name: &str, page: &DataApiHoldersPage) -> Value {
    persist_case(root, name, &page.http, |value| {
        let (groups, status) = parse_data_api_holders_value(value)?;
        Ok(json!({"status": format!("{:?}", status), "groups": groups}))
    })
}

fn persist_market_positions(root: &Path, name: &str, page: &DataApiMarketPositionsPage) -> Value {
    persist_case(root, name, &page.http, |value| {
        let (groups, status) = parse_data_api_market_positions_value(value)?;
        Ok(json!({"status": format!("{:?}", status), "groups": groups}))
    })
}

fn persist_positions(root: &Path, name: &str, page: &DataApiPositionsPage) -> Value {
    persist_case(root, name, &page.http, |value| {
        let (positions, status) = parse_data_api_positions_value(value)?;
        Ok(json!({"status": format!("{:?}", status), "positions": positions}))
    })
}

fn persist_activity(root: &Path, name: &str, page: &DataApiActivityPage) -> Value {
    persist_case(root, name, &page.http, |value| {
        let (activity, status) = parse_data_api_activity_value(value)?;
        Ok(json!({"status": format!("{:?}", status), "activity": activity}))
    })
}

fn persist_oi(root: &Path, name: &str, page: &DataApiOpenInterestPage) -> Value {
    persist_case(root, name, &page.http, |value| {
        Ok(serde_json::to_value(parse_data_api_open_interest_value(value)?).expect("OI JSON"))
    })
}

fn persist_bounded_window(root: &Path, name: &str, page: &DataApiBoundedWindowPage) -> Value {
    persist_case(root, name, &page.http, |value| {
        Ok(json!({
            "status_code": page.http.status_code,
            "bounded": page.bounded,
            "reason": page.reason,
            "body_shape": if value.is_null() { "null" } else { "json" }
        }))
    })
}

fn persist_case<F>(root: &Path, name: &str, page: &DataApiJsonPage, parse: F) -> Value
where
    F: FnOnce(&Value) -> calyx_poly::Result<Value>,
{
    let case_root = root.join(name);
    fs::create_dir_all(&case_root).expect("create case root");
    let body_path = case_root.join("body.json");
    let parsed_path = case_root.join("parsed.json");
    let summary_path = case_root.join("summary.json");
    fs::write(&body_path, &page.raw_body).expect("write raw body");
    let raw_readback = fs::read(&body_path).expect("read raw body");
    assert_eq!(raw_readback, page.raw_body, "raw body readback is exact");
    let value: Value = if raw_readback.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&raw_readback).expect("decode raw readback")
    };
    let parsed = parse(&value).expect("parse raw readback");
    write_json(&parsed_path, &parsed);
    let parsed_readback: Value =
        serde_json::from_slice(&fs::read(&parsed_path).expect("read parsed")).expect("decode");
    assert_eq!(parsed_readback, parsed);
    let summary = json!({
        "method": page.method,
        "url": page.url,
        "status_code": page.status_code,
        "body_path": body_path.display().to_string(),
        "body_bytes": page.body_bytes,
        "body_sha256": page.body_sha256,
        "parsed_path": parsed_path.display().to_string(),
        "readback_equal": true
    });
    write_json(&summary_path, &summary);
    let summary_readback: Value =
        serde_json::from_slice(&fs::read(&summary_path).expect("read summary")).expect("decode");
    assert_eq!(summary_readback, summary);
    summary_readback
}

fn dummy_page() -> DataApiJsonPage {
    DataApiJsonPage {
        method: "GET".to_string(),
        url: "https://data-api.polymarket.com/trades?limit=2".to_string(),
        status_code: 200,
        body_bytes: 2,
        body_sha256: "[]".to_string(),
        raw_body: Vec::new(),
        value: Value::Null,
    }
}

fn holders_fixture() -> Value {
    json!([{
        "token": "tok_yes",
        "holders": [{
            "proxyWallet": "0x1111111111111111111111111111111111111111",
            "asset": "tok_yes",
            "amount": "12.5",
            "outcomeIndex": 0
        }]
    }])
}

fn trades_fixture() -> Value {
    json!([
        {
            "proxyWallet": "0x1111111111111111111111111111111111111111",
            "side": "BUY",
            "asset": "tok_yes",
            "conditionId": "0xcondition",
            "size": "10",
            "price": "0.5",
            "timestamp": 1783400000,
            "outcomeIndex": 0
        },
        {
            "proxyWallet": "0x1111111111111111111111111111111111111111",
            "side": "SELL",
            "asset": "tok_yes",
            "conditionId": "0xcondition",
            "size": "5",
            "price": "0.5",
            "timestamp": 1783400001,
            "outcomeIndex": 0
        }
    ])
}
