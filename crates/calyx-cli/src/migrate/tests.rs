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
fn status_reports_ledger_chain_and_source_chunk_extents() {
    let root = temp_root("status-extents");
    let sqlite = root.join("vault.db");
    let vault = root.join("vault.calyx");
    std::fs::create_dir_all(&root).unwrap();
    seed_sqlite(&sqlite);

    migrate_vault(&sqlite, &vault, MigrationOptions::default()).unwrap();
    let report = run_status(&vault).unwrap();

    assert_eq!(report.base_rows, 2);
    assert_eq!(report.first_chunk_id.as_deref(), Some("kernel-1"));
    assert_eq!(report.last_chunk_id.as_deref(), Some("hot-2"));
    assert_eq!(report.ledger_chain.state, "Intact");
    assert_eq!(report.ledger_chain.count, 2);
    assert_eq!(report.ledger_chain.checked_range, "0..2");
    assert_eq!(report.ledger_chain.at_seq, None);
    assert_eq!(report.ledger_chain.reason, None);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn migrate_verify_reports_one_hundred_exact_matches_with_metadata() {
    let root = temp_root("verify-100");
    let sqlite = root.join("vault.db");
    let vault_dir = root.join("vault.calyx");
    std::fs::create_dir_all(&root).unwrap();
    seed_numbered_sqlite(&sqlite, 100);
    migrate_vault(&sqlite, &vault_dir, MigrationOptions::default()).unwrap();
    let manifest = MigrationManifest::load(&vault_dir).unwrap();
    let rows = stream_rows(&open_sqlite(&sqlite).unwrap()).unwrap();
    let vault = open_vault(&vault_dir, &manifest).unwrap();
    let adapter = adapter(&manifest).unwrap();

    let report = verify_migration(&vault, &rows, &adapter, false).unwrap();

    assert_eq!(report.total, 100);
    assert_eq!(report.matched, 100);
    assert_eq!(report.mismatched, 0);
    for row in &rows {
        let cx = vault.get(adapter.cx_id(row), vault.snapshot()).unwrap();
        assert_eq!(cx.chunk_id(), Some(row.chunk_id.as_str()));
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn verify_against_empty_vault_reports_all_rows_mismatched() {
    let root = temp_root("empty-vault-mismatch");
    let sqlite = root.join("vault.db");
    let vault_dir = root.join("vault.calyx");
    std::fs::create_dir_all(&root).unwrap();
    seed_numbered_sqlite(&sqlite, 4);
    let conn = open_sqlite(&sqlite).unwrap();
    let rows = stream_rows(&conn).unwrap();
    let manifest = MigrationManifest::load_or_create(
        &vault_dir,
        &sqlite,
        &rows,
        default_base_lens_id(),
        default_panel_version(),
    )
    .unwrap();
    manifest.write(&vault_dir).unwrap();
    let vault = open_vault(&vault_dir, &manifest).unwrap();
    let adapter = adapter(&manifest).unwrap();

    let report = verify_migration(&vault, &rows, &adapter, false).unwrap();

    assert_eq!(report.total, 4);
    assert_eq!(report.matched, 0);
    assert_eq!(report.mismatched, 4);
    assert_eq!(report.errors.len(), 4);
    assert!(
        report
            .errors
            .iter()
            .all(|error| error.actual_hash == [0; 32])
    );
    assert_eq!(report.gate, "FAIL");
    drop(vault);
    drop(conn);
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
    assert_eq!(report.status.as_ref().unwrap().first_chunk_id, None);
    assert_eq!(report.status.as_ref().unwrap().last_chunk_id, None);
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

#[test]
fn created_at_is_preserved_and_temporal_vectors_are_active() {
    let root = temp_root("temporal-active");
    let sqlite = root.join("vault.db");
    let vault = root.join("vault.calyx");
    std::fs::create_dir_all(&root).unwrap();
    seed_temporal_sqlite(
        &sqlite,
        &[
            ("kernel-1", "alpha beta", 1.0, "2024-01-02T14:00:00Z"),
            ("hot-2", "gamma delta", 0.0, "2024-01-02T15:00:00Z"),
        ],
    );

    let report = migrate_vault(
        &sqlite,
        &vault,
        MigrationOptions {
            verify: true,
            backfill: true,
            mode: Some(BackfillMode::OfflineDeterministic),
            ..MigrationOptions::default()
        },
    )
    .unwrap();
    let readback = run_readback(&sqlite, &vault, "hot-2").unwrap();

    assert_eq!(report.status.as_ref().unwrap().temporal_active_rows, 2);
    assert_eq!(report.status.as_ref().unwrap().temporal_inactive_rows, 0);
    assert_eq!(readback["created_at"], 1_704_207_600_u64);
    assert_eq!(readback["source_event_time_secs"], 1_704_207_600_u64);
    assert_eq!(readback["temporal_lane_state"], "active");
    assert_eq!(
        readback["slots"]["5"]["dense_values"],
        serde_json::json!([1.0])
    );
    assert_eq!(readback["slots"]["6"]["kind"], "dense:2");
    assert_eq!(readback["slots"]["7"]["kind"], "dense:4");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn timeless_fixture_backfill_marks_temporal_slots_absent() {
    let root = temp_root("temporal-inactive");
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
            mode: Some(BackfillMode::OfflineDeterministic),
            ..MigrationOptions::default()
        },
    )
    .unwrap();
    let readback = run_readback(&sqlite, &vault, "kernel-1").unwrap();

    assert_eq!(report.status.as_ref().unwrap().temporal_active_rows, 0);
    assert_eq!(report.status.as_ref().unwrap().temporal_inactive_rows, 2);
    assert_eq!(readback["source_event_time_secs"], serde_json::Value::Null);
    assert_eq!(readback["temporal_lane_state"], "inactive");
    for slot in ["5", "6", "7"] {
        assert_eq!(readback["slots"][slot]["kind"], "absent");
        assert_eq!(
            readback["slots"][slot]["absent_reason"]["error"],
            "source_missing_created_at"
        );
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn status_reports_duplicate_and_out_of_order_source_times() {
    let root = temp_root("temporal-order");
    let sqlite = root.join("vault.db");
    let vault = root.join("vault.calyx");
    std::fs::create_dir_all(&root).unwrap();
    seed_temporal_sqlite(
        &sqlite,
        &[
            ("first", "alpha", 1.0, "2024-01-02T15:00:00Z"),
            ("dupe", "beta", 2.0, "2024-01-02T15:00:00Z"),
            ("older", "gamma", 3.0, "2024-01-02T14:00:00Z"),
        ],
    );

    let report = migrate_vault(&sqlite, &vault, MigrationOptions::default()).unwrap();

    let status = report.status.unwrap();
    assert_eq!(status.temporal_active_rows, 3);
    assert_eq!(status.temporal_duplicate_event_time_rows, 1);
    assert_eq!(status.temporal_out_of_order_rows, 1);
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

fn seed_temporal_sqlite(path: &Path, rows: &[(&str, &str, f32, &str)]) {
    let conn = Connection::open(path).unwrap();
    create_chunks_table_with_created_at(&conn);
    for (chunk_id, content, first, created_at) in rows {
        conn.execute(
            "INSERT INTO chunks VALUES(?1,'db',?2,?3,?4)",
            params![chunk_id, content, embedding(*first), created_at],
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

fn create_chunks_table_with_created_at(conn: &Connection) {
    conn.execute(
        "CREATE TABLE chunks(
            chunk_id TEXT,database_name TEXT,content TEXT,embedding BLOB,created_at TEXT)",
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
