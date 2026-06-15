use calyx_anneal::{
    AsterBanditStorage, BanditPolicy, BanditStorage, ConfigBandit, MetricSample, check_oscillation,
    decode_config_bandit, encode_config_bandit, shape_key_hash,
};
use calyx_aster::cf::{ColumnFamily, XTermKind, xterm_key};
use calyx_aster::gc::{
    PanelVersionGc, PanelVersionGcTarget, RetentionPolicy, VaultPanelVersionGcTarget, VersionTier,
};
use calyx_core::{
    Constellation, CxFlags, CxId, InputRef, LedgerRef, Modality, VaultId, VaultStore,
};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use super::resource::ProbeResult;
use super::resource_support::{case_dir, err, open_vault};

const START_TS: u64 = 1_800_500_000_000;
const MEMTABLE_BYTES: usize = 64 * 1024 * 1024;

pub(super) fn probe_h20_anneal_thrashing(root: &Path) -> ProbeResult {
    let dir = case_dir(root, "h20_anneal_thrashing")?;
    let vault_dir = dir.join("vault");
    let vault = open_vault(&vault_dir, START_TS + 20, b"ph59-h20", MEMTABLE_BYTES, None)?;
    let hash = shape_key_hash("ph59:h20:anneal-thrash:storage");
    let mut bandit =
        ConfigBandit::new(BanditPolicy::EpsilonGreedy { epsilon: 0.0 }, 20).with_hysteresis(3);
    bandit.add_arm(b"incumbent:stable".to_vec());
    bandit.add_arm(b"candidate:a".to_vec());
    bandit.add_arm(b"candidate:b".to_vec());
    let before = scan_bandit_rows(&vault)?;
    let oscillating_schedule = [
        (1, true),
        (2, true),
        (1, false),
        (2, false),
        (1, true),
        (2, true),
        (1, false),
        (2, false),
    ];
    let mut trace = Vec::new();
    for (round, (arm, won)) in oscillating_schedule.into_iter().enumerate() {
        bandit.record_result(arm, won).map_err(err)?;
        trace.push(
            json!({"round": round, "arm": arm, "won": won, "incumbent": bandit.incumbent_idx}),
        );
    }
    let incumbent_after_oscillation = bandit.incumbent_idx;
    for round in 0..3 {
        bandit.record_result(1, true).map_err(err)?;
        trace.push(
            json!({"round": round + 100, "arm": 1, "won": true, "incumbent": bandit.incumbent_idx}),
        );
    }
    let rising_samples = [
        MetricSample {
            p99_ns: 100,
            recall_10: 0.99,
            query_count: 1_000,
        },
        MetricSample {
            p99_ns: 112,
            recall_10: 0.99,
            query_count: 2_000,
        },
    ];
    let stable_samples = [
        MetricSample {
            p99_ns: 100,
            recall_10: 0.99,
            query_count: 1_000,
        },
        MetricSample {
            p99_ns: 101,
            recall_10: 0.99,
            query_count: 2_000,
        },
    ];
    let rising_oscillation = check_oscillation(&rising_samples, 2_000);
    let stable_oscillation = check_oscillation(&stable_samples, 2_000);
    let row_key = calyx_anneal::bandit_key(hash);
    let row_value = encode_config_bandit(&bandit).map_err(err)?;
    {
        let storage = AsterBanditStorage::new(&vault);
        storage
            .save(row_key.clone(), row_value.clone())
            .map_err(err)?;
    }
    vault.flush().map_err(err)?;
    let after = scan_bandit_rows(&vault)?;
    let stored = vault
        .read_cf_at(vault.latest_seq(), ColumnFamily::AnnealBandit, &row_key)
        .map_err(err)?
        .ok_or_else(|| "saved anneal bandit row missing".to_string())?;
    let decoded = decode_config_bandit(&stored).map_err(err)?;
    let passed = before.is_empty()
        && after.len() == 1
        && incumbent_after_oscillation == 0
        && decoded.incumbent_idx == 1
        && decoded.arms[1].consecutive_wins == 0
        && rising_oscillation
        && !stable_oscillation;

    Ok((
        passed,
        json!({
            "trigger": "oscillating Anneal candidate wins followed by stable candidate wins, persisted to anneal_bandit CF",
            "expected": {
                "oscillation_does_not_promote": true,
                "stable_candidate_promotes_after_hysteresis": true,
                "bandit_row_persisted": true
            },
            "actual": {
                "rows_before": before,
                "rows_after": after,
                "incumbent_after_oscillation": incumbent_after_oscillation,
                "stored_status": decoded.status(hash).map_err(err)?,
                "trace": trace,
                "row_key_hex": hex_bytes(&row_key),
                "row_value_len": row_value.len(),
                "rising_oscillation_detected": rising_oscillation,
                "stable_oscillation_detected": stable_oscillation,
                "panic_free": true
            },
            "metrics_text": format!(
                "calyx_anneal_bandit_rows{{vault=\"ph59-h20\"}} {}\ncalyx_anneal_promoted_arm{{vault=\"ph59-h20\"}} {}\n",
                after.len(),
                decoded.incumbent_idx
            )
        }),
    ))
}

