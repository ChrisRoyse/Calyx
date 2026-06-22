use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use calyx_aster::cf::ColumnFamily;
use calyx_aster::dedup::EpochSecs;
use calyx_aster::recurrence::FREQUENCY_SCALAR;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{
    Constellation, CxFlags, CxId, InputRef, LedgerRef, Modality, VaultId, VaultStore,
};
use calyx_loom::{
    CALYX_LOOM_SERIES_READ_ERROR, OccurrenceContext, SeriesStore, decode_lead_lag_result,
    temporal_cross_term,
};
use serde_json::{Value, json};

const LEAD_LAG_F64_OFFSET: usize = 37;

#[test]
#[ignore = "FSV trigger writes durable manual evidence under CALYX_LOOM_ISSUE388_FSV_DIR"]
fn issue388_temporal_cross_term_fsv_writes_artifact() {
    let root = PathBuf::from(
        env::var("CALYX_LOOM_ISSUE388_FSV_DIR").expect("set CALYX_LOOM_ISSUE388_FSV_DIR"),
    );
    reset_dir(&root);
    let vault_dir = root.join("vault");
    let vault = AsterVault::new_durable(
        &vault_dir,
        vault_id(),
        b"issue388-temporal-xterm-fsv".to_vec(),
        VaultOptions::default(),
    )
    .expect("open durable vault");

    let a = cx(1);
    let b = cx(2);
    put_base(&vault, a, 0.0);
    put_base(&vault, b, 0.0);
    append_times(&vault, a, &[100, 200, 300, 400, 500]);
    append_times(&vault, b, &[115, 215, 315, 415, 515]);
    vault.flush().expect("flush before happy");
    let before = raw_state(&vault);

    let happy = temporal_cross_term(a, b, &vault, 30)
        .expect("happy temporal xterm")
        .expect("happy result");
    vault.flush().expect("flush happy");
    let row = vault
        .read_temporal_xterm(vault.latest_seq(), a, b)
        .expect("read temporal xterm")
        .expect("stored row");
    let decoded = decode_lead_lag_result(&row).expect("decode stored row");
    assert_eq!(happy.lead_lag_secs, 15.0);
    assert_eq!(happy.n_pairs, 5);
    assert_eq!(decoded, happy);
    assert_eq!(
        &row[LEAD_LAG_F64_OFFSET..LEAD_LAG_F64_OFFSET + 8],
        &15.0_f64.to_be_bytes()
    );
    let after_happy = raw_state(&vault);

    let edges = json!({
        "window_zero": edge_window_zero(&vault),
        "self_correlation": edge_self_correlation(&vault, a),
        "insufficient_pairs": edge_insufficient(&vault),
        "series_read_error": edge_series_read_error(&vault),
    });
    let after_edges = raw_state(&vault);

    let artifact = json!({
        "schema_version": 1,
        "surface": "temporal-cross-term",
        "artifact_kind": "ph42.temporal-cross-term.v1",
        "source_of_truth": "PH42 persisted artifact",
        "issue": 388,
        "cx_a": a.to_string(),
        "cx_b": b.to_string(),
        "lead_lag_secs": happy.lead_lag_secs,
        "n_pairs": happy.n_pairs,
        "proximity_window_secs": happy.proximity_window_secs,
        "hand_computed_expected": {
            "a_times": [100, 200, 300, 400, 500],
            "b_times": [115, 215, 315, 415, 515],
            "deltas_secs": [15, 15, 15, 15, 15],
            "median_delta_secs": 15.0,
            "n_pairs": 5
        },
        "trigger": {
            "operation": "temporal_cross_term(cx_a, cx_b, vault, window_secs=30)",
            "intended_outcome": "temporal_xterm CF row persisted under key cx_a||cx_b"
        },
        "source_of_truth_bytes": {
            "vault_dir": vault_dir.display().to_string(),
            "before": before,
            "after_happy": after_happy,
            "after_edges": after_edges,
            "temporal_xterm_key_hex": temporal_key_hex(a, b),
            "temporal_xterm_value_hex": hex(&row),
            "temporal_xterm_value_len": row.len(),
            "lead_lag_f64_offset": LEAD_LAG_F64_OFFSET,
            "lead_lag_f64_be_hex": hex(&row[LEAD_LAG_F64_OFFSET..LEAD_LAG_F64_OFFSET + 8])
        },
        "edges": edges
    });

    let artifact_path = root.join("temporal-cross-term.json");
    write_json(&artifact_path, &artifact);
    write_blake3_manifest(&root, std::slice::from_ref(&artifact_path));
    println!("issue388_fsv_root={}", root.display());
    println!("issue388_temporal_cross_term={}", artifact_path.display());
    println!("{}", serde_json::to_string_pretty(&artifact).unwrap());
}

fn edge_window_zero(vault: &AsterVault) -> Value {
    let before = raw_temporal_xterm(vault);
    let result = temporal_cross_term(cx(1), cx(2), vault, 0).expect("window zero");
    vault.flush().expect("flush window zero");
    let after = raw_temporal_xterm(vault);
    assert!(result.is_none());
    json!({
        "expected": "None because strict proximity with window=0 admits no pairs",
        "actual": result.map(|value| value.lead_lag_secs),
        "before": before,
        "after": after
    })
}

