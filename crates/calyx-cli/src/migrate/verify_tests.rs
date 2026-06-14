use std::path::{Path, PathBuf};

use proptest::prelude::*;
use rusqlite::{Connection, params};

use super::*;

#[test]
fn verify_migration_reports_five_exact_matches() {
    let root = temp_root("verify-exact");
    let sqlite = root.join("source.db");
    let vault = root.join("vault.calyx");
    std::fs::create_dir_all(&root).unwrap();
    seed_numbered_sqlite(&sqlite, 5);
    migrate_vault(&sqlite, &vault, MigrationOptions::default()).unwrap();

    let report = run_verify(&sqlite, &vault, false).unwrap();

    assert_eq!(report.total, 5);
    assert_eq!(report.matched, 5);
    assert_eq!(report.mismatched, 0);
    assert_eq!(report.errors, Vec::new());
    assert_eq!(report.base_slot_matches, 5);
    assert_eq!(report.gate, "PASS");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn verify_migration_reports_all_missing_rows_without_short_circuit() {
    let root = temp_root("verify-all-missing");
    let sqlite = root.join("source.db");
    let vault = root.join("vault.calyx");
    std::fs::create_dir_all(&root).unwrap();
    seed_numbered_sqlite(&sqlite, 3);
    migrate_vault(&sqlite, &vault, MigrationOptions::default()).unwrap();
    Connection::open(&sqlite)
        .unwrap()
        .execute("UPDATE chunks SET content=chunk_id || '-changed'", [])
        .unwrap();

    let report = run_verify(&sqlite, &vault, false).unwrap();

    assert_eq!(report.total, 3);
    assert_eq!(report.matched, 0);
    assert_eq!(report.mismatched, 3);
    assert_eq!(report.errors[0].row_num, 1);
    assert_eq!(report.errors[0].chunk_id, "chunk-0");
    assert_eq!(report.errors[1].row_num, 2);
    assert_eq!(report.errors[1].chunk_id, "chunk-1");
    assert!(
        report
            .errors
            .iter()
            .all(|error| error.actual_hash == [0; 32])
    );
    assert_eq!(report.gate, "FAIL");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn verify_migration_reports_missing_cx_id_as_zero_actual_hash() {
    let root = temp_root("verify-missing");
    let sqlite = root.join("source.db");
    let vault = root.join("vault.calyx");
    std::fs::create_dir_all(&root).unwrap();
    seed_numbered_sqlite(&sqlite, 2);
    migrate_vault(&sqlite, &vault, MigrationOptions::default()).unwrap();
    Connection::open(&sqlite)
        .unwrap()
        .execute(
            "UPDATE chunks SET content='changed content' WHERE chunk_id='chunk-0'",
            [],
        )
        .unwrap();

    let report = run_verify(&sqlite, &vault, false).unwrap();

    assert_eq!(report.total, 2);
    assert_eq!(report.matched, 1);
    assert_eq!(report.mismatched, 1);
    assert_eq!(report.errors[0].row_num, 1);
    assert_eq!(report.errors[0].chunk_id, "chunk-0");
    assert_eq!(report.errors[0].actual_hash, [0; 32]);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn empty_sqlite_ignores_extra_vault_constellations() {
    let root = temp_root("verify-empty-extra");
    let source = root.join("source.db");
    let empty = root.join("empty.db");
    let vault = root.join("vault.calyx");
    std::fs::create_dir_all(&root).unwrap();
    seed_numbered_sqlite(&source, 1);
    create_chunks_table(&Connection::open(&empty).unwrap());
    migrate_vault(&source, &vault, MigrationOptions::default()).unwrap();

    let report = run_verify(&empty, &vault, false).unwrap();

    assert_eq!(report.total, 0);
    assert_eq!(report.matched, 0);
    assert_eq!(report.mismatched, 0);
    assert_eq!(report.errors, Vec::new());
    assert_eq!(report.gate, "PASS");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn verify_error_line_formats_hex_hashes() {
    let line = verify_error_line(&VerifyError {
        row_num: 7,
        chunk_id: "abc123".to_string(),
        expected_hash: [0x11; 32],
        actual_hash: [0; 32],
    });

    assert_eq!(
        line,
        concat!(
            "MISMATCH row=7 chunk_id=abc123 ",
            "expected=1111111111111111111111111111111111111111111111111111111111111111 ",
            "actual=0000000000000000000000000000000000000000000000000000000000000000"
        )
    );
}

proptest! {
    #[test]
    fn verifier_content_hash_matches_blake3(content in proptest::collection::vec(any::<u8>(), 0..2048)) {
        prop_assert_eq!(verifier::content_hash(&content), *blake3::hash(&content).as_bytes());
    }
}

fn temp_root(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "calyx-migrate-{name}-{}-{}",
        std::process::id(),
        manifest::now_ms()
    ))
}

fn seed_numbered_sqlite(path: &Path, rows: usize) {
    let conn = Connection::open(path).unwrap();
    create_chunks_table(&conn);
    for idx in 0..rows {
        conn.execute(
            "INSERT INTO chunks VALUES(?1,'db',?2,?3)",
            params![
                format!("chunk-{idx}"),
                format!("content-{idx}"),
                embedding(idx as f32)
            ],
        )
        .unwrap();
    }
}

fn create_chunks_table(conn: &Connection) {
    conn.execute(
        "CREATE TABLE chunks(chunk_id TEXT,database_name TEXT,content TEXT,embedding BLOB)",
        [],
    )
    .unwrap();
}

fn embedding(first: f32) -> Vec<u8> {
    std::iter::once(first)
        .chain((1..768).map(|idx| idx as f32 / 768.0))
        .flat_map(|value| value.to_le_bytes())
        .collect()
}
