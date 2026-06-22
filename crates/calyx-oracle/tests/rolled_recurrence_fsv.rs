use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use calyx_aster::cf::ColumnFamily;
use calyx_aster::dedup::EpochSecs;
use calyx_aster::recurrence::FREQUENCY_SCALAR;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{
    Clock, Constellation, CxFlags, CxId, InputRef, LedgerRef, Modality, VaultId, VaultStore,
};
use calyx_loom::{
    OccurrenceContext, PeriodicRecallQuery, RetentionPolicy, SeriesStore, decode_recurrence_row,
};
use calyx_oracle::predict_next_occurrence;
use serde_json::json;

const TUESDAY_2024_01_02_14H_UTC: i64 = 1_704_204_000;
const WEEK_SECS: i64 = 604_800;

#[test]
#[ignore = "FSV: writes durable rolled recurrence support evidence under CALYX_FSV_ROOT"]
fn issue634_rolled_recurrence_summary_manual_fsv() {
    let root = fsv_root().join("issue634-rolled-recurrence-summary");
    fs::create_dir_all(&root).expect("create fsv root");
    let vault_dir = root.join("vault");
    let vault = AsterVault::new_durable(
        &vault_dir,
        vault_id(),
        b"issue634-rolled-recurrence-salt".to_vec(),
        VaultOptions::default(),
    )
    .expect("durable vault");

    let cx_id = put_base(&vault, b"issue634-weekly-rolled");
    append_rolled_weekly(&vault, cx_id, 100, 4 * WEEK_SECS as u64, 11);
    let sparse_cx_id = put_base(&vault, b"issue634-sparse-active-rolled");
    append_rolled_weekly(&vault, sparse_cx_id, 1, u64::MAX, 3);
    vault.flush().expect("flush vault");

    let store = SeriesStore::new(&vault);
    let series = store.read_series(cx_id).expect("read rolled series");
    let loom_read = store.recurrence_series(cx_id).expect("loom read");
    let recall = store
        .periodic_recall_readback(PeriodicRecallQuery::new(Some(14), Some(1)).expect("query"))
        .expect("recall");
    let prediction = predict_next_occurrence(&vault, cx_id, 1.0).expect("prediction");
    let sparse_error =
        predict_next_occurrence(&vault, sparse_cx_id, 1.0).expect_err("sparse active");
    let base = vault.get(cx_id, vault.snapshot()).expect("base cx");
    let sparse_series = store.read_series(sparse_cx_id).expect("sparse series");

    assert_eq!(series.frequency, 12);
    assert_eq!(series.occurrences.len(), 5);
    assert_eq!(series.rollup_summary.as_ref().unwrap().count_rolled, 7);
    assert_eq!(loom_read.periodic_fit.support, 12);
    assert_eq!(loom_read.periodic_fit.active_support, 5);
    assert_eq!(loom_read.periodic_fit.rolled_support, 7);
    assert_eq!(recall.hits.len(), 1);
    assert_eq!(prediction.support, 12);
    assert_eq!(prediction.active_support, 5);
    assert_eq!(prediction.rolled_support, 7);
    assert_eq!(prediction.confidence, 1.0);
    assert_eq!(sparse_error.code, calyx_oracle::CALYX_ORACLE_INSUFFICIENT);
    assert!(sparse_error.message.contains("active support=1"));

    let report = json!({
        "issue": 634,
        "semantics": {
            "cadence_and_phase_source": "active occurrence rows",
            "support_source": "max(base recurrence.frequency, active occurrence count)",
            "rolled_history_rule": "rollup summary supports confidence/readback but cannot define phase without active rows",
            "active_sparse_behavior": "fail closed with CALYX_ORACLE_INSUFFICIENT",
        },
        "cx_id": cx_id,
        "base_frequency": base.scalars.get(FREQUENCY_SCALAR).copied(),
        "series": series,
        "loom_read": loom_read,
        "recall": recall,
        "oracle_prediction": prediction,
        "sparse_edge": {
            "cx_id": sparse_cx_id,
            "series": sparse_series,
            "error_code": sparse_error.code,
            "error_message": sparse_error.message,
        },
        "recurrence_rows": recurrence_rows_json(&vault),
        "recurrence_files": files_under(&vault_dir.join("cf").join("recurrence")),
        "wal_files": files_under(&vault_dir.join("wal")),
        "vault_dir": vault_dir.display().to_string(),
    });
    let path = root.join("rolled-recurrence-readback.json");
    fs::write(
        &path,
        serde_json::to_vec_pretty(&report).expect("report json"),
    )
    .expect("write report");
    let readback = fs::read(&path).expect("read report");
    println!("issue634_rolled_recurrence_fsv_root={}", root.display());
    println!("issue634_readback_b3={}", blake3::hash(&readback).to_hex());
}

