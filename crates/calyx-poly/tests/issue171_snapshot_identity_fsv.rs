use std::fs;
use std::path::{Path, PathBuf};

use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::VaultId;
use calyx_core::VaultStore;
use calyx_poly::constellation::build_constellation;
use calyx_poly::lenses::default_panel;
use calyx_poly::pipeline::ingest_snapshot;
use calyx_poly::{Book, Level, MarketSnapshot};
use serde_json::json;

#[test]
fn issue171_snapshot_identity_fsv() {
    let report_path = repo_root()
        .join("target")
        .join("fsv")
        .join("issue171_snapshot_identity")
        .join("readback-report.json");
    let vault_path = report_path.parent().unwrap().join("vault-source-of-truth");
    let before = file_state(&report_path);
    let vault_before = dir_state(&vault_path);
    if vault_path.exists() {
        fs::remove_dir_all(&vault_path).unwrap();
    }
    fs::create_dir_all(&vault_path).unwrap();

    let panel = default_panel(1, vec!["global".to_string()]);
    let vault_id: VaultId = "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap();
    let store = AsterVault::new_durable(
        &vault_path,
        vault_id,
        b"issue171-vault-salt".to_vec(),
        VaultOptions::default(),
    )
    .unwrap();
    let base = sample_snapshot();

    let base_bytes = base.canonical_input_bytes().unwrap();
    assert!(
        !base_bytes.is_empty(),
        "happy path must never produce empty identity bytes"
    );
    let base_id = build_constellation(&base, &panel, vault_id, b"issue171-salt")
        .unwrap()
        .cx_id
        .to_string();
    let repeat_id = build_constellation(&base, &panel, vault_id, b"issue171-salt")
        .unwrap()
        .cx_id
        .to_string();
    assert_eq!(base_id, repeat_id, "identical snapshots must be idempotent");

    let mut changed = base.clone();
    changed.book.bids.push(Level {
        price: 0.615,
        size: 150_000.0,
    });
    changed.liquidity = Some(250_000.0);
    let changed_id = build_constellation(&changed, &panel, vault_id, b"issue171-salt")
        .unwrap()
        .cx_id
        .to_string();
    assert_ne!(
        base_id, changed_id,
        "microstructure differences must change the content address"
    );
    let persisted_base_id =
        ingest_snapshot(&store, &panel, &base, vault_id, b"issue171-salt").unwrap();
    let persisted_changed_id =
        ingest_snapshot(&store, &panel, &changed, vault_id, b"issue171-salt").unwrap();
    store.flush().unwrap();
    assert_ne!(persisted_base_id, persisted_changed_id);
    let snapshot = store.snapshot();
    let stored_base = store.get(persisted_base_id, snapshot).unwrap();
    let stored_changed = store.get(persisted_changed_id, snapshot).unwrap();
    assert_eq!(stored_base.scalars.get("liquidity"), Some(&40_000.0));
    assert_eq!(stored_changed.scalars.get("liquidity"), Some(&250_000.0));
    let vault_after = dir_state(&vault_path);

    let mut invalid = base.clone();
    invalid.liquidity = Some(f64::INFINITY);
    let invalid_error = invalid
        .canonical_input_bytes()
        .expect_err("must fail closed");
    assert_eq!(
        invalid_error.code(),
        "CALYX_POLY_SNAPSHOT_IDENTITY_NON_FINITE"
    );

    let report = json!({
        "schema_version": "poly.issue171.snapshot_identity_fsv.v1",
        "source_of_truth": "durable local AsterVault readback plus physical report file",
        "before": before,
        "happy_path": {
            "input_bytes": base_bytes.len(),
            "input_blake3": blake3::hash(&base_bytes).to_hex().to_string(),
            "cx_id": base_id,
            "repeat_cx_id": repeat_id,
            "idempotent": base_id == repeat_id
        },
        "edge_microstructure": {
            "changed_cx_id": changed_id,
            "distinct_from_base": changed_id != base_id
        },
        "durable_vault_readback": {
            "vault_path": vault_path,
            "before": vault_before,
            "after": vault_after,
            "base_cx_id": persisted_base_id.to_string(),
            "changed_cx_id": persisted_changed_id.to_string(),
            "distinct_cx_ids": persisted_base_id != persisted_changed_id,
            "base_liquidity": stored_base.scalars.get("liquidity").copied(),
            "changed_liquidity": stored_changed.scalars.get("liquidity").copied(),
            "base_slot_count": stored_base.slots.len(),
            "changed_slot_count": stored_changed.slots.len()
        },
        "edge_non_finite": {
            "error_code": invalid_error.code(),
            "error_kind": invalid_error.kind(),
            "message": invalid_error.message()
        }
    });
    fs::create_dir_all(report_path.parent().unwrap()).unwrap();
    fs::write(&report_path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    let after = file_state(&report_path);
    let readback: serde_json::Value =
        serde_json::from_slice(&fs::read(&report_path).unwrap()).unwrap();
    assert_eq!(readback["happy_path"]["idempotent"], true);
    assert_eq!(readback["edge_microstructure"]["distinct_from_base"], true);
    assert_eq!(readback["durable_vault_readback"]["distinct_cx_ids"], true);
    assert_eq!(
        readback["durable_vault_readback"]["changed_liquidity"],
        250_000.0
    );
    assert_eq!(
        readback["edge_non_finite"]["error_code"],
        "CALYX_POLY_SNAPSHOT_IDENTITY_NON_FINITE"
    );
    assert_eq!(after["exists"], true);
    assert!(after["bytes"].as_u64().unwrap() > 0);
}

fn sample_snapshot() -> MarketSnapshot {
    MarketSnapshot {
        token_id: "tok-issue171".to_string(),
        condition_id: "0xissue171".to_string(),
        outcome_index: 0,
        slug: "issue171-canonical-identity".to_string(),
        question: Some("Issue 171 canonical identity market?".to_string()),
        event_id: Some("evt-issue171".to_string()),
        category: Some("crypto".to_string()),
        region: Some("global".to_string()),
        tags: vec!["identity".to_string()],
        resolution_source: Some("uma".to_string()),
        neg_risk: false,
        snapshot_ts: 1_783_133_600,
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
        holders: Vec::new(),
        makers: Vec::new(),
        counterparty_volumes: Vec::new(),
        onchain_fills: Vec::new(),
        temporal_reference_ts: None,
        sequence_position: None,
        sequence_total: None,
        oracle_risk: Default::default(),
        book: Book {
            bids: vec![Level {
                price: 0.61,
                size: 1000.0,
            }],
            asks: vec![Level {
                price: 0.63,
                size: 1200.0,
            }],
        },
    }
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn file_state(path: &Path) -> serde_json::Value {
    if !path.exists() {
        return json!({
            "exists": false,
            "bytes": 0,
            "blake3": null
        });
    }
    let bytes = fs::read(path).unwrap();
    json!({
        "exists": true,
        "bytes": bytes.len(),
        "blake3": blake3::hash(&bytes).to_hex().to_string()
    })
}

fn dir_state(path: &Path) -> serde_json::Value {
    if !path.exists() {
        return json!({
            "exists": false,
            "file_count": 0,
            "bytes": 0
        });
    }
    let mut file_count = 0_u64;
    let mut bytes = 0_u64;
    let mut stack = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let metadata = entry.metadata().unwrap();
            if metadata.is_dir() {
                stack.push(entry.path());
            } else {
                file_count += 1;
                bytes += metadata.len();
            }
        }
    }
    json!({
        "exists": true,
        "file_count": file_count,
        "bytes": bytes
    })
}
