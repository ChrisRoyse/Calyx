//! Issue #238 - token probability selection for captured crypto snapshots.
//!
//! Source of truth: known CLOB one-sided book shape where `last_trade_price` is not sufficient
//! token-specific evidence for both outcomes.

use calyx_poly::crypto_ingestor::{CryptoMarketInputs, build_crypto_market_snapshots};
use calyx_poly::{
    ClobBookStatus, ClobOrderBook, DataApiTradeRecord, GammaJoinKey, GammaMarketRecord,
    GammaOutcomeShape, PublicBookLevel,
};

const CAPTURE_TS: u64 = 1_783_429_995;

#[test]
fn issue238_snapshot_price_prefers_token_midpoint_then_gamma_outcome_fsv() {
    let snapshots = build_crypto_market_snapshots(&CryptoMarketInputs {
        market: GammaMarketRecord {
            market_id: "2744242".to_string(),
            condition_id: "0x39ea14226927f879d93550628e70f22bc444915cea787c5734c53e89d5cf6d20"
                .to_string(),
            slug: Some("bitcoin-above-48k-on-july-7-2026".to_string()),
            question: Some("Will the price of Bitcoin be above $48,000 on July 7?".to_string()),
            event_id: Some("651841".to_string()),
            event_slug: Some("bitcoin-above-on-july-7-2026".to_string()),
            active: true,
            closed: false,
            neg_risk: false,
            enable_order_book: Some(true),
            outcomes: vec!["Yes".to_string(), "No".to_string()],
            outcome_prices: vec![0.9995, 0.0005],
            clob_token_ids: vec!["yes-token".to_string(), "no-token".to_string()],
            outcome_shape: GammaOutcomeShape::Binary,
            category: Some("crypto".to_string()),
            resolution_source: Some("Binance".to_string()),
            volume_24h: Some(1_000_000.0),
            liquidity: Some(900_000.0),
            best_bid: Some(0.999),
            best_ask: Some(1.0),
            spread: Some(0.001),
            last_trade_price: Some(0.999),
            end_ts: Some(CAPTURE_TS + 10_000),
            join_key: GammaJoinKey {
                market_id: "2744242".to_string(),
                condition_id: "0x39ea14226927f879d93550628e70f22bc444915cea787c5734c53e89d5cf6d20"
                    .to_string(),
                token_ids: vec!["yes-token".to_string(), "no-token".to_string()],
                event_id: Some("651841".to_string()),
            },
        },
        books: vec![
            one_sided_book("yes-token", vec![0.999], vec![], 0.999),
            one_sided_book("no-token", vec![], vec![0.001], 0.999),
        ],
        holders: Vec::new(),
        trades: Vec::<DataApiTradeRecord>::new(),
        captured_ts: CAPTURE_TS,
    })
    .expect("snapshots build");
    assert_eq!(snapshots.len(), 2);
    assert_eq!(snapshots[0].price, Some(0.9995));
    assert_eq!(snapshots[1].price, Some(0.0005));
}

fn one_sided_book(token_id: &str, bids: Vec<f64>, asks: Vec<f64>, last: f64) -> ClobOrderBook {
    ClobOrderBook {
        condition_id: "0x39ea14226927f879d93550628e70f22bc444915cea787c5734c53e89d5cf6d20"
            .to_string(),
        token_id: token_id.to_string(),
        timestamp_ms: CAPTURE_TS * 1000,
        hash: None,
        bids: bids
            .into_iter()
            .map(|price| PublicBookLevel { price, size: 10.0 })
            .collect(),
        asks: asks
            .into_iter()
            .map(|price| PublicBookLevel { price, size: 10.0 })
            .collect(),
        min_order_size: Some(5.0),
        tick_size: Some(0.001),
        neg_risk: Some(false),
        last_trade_price: Some(last),
        best_bid: None,
        best_ask: None,
        midpoint: None,
        spread: None,
        status: ClobBookStatus::ThinOrEmpty,
    }
}
