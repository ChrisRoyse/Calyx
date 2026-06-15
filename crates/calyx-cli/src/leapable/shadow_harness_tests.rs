use std::path::{Path, PathBuf};

use proptest::prelude::*;
use rusqlite::{Connection, params};

use super::shadow_harness::{
    CALYX_CONTRACT_NAME_MISSING, CALYX_MANIFEST_CORRUPT, CALYX_PG_CONTRACT_VIOLATION,
    CALYX_VAULT_MODE_ROLLBACK_DENIED, CALYX_VAULT_NOT_FOUND, ShadowVault, VaultMode,
    read_shadow_manifest,
};

#[test]
fn opens_shadow_vault_and_preserves_verbatim_database_name() {
    let root = temp_root("open");
    let sqlite = root.join("vault.db");
    let vault = root.join("vault.calyx");
    std::fs::create_dir_all(&root).unwrap();
    seed_sqlite(&sqlite, "test_vault", 3);
    let before = sqlite_mtime(&sqlite);

    let shadow = ShadowVault::open(&sqlite, &vault).unwrap();

    assert_eq!(shadow.vault_name(), "test_vault");
    assert_eq!(shadow.mode(), VaultMode::Shadow);
    assert_eq!(shadow.verify_pg_contract().unwrap().tables.len(), 2);
    shadow.close().unwrap();
    assert_eq!(sqlite_mtime(&sqlite), before);
    let readback = read_shadow_manifest(&vault).unwrap();
    assert_eq!(readback.database_name, "test_vault");
    assert_eq!(readback.mode, VaultMode::Shadow);
    assert_eq!(readback.mode_byte, 0);
    assert_eq!(readback.chunk_count, 0);

    cleanup(root);
}

#[test]
fn close_flushes_shadow_wal_and_reopen_keeps_marker() {
    let root = temp_root("wal-marker");
    let sqlite = root.join("vault.db");
    let vault = root.join("vault.calyx");
    std::fs::create_dir_all(&root).unwrap();
    seed_sqlite(&sqlite, "test_vault", 3);
    let mut shadow = ShadowVault::open(&sqlite, &vault).unwrap();

    shadow
        .append_shadow_wal_marker(b"partial-shadow-write")
        .unwrap();
    shadow.close().unwrap();
    let reopened = ShadowVault::open(&sqlite, &vault).unwrap();
    let readback = reopened.manifest_readback().unwrap();

    assert_eq!(readback.wal_bytes, b"partial-shadow-write".len() as u64);
    reopened.close().unwrap();
    cleanup(root);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1))]
    #[test]
    fn vault_name_is_pure_across_hundred_open_close_cycles(_seed in Just(0xDEAD_BEEFu64)) {
        let root = temp_root("pure");
        let sqlite = root.join("vault.db");
        let vault = root.join("vault.calyx");
        std::fs::create_dir_all(&root).unwrap();
        seed_sqlite(&sqlite, "test_vault", 3);
        let before = sqlite_mtime(&sqlite);

        for _ in 0..100 {
            let shadow = ShadowVault::open(&sqlite, &vault).unwrap();
            prop_assert_eq!(shadow.vault_name(), "test_vault");
            shadow.close().unwrap();
        }

        prop_assert_eq!(sqlite_mtime(&sqlite), before);
        cleanup(root);
    }
}

#[test]
fn missing_sqlite_fails_before_vault_creation() {
    let root = temp_root("missing-db");
    let sqlite = root.join("missing.db");
    let vault = root.join("vault.calyx");
    std::fs::create_dir_all(&root).unwrap();

    let error = ShadowVault::open(&sqlite, &vault).unwrap_err();

    assert_eq!(error.code, CALYX_VAULT_NOT_FOUND);
    assert!(!vault.exists());
    cleanup(root);
}

#[test]
fn cli_missing_sqlite_preserves_vault_not_found_code() {
    let root = temp_root("cli-missing-db");
    let sqlite = root.join("missing.db");
    let vault = root.join("vault.calyx");
    std::fs::create_dir_all(&root).unwrap();

    let error = super::shadow_harness_cli::run_shadow_open(&[
        "--sqlite".to_string(),
        sqlite.display().to_string(),
        "--vault".to_string(),
        vault.display().to_string(),
    ])
    .unwrap_err();

    assert_eq!(error.code(), CALYX_VAULT_NOT_FOUND);
    assert!(!vault.exists());
    cleanup(root);
}

