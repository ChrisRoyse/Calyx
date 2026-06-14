use std::path::Path;

use rusqlite::{Connection, params};

use super::*;

#[test]
fn migrates_and_offline_backfills_default_panel() {
    let root = temp_root("offline");
    let sqlite = root.join("vault.db");
    let vault = root.join("vault.calyx");
    std::fs::create_dir_all(&root).unwrap();
    seed_sqlite(&sqlite);

    let report = migrate_vault(
        &sqlite,
        &vault,
        MigrationOptions {
            verify: true,
            backfill: true,
            batch_size: 1,
            mode: Some(BackfillMode::OfflineDeterministic),
            ..MigrationOptions::default()
        },
    )
    .unwrap();

    assert_eq!(report.source_rows, 2);
    assert_eq!(report.written_rows, 2);
    assert_eq!(report.skipped_rows, 0);
    assert_eq!(
        report.verify.unwrap().missing_backfill,
        Vec::<String>::new()
    );
    assert!(
        report
            .status
            .unwrap()
            .slot_rows
            .values()
            .all(|count| *count == 2)
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn duplicate_content_rows_fail_before_vault_creation() {
    let root = temp_root("duplicate-content");
    let sqlite = root.join("vault.db");
    let vault = root.join("vault.calyx");
    std::fs::create_dir_all(&root).unwrap();
    seed_duplicate_content_sqlite(&sqlite);

    let error = migrate_vault(&sqlite, &vault, MigrationOptions::default()).unwrap_err();

    assert_eq!(error.code(), errors::CALYX_MIGRATE_SQLITE_SCHEMA);
    assert!(error.message().contains("rows 1 and 2"));
    assert!(error.message().contains("content-addressed cx_id"));
    assert!(!vault.exists());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn dry_run_validates_rows_without_creating_vault() {
    let root = temp_root("dry-run");
    let sqlite = root.join("vault.db");
    let vault = root.join("vault.calyx");
    std::fs::create_dir_all(&root).unwrap();
    seed_numbered_sqlite(&sqlite, 5);

    let report = migrate_vault(
        &sqlite,
        &vault,
        MigrationOptions {
            dry_run: true,
            batch_size: 2,
            ..MigrationOptions::default()
        },
    )
    .unwrap();

    assert_eq!(report.source_rows, 5);
    assert_eq!(report.migrated_rows, 5);
    assert_eq!(report.written_rows, 0);
    assert_eq!(report.skipped_rows, 0);
    assert_eq!(report.batches_completed, 3);
    assert!(report.dry_run);
    assert!(report.status.is_none());
    assert!(!vault.exists());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn rerun_skips_existing_constellations_without_growing_vault() {
    let root = temp_root("rerun");
    let sqlite = root.join("vault.db");
    let vault = root.join("vault.calyx");
    std::fs::create_dir_all(&root).unwrap();
    seed_numbered_sqlite(&sqlite, 10);

    let first = migrate_vault(
        &sqlite,
        &vault,
        MigrationOptions {
            batch_size: 3,
            ..MigrationOptions::default()
        },
    )
    .unwrap();
    let second = migrate_vault(
        &sqlite,
        &vault,
        MigrationOptions {
            batch_size: 3,
            ..MigrationOptions::default()
        },
    )
    .unwrap();

    assert_eq!(first.written_rows, 10);
    assert_eq!(first.skipped_rows, 0);
    assert_eq!(first.batches_completed, 4);
    assert_eq!(first.status.as_ref().unwrap().base_rows, 10);
    assert_eq!(second.written_rows, 0);
    assert_eq!(second.skipped_rows, 10);
    assert_eq!(second.batches_completed, 4);
    assert_eq!(second.status.as_ref().unwrap().base_rows, 10);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn empty_sqlite_completes_zero_rows() {
    let root = temp_root("empty");
    let sqlite = root.join("vault.db");
    let vault = root.join("vault.calyx");
    std::fs::create_dir_all(&root).unwrap();
    create_chunks_table(&Connection::open(&sqlite).unwrap());

    let report = migrate_vault(&sqlite, &vault, MigrationOptions::default()).unwrap();

    assert_eq!(report.source_rows, 0);
    assert_eq!(report.written_rows, 0);
    assert_eq!(report.skipped_rows, 0);
    assert_eq!(report.batches_completed, 0);
    assert_eq!(report.status.as_ref().unwrap().base_rows, 0);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn custom_gte_lens_id_is_persisted_in_readback_metadata() {
    let root = temp_root("custom-lens");
    let sqlite = root.join("vault.db");
    let vault = root.join("vault.calyx");
    let lens_id = "01010101010101010101010101010101".to_string();
    std::fs::create_dir_all(&root).unwrap();
    seed_sqlite(&sqlite);

    let report = migrate_vault(
        &sqlite,
        &vault,
        MigrationOptions {
            gte_lens_id: Some(lens_id.clone()),
            ..MigrationOptions::default()
        },
    )
    .unwrap();
    let readback = run_readback(&sqlite, &vault, "kernel-1").unwrap();

    assert_eq!(report.gte_lens_id, lens_id);
    assert_eq!(readback["metadata"]["gte_lens_id"], lens_id);
    std::fs::remove_dir_all(root).unwrap();
}

fn temp_root(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "calyx-migrate-{name}-{}-{}",
        std::process::id(),
        manifest::now_ms()
    ))
}

fn seed_sqlite(path: &Path) {
    let conn = Connection::open(path).unwrap();
    create_chunks_table(&conn);
    conn.execute(
        "INSERT INTO chunks VALUES('kernel-1','db','alpha beta',?1)",
        [embedding(1.0)],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO chunks VALUES('hot-2','db','gamma delta',?1)",
        [embedding(0.0)],
    )
    .unwrap();
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

fn seed_duplicate_content_sqlite(path: &Path) {
    let conn = Connection::open(path).unwrap();
    create_chunks_table(&conn);
    conn.execute(
        "INSERT INTO chunks VALUES('first','db','same content',?1)",
        [embedding(1.0)],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO chunks VALUES('second','db','same content',?1)",
        [embedding(2.0)],
    )
    .unwrap();
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
