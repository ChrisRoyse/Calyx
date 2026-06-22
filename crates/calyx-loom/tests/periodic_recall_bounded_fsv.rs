use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use calyx_aster::cf::ColumnFamily;
use calyx_aster::dedup::EpochSecs;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{
    Clock, Constellation, CxFlags, CxId, InputRef, LedgerRef, Modality, VaultId, VaultStore,
};
use calyx_loom::{OccurrenceContext, PeriodicRecallQuery, SeriesStore, decode_recurrence_row};
use serde_json::json;

const TUESDAY_2024_01_02_14H_UTC: i64 = 1_704_204_000;
const WEEK_SECS: i64 = 604_800;

#[test]
#[ignore = "FSV: writes durable vault bytes and readback artifacts under CALYX_FSV_ROOT"]
fn issue636_periodic_recall_bounded_readback_manual_fsv() {
    let root = fsv_root().join("issue636-periodic-recall-bounded");
    fs::create_dir_all(&root).expect("create fsv root");
    let vault_dir = root.join("vault");
    let vault = AsterVault::new_durable(
        &vault_dir,
        vault_id(),
        b"issue636-periodic-bounded-salt".to_vec(),
        VaultOptions::default(),
    )
    .expect("durable vault");
    let before_rows = recurrence_rows_json(&vault);

    let matching = put_base(&vault, b"issue636-weekly-tuesday-14");
    let nonmatching = put_base(&vault, b"issue636-weekly-wednesday-15");
    let single_support = put_base(&vault, b"issue636-single-support");
    let tied = put_base(&vault, b"issue636-tied-buckets");
    let store = SeriesStore::new(&vault);

    append_weekly(&store, matching, TUESDAY_2024_01_02_14H_UTC, 4, "match");
    append_weekly(
        &store,
        nonmatching,
        TUESDAY_2024_01_02_14H_UTC + 25 * 3_600,
        3,
        "nonmatch",
    );
    store
        .append_occurrence(
            single_support,
            EpochSecs(TUESDAY_2024_01_02_14H_UTC),
            ctx("single"),
        )
        .expect("append single support");
    store
        .append_occurrence(tied, EpochSecs(TUESDAY_2024_01_02_14H_UTC), ctx("tied-a"))
        .expect("append tied a");
    store
        .append_occurrence(
            tied,
            EpochSecs(TUESDAY_2024_01_02_14H_UTC + 3_600),
            ctx("tied-b"),
        )
        .expect("append tied b");
    vault.flush().expect("flush durable recurrence rows");

    let after_rows = recurrence_rows_json(&vault);
    let query = PeriodicRecallQuery::new(Some(14), Some(1)).expect("joint query");
    let readback = store
        .periodic_recall_readback(query)
        .expect("periodic readback");
    assert_eq!(readback.hits.len(), 1);
    assert_eq!(readback.hits[0].cx_id, matching);
    assert_eq!(readback.stats.index_rows_visited, after_rows.len());
    assert_eq!(readback.stats.candidate_series_count, 4);
    assert_eq!(readback.stats.series_read_count, 4);
    assert_eq!(readback.stats.series_range_rows_visited, after_rows.len());
    assert_eq!(readback.stats.series_rows_decoded, after_rows.len());

    let legacy_full_scan_rows_if_per_series =
        readback.stats.index_rows_visited * readback.stats.candidate_series_count;
    assert!(legacy_full_scan_rows_if_per_series > readback.stats.series_range_rows_visited);

    let matching_read = store.recurrence_series(matching).expect("matching read");
    let nonmatching_read = store
        .recurrence_series(nonmatching)
        .expect("nonmatching read");
    let single_read = store
        .recurrence_series(single_support)
        .expect("single support read");
    let tied_read = store.recurrence_series(tied).expect("tied read");
    assert_eq!(matching_read.read_stats.range_scan_rows, 4);
    assert_eq!(nonmatching_read.read_stats.range_scan_rows, 3);
    assert_eq!(single_read.read_stats.range_scan_rows, 1);
    assert_eq!(tied_read.read_stats.range_scan_rows, 2);

    let hour_only = store
        .periodic_recall_readback(PeriodicRecallQuery::new(Some(14), None).expect("hour query"))
        .expect("hour-only readback");
    assert_eq!(hour_only.hits.len(), 1);
    assert_eq!(hour_only.hits[0].cx_id, matching);

    let empty_vault_dir = root.join("empty-vault");
    let empty_vault = AsterVault::new_durable(
        &empty_vault_dir,
        vault_id(),
        b"issue636-empty-salt".to_vec(),
        VaultOptions::default(),
    )
    .expect("empty durable vault");
    let empty_store = SeriesStore::new(&empty_vault);
    let empty_readback = empty_store
        .periodic_recall_readback(query)
        .expect("empty readback");
    assert!(empty_readback.hits.is_empty());
    assert_eq!(empty_readback.stats.index_rows_visited, 0);
    assert_eq!(empty_readback.stats.candidate_series_count, 0);
    assert_eq!(empty_readback.stats.series_read_count, 0);

    let invalid_query = PeriodicRecallQuery::new(Some(24), None).expect_err("invalid query");
    assert_eq!(
        invalid_query.code,
        calyx_core::CALYX_TEMPORAL_INVALID_PERIOD
    );

    let report = json!({
        "issue": 636,
        "vault_dir": vault_dir.display().to_string(),
        "before": {
            "recurrence_rows": before_rows,
        },
        "after": {
            "recurrence_rows": after_rows,
            "recurrence_file_manifest": files_under(&vault_dir.join("cf").join("recurrence")),
            "wal_file_manifest": files_under(&vault_dir.join("wal")),
        },
        "query": {
            "target_hour": 14,
            "target_day_of_week": 1,
        },
        "readback": readback,
        "per_series_range_rows": {
            "matching": matching_read.read_stats,
            "nonmatching": nonmatching_read.read_stats,
            "single_support": single_read.read_stats,
            "tied": tied_read.read_stats,
        },
        "edges": {
            "hour_only": hour_only,
            "empty_vault": empty_readback,
            "invalid_query_code": invalid_query.code,
            "legacy_full_scan_rows_if_per_series": legacy_full_scan_rows_if_per_series,
            "range_scan_saved_rows": legacy_full_scan_rows_if_per_series - readback.stats.series_range_rows_visited,
        },
    });
    fs::write(
        root.join("periodic-recall-bounded-readback.json"),
        serde_json::to_vec_pretty(&report).expect("report json"),
    )
    .expect("write report");

    println!(
        "issue636_periodic_recall_bounded_fsv_root={}",
        root.display()
    );
}

fn append_weekly<C: Clock>(
    store: &SeriesStore<'_, C>,
    cx_id: CxId,
    start_secs: i64,
    count: usize,
    label: &str,
) {
    for week in 0..count {
        store
            .append_occurrence(
                cx_id,
                EpochSecs(start_secs + week as i64 * WEEK_SECS),
                ctx(label),
            )
            .expect("append weekly occurrence");
    }
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
            std::env::temp_dir().join(format!("calyx-issue636-fsv-{}", std::process::id()))
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
