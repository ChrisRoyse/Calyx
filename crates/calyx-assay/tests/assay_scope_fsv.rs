use std::fs;
use std::path::PathBuf;

use calyx_assay::{AssayCacheKey, AssayStore, AssaySubject, EstimatorKind, MiEstimate, TrustTag};
use calyx_aster::cf::{CfRouter, ColumnFamily};
use calyx_core::{AnchorKind, SlotId, VaultId};
use serde_json::json;

#[test]
fn assay_cache_scope_separates_vaults_and_anchors() {
    let (readback, loaded) = scoped_store_probe();

    assert_eq!(loaded.len(), 3);
    assert_eq!(readback["same_panel_shard_subject_rows"], 3);
    assert_close(readback["vault_a_reward_bits"].as_f64().unwrap(), 0.31);
    assert_close(readback["vault_b_reward_bits"].as_f64().unwrap(), 0.32);
    assert_close(readback["vault_a_label_bits"].as_f64().unwrap(), 0.33);
    assert_eq!(readback["bad_key_error"], "CALYX_ASTER_CORRUPT_SHARD");
}

#[test]
#[ignore = "manual FSV writes source-of-truth artifacts"]
fn assay_scope_manual_fsv() {
    let (readback, _) = scoped_store_probe();
    let root = fsv_root();
    fs::create_dir_all(&root).unwrap();
    let path = root.join("assay-scope-readback.json");
    fs::write(&path, serde_json::to_vec_pretty(&readback).unwrap()).unwrap();
    println!("ASSAY_SCOPE_READBACK={}", path.display());
}

fn scoped_store_probe() -> (serde_json::Value, AssayStore) {
    let dir = fsv_root().join(format!("assay-scope-cf-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    let mut router = CfRouter::open(&dir, 1_048_576).unwrap();
    let mut store = AssayStore::default();
    let subject = AssaySubject::Lens {
        slot: SlotId::new(2),
    };
    let key_a = AssayCacheKey::scoped(7, "shared", vault_a(), AnchorKind::Reward);
    let key_b = AssayCacheKey::scoped(7, "shared", vault_b(), AnchorKind::Reward);
    let key_c = AssayCacheKey::scoped(
        7,
        "shared",
        vault_a(),
        AnchorKind::Label("gold".to_string()),
    );

    store.put(key_a.clone(), subject.clone(), estimate(0.31), "vault-a", 1);
    store.put(key_b.clone(), subject.clone(), estimate(0.32), "vault-b", 2);
    store.put(key_c.clone(), subject.clone(), estimate(0.33), "label", 3);
    let persisted_rows = store.persist_to_aster(&mut router).unwrap();
    let loaded = AssayStore::load_from_aster(&router).unwrap();
    let raw_cf_rows = router.iter_cf(ColumnFamily::Assay).unwrap().len();
    let bad_key_error = bad_key_error(&dir);

    let readback = json!({
        "source_of_truth": "Aster Assay CF rows loaded by scoped AssayCacheKey",
        "cf_root": dir.join("cf/assay").display().to_string(),
        "same_panel_shard_subject_rows": loaded.len(),
        "persisted_rows": persisted_rows,
        "raw_cf_rows": raw_cf_rows,
        "vault_a_reward_hit": loaded.cache_hit(&key_a, &subject),
        "vault_b_reward_hit": loaded.cache_hit(&key_b, &subject),
        "vault_a_label_hit": loaded.cache_hit(&key_c, &subject),
        "vault_a_reward_bits": loaded.get(&key_a, &subject).unwrap().estimate.bits,
        "vault_b_reward_bits": loaded.get(&key_b, &subject).unwrap().estimate.bits,
        "vault_a_label_bits": loaded.get(&key_c, &subject).unwrap().estimate.bits,
        "bad_key_error": bad_key_error,
    });
    (readback, loaded)
}

fn bad_key_error(parent: &std::path::Path) -> String {
    let dir = parent.with_extension("bad-key");
    let _ = fs::remove_dir_all(&dir);
    let mut router = CfRouter::open(&dir, 1_048_576).unwrap();
    let mut store = AssayStore::default();
    store.put(
        AssayCacheKey::scoped(7, "shared", vault_a(), AnchorKind::Reward),
        AssaySubject::Lens {
            slot: SlotId::new(2),
        },
        estimate(0.42),
        "bad-key",
        9,
    );
    let row = store.rows().into_iter().next().unwrap();
    router
        .put(
            ColumnFamily::Assay,
            b"wrong-assay-key",
            &serde_json::to_vec(&row).unwrap(),
        )
        .unwrap();
    router.flush_cf(ColumnFamily::Assay).unwrap();
    AssayStore::load_from_aster(&router)
        .unwrap_err()
        .code
        .to_string()
}

fn estimate(bits: f32) -> MiEstimate {
    MiEstimate::new(
        bits,
        bits - 0.01,
        bits + 0.01,
        120,
        EstimatorKind::LogisticProbe,
        TrustTag::Trusted,
    )
}

fn fsv_root() -> PathBuf {
    std::env::var("CALYX_FSV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("calyx-assay-scope-fsv"))
}

fn vault_a() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}

fn vault_b() -> VaultId {
    "01BX5ZZKBKACTAV9WEVGEMMVS0".parse().unwrap()
}

fn assert_close(actual: f64, expected: f64) {
    assert!((actual - expected).abs() <= 1.0e-6);
}
