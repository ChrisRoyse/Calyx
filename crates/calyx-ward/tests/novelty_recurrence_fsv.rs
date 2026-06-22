use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use calyx_aster::cf::ColumnFamily;
use calyx_aster::dedup::EpochSecs;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{CxId, FixedClock, VaultStore};
use calyx_ward::{
    CALYX_WARD_INVALID_FREQUENCY, CALYX_WARD_MISSING_FREQUENCY, Domain, NoveltySignal,
    SurpriseScore, classify_novelty, novelty_action_for_signal, overdue_recurrence_scan,
    surprise_bits,
};
use serde_json::{Value, json};

#[path = "novelty_recurrence_support/mod.rs"]
mod novelty_recurrence_support;
use novelty_recurrence_support::{append_times, cx, put_base, vault_id};

#[test]
#[ignore = "FSV trigger writes durable manual evidence under CALYX_WARD_ISSUE390_FSV_DIR"]
fn issue390_ward_novelty_recurrence_fsv_writes_artifacts() {
    let root = PathBuf::from(
        env::var("CALYX_WARD_ISSUE390_FSV_DIR").expect("set CALYX_WARD_ISSUE390_FSV_DIR"),
    );
    reset_dir(&root);
    let vault_dir = root.join("vault");
    let vault = AsterVault::new_durable(
        &vault_dir,
        vault_id(),
        b"issue390-ward-novelty-recurrence-fsv".to_vec(),
        VaultOptions::default(),
    )
    .expect("open durable vault");
    let singleton = cx(1);
    let common = cx(2);
    let overdue = cx(3);
    let zero = cx(4);
    let missing = cx(5);
    let invalid = cx(6);
    put_base(&vault, singleton, Some(1.0));
    put_base(&vault, common, None);
    put_base(&vault, overdue, None);
    put_base(&vault, zero, Some(0.0));
    append_times(
        &vault,
        common,
        &(0..20).map(|idx| 1_150 + (idx * 10)).collect::<Vec<_>>(),
    );
    append_times(
        &vault,
        overdue,
        &[100, 200, 300, 400, 500, 600, 700, 800, 900, 1_000],
    );
    vault.flush().expect("flush setup");
    let before = raw_state(&vault);
    let clock = FixedClock::new(1_350_000);
    let domain = Domain::new("issue390-domain", vec![singleton, common]);
    let singleton_signal = classify_novelty(singleton, &vault, &clock).expect("singleton");
    let overdue_signal = classify_novelty(overdue, &vault, &clock).expect("overdue");
    let surprise = surprise_bits(singleton, &domain, &vault).expect("surprise");
    let anomaly = NoveltySignal::Anomaly {
        surprise_bits: surprise,
    };
    let after_happy = raw_state(&vault);
    assert_eq!(singleton_signal, NoveltySignal::NonRecurring);
    assert_eq!(
        overdue_signal,
        NoveltySignal::OverdueRecurrence {
            expected_t: EpochSecs(1_100),
            overdue_by_secs: 250
        }
    );
    assert!((surprise.get() - 4.392_317).abs() < 1e-5);

    let zero_frequency = edge_zero_frequency(&vault, zero, &clock);
    let empty_domain = edge_empty_domain(&vault, singleton);
    let missing_frequency = edge_missing_frequency(&vault, missing, &clock);
    let invalid_frequency = edge_invalid_frequency(&vault, invalid, &clock);
    let after_edges = raw_state(&vault);
    let overdue_rows = overdue_recurrence_scan(
        &Domain::new("overdue-scan", vec![singleton, common, overdue]),
        &vault,
        &clock,
    )
    .expect("overdue scan");
    assert_eq!(overdue_rows.len(), 1);

    let artifact = json!({
        "schema_version": 1,
        "surface": "ward-novelty",
        "artifact_kind": "ph42.ward-novelty.v1",
        "source_of_truth": "PH42 persisted artifact",
        "issue": 390,
        "trigger": {
            "singleton": "classify_novelty(singleton, vault, fixed_clock)",
            "overdue": "classify_novelty(overdue, vault, fixed_clock)",
            "surprise": "surprise_bits(singleton, domain, vault)"
        },
        "hand_computed_expected": {
            "singleton_frequency": 1,
            "singleton_signal": "non_recurring",
            "overdue_frequency": 10,
            "overdue_last_t": 1000,
            "overdue_cadence_secs": 100.0,
            "overdue_expected_t": 1100,
            "overdue_by_secs": 250,
            "domain_total_events": 21,
            "singleton_surprise_bits": -(1.0_f32 / 21.0).ln() / 2.0_f32.ln(),
            "surprise_f32_be_hex": f32_hex(surprise.get())
        },
        "signals": {
            "singleton": singleton_signal,
            "overdue": overdue_signal,
            "anomaly": anomaly
        },
        "actions": {
            "singleton": novelty_action_for_signal(&singleton_signal),
            "anomaly": novelty_action_for_signal(&anomaly)
        },
        "overdue_scan": overdue_rows,
        "source_of_truth_bytes": {
            "vault_dir": vault_dir.display().to_string(),
            "before": before,
            "after_happy": after_happy,
            "after_edges": after_edges
        },
        "edges": {
            "zero_frequency": zero_frequency,
            "empty_domain": empty_domain,
            "missing_frequency": missing_frequency,
            "invalid_frequency": invalid_frequency
        }
    });
    let artifact_path = root.join("ward-novelty.json");
    write_json(&artifact_path, &artifact);
    write_blake3_manifest(&root, std::slice::from_ref(&artifact_path));
    println!("issue390_fsv_root={}", root.display());
    println!("issue390_ward_novelty={}", artifact_path.display());
    println!("{}", serde_json::to_string_pretty(&artifact).unwrap());
}

