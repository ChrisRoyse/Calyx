use std::path::{Path, PathBuf};

use proptest::prelude::*;
use rusqlite::{Connection, params};

use super::dual_write::replay_existing_sqlite;
use super::panel_guard_enable::{PanelGuardEnable, PanelSpec};
use super::production_fsv::{
    CALYX_PG_STATE_CHANGED, CALYX_PG_WRITE_ATTEMPTED, CALYX_REPRODUCE_MISMATCH,
    CALYX_VAULT_NOT_CALYX_ONLY, CALYX_VAULT_NOT_IN_PG, PgConn, ProductionFSV, REQUIRED_TABLES,
    snapshot_pg_state,
};
use super::read_flip::ReadFlip;
use super::shadow_harness::{ShadowVault, VaultMode, read_shadow_manifest};
use super::shadow_removal::{DefaultPanels, ShadowRemoval, VaultType};

#[test]
fn verify_pg_unchanged_accepts_matching_snapshots() {
    let root = temp_root("pg-same");
    let before = root.join("before-dump");
    let after = root.join("after-dump");
    write_pg_dump(&before, "prod_vault", None);
    write_pg_dump(&after, "prod_vault", None);

    let before_snapshot = snapshot_pg_state(
        &PgConn::DumpDir { root: before },
        "prod_vault",
        &root.join("before"),
    )
    .unwrap();
    let after_snapshot = snapshot_pg_state(
        &PgConn::DumpDir { root: after },
        "prod_vault",
        &root.join("after"),
    )
    .unwrap();
    let proof = ProductionFSV::verify_pg_unchanged(&before_snapshot, &after_snapshot).unwrap();

    assert_eq!(proof.matched_tables, REQUIRED_TABLES.len());
    assert!(proof.all_hashes_match);
    cleanup(root);
}

#[test]
fn verify_pg_unchanged_reports_changed_table_hashes() {
    let root = temp_root("pg-changed");
    let before = root.join("before-dump");
    let after = root.join("after-dump");
    write_pg_dump(&before, "prod_vault", None);
    write_pg_dump(&after, "prod_vault", Some(("outbox", "changed_after=1\n")));
    let before_snapshot = snapshot_pg_state(
        &PgConn::DumpDir { root: before },
        "prod_vault",
        &root.join("before"),
    )
    .unwrap();
    let after_snapshot = snapshot_pg_state(
        &PgConn::DumpDir { root: after },
        "prod_vault",
        &root.join("after"),
    )
    .unwrap();

    let error = ProductionFSV::verify_pg_unchanged(&before_snapshot, &after_snapshot).unwrap_err();

    assert_eq!(error.code, CALYX_PG_STATE_CHANGED);
    assert!(error.message.contains("outbox"));
    cleanup(root);
}

#[test]
fn snapshot_pg_state_rejects_missing_vault_and_write_capable_conn() {
    let root = temp_root("pg-edges");
    let dump = root.join("dump");
    write_pg_dump(&dump, "other_vault", None);

    let missing = snapshot_pg_state(
        &PgConn::DumpDir { root: dump },
        "prod_vault",
        &root.join("snapshot"),
    )
    .unwrap_err();
    assert_eq!(missing.code, CALYX_VAULT_NOT_IN_PG);

    let write_conn = snapshot_pg_state(
        &PgConn::WriteCapableForTest,
        "prod_vault",
        &root.join("write"),
    )
    .unwrap_err();
    assert_eq!(write_conn.code, CALYX_PG_WRITE_ATTEMPTED);
    cleanup(root);
}

#[test]
fn calyx_only_ask_cycle_proves_ledger_refs_and_base_bytes() {
    let (root, _sqlite, vault) = prepared_calyx_only("ask-proof", "prod_vault", 5);

    let proof = ProductionFSV::run_full_ask_cycle(&vault, &vector(3.0), 3).unwrap();

    assert_eq!(proof.mode, VaultMode::CalyxOnly);
    assert_eq!(proof.hits.len(), 3);
    assert!(proof.all_ledger_refs_valid);
    assert!(proof.reproduced_byte_exact);
    assert!(proof.hits.iter().all(|hit| hit.ledger_hash_matches_ref));
    assert!(proof.hits.iter().all(|hit| hit.chunk_id_byte_exact));
    assert!(proof.hits.iter().all(|hit| hit.text_hash_byte_exact));
    cleanup(root);
}

#[test]
fn run_full_ask_cycle_requires_calyx_only_mode() {
    let (root, _sqlite, vault, shadow) = prepared_flipped("not-calyx-only", "prod_vault", 2);
    shadow.close().unwrap();

    let error = ProductionFSV::run_full_ask_cycle(&vault, &vector(1.0), 1).unwrap_err();

    assert_eq!(error.code, CALYX_VAULT_NOT_CALYX_ONLY);
    assert_eq!(read_shadow_manifest(&vault).unwrap().mode, VaultMode::Calyx);
    cleanup(root);
}

#[test]
fn evidence_bundle_writes_all_true_flags() {
    let root = temp_root("evidence");
    let (vault_root, _sqlite, vault) = prepared_calyx_only("evidence-vault", "prod_vault", 3);
    let dump = root.join("dump");
    write_pg_dump(&dump, "prod_vault", None);
    let before = snapshot_pg_state(
        &PgConn::DumpDir { root: dump.clone() },
        "prod_vault",
        &root.join("before"),
    )
    .unwrap();
    let after = snapshot_pg_state(
        &PgConn::DumpDir { root: dump },
        "prod_vault",
        &root.join("after"),
    )
    .unwrap();
    let ask = ProductionFSV::run_full_ask_cycle(&vault, &vector(2.0), 2).unwrap();
    let bundle = ProductionFSV::bundle(&vault, before, after, ask).unwrap();
    let out = root.join("ph71_v2_evidence.json");

    ProductionFSV::emit_evidence(&out, &bundle).unwrap();

    let value: serde_json::Value = serde_json::from_slice(&std::fs::read(&out).unwrap()).unwrap();
    assert_eq!(value["all_hashes_match"], true);
    assert_eq!(value["all_ledger_refs_valid"], true);
    assert_eq!(value["reproduced_byte_exact"], true);
    cleanup(vault_root);
    cleanup(root);
}

