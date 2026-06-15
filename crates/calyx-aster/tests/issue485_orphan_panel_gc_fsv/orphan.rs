use super::{constellation, cx, dir_inventory, durable_vault, hex, hex_prefix};
use calyx_aster::cf::{ColumnFamily, base_key, slot_key};
use calyx_aster::gc::{OrphanReconciler, OrphanRepairResult, OrphanReport, VaultOrphanGcTarget};
use calyx_aster::vault::AsterVault;
use calyx_aster::vault::encode::{decode_constellation_base, encode_constellation_base};
use calyx_core::{FixedClock, SlotId};
use calyx_ledger::decode as decode_ledger;
use serde_json::{Value, json};
use std::fs;
use std::path::Path;
use std::time::Duration;

pub fn orphan_fsv(root: &Path) -> Value {
    let happy_dir = root.join("happy").join("vault");
    fs::create_dir_all(&happy_dir).expect("create orphan happy dir");
    let vault = durable_vault(&happy_dir);
    let slot = SlotId::new(0);
    for seed in 1..=5 {
        vault
            .write_cf(
                ColumnFamily::Base,
                base_key(cx(seed)),
                encode_constellation_base(&constellation(seed, 1, &[slot])).unwrap(),
            )
            .unwrap();
    }
    for seed in [1, 2, 3, 6, 7, 8] {
        vault
            .write_cf(ColumnFamily::slot(slot), slot_key(cx(seed)), vec![seed])
            .unwrap();
    }
    vault.flush().unwrap();

    let before = orphan_readback(&vault, slot, &happy_dir);
    let target = VaultOrphanGcTarget::new(&vault, [slot]);
    let reconciler = OrphanReconciler::new(Duration::ZERO, 10);
    let report = reconciler.scan(&target).unwrap();
    let repair = reconciler.repair(&target, &report).unwrap();
    vault.flush().unwrap();
    let after = orphan_readback(&vault, slot, &happy_dir);
    let repeat = reconciler.scan(&target).unwrap();
    let metrics = [
        report.to_metrics_text("issue485-orphan"),
        repair.to_metrics_text("issue485-orphan"),
    ]
    .join("");
    fs::write(root.join("orphan-metrics.prom"), &metrics).expect("write orphan metrics");

    json!({
        "source_of_truth": {
            "vault": happy_dir.display().to_string(),
            "base_cf": "vault/cf/base",
            "slot_cf": "vault/cf/slot_0000",
            "anneal_replay_cf": "vault/cf/anneal_replay",
            "ledger_cf": "vault/cf/ledger"
        },
        "synthetic_input": {
            "base_cx": ["01","02","03","04","05"],
            "slot_cx": ["01","02","03","06","07","08"],
            "hand_expected": {
                "orphan_index": ["06","07","08"],
                "orphan_base": ["04","05"],
                "after_slot_rows": 3,
                "anneal_replay_rows": 2,
                "ledger_delta": 5
            }
        },
        "happy": {
            "before": before,
            "report": report_json(&report),
            "repair": repair_json(&repair),
            "after": after,
            "repeat_scan": report_json(&repeat)
        },
        "edges": {
            "empty_input": orphan_empty_edge(&root.join("edge-empty")),
            "rate_limit": orphan_rate_limit_edge(&root.join("edge-rate-limit")),
            "fail_closed_missing_base": orphan_fail_closed_edge(&root.join("edge-fail-closed"))
        },
        "metrics": metrics
    })
}

fn orphan_empty_edge(root: &Path) -> Value {
    fs::create_dir_all(root).unwrap();
    let vault = durable_vault(&root.join("vault"));
    let slot = SlotId::new(0);
    let target = VaultOrphanGcTarget::new(&vault, [slot]);
    let reconciler = OrphanReconciler::new(Duration::ZERO, 3);
    let before = orphan_readback(&vault, slot, root);
    let report = reconciler.scan(&target).unwrap();
    let repair = reconciler.repair(&target, &report).unwrap();
    vault.flush().unwrap();
    let after = orphan_readback(&vault, slot, root);
    json!({
        "before": before,
        "report": report_json(&report),
        "repair": repair_json(&repair),
        "after": after
    })
}

