use super::*;

#[test]
fn open_rejects_non_empty_durable_ledger_without_head_anchor() {
    let root = test_vault_dir("issue1395-missing-head-anchor");
    let vault_id = vault_id();
    let vault = AsterVault::new_durable(
        &root,
        vault_id,
        b"issue1395-missing-anchor-salt",
        VaultOptions::default(),
    )
    .expect("open durable vault");
    for seed in 0..3 {
        vault
            .put(sample_constellation(vault_id, seed))
            .expect("put sample row");
    }
    vault.flush().expect("flush physical rows");
    drop(vault);
    let anchor_path = crate::ledger_head::head_anchor_path(&root);
    assert!(
        anchor_path.exists(),
        "test must create a durable head anchor"
    );
    fs::remove_file(anchor_path).expect("remove head anchor");

    let error = AsterLedgerCfStore::open(&root).unwrap_err();

    assert_eq!(error.code, "CALYX_LEDGER_CHAIN_BROKEN");
    assert!(error.message.contains("head anchor missing"));
    fs::remove_dir_all(root).ok();
}