proptest! {
    #[test]
    fn contract_proof_preserves_database_name_bytes(name in "[A-Za-z0-9_]{1,64}") {
        let root = temp_root("contract-prop");
        let dump = root.join("dump");
        write_pg_dump(&dump, &name, None);
        let snapshot = snapshot_pg_state(
            &PgConn::DumpDir { root: dump },
            &name,
            &root.join("snapshot"),
        ).unwrap();

        let proof = ProductionFSV::verify_control_plane_contract(&name, &snapshot).unwrap();

        prop_assert_eq!(proof.database_name, name);
        cleanup(root);
    }
}

#[test]
fn ask_proof_fails_closed_when_ledger_row_is_missing() {
    let (root, _sqlite, vault) = prepared_calyx_only("missing-ledger", "prod_vault", 2);
    remove_ledger_rows(&vault);

    let error = ProductionFSV::run_full_ask_cycle(&vault, &vector(1.0), 1).unwrap_err();

    assert_eq!(error.code, CALYX_REPRODUCE_MISMATCH);
    cleanup(root);
}

fn prepared_calyx_only(
    name: &str,
    database_name: &str,
    rows: usize,
) -> (PathBuf, PathBuf, PathBuf) {
    let (root, sqlite, vault, mut shadow) = prepared_flipped(name, database_name, rows);
    DefaultPanels::install(&mut shadow, VaultType::Text).unwrap();
    ShadowRemoval::execute(&mut shadow).unwrap();
    shadow.close().unwrap();
    (root, sqlite, vault)
}

fn prepared_flipped(
    name: &str,
    database_name: &str,
    rows: usize,
) -> (PathBuf, PathBuf, PathBuf, ShadowVault) {
    let root = temp_root(name);
    let sqlite = root.join("vault.db");
    let vault = root.join("vault.calyx");
    std::fs::create_dir_all(&root).unwrap();
    seed_sqlite(&sqlite, database_name, rows);
    replay_existing_sqlite(&sqlite, &vault).unwrap();
    let mut shadow = ShadowVault::open(&sqlite, &vault).unwrap();
    PanelGuardEnable::enable(&mut shadow, &PanelSpec::without_backfill()).unwrap();
    PanelGuardEnable::enable_kernel(&mut shadow).unwrap();
    PanelGuardEnable::enable_guard(&mut shadow, 0.72).unwrap();
    ReadFlip::execute(&mut shadow).unwrap();
    (root, sqlite, vault, shadow)
}

fn seed_sqlite(path: &Path, database_name: &str, rows: usize) {
    let conn = Connection::open(path).unwrap();
    conn.execute(
        "CREATE TABLE database_metadata(id INTEGER PRIMARY KEY, database_name TEXT NOT NULL)",
        [],
    )
    .unwrap();
    conn.execute(
        "CREATE TABLE chunks(chunk_id TEXT,database_name TEXT,content TEXT,embedding BLOB)",
        [],
    )
    .unwrap();
    conn.execute(
        "CREATE TABLE creator_databases(id INTEGER,database_name TEXT,created_at TEXT)",
        [],
    )
    .unwrap();
    conn.execute(
        "CREATE TABLE queries(id INTEGER,database_name TEXT,query_text TEXT)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO database_metadata VALUES(1,?1)",
        [database_name],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO creator_databases VALUES(1,?1,'2026-06-15T00:00:00Z')",
        [database_name],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO queries VALUES(1,?1,'known query')",
        [database_name],
    )
    .unwrap();
    for idx in 0..rows {
        conn.execute(
            "INSERT INTO chunks VALUES(?1,?2,?3,?4)",
            params![
                format!("c{idx:03}"),
                database_name,
                format!("content-{idx}"),
                vector_blob(idx as f32)
            ],
        )
        .unwrap();
    }
}

fn write_pg_dump(root: &Path, database_name: &str, change: Option<(&str, &str)>) {
    std::fs::create_dir_all(root).unwrap();
    for table in REQUIRED_TABLES {
        let mut content =
            format!("{{\"table\":\"{table}\",\"database_name\":\"{database_name}\",\"row\":1}}\n");
        if change.is_some_and(|(changed, _)| changed == *table) {
            content.push_str(change.expect("checked").1);
        }
        std::fs::write(root.join(format!("{table}.dump")), content).unwrap();
    }
}

fn remove_ledger_rows(vault: &Path) {
    let ledger_dir = vault.join("aster").join("cf").join("ledger");
    if ledger_dir.is_dir() {
        for entry in std::fs::read_dir(ledger_dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|ext| ext == "sst") {
                std::fs::remove_file(path).unwrap();
            }
        }
    }
}

fn vector(first: f32) -> Vec<f32> {
    std::iter::once(first)
        .chain((1..768).map(|idx| idx as f32 / 768.0))
        .collect()
}

fn vector_blob(first: f32) -> Vec<u8> {
    vector(first)
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

fn temp_root(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "calyx-production-fsv-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

fn cleanup(root: PathBuf) {
    let _ = std::fs::remove_dir_all(root);
}