#[test]
fn corrupt_manifest_fails_and_leaves_sqlite_mtime_unchanged() {
    let root = temp_root("corrupt-manifest");
    let sqlite = root.join("vault.db");
    let vault = root.join("vault.calyx");
    std::fs::create_dir_all(&vault).unwrap();
    seed_sqlite(&sqlite, "test_vault", 3);
    std::fs::write(vault.join("MANIFEST"), b"CXSHDW1!\0{\"schema_version\"").unwrap();
    let before = sqlite_mtime(&sqlite);

    let error = ShadowVault::open(&sqlite, &vault).unwrap_err();

    assert_eq!(error.code, CALYX_MANIFEST_CORRUPT);
    assert_eq!(sqlite_mtime(&sqlite), before);
    cleanup(root);
}

#[test]
fn missing_database_name_row_fails_closed() {
    let root = temp_root("missing-name");
    let sqlite = root.join("vault.db");
    let vault = root.join("vault.calyx");
    std::fs::create_dir_all(&root).unwrap();
    seed_sqlite_without_database_name(&sqlite);

    let error = ShadowVault::open(&sqlite, &vault).unwrap_err();

    assert_eq!(error.code, CALYX_CONTRACT_NAME_MISSING);
    assert!(!vault.exists());
    cleanup(root);
}

#[test]
fn mode_ratchet_allows_noop_and_rejects_reverse() {
    let root = temp_root("mode");
    let sqlite = root.join("vault.db");
    let vault = root.join("vault.calyx");
    std::fs::create_dir_all(&root).unwrap();
    seed_sqlite(&sqlite, "test_vault", 3);
    let mut shadow = ShadowVault::open(&sqlite, &vault).unwrap();

    shadow.set_mode(VaultMode::Shadow).unwrap();
    shadow.set_mode(VaultMode::Calyx).unwrap();
    shadow.set_mode(VaultMode::CalyxOnly).unwrap();
    let error = shadow.set_mode(VaultMode::Shadow).unwrap_err();

    assert_eq!(error.code, CALYX_VAULT_MODE_ROLLBACK_DENIED);
    shadow.close().unwrap();
    assert_eq!(
        read_shadow_manifest(&vault).unwrap().mode,
        VaultMode::CalyxOnly
    );
    cleanup(root);
}

#[test]
fn pg_contract_type_change_fails_closed() {
    let root = temp_root("contract-type");
    let sqlite = root.join("vault.db");
    let vault = root.join("vault.calyx");
    std::fs::create_dir_all(&root).unwrap();
    seed_sqlite_with_bad_query_contract(&sqlite);

    let error = ShadowVault::open(&sqlite, &vault).unwrap_err();

    assert_eq!(error.code, CALYX_PG_CONTRACT_VIOLATION);
    assert!(!vault.exists());
    cleanup(root);
}

fn seed_sqlite(path: &Path, database_name: &str, rows: usize) {
    let conn = Connection::open(path).unwrap();
    create_schema(&conn, true);
    conn.execute(
        "INSERT INTO database_metadata(database_name) VALUES(?1)",
        [database_name],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO creator_databases VALUES(1, ?1, '2026-06-15T00:00:00Z')",
        [database_name],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO queries VALUES(1, ?1, 'known query')",
        [database_name],
    )
    .unwrap();
    for idx in 0..rows {
        conn.execute(
            "INSERT INTO chunks VALUES(?1, ?2, ?3, ?4)",
            params![
                format!("chunk-{idx}"),
                database_name,
                format!("content-{idx}"),
                embedding(idx as f32)
            ],
        )
        .unwrap();
    }
}

fn seed_sqlite_without_database_name(path: &Path) {
    let conn = Connection::open(path).unwrap();
    create_schema(&conn, true);
}

fn seed_sqlite_with_bad_query_contract(path: &Path) {
    let conn = Connection::open(path).unwrap();
    create_schema(&conn, false);
    conn.execute(
        "INSERT INTO database_metadata(database_name) VALUES('test_vault')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO creator_databases VALUES(1, 'test_vault', '2026-06-15T00:00:00Z')",
        [],
    )
    .unwrap();
}

fn create_schema(conn: &Connection, good_queries: bool) {
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
    let query_type = if good_queries { "TEXT" } else { "INTEGER" };
    conn.execute(
        &format!("CREATE TABLE queries(id INTEGER,database_name TEXT,query_text {query_type})"),
        [],
    )
    .unwrap();
}

fn embedding(first: f32) -> Vec<u8> {
    std::iter::once(first)
        .chain((1..16).map(|idx| idx as f32 / 16.0))
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

fn sqlite_mtime(path: &Path) -> SystemTimeKey {
    let modified = std::fs::metadata(path).unwrap().modified().unwrap();
    SystemTimeKey(
        modified
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SystemTimeKey(u128);

fn temp_root(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "calyx-shadow-{name}-{}-{}",
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