fn append_rolled_weekly<C: Clock>(
    vault: &AsterVault<C>,
    cx_id: CxId,
    max_occurrences: usize,
    max_age_secs: u64,
    final_week: i64,
) {
    let seed_store = SeriesStore::new(vault);
    for week in 0..final_week {
        seed_store
            .append_occurrence(cx_id, EpochSecs(weekly_time(week)), ctx("seed"))
            .expect("seed append");
    }
    let retention = RetentionPolicy::new(max_occurrences, max_age_secs).expect("retention");
    let rolling_store = SeriesStore::with_retention(vault, retention).expect("rolling store");
    rolling_store
        .append_occurrence_observed_at(
            cx_id,
            EpochSecs(weekly_time(final_week)),
            ctx("roll"),
            EpochSecs(weekly_time(final_week)),
        )
        .expect("rolling append");
}

fn recurrence_rows_json<C: Clock>(vault: &AsterVault<C>) -> Vec<serde_json::Value> {
    vault
        .scan_cf_at(vault.snapshot(), ColumnFamily::Recurrence)
        .expect("scan recurrence")
        .into_iter()
        .map(|(key, value)| {
            json!({
                "key_hex": hex(&key),
                "value_b3": blake3::hash(&value).to_hex().to_string(),
                "decoded": decode_recurrence_row(&value).expect("decode recurrence row"),
            })
        })
        .collect()
}

fn files_under(dir: &Path) -> Vec<serde_json::Value> {
    if !dir.exists() {
        return Vec::new();
    }
    let mut files = fs::read_dir(dir)
        .expect("read dir")
        .map(|entry| entry.expect("dir entry").path())
        .filter(|path| path.is_file())
        .map(|path| {
            json!({
                "path": path.display().to_string(),
                "bytes": fs::metadata(&path).expect("metadata").len(),
            })
        })
        .collect::<Vec<_>>();
    files.sort_by_key(|value| value["path"].as_str().unwrap_or_default().to_string());
    files
}

fn weekly_time(week: i64) -> i64 {
    TUESDAY_2024_01_02_14H_UTC + week * WEEK_SECS
}

fn ctx(value: &str) -> OccurrenceContext {
    OccurrenceContext::new(value.as_bytes().to_vec()).expect("context")
}

fn put_base<C: Clock>(vault: &AsterVault<C>, input: &[u8]) -> CxId {
    let cx_id = vault.cx_id_for_input(input, 41);
    let cx = Constellation {
        cx_id,
        vault_id: vault.vault_id(),
        panel_version: 41,
        created_at: 100,
        input_ref: InputRef {
            hash: *blake3::hash(input).as_bytes(),
            pointer: None,
            redacted: true,
        },
        modality: Modality::Text,
        slots: BTreeMap::new(),
        scalars: BTreeMap::new(),
        metadata: BTreeMap::new(),
        anchors: Vec::new(),
        provenance: LedgerRef {
            seq: 0,
            hash: [0; 32],
        },
        flags: CxFlags {
            ungrounded: true,
            redacted_input: true,
            ..CxFlags::default()
        },
    };
    vault.put(cx).expect("put base");
    cx_id
}

fn fsv_root() -> PathBuf {
    std::env::var_os("CALYX_FSV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::temp_dir().join(format!("calyx-issue634-fsv-{}", std::process::id()))
        })
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().expect("vault id")
}

fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}
