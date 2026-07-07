use calyx_core::{SlotId, SlotVector};
use calyx_poly::QUESTION_BM25_DIM;
use calyx_poly::lenses::default_panel;
use calyx_poly::model::MarketSnapshot;

fn sample() -> MarketSnapshot {
    MarketSnapshot {
        token_id: "tok".into(),
        condition_id: "cond".into(),
        outcome_index: 0,
        slug: "will-x".into(),
        question: Some("Will X happen?".into()),
        event_id: None,
        category: Some("crypto".into()),
        region: None,
        tags: vec![],
        resolution_source: None,
        neg_risk: false,
        snapshot_ts: 1_785_500_000,
        price: Some(0.62),
        mid: Some(0.62),
        best_bid: Some(0.61),
        best_ask: Some(0.63),
        spread: Some(0.02),
        tick_size: Some(0.01),
        volume_24h: Some(125_000.0),
        liquidity: Some(40_000.0),
        one_hour_change: Some(0.01),
        one_day_change: Some(-0.03),
        ofi: Some(0.2),
        yes_no_residual: Some(0.0),
        secs_to_resolution: Some(86_400.0),
        holders: vec![],
        makers: vec![],
        counterparty_volumes: vec![],
        onchain_fills: vec![],
        temporal_reference_ts: None,
        sequence_position: None,
        sequence_total: None,
        oracle_risk: Default::default(),
        book: Default::default(),
    }
}

#[test]
fn panel_measures_all_slots() {
    let panel = default_panel(1, vec!["us".into(), "eu".into()]);
    let slots = panel.measure_all(&sample());
    assert_eq!(slots.len(), panel.lenses.len());
    let price = slots.get(&SlotId::new(0)).unwrap();
    assert!(matches!(price, SlotVector::Dense { .. }));
}

#[test]
fn missing_field_is_absent_not_zero() {
    let panel = default_panel(1, vec![]);
    let mut s = sample();
    s.ofi = None;
    let slots = panel.measure_all(&s);
    assert!(matches!(
        slots.get(&SlotId::new(5)).unwrap(),
        SlotVector::Absent { .. }
    ));
}

#[test]
fn category_one_hot_selects() {
    let panel = default_panel(1, vec![]);
    let slots = panel.measure_all(&sample());
    if let SlotVector::Dense { data, .. } = slots.get(&SlotId::new(8)).unwrap() {
        assert!((data.iter().sum::<f32>() - 1.0).abs() < 1.0e-6);
    } else {
        panic!("category slot should be dense");
    }
}

#[test]
fn question_bm25_slot_is_sparse() {
    let panel = default_panel(1, vec![]);
    let slots = panel.measure_all(&sample());
    if let SlotVector::Sparse { dim, entries } = slots.get(&SlotId::new(16)).unwrap() {
        assert_eq!(*dim, QUESTION_BM25_DIM);
        assert!(!entries.is_empty());
    } else {
        panic!("question BM25 slot should be sparse");
    }
}