pub(super) fn probe_h21_panel_explosion(root: &Path) -> ProbeResult {
    let dir = case_dir(root, "h21_panel_explosion")?;
    let vault_dir = dir.join("vault");
    let cold_dir = dir.join("cold");
    let vault = open_vault(&vault_dir, START_TS + 21, b"ph59-h21", MEMTABLE_BYTES, None)?;
    write_panel_files(&vault_dir, 1..=12)?;
    let live_ids = [10_u32, 11, 12]
        .into_iter()
        .map(|panel_version| put_live_constellation(&vault, panel_version))
        .collect::<Result<Vec<_>, _>>()?;
    let xterm_cap_per_cx = 4usize;
    let skipped_pairs = write_capped_xterms(&vault, &live_ids, xterm_cap_per_cx)?;
    vault.flush().map_err(err)?;

    let target = VaultPanelVersionGcTarget::new(&vault, &vault_dir, &cold_dir);
    let records_before = panel_records_json(target.panel_versions().map_err(err)?);
    let unreferenced = PanelVersionGc::new(RetentionPolicy {
        hot_versions_to_keep: 2,
        cold_tier_first: true,
        max_versions_per_run: 32,
    })
    .find_unreferenced(&target)
    .map_err(err)?;
    let gc = PanelVersionGc::new(RetentionPolicy {
        hot_versions_to_keep: 2,
        cold_tier_first: true,
        max_versions_per_run: 32,
    });
    let moved = gc.prune(&target, &unreferenced).map_err(err)?;
    let pruned = gc.prune(&target, &unreferenced).map_err(err)?;
    let records_after = panel_records_json(target.panel_versions().map_err(err)?);
    let live_after = target.live_panel_versions().map_err(err)?;
    let hot_after = list_panel_ids(&vault_dir.join("panel"))?;
    let cold_after = list_panel_ids(&cold_dir.join("panel"))?;
    let xterm_rows = vault
        .scan_cf_at(vault.latest_seq(), ColumnFamily::XTerm)
        .map_err(err)?;
    let temporal_rows = vault
        .scan_cf_at(vault.latest_seq(), ColumnFamily::TemporalXTerm)
        .map_err(err)?
        .len();
    let max_xterms_per_cx = max_xterms_per_cx(&xterm_rows);
    let skipped_absent = vault
        .read_cf_at(
            vault.latest_seq(),
            ColumnFamily::XTerm,
            &xterm_key(
                live_ids[0],
                calyx_core::SlotId::new(2),
                calyx_core::SlotId::new(4),
                XTermKind::Agreement,
            ),
        )
        .map_err(err)?
        .is_none();
    let passed = unreferenced.len() == 9
        && moved.moved_to_cold == 9
        && pruned.pruned == 9
        && live_after == BTreeSet::from([10, 11, 12])
        && hot_after == vec![10, 11, 12]
        && cold_after.is_empty()
        && xterm_rows.len() == live_ids.len() * xterm_cap_per_cx
        && max_xterms_per_cx <= xterm_cap_per_cx
        && skipped_pairs > 0
        && skipped_absent
        && temporal_rows == 0;

    Ok((
        passed,
        json!({
            "trigger": "12 panel files, 3 live panel-version base rows, capped xterm materialization, then panel GC",
            "expected": {
                "unreferenced_panel_versions": 9,
                "live_versions_preserved": [10, 11, 12],
                "xterm_rows_lte_cap_per_cx": xterm_cap_per_cx,
                "skipped_pairs_absent": true
            },
            "actual": {
                "records_before": records_before,
                "unreferenced": unreferenced,
                "gc_moved": gc_result_json(&moved),
                "gc_pruned": gc_result_json(&pruned),
                "records_after": records_after,
                "hot_panel_ids_after": hot_after,
                "cold_panel_ids_after": cold_after,
                "live_versions_after": live_after,
                "xterm_rows": xterm_rows.iter().map(|(key, value)| json!({"key_hex": hex_bytes(key), "value_len": value.len()})).collect::<Vec<_>>(),
                "xterm_rows_total": xterm_rows.len(),
                "max_xterms_per_cx": max_xterms_per_cx,
                "skipped_pairs": skipped_pairs,
                "skipped_pair_absent": skipped_absent,
                "temporal_xterm_rows": temporal_rows,
                "panic_free": true
            },
            "metrics_text": moved.to_metrics_text("ph59-h21", live_after.len()) + &pruned.to_metrics_text("ph59-h21", live_after.len())
        }),
    ))
}

