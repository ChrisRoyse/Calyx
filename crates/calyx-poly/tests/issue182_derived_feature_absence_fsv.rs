use std::fs;
use std::path::PathBuf;

use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{SlotId, SlotVector, VaultId, VaultStore};
use calyx_poly::constellation::build_constellation;
use calyx_poly::features;
use calyx_poly::lenses::default_panel;
use calyx_poly::{Book, MarketSnapshot};
use serde_json::json;

#[test]
fn issue182_derived_feature_absence_fsv() {
    let root = fsv_root();
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let vault_dir = root.join("vault-source-of-truth");
    let panel = default_panel(1, vec!["global".to_string()]);
    let vault_id: VaultId = "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap();
    let vault = AsterVault::new_durable(
        &vault_dir,
        vault_id,
        b"issue182-vault-salt".to_vec(),
        VaultOptions::default(),
    )
    .unwrap();

    let mut absent = snapshot("absent-derived-features");
    absent.best_bid = None;
    absent.best_ask = None;
    absent.spread = features::spread(f64::NAN, f64::NAN);
    absent.ofi = features::order_flow_imbalance(0.0, 0.0);

    let mut measured_zero = snapshot("measured-zero-features");
    measured_zero.best_bid = Some(0.50);
    measured_zero.best_ask = Some(0.50);
    measured_zero.spread = features::spread(0.50, 0.50);
    measured_zero.ofi = features::order_flow_imbalance(100.0, 100.0);

    let absent_cx = build_constellation(&absent, &panel, vault_id, b"issue182-panel-salt").unwrap();
    let measured_cx =
        build_constellation(&measured_zero, &panel, vault_id, b"issue182-panel-salt").unwrap();
    let absent_id = absent_cx.cx_id;
    let measured_id = measured_cx.cx_id;
    vault.put(absent_cx).unwrap();
    vault.put(measured_cx).unwrap();
    vault.flush().unwrap();

    let snapshot_seq = vault.snapshot();
    let stored_absent = vault.get(absent_id, snapshot_seq).unwrap();
    let stored_measured = vault.get(measured_id, snapshot_seq).unwrap();

    assert!(!stored_absent.scalars.contains_key("spread"));
    assert!(!stored_absent.scalars.contains_key("ofi"));
    assert_eq!(stored_measured.scalars.get("spread"), Some(&0.0));
    assert_eq!(stored_measured.scalars.get("ofi"), Some(&0.0));
    assert!(matches!(
        stored_absent.slots.get(&SlotId::new(2)),
        Some(SlotVector::Absent { .. })
    ));
    assert!(matches!(
        stored_absent.slots.get(&SlotId::new(5)),
        Some(SlotVector::Absent { .. })
    ));
    assert!(matches!(
        stored_measured.slots.get(&SlotId::new(2)),
        Some(SlotVector::Dense { .. })
    ));
    assert!(matches!(
        stored_measured.slots.get(&SlotId::new(5)),
        Some(SlotVector::Dense { .. })
    ));

    let readback = json!({
        "source_of_truth": "durable AsterVault Constellation scalars and hydrated Slots readback",
        "snapshot_seq": snapshot_seq,
        "absent": {
            "cx_id": absent_id.to_string(),
            "spread_helper": absent.spread,
            "ofi_helper": absent.ofi,
            "spread_scalar_present": stored_absent.scalars.contains_key("spread"),
            "ofi_scalar_present": stored_absent.scalars.contains_key("ofi"),
            "spread_slot": slot_kind(stored_absent.slots.get(&SlotId::new(2)).unwrap()),
            "ofi_slot": slot_kind(stored_absent.slots.get(&SlotId::new(5)).unwrap()),
        },
        "measured_zero": {
            "cx_id": measured_id.to_string(),
            "spread_helper": measured_zero.spread,
            "ofi_helper": measured_zero.ofi,
            "spread_scalar": stored_measured.scalars.get("spread"),
            "ofi_scalar": stored_measured.scalars.get("ofi"),
            "spread_slot": slot_kind(stored_measured.slots.get(&SlotId::new(2)).unwrap()),
            "ofi_slot": slot_kind(stored_measured.slots.get(&SlotId::new(5)).unwrap()),
        }
    });
    let out = root.join("readback.json");
    fs::write(&out, serde_json::to_vec_pretty(&readback).unwrap()).unwrap();
    println!(
        "ISSUE182_DERIVED_FEATURE_ABSENCE_READBACK={}",
        out.display()
    );
}

fn snapshot(slug: &str) -> MarketSnapshot {
    MarketSnapshot {
        token_id: format!("token-{slug}"),
        condition_id: format!("condition-{slug}"),
        outcome_index: 0,
        slug: slug.to_string(),
        question: Some(format!("Derived feature absence {slug}?")),
        event_id: None,
        category: Some("crypto".to_string()),
        region: Some("global".to_string()),
        tags: vec!["fsv".to_string()],
        resolution_source: Some("uma".to_string()),
        neg_risk: false,
        snapshot_ts: 1_785_500_000,
        price: Some(0.50),
        mid: Some(0.50),
        best_bid: None,
        best_ask: None,
        spread: None,
        tick_size: Some(0.01),
        volume_24h: Some(100_000.0),
        liquidity: Some(50_000.0),
        one_hour_change: Some(0.0),
        one_day_change: Some(0.0),
        ofi: None,
        yes_no_residual: Some(0.0),
        secs_to_resolution: Some(86_400.0),
        holders: Vec::new(),
        makers: Vec::new(),
        counterparty_volumes: Vec::new(),
        onchain_fills: Vec::new(),
        temporal_reference_ts: None,
        sequence_position: None,
        sequence_total: None,
        oracle_risk: Default::default(),
        book: Book::default(),
    }
}

fn slot_kind(slot: &SlotVector) -> &'static str {
    match slot {
        SlotVector::Dense { .. } => "dense",
        SlotVector::Sparse { .. } => "sparse",
        SlotVector::Multi { .. } => "multi",
        SlotVector::Absent { .. } => "absent",
    }
}

fn fsv_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/fsv/issue182_derived_feature_absence")
}
