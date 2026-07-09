use super::*;
use crate::erase::NoopEraseHandler;

#[test]
fn issue504_retention_fsv_fixture() {
    let root = calyx_fsv::fsv_root_or_else("CALYX_FSV_ROOT", || {
        std::env::temp_dir().join("calyx-issue504-fsv")
    });
    fs::create_dir_all(&root).unwrap();
    let vault_dir = root.join("issue504-retention-vault");
    if vault_dir.exists() {
        fs::remove_dir_all(&vault_dir).unwrap();
    }
    let vault = AsterVault::new_durable(
        &vault_dir,
        vault_id(),
        b"retention-salt",
        crate::vault::VaultOptions::default(),
    )
    .unwrap();
    let mut ctx = context();
    let registry = EraseRegistry::new();
    let expired_a = cx(&vault, b"FSV_ISSUE504_EXPIRED_A", Some("100000"));
    let expired_b = cx(&vault, b"FSV_ISSUE504_EXPIRED_B", Some("90000"));
    let live = cx(&vault, b"FSV_ISSUE504_RETAINED", Some("150000"));
    let expired_a_id = expired_a.cx_id;
    let expired_b_id = expired_b.cx_id;
    let live_id = live.cx_id;
    vault.put(expired_a).unwrap();
    vault.put(expired_b).unwrap();
    vault.put(live).unwrap();

    println!("FSV_ISSUE504_VAULT={}", vault_dir.display());
    println!("FSV_ISSUE504_EXPIRED_A={expired_a_id}");
    println!("FSV_ISSUE504_EXPIRED_B={expired_b_id}");
    println!("FSV_ISSUE504_LIVE={live_id}");
    println!(
        "FSV_ISSUE504_BASE_BEFORE={}",
        vault
            .scan_cf_at(vault.snapshot(), ColumnFamily::Base)
            .unwrap()
            .len()
    );
    println!(
        "FSV_ISSUE504_EXPIRED_BEFORE={}",
        scan_expired_cxs(&vault, &ctx, &store(60), NOW_MS)
            .unwrap()
            .len()
    );

    let results = apply_retention(&vault, &mut ctx, &store(60), &registry, NOW_MS).unwrap();

    println!("FSV_ISSUE504_ERASED={}", results.len());
    println!(
        "FSV_ISSUE504_BASE_AFTER={}",
        vault
            .scan_cf_at(vault.snapshot(), ColumnFamily::Base)
            .unwrap()
            .len()
    );
    println!(
        "FSV_ISSUE504_TOMBSTONES_AFTER={}",
        ledger_tombstone_count(&vault)
    );
    println!(
        "FSV_ISSUE504_LIVE_PRESENT={}",
        vault.get(live_id, vault.snapshot()).is_ok()
    );
    println!(
        "FSV_ISSUE504_EXPIRED_A_PRESENT={}",
        vault.get(expired_a_id, vault.snapshot()).is_ok()
    );

    retention_edge_fsv(&root);
}

fn retention_edge_fsv(root: &std::path::Path) {
    let registry = EraseRegistry::new();
    let edge_dir = root.join("issue504-no-policy");
    if edge_dir.exists() {
        fs::remove_dir_all(&edge_dir).unwrap();
    }
    let edge = AsterVault::new_durable(
        &edge_dir,
        vault_id(),
        b"retention-salt",
        crate::vault::VaultOptions::default(),
    )
    .unwrap();
    let mut edge_ctx = context();
    let edge_row = cx(&edge, b"FSV_ISSUE504_NO_POLICY", Some("1"));
    let edge_id = edge_row.cx_id;
    edge.put(edge_row).unwrap();
    println!(
        "FSV_ISSUE504_NO_POLICY_ERASED={}",
        apply_retention(
            &edge,
            &mut edge_ctx,
            &RetentionStore::new(),
            &registry,
            NOW_MS
        )
        .unwrap()
        .len()
    );
    println!(
        "FSV_ISSUE504_NO_POLICY_PRESENT={}",
        edge.get(edge_id, edge.snapshot()).is_ok()
    );
    println!(
        "FSV_ISSUE504_ZERO_TTL_ERASED={}",
        apply_retention(&edge, &mut edge_ctx, &store(0), &registry, NOW_MS)
            .unwrap()
            .len()
    );
    println!(
        "FSV_ISSUE504_ZERO_TTL_PRESENT={}",
        edge.get(edge_id, edge.snapshot()).is_ok()
    );

    let bad_dir = root.join("issue504-invalid-ts");
    if bad_dir.exists() {
        fs::remove_dir_all(&bad_dir).unwrap();
    }
    let bad = AsterVault::new_durable(
        &bad_dir,
        vault_id(),
        b"retention-salt",
        crate::vault::VaultOptions::default(),
    )
    .unwrap();
    let bad_ctx = context();
    let bad_row = cx(&bad, b"FSV_ISSUE504_BAD_TS", Some("bad-ts"));
    let bad_id = bad_row.cx_id;
    bad.put(bad_row).unwrap();
    let bad_error = scan_expired_cxs(&bad, &bad_ctx, &store(60), NOW_MS).unwrap_err();
    println!("FSV_ISSUE504_INVALID_TS_ERROR={}", bad_error.code);
    println!(
        "FSV_ISSUE504_INVALID_TS_PRESENT={}",
        bad.get(bad_id, bad.snapshot()).is_ok()
    );

    let all_dir = root.join("issue504-all-expired");
    if all_dir.exists() {
        fs::remove_dir_all(&all_dir).unwrap();
    }
    let all = AsterVault::new_durable(
        &all_dir,
        vault_id(),
        b"retention-salt",
        crate::vault::VaultOptions::default(),
    )
    .unwrap();
    let mut all_ctx = context();
    all.put(cx(&all, b"FSV_ISSUE504_ALL_A", Some("1"))).unwrap();
    all.put(cx(&all, b"FSV_ISSUE504_ALL_B", Some("2"))).unwrap();
    println!(
        "FSV_ISSUE504_ALL_EXPIRED_BEFORE={}",
        scan_expired_cxs(&all, &all_ctx, &store(60), NOW_MS)
            .unwrap()
            .len()
    );
    let all_results = apply_retention(&all, &mut all_ctx, &store(60), &registry, NOW_MS).unwrap();
    println!("FSV_ISSUE504_ALL_EXPIRED_ERASED={}", all_results.len());
    println!(
        "FSV_ISSUE504_ALL_EXPIRED_BASE_AFTER={}",
        all.scan_cf_at(all.snapshot(), ColumnFamily::Base)
            .unwrap()
            .len()
    );

    let mut noop_registry = EraseRegistry::new();
    noop_registry.add_handler(NoopEraseHandler);
    println!("FSV_ISSUE504_NOOP_HANDLER_REGISTERED=true");
}