fn orphan_rate_limit_edge(root: &Path) -> Value {
    let vault_dir = root.join("vault");
    fs::create_dir_all(&vault_dir).unwrap();
    let vault = durable_vault(&vault_dir);
    let slot = SlotId::new(0);
    for seed in 1..=10 {
        vault
            .write_cf(ColumnFamily::slot(slot), slot_key(cx(seed)), vec![seed])
            .unwrap();
    }
    vault.flush().unwrap();
    let before = orphan_readback(&vault, slot, &vault_dir);
    let target = VaultOrphanGcTarget::new(&vault, [slot]);
    let reconciler = OrphanReconciler::new(Duration::ZERO, 3);
    let report = reconciler.scan(&target).unwrap();
    let repair = reconciler.repair(&target, &report).unwrap();
    vault.flush().unwrap();
    let after = orphan_readback(&vault, slot, &vault_dir);
    json!({
        "before": before,
        "report": report_json(&report),
        "repair": repair_json(&repair),
        "after": after,
        "hand_expected_after_slot_rows": 7
    })
}

fn orphan_fail_closed_edge(root: &Path) -> Value {
    let vault_dir = root.join("vault");
    fs::create_dir_all(&vault_dir).unwrap();
    let vault = durable_vault(&vault_dir);
    let slot = SlotId::new(0);
    let before = orphan_readback(&vault, slot, &vault_dir);
    let target = VaultOrphanGcTarget::new(&vault, [slot]);
    let reconciler = OrphanReconciler::new(Duration::ZERO, 1);
    let report = OrphanReport {
        orphan_base: vec![cx(250)],
        inconsistencies: 1,
        ..OrphanReport::default()
    };
    let error = reconciler.repair(&target, &report).unwrap_err();
    vault.flush().unwrap();
    let after = orphan_readback(&vault, slot, &vault_dir);
    json!({
        "before": before,
        "after": after,
        "error_code": error.code,
        "error_message": error.message
    })
}

fn orphan_readback(vault: &AsterVault<FixedClock>, slot: SlotId, vault_dir: &Path) -> Value {
    let base_rows = vault
        .scan_cf_at(vault.latest_seq(), ColumnFamily::Base)
        .unwrap();
    let slot_rows = vault
        .scan_cf_at(vault.latest_seq(), ColumnFamily::slot(slot))
        .unwrap();
    let replay_rows = vault
        .scan_cf_at(vault.latest_seq(), ColumnFamily::AnnealReplay)
        .unwrap();
    let ledger_rows = vault
        .scan_cf_at(vault.latest_seq(), ColumnFamily::Ledger)
        .unwrap();
    let degraded = base_rows
        .iter()
        .filter_map(|(key, bytes)| {
            let cx = decode_constellation_base(bytes).ok()?;
            cx.flags.degraded.then(|| hex(key))
        })
        .collect::<Vec<_>>();
    json!({
        "seq": vault.latest_seq(),
        "base_rows": base_rows.len(),
        "slot_rows": slot_rows.len(),
        "anneal_replay_rows": replay_rows.len(),
        "ledger_rows": ledger_rows.len(),
        "base": cf_json(&base_rows),
        "slot": cf_json(&slot_rows),
        "anneal_replay": cf_json(&replay_rows),
        "ledger_events": ledger_events(&ledger_rows),
        "degraded_base_keys": degraded,
        "cf_files": dir_inventory(&vault_dir.join("cf"))
    })
}

fn cf_json(rows: &[(Vec<u8>, Vec<u8>)]) -> Vec<Value> {
    rows.iter()
        .map(|(key, value)| {
            json!({
                "key_hex": hex(key),
                "value_len": value.len(),
                "value_hex_prefix": hex_prefix(value, 96)
            })
        })
        .collect()
}

fn ledger_events(rows: &[(Vec<u8>, Vec<u8>)]) -> Vec<Value> {
    rows.iter()
        .filter_map(|(key, bytes)| {
            let entry = decode_ledger(bytes).ok()?;
            let payload = serde_json::from_slice::<Value>(&entry.payload).ok()?;
            Some(json!({
                "key_hex": hex(key),
                "subject": format!("{:?}", entry.subject),
                "payload": payload
            }))
        })
        .collect()
}

fn report_json(report: &OrphanReport) -> Value {
    json!({
        "orphan_index": report.orphan_index.iter().map(|cx| hex(cx.as_bytes())).collect::<Vec<_>>(),
        "orphan_base": report.orphan_base.iter().map(|cx| hex(cx.as_bytes())).collect::<Vec<_>>(),
        "orphan_index_entries": report.orphan_index_entries.iter().map(|entry| json!({
            "cx": hex(entry.cx_id.as_bytes()),
            "slot": entry.slot.get()
        })).collect::<Vec<_>>(),
        "inconsistencies": report.inconsistencies
    })
}

fn repair_json(result: &OrphanRepairResult) -> Value {
    json!({
        "orphan_index_repaired": result.orphan_index_repaired,
        "orphan_base_degraded": result.orphan_base_degraded,
        "repairs_total": result.repairs_total,
        "remaining_inconsistencies": result.remaining_inconsistencies,
        "rate_limited": result.rate_limited
    })
}