fn scan_bandit_rows<C>(
    vault: &calyx_aster::vault::AsterVault<C>,
) -> Result<Vec<serde_json::Value>, String>
where
    C: calyx_core::Clock,
{
    Ok(vault
        .scan_cf_at(vault.latest_seq(), ColumnFamily::AnnealBandit)
        .map_err(err)?
        .into_iter()
        .map(|(key, value)| json!({"key_hex": hex_bytes(&key), "value_len": value.len()}))
        .collect())
}

fn write_panel_files(vault_dir: &Path, versions: impl Iterator<Item = u32>) -> Result<(), String> {
    let panel_dir = vault_dir.join("panel");
    fs::create_dir_all(&panel_dir).map_err(err)?;
    for version in versions {
        fs::write(
            panel_dir.join(format!("panel-v{version:08}.json")),
            format!("{{\"version\":{version},\"slots\":[]}}\n"),
        )
        .map_err(err)?;
    }
    Ok(())
}

fn put_live_constellation<C>(
    vault: &calyx_aster::vault::AsterVault<C>,
    panel_version: u32,
) -> Result<CxId, String>
where
    C: calyx_core::Clock,
{
    let input = format!("ph59-h21-panel-{panel_version}").into_bytes();
    let cx_id = vault.cx_id_for_input(&input, panel_version);
    let constellation = Constellation {
        cx_id,
        vault_id: vault_id(),
        panel_version,
        created_at: START_TS + u64::from(panel_version),
        input_ref: InputRef {
            hash: input_hash(&input),
            pointer: None,
            redacted: false,
        },
        modality: Modality::Text,
        slots: BTreeMap::new(),
        scalars: BTreeMap::new(),
        metadata: BTreeMap::new(),
        anchors: Vec::new(),
        provenance: LedgerRef {
            seq: u64::from(panel_version),
            hash: [panel_version as u8; 32],
        },
        flags: CxFlags::default(),
    };
    vault.put(constellation).map_err(err)?;
    Ok(cx_id)
}