fn edge_self_correlation(vault: &AsterVault, id: CxId) -> Value {
    let before = raw_temporal_xterm(vault);
    let result = temporal_cross_term(id, id, vault, 30)
        .expect("self correlation")
        .expect("self result");
    vault.flush().expect("flush self edge");
    let stored = vault
        .read_temporal_xterm(vault.latest_seq(), id, id)
        .expect("read self row");
    let after = raw_temporal_xterm(vault);
    assert_eq!(result.lead_lag_secs, 0.0);
    assert!(stored.is_none());
    json!({
        "expected": "0.0 self-correlation, not stored",
        "actual": {
            "lead_lag_secs": result.lead_lag_secs,
            "n_pairs": result.n_pairs,
            "stored_row_present": stored.is_some()
        },
        "before": before,
        "after": after
    })
}

fn edge_insufficient(vault: &AsterVault) -> Value {
    let a = cx(20);
    let b = cx(21);
    put_base(vault, a, 0.0);
    put_base(vault, b, 0.0);
    append_times(vault, a, &[100, 200, 300, 400, 500]);
    append_times(vault, b, &[105, 205]);
    vault.flush().expect("flush insufficient setup");
    let before = raw_temporal_xterm(vault);
    let result = temporal_cross_term(a, b, vault, 10).expect("insufficient pairs");
    vault.flush().expect("flush insufficient edge");
    let after = raw_temporal_xterm(vault);
    assert!(result.is_none());
    json!({
        "expected": "None because n_pairs=2 is below the minimum of 3",
        "actual": result.map(|value| value.n_pairs),
        "before": before,
        "after": after
    })
}

fn edge_series_read_error(vault: &AsterVault) -> Value {
    let bad = cx(30);
    let other = cx(31);
    put_base(vault, bad, 1.5);
    put_base(vault, other, 0.0);
    vault.flush().expect("flush read-error setup");
    let before = raw_temporal_xterm(vault);
    let error = temporal_cross_term(bad, other, vault, 30).expect_err("series read error");
    vault.flush().expect("flush read-error edge");
    let after = raw_temporal_xterm(vault);
    assert_eq!(error.code, CALYX_LOOM_SERIES_READ_ERROR);
    json!({
        "expected_code": CALYX_LOOM_SERIES_READ_ERROR,
        "actual_code": error.code,
        "message": error.message,
        "before": before,
        "after": after
    })
}

fn raw_state(vault: &AsterVault) -> Value {
    json!({
        "snapshot": vault.latest_seq(),
        "base": raw_rows(vault, ColumnFamily::Base),
        "recurrence": raw_rows(vault, ColumnFamily::Recurrence),
        "temporal_xterm": raw_temporal_xterm(vault),
        "ledger": raw_rows(vault, ColumnFamily::Ledger)
    })
}

fn raw_temporal_xterm(vault: &AsterVault) -> Value {
    raw_rows(vault, ColumnFamily::TemporalXTerm)
}

fn raw_rows(vault: &AsterVault, cf: ColumnFamily) -> Value {
    let rows = vault.scan_cf_at(vault.latest_seq(), cf).expect("scan cf");
    json!({
        "row_count": rows.len(),
        "rows": rows.iter().map(|(key, value)| {
            json!({ "key_hex": hex(key), "value_hex": hex(value) })
        }).collect::<Vec<_>>()
    })
}

fn append_times(vault: &AsterVault, cx_id: CxId, times: &[i64]) {
    let store = SeriesStore::new(vault);
    for time in times {
        store
            .append_occurrence(
                cx_id,
                EpochSecs(*time),
                OccurrenceContext::new(format!("t={time}").into_bytes()).unwrap(),
            )
            .expect("append occurrence");
    }
}

fn put_base(vault: &AsterVault, cx_id: CxId, frequency: f64) {
    let mut cx = base_cx(cx_id);
    cx.scalars.insert(FREQUENCY_SCALAR.to_string(), frequency);
    vault.put(cx).expect("put base");
}

fn base_cx(cx_id: CxId) -> Constellation {
    Constellation {
        cx_id,
        vault_id: vault_id(),
        panel_version: 42,
        created_at: 1_786_406_600,
        input_ref: InputRef {
            hash: [cx_id.to_bytes()[0]; 32],
            pointer: None,
            redacted: false,
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
        flags: CxFlags::default(),
    }
}

fn write_json(path: &Path, value: &Value) {
    fs::write(path, serde_json::to_vec_pretty(value).expect("json")).expect("write json");
}

fn write_blake3_manifest(root: &Path, paths: &[PathBuf]) {
    let mut manifest = String::new();
    for path in paths {
        let bytes = fs::read(path).expect("read artifact");
        let name = path.file_name().unwrap().to_string_lossy();
        manifest.push_str(&format!("{}  {name}\n", blake3::hash(&bytes).to_hex()));
    }
    fs::write(root.join("BLAKE3SUMS.txt"), manifest).expect("write blake3 manifest");
}

fn reset_dir(path: &Path) {
    if path.exists() {
        fs::remove_dir_all(path).expect("reset fsv dir");
    }
    fs::create_dir_all(path).expect("create fsv dir");
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn temporal_key_hex(cx_a: CxId, cx_b: CxId) -> String {
    let mut bytes = Vec::with_capacity(32);
    bytes.extend_from_slice(cx_a.as_bytes());
    bytes.extend_from_slice(cx_b.as_bytes());
    hex(&bytes)
}

fn cx(seed: u8) -> CxId {
    CxId::from_bytes([seed; 16])
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV"
        .parse()
        .expect("valid vault id")
}
