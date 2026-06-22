use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use calyx_aster::cf::ColumnFamily;
use calyx_aster::dedup::EpochSecs;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{
    Clock, Constellation, CxFlags, CxId, InputRef, LedgerRef, Modality, VaultId, VaultStore,
};
use calyx_loom::{
    OccurrenceContext, PeriodicRecallQuery, SeriesStore, decode_recurrence_row,
    periodic_time_bucket as loom_time_bucket,
};
use calyx_oracle::{
    predict_next_occurrence_from_series_with_tz_offset, time_bucket as oracle_time_bucket,
};
use calyx_sextant::{PeriodicOptions, score_e3_periodic, temporal_time_bucket};
use serde_json::json;

#[path = "sextant_support/mod.rs"]
mod sextant_support;
use sextant_support::hex;

const TUESDAY_2024_01_02_14H_UTC: i64 = 1_704_204_000;
const TUESDAY_2024_01_02_19H_UTC: i64 = TUESDAY_2024_01_02_14H_UTC + 5 * 3_600;
const WEEK_SECS: i64 = 604_800;
const UTC_MINUS_FIVE_SECS: i32 = -18_000;

#[test]
#[ignore = "FSV: writes durable cross-engine timezone evidence under CALYX_FSV_ROOT"]
fn issue635_temporal_timezone_alignment_manual_fsv() {
    let root = fsv_root().join("issue635-timezone-alignment");
    fs::create_dir_all(&root).expect("create fsv root");
    let vault_dir = root.join("vault");
    let vault = AsterVault::new_durable(
        &vault_dir,
        vault_id(),
        b"issue635-timezone-alignment-salt".to_vec(),
        VaultOptions::default(),
    )
    .expect("durable vault");
    let cx_id = put_base(&vault, b"issue635-weekly-local-14-utc-minus-5");
    let store = SeriesStore::new(&vault);
    let timestamps = (0..4)
        .map(|week| TUESDAY_2024_01_02_19H_UTC + week * WEEK_SECS)
        .collect::<Vec<_>>();
    for time in &timestamps {
        store
            .append_occurrence(cx_id, EpochSecs(*time), ctx("local-14"))
            .expect("append timezone occurrence");
    }
    vault.flush().expect("flush durable vault");

    let utc_read = store.recurrence_series(cx_id).expect("utc loom read");
    let local_read = store
        .recurrence_series_with_tz_offset(cx_id, UTC_MINUS_FIVE_SECS)
        .expect("local loom read");
    let utc_query = PeriodicRecallQuery::new(Some(14), Some(1)).expect("utc query");
    let local_query = PeriodicRecallQuery::with_tz_offset(Some(14), Some(1), UTC_MINUS_FIVE_SECS)
        .expect("local query");
    let local_hour_query = PeriodicRecallQuery::with_tz_offset(Some(14), None, UTC_MINUS_FIVE_SECS)
        .expect("local hour query");
    let utc_recall = store
        .periodic_recall_readback(utc_query)
        .expect("utc recall");
    let local_recall = store
        .periodic_recall_readback(local_query)
        .expect("local recall");
    let local_hour_recall = store
        .periodic_recall_readback(local_hour_query)
        .expect("local hour recall");

    assert!(utc_recall.hits.is_empty());
    assert_eq!(local_recall.hits.len(), 1);
    assert_eq!(local_hour_recall.hits.len(), 1);
    assert_eq!(utc_read.periodic_fit.target_hour, Some(19));
    assert_eq!(local_read.periodic_fit.target_hour, Some(14));
    assert_eq!(local_read.periodic_fit.tz_offset_secs, UTC_MINUS_FIVE_SECS);

    let options = PeriodicOptions::new(Some(14), Some(1)).expect("periodic options");
    let sextant_local_score =
        score_e3_periodic(timestamps[0], timestamps[0], &options, UTC_MINUS_FIVE_SECS);
    let sextant_utc_score = score_e3_periodic(timestamps[0], timestamps[0], &options, 0);
    assert_eq!(sextant_local_score, 1.0);
    assert_eq!(sextant_utc_score, 0.5);

    let oracle_utc = predict_next_occurrence_from_series_with_tz_offset(&local_read.series, 1.0, 0)
        .expect("oracle utc");
    let oracle_local = predict_next_occurrence_from_series_with_tz_offset(
        &local_read.series,
        1.0,
        UTC_MINUS_FIVE_SECS,
    )
    .expect("oracle local");
    assert_eq!(oracle_local.tz_offset_secs, UTC_MINUS_FIVE_SECS);
    assert_eq!(
        oracle_local.t_hat,
        EpochSecs(TUESDAY_2024_01_02_19H_UTC + 4 * WEEK_SECS)
    );

    let invalid_query = PeriodicRecallQuery::with_tz_offset(Some(24), None, UTC_MINUS_FIVE_SECS)
        .expect_err("invalid query");
    assert_eq!(
        invalid_query.code,
        calyx_core::CALYX_TEMPORAL_INVALID_PERIOD
    );

    let report = json!({
        "issue": 635,
        "canonical_timezone_model": {
            "timestamp_storage": "UTC epoch seconds",
            "active_context": "explicit fixed tz_offset_secs supplied by query/vault context",
            "compatibility_default": "UTC when callers use existing wrappers",
            "dst_scope": "callers supply the effective offset for the context; named timezone database conversion is not implicit in these engines",
        },
        "vault_dir": vault_dir.display().to_string(),
        "raw_timestamps_utc": timestamps,
        "tz_offset_secs": UTC_MINUS_FIVE_SECS,
        "buckets": {
            "sextant": {
                "utc": temporal_time_bucket(timestamps[0], 0),
                "local": temporal_time_bucket(timestamps[0], UTC_MINUS_FIVE_SECS),
            },
            "loom": {
                "utc": loom_time_bucket(timestamps[0], 0),
                "local": loom_time_bucket(timestamps[0], UTC_MINUS_FIVE_SECS),
            },
            "oracle": {
                "utc": oracle_time_bucket(timestamps[0], 0),
                "local": oracle_time_bucket(timestamps[0], UTC_MINUS_FIVE_SECS),
            },
        },
        "loom": {
            "utc_fit": utc_read.periodic_fit,
            "local_fit": local_read.periodic_fit,
            "utc_recall": utc_recall,
            "local_recall": local_recall,
            "local_hour_recall": local_hour_recall,
        },
        "sextant": {
            "local_score": sextant_local_score,
            "utc_score": sextant_utc_score,
            "target_hour": 14,
            "target_day_of_week": 1,
        },
        "oracle": {
            "utc_prediction": oracle_utc,
            "local_prediction": oracle_local,
        },
        "edges": {
            "utc_default_visible_difference": "UTC wrapper misses the local 14:00 joint query and scores only day match",
            "invalid_query_code": invalid_query.code,
        },
        "recurrence_rows": recurrence_rows_json(&vault),
        "recurrence_files": files_under(&vault_dir.join("cf").join("recurrence")),
        "wal_files": files_under(&vault_dir.join("wal")),
    });
    fs::write(
        root.join("timezone-alignment-readback.json"),
        serde_json::to_vec_pretty(&report).expect("report json"),
    )
    .expect("write report");

    println!("issue635_timezone_alignment_fsv_root={}", root.display());
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
            std::env::temp_dir().join(format!("calyx-issue635-fsv-{}", std::process::id()))
        })
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().expect("vault id")
}
