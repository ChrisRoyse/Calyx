use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[path = "fsv_support/mod.rs"]
mod fsv_support;
use calyx_anneal::{
    ABRunner, AnnealLedger, AsterAnnealLedgerStore, AsterSoakStorage, CALYX_ASTER_CF_UNAVAILABLE,
    NoopABBudget, SoakConfig, SoakHarness, SoakRowKind, TripwireRegistry,
    decode_anneal_ledger_payload, decode_soak_row,
};
use calyx_aster::cf::{ColumnFamily, ledger_key};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::FixedClock;
use calyx_forge::AutotuneCache;
use calyx_ledger::{ActorId, EntryKind, LedgerAppender, decode as decode_ledger};
use fsv_support::{
    hex, physical_files, read_json, reset_dir, vault_id, write_json, write_manifest,
};
use serde_json::{Value, json};

const FSV_TS: u64 = 1_785_500_417;

#[test]
#[ignore = "requires CALYX_ISSUE417_FSV_ROOT in a manual verification run"]
fn issue417_soak_harness_cf_ledger_and_report_fsv() {
    let root = PathBuf::from(env::var("CALYX_ISSUE417_FSV_ROOT").expect("set FSV root"));
    reset_dir(&root);
    let vault_dir = root.join("vault");
    let vault = open_vault(&vault_dir);
    let cache_path = root.join("autotune-cache.json");
    let queries = env::var("CALYX_ISSUE417_QUERIES")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(1_000_000);

    let before_soak = read_soak_rows(&vault);
    let before_ledger = read_ledger_rows(&vault);
    assert!(before_soak.is_empty());
    assert!(before_ledger.is_empty());

    let runner = make_runner(&vault, &vault_dir, &cache_path);
    let storage = AsterSoakStorage::new(&vault);
    let config = SoakConfig {
        n_queries: queries,
        sample_interval: 1_000,
        max_runtime_ms: Some(2 * 60 * 60 * 1_000),
        ..SoakConfig::default()
    };
    let mut harness = SoakHarness::seeded(
        config,
        AutotuneCache::load(&cache_path).unwrap(),
        runner,
        storage,
    );
    let report = harness.run(&vault).expect("run soak");
    assert!(report.p99_reduction >= 0.20);
    assert!(report.recall_final >= report.recall_baseline);
    assert!(!report.oscillation_detected);
    assert!(!report.promotions.is_empty());
    drop(harness);
    vault.flush().expect("flush soak vault");

    let after_soak = read_soak_rows(&vault);
    let after_ledger = read_ledger_rows(&vault);
    let sample_count = after_soak
        .iter()
        .filter(|row| row["row_kind"] == "sample")
        .count();
    let report_count = after_soak
        .iter()
        .filter(|row| row["row_kind"] == "report")
        .count();
    assert_eq!(report_count, 1);
    assert_eq!(sample_count as u64, queries.div_ceil(1_000));
    assert_eq!(
        after_ledger[0]["payload_json"]["action"],
        "autotune_promote"
    );

    let readback = json!({
        "surface": "anneal.soak_harness",
        "source_of_truth": "Aster anneal_soak CF rows, Aster ledger CF rows, WAL, and persisted AutotuneCache JSON",
        "vault": vault_dir.display().to_string(),
        "queries": queries,
        "before_soak": before_soak,
        "after_soak": after_soak,
        "before_ledger": before_ledger,
        "after_ledger": after_ledger,
        "report": report,
        "cache_exists": cache_path.exists(),
        "cache_json": read_json(&cache_path),
    });
    let readback_path = root.join("soak-fsv-readback.json");
    write_json(&readback_path, &readback);
    let physical_path = root.join("physical-files.txt");
    fs::write(&physical_path, physical_files(&root).join("\n")).unwrap();
    write_manifest(&root, &[readback_path, physical_path]);
}

fn make_runner<'a>(
    vault: &'a AsterVault,
    vault_dir: &Path,
    cache_path: &Path,
) -> ABRunner<
    AnnealLedger<AsterAnnealLedgerStore<'a, calyx_core::SystemClock>, FixedClock>,
    NoopABBudget,
> {
    let store = AsterAnnealLedgerStore::new(vault);
    let appender = LedgerAppender::open(store, FixedClock::new(FSV_TS)).unwrap();
    let ledger =
        AnnealLedger::new(appender, ActorId::Service("calyx-soak-fsv".to_string())).unwrap();
    ABRunner::new(
        TripwireRegistry::load_from_vault(vault_dir).unwrap(),
        ledger,
        NoopABBudget::default(),
        Arc::new(FixedClock::new(FSV_TS)),
    )
    .with_cache(AutotuneCache::load(cache_path).unwrap())
}

fn read_soak_rows(vault: &AsterVault) -> Vec<Value> {
    vault
        .scan_cf_at(vault.latest_seq(), ColumnFamily::AnnealSoak)
        .unwrap_or_else(|error| {
            assert_eq!(error.code, CALYX_ASTER_CF_UNAVAILABLE);
            Vec::new()
        })
        .into_iter()
        .map(|(key, bytes)| {
            let decoded = decode_soak_row(&bytes).expect("decode soak row");
            let (row_kind, payload) = match decoded.row {
                SoakRowKind::Report { report } => ("report", json!(report)),
                SoakRowKind::Sample { sample } => ("sample", json!(sample)),
            };
            json!({
                "key_hex": hex(&key),
                "value_hex": hex(&bytes),
                "run_id": hex(&decoded.run_id),
                "row_kind": row_kind,
                "payload": payload,
            })
        })
        .collect()
}

fn read_ledger_rows(vault: &AsterVault) -> Vec<Value> {
    vault
        .scan_cf_at(vault.latest_seq(), ColumnFamily::Ledger)
        .expect("scan ledger CF")
        .into_iter()
        .map(|(key, bytes)| {
            let entry = decode_ledger(&bytes).expect("decode ledger entry");
            let anneal =
                decode_anneal_ledger_payload(&entry.payload).expect("decode anneal payload");
            assert_eq!(entry.kind, EntryKind::Anneal);
            assert_eq!(key, ledger_key(entry.seq));
            json!({
                "seq": entry.seq,
                "key_hex": hex(&key),
                "entry_hash": hex(&entry.entry_hash),
                "payload_hex": hex(&entry.payload),
                "payload_json": anneal,
            })
        })
        .collect()
}

fn open_vault(vault_dir: &Path) -> AsterVault {
    AsterVault::new_durable(
        vault_dir,
        vault_id(),
        b"issue417-soak".to_vec(),
        VaultOptions::default(),
    )
    .expect("open durable vault")
}