fn edge_zero_frequency(vault: &AsterVault, id: CxId, clock: &FixedClock) -> Value {
    let before = raw_state(vault);
    let signal = classify_novelty(id, vault, clock).expect("zero frequency");
    let after = raw_state(vault);
    assert_eq!(signal, NoveltySignal::NonRecurring);
    json!({ "frequency": 0, "signal": signal, "before": before, "after": after })
}

fn edge_empty_domain(vault: &AsterVault, singleton: CxId) -> Value {
    let before = raw_state(vault);
    let score = surprise_bits(singleton, &Domain::new("empty", Vec::new()), vault).unwrap();
    let after = raw_state(vault);
    assert_eq!(score, SurpriseScore::new(0.0).unwrap());
    json!({ "score": score, "before": before, "after": after })
}

fn edge_missing_frequency(vault: &AsterVault, id: CxId, clock: &FixedClock) -> Value {
    put_base(vault, id, None);
    let before = raw_state(vault);
    let error = classify_novelty(id, vault, clock).expect_err("missing frequency");
    let after = raw_state(vault);
    assert_eq!(error.code(), CALYX_WARD_MISSING_FREQUENCY);
    json!({ "expected_code": CALYX_WARD_MISSING_FREQUENCY, "actual_code": error.code(), "before": before, "after": after })
}

fn edge_invalid_frequency(vault: &AsterVault, id: CxId, clock: &FixedClock) -> Value {
    put_base(vault, id, Some(1.5));
    let before = raw_state(vault);
    let error = classify_novelty(id, vault, clock).expect_err("invalid frequency");
    let after = raw_state(vault);
    assert_eq!(error.code(), CALYX_WARD_INVALID_FREQUENCY);
    json!({ "expected_code": CALYX_WARD_INVALID_FREQUENCY, "actual_code": error.code(), "before": before, "after": after })
}

fn raw_state(vault: &AsterVault) -> Value {
    json!({
        "snapshot": vault.snapshot(),
        "base": raw_rows(vault, ColumnFamily::Base),
        "recurrence": raw_rows(vault, ColumnFamily::Recurrence),
        "ledger": raw_rows(vault, ColumnFamily::Ledger)
    })
}

fn raw_rows(vault: &AsterVault, cf: ColumnFamily) -> Value {
    let rows = vault.scan_cf_at(vault.snapshot(), cf).expect("scan cf");
    json!({
        "row_count": rows.len(),
        "rows": rows.iter().map(|(key, value)| {
            json!({ "key_hex": hex(key), "value_hex": hex(value) })
        }).collect::<Vec<_>>()
    })
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
        fs::remove_dir_all(path).expect("remove old fsv root");
    }
    fs::create_dir_all(path).expect("create fsv root");
}

fn f32_hex(value: f32) -> String {
    hex(&value.to_be_bytes())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
