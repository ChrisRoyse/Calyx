use std::path::{Path, PathBuf};
use std::process::Command;

use rusqlite::{Connection, params};

#[test]
fn migrate_verify_cli_prints_success_and_fail_closed_mismatch() {
    let root = temp_root("migrate-verify-cli");
    let sqlite = root.join("source.db");
    let vault = root.join("vault.calyx");
    std::fs::create_dir_all(&root).unwrap();
    seed_sqlite(&sqlite);

    let migrate = calyx()
        .args([
            "migrate",
            "vault",
            sqlite.to_str().unwrap(),
            vault.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(migrate.status.success(), "{}", stderr(&migrate));

    let success = calyx()
        .args([
            "migrate",
            "verify",
            sqlite.to_str().unwrap(),
            vault.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(success.status.success(), "{}", stderr(&success));
    assert!(stdout(&success).contains("verified 2/2 rows: byte-exact on content"));

    Connection::open(&sqlite)
        .unwrap()
        .execute(
            "UPDATE chunks SET content='changed byte-exact content' WHERE chunk_id='chunk-0'",
            [],
        )
        .unwrap();
    let failed = calyx()
        .args([
            "migrate",
            "verify",
            sqlite.to_str().unwrap(),
            vault.to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert_eq!(failed.status.code(), Some(2));
    assert!(stdout(&failed).contains("MISMATCH row=1 chunk_id=chunk-0"));
    assert!(stdout(&failed).contains("actual=00000000000000000000000000000000"));
    assert!(stderr(&failed).contains("FAILED: 1 mismatches"));
    assert!(stderr(&failed).contains("\"code\":\"CALYX_ASTER_CORRUPT_SHARD\""));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn migrate_vault_verify_reports_fifty_row_summary() {
    let root = temp_root("migrate-vault-verify-cli");
    let sqlite = root.join("source.db");
    let vault = root.join("vault.calyx");
    std::fs::create_dir_all(&root).unwrap();
    seed_sqlite_rows(&sqlite, 50);

    let migrate = calyx()
        .args([
            "migrate",
            "vault",
            sqlite.to_str().unwrap(),
            vault.to_str().unwrap(),
            "--verify",
            "--batch-size",
            "7",
        ])
        .output()
        .unwrap();

    assert!(migrate.status.success(), "{}", stderr(&migrate));
    let stdout = stdout(&migrate);
    assert!(stdout.contains("\"source_rows\":50"));
    assert!(stdout.contains("\"matched\":50"));
    assert!(stdout.contains("\"mismatched\":0"));
    assert!(stdout.contains("verified 50/50 rows: byte-exact on content"));
    std::fs::remove_dir_all(root).unwrap();
}

fn calyx() -> Command {
    Command::new(env!("CARGO_BIN_EXE_calyx"))
}

fn temp_root(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("calyx-{name}-{}", std::process::id()))
}

fn seed_sqlite(path: &Path) {
    seed_sqlite_rows(path, 2);
}

fn seed_sqlite_rows(path: &Path, rows: usize) {
    let conn = Connection::open(path).unwrap();
    conn.execute(
        "CREATE TABLE chunks(chunk_id TEXT,database_name TEXT,content TEXT,embedding BLOB)",
        [],
    )
    .unwrap();
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

fn embedding(first: f32) -> Vec<u8> {
    std::iter::once(first)
        .chain((1..768).map(|idx| idx as f32 / 768.0))
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

fn stdout(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}