fn write_capped_xterms<C>(
    vault: &calyx_aster::vault::AsterVault<C>,
    cx_ids: &[CxId],
    cap_per_cx: usize,
) -> Result<usize, String>
where
    C: calyx_core::Clock,
{
    let pairs = [
        (0, 1, XTermKind::Concat),
        (0, 2, XTermKind::Interaction),
        (1, 2, XTermKind::Agreement),
        (1, 3, XTermKind::Delta),
        (2, 4, XTermKind::Agreement),
        (3, 5, XTermKind::Delta),
        (4, 6, XTermKind::Concat),
        (5, 7, XTermKind::Interaction),
    ];
    let mut skipped = 0usize;
    for cx_id in cx_ids {
        for (idx, (a, b, kind)) in pairs.into_iter().enumerate() {
            if idx >= cap_per_cx {
                skipped += 1;
                continue;
            }
            vault
                .write_cf(
                    ColumnFamily::XTerm,
                    xterm_key(
                        *cx_id,
                        calyx_core::SlotId::new(a),
                        calyx_core::SlotId::new(b),
                        kind,
                    ),
                    format!("xterm:{idx}").into_bytes(),
                )
                .map_err(err)?;
        }
    }
    Ok(skipped)
}

fn max_xterms_per_cx(rows: &[(Vec<u8>, Vec<u8>)]) -> usize {
    let mut counts = BTreeMap::<Vec<u8>, usize>::new();
    for (key, _) in rows {
        if key.len() >= 16 {
            *counts.entry(key[..16].to_vec()).or_default() += 1;
        }
    }
    counts.values().copied().max().unwrap_or(0)
}

fn panel_records_json(records: Vec<calyx_aster::gc::PanelVersionRecord>) -> Vec<serde_json::Value> {
    records
        .into_iter()
        .map(|record| {
            json!({
                "id": record.id,
                "tier": match record.tier { VersionTier::Hot => "hot", VersionTier::Cold => "cold" },
                "ledger_referenced": record.ledger_referenced,
                "bytes": record.bytes
            })
        })
        .collect()
}

fn list_panel_ids(dir: &Path) -> Result<Vec<u32>, String> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut ids = Vec::new();
    for entry in fs::read_dir(dir).map_err(err)? {
        let path = entry.map_err(err)?.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let Some(rest) = name.strip_prefix("panel-v") else {
            continue;
        };
        if let Some(id) = rest.get(0..8).and_then(|value| value.parse::<u32>().ok()) {
            ids.push(id);
        }
    }
    ids.sort_unstable();
    Ok(ids)
}

fn gc_result_json(result: &calyx_aster::gc::PanelVersionGcResult) -> serde_json::Value {
    json!({
        "moved_to_cold": result.moved_to_cold,
        "pruned": result.pruned,
        "skipped_ledger_referenced": result.skipped_ledger_referenced,
        "bytes_freed": result.bytes_freed,
        "rate_limited": result.rate_limited,
        "panel_versions_pruned_total": result.panel_versions_pruned_total,
        "codebook_versions_pruned_total": result.codebook_versions_pruned_total,
        "retired_lens_bytes_freed_total": result.retired_lens_bytes_freed_total
    })
}

fn input_hash(input: &[u8]) -> [u8; 32] {
    let mut hash = [0_u8; 32];
    for (idx, byte) in input.iter().enumerate() {
        hash[idx % 32] = hash[idx % 32].wrapping_add(*byte).rotate_left(1);
    }
    hash
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV"
        .parse()
        .expect("valid PH59 soak vault id")
}

fn hex_bytes(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(hex_digit(byte >> 4));
        out.push(hex_digit(byte & 0x0f));
    }
    out
}

fn hex_digit(value: u8) -> char {
    match value {
        0..=9 => char::from(b'0' + value),
        10..=15 => char::from(b'a' + value - 10),
        _ => unreachable!("nibble out of range"),
    }
}
