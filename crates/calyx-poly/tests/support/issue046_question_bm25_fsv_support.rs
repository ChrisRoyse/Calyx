use calyx_core::CxId;
use calyx_poly::model::{Book, MarketSnapshot, OracleRiskEvidence};

pub(super) fn snapshot(slug: &str, question: &str, tags: &[&str]) -> MarketSnapshot {
    MarketSnapshot {
        token_id: format!("tok-{slug}"),
        condition_id: format!("cond-{slug}"),
        outcome_index: 0,
        slug: slug.to_string(),
        question: Some(question.to_string()),
        event_id: Some(format!("event-{slug}")),
        category: Some("crypto".to_string()),
        region: Some("global".to_string()),
        tags: tags.iter().map(|tag| (*tag).to_string()).collect(),
        resolution_source: Some("uma".to_string()),
        neg_risk: false,
        snapshot_ts: 1_785_500_046,
        price: Some(0.50),
        mid: Some(0.50),
        best_bid: Some(0.49),
        best_ask: Some(0.51),
        spread: Some(0.02),
        tick_size: Some(0.01),
        volume_24h: Some(10_000.0),
        liquidity: Some(5_000.0),
        one_hour_change: Some(0.0),
        one_day_change: Some(0.0),
        ofi: Some(0.0),
        yes_no_residual: Some(0.0),
        secs_to_resolution: Some(86_400.0),
        holders: vec![],
        makers: vec![],
        counterparty_volumes: vec![],
        onchain_fills: vec![],
        temporal_reference_ts: Some(1_785_500_046),
        sequence_position: Some(1),
        sequence_total: Some(3),
        oracle_risk: OracleRiskEvidence::default(),
        book: Book::default(),
    }
}

pub(super) fn cx_for(value: &str) -> CxId {
    let mut out = [0_u8; 16];
    out.copy_from_slice(&blake3::hash(value.as_bytes()).as_bytes()[..16]);
    CxId::from_bytes(out)
}
