use std::cmp::Ordering;
use std::path::{Path, PathBuf};

use proptest::prelude::*;
use rusqlite::{Connection, params};

use super::dual_write::replay_existing_sqlite;
use super::panel_guard_enable::{CALYX_GUARD_TAU_NOT_CALIBRATED, PanelGuardEnable, PanelSpec};
use super::read_flip::{ReadFlip, run_read_flip};
use super::shadow_harness::{ShadowVault, VaultMode, read_shadow_manifest};
use crate::migrate::reader::{open_sqlite, stream_rows};

#[test]
fn read_flip_routes_ask_to_calyx_and_preserves_sqlite_writability() {
    let root = temp_root("happy");
    let sqlite = root.join("vault.db");
    let vault = root.join("vault.calyx");
    std::fs::create_dir_all(&root).unwrap();
    seed_sqlite(&sqlite, "flip_db", 5);
    replay_existing_sqlite(&sqlite, &vault).unwrap();
    let mut shadow = ShadowVault::open(&sqlite, &vault).unwrap();
    let panel = PanelGuardEnable::enable(&mut shadow, &PanelSpec::default()).unwrap();
    PanelGuardEnable::enable_kernel(&mut shadow).unwrap();
    PanelGuardEnable::enable_guard(&mut shadow, 0.72).unwrap();

    let receipt = ReadFlip::execute(&mut shadow).unwrap();
    let ask = shadow.ask(&vector(3.0), 3).unwrap();

    assert_eq!(receipt.database_name, "flip_db");
    assert_eq!(receipt.panel_lens_count, panel.panel_lens_count);
    assert_eq!(ask.mode, VaultMode::Calyx);
    assert_eq!(ids(&ask.hits), sqlite_top(&sqlite, &vector(3.0), 3));
    assert!(ask.hits.iter().all(|hit| hit.ledger_ref.seq > 0));
    let manifest = read_shadow_manifest(&vault).unwrap();
    assert_eq!(manifest.mode_byte, 1);
    assert_eq!(manifest.features["read_path"], "calyx");
    shadow.close().unwrap();
    insert_query_row(&sqlite, "flip_db", "shadow writable after flip");
    cleanup(root);
}

#[test]
fn read_flip_is_idempotent_after_calyx_mode() {
    let (root, sqlite, vault, mut shadow) = prepared_shadow("idempotent", "idem_db");
    PanelGuardEnable::enable(&mut shadow, &PanelSpec::default()).unwrap();
    PanelGuardEnable::enable_kernel(&mut shadow).unwrap();
    PanelGuardEnable::enable_guard(&mut shadow, 0.72).unwrap();

    let first = ReadFlip::execute(&mut shadow).unwrap();
    let second = ReadFlip::execute(&mut shadow).unwrap();

    assert_eq!(second, first);
    assert_eq!(
        read_shadow_manifest(&vault).unwrap().features["flip_ledger_seq"],
        first.ledger_ref.seq.to_string()
    );
    shadow.close().unwrap();
    assert_eq!(sqlite_count(&sqlite), 5);
    cleanup(root);
}

#[test]
fn guard_tau_zero_fails_closed() {
    let (root, _sqlite, _vault, mut shadow) = prepared_shadow("tau-zero", "tau_db");

    let error = PanelGuardEnable::enable_guard(&mut shadow, 0.0).unwrap_err();

    assert_eq!(error.code, CALYX_GUARD_TAU_NOT_CALIBRATED);
    shadow.close().unwrap();
    cleanup(root);
}

#[test]
fn read_flip_tau_zero_preflight_leaves_manifest_bytes_unmodified() {
    let root = temp_root("tau-zero-cli");
    let sqlite = root.join("vault.db");
    let vault = root.join("vault.calyx");
    std::fs::create_dir_all(&root).unwrap();
    seed_sqlite(&sqlite, "tau_cli_db", 5);
    replay_existing_sqlite(&sqlite, &vault).unwrap();
    let manifest_path = vault.join("MANIFEST");
    let before_bytes = std::fs::read(&manifest_path).unwrap();

    let error = run_read_flip(&[
        "--sqlite".to_string(),
        sqlite.display().to_string(),
        "--calyx".to_string(),
        vault.display().to_string(),
        "--tau".to_string(),
        "0.0".to_string(),
    ])
    .unwrap_err();

    assert_eq!(error.code(), CALYX_GUARD_TAU_NOT_CALIBRATED);
    assert_eq!(std::fs::read(&manifest_path).unwrap(), before_bytes);
    let manifest = read_shadow_manifest(&vault).unwrap();
    assert_eq!(manifest.mode, VaultMode::Shadow);
    assert!(manifest.features.is_empty());
    cleanup(root);
}

#[test]
fn incomplete_backfill_flips_with_grounding_gaps() {
    let (root, _sqlite, vault, mut shadow) = prepared_shadow("gaps", "gap_db");
    let panel = PanelGuardEnable::enable(&mut shadow, &PanelSpec::without_backfill()).unwrap();
    PanelGuardEnable::enable_kernel(&mut shadow).unwrap();
    PanelGuardEnable::enable_guard(&mut shadow, 0.72).unwrap();

    let receipt = ReadFlip::execute(&mut shadow).unwrap();

    assert_eq!(receipt.panel_lens_count, panel.panel_lens_count);
    assert!(!panel.grounding_gaps.is_empty());
    let manifest = read_shadow_manifest(&vault).unwrap();
    assert_eq!(manifest.mode, VaultMode::Calyx);
    assert_ne!(manifest.features["grounding_gaps"], "[]");
    shadow.close().unwrap();
    cleanup(root);
}

#[test]
fn lens_mismatch_rolls_back_panel_enable_before_mode_change() {
    let (root, _sqlite, vault, mut shadow) = prepared_shadow("lens-mismatch", "lens_db");

    let error = PanelGuardEnable::enable(
        &mut shadow,
        &PanelSpec::default().expecting_lens("definitely-not-the-base-lens"),
    )
    .unwrap_err();

    assert_eq!(error.code, "CALYX_LENS_FROZEN_VIOLATION");
    let manifest = read_shadow_manifest(&vault).unwrap();
    assert_eq!(manifest.mode, VaultMode::Shadow);
    assert!(!manifest.features.contains_key("panel_enabled"));
    shadow.close().unwrap();
    cleanup(root);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(50))]
    #[test]
    fn flip_receipt_preserves_database_name_byte_exact(name in "[a-z][a-z0-9_]{0,10}") {
        let root = temp_root("prop-name");
        let sqlite = root.join("vault.db");
        let vault = root.join("vault.calyx");
        std::fs::create_dir_all(&root).unwrap();
        seed_sqlite(&sqlite, &name, 1);
        replay_existing_sqlite(&sqlite, &vault).unwrap();
        let mut shadow = ShadowVault::open(&sqlite, &vault).unwrap();
        PanelGuardEnable::enable(&mut shadow, &PanelSpec::without_backfill()).unwrap();
        PanelGuardEnable::enable_kernel(&mut shadow).unwrap();
        PanelGuardEnable::enable_guard(&mut shadow, 0.72).unwrap();

        let receipt = ReadFlip::execute(&mut shadow).unwrap();

        prop_assert_eq!(receipt.database_name, name);
        shadow.close().unwrap();
        cleanup(root);
    }
}

fn prepared_shadow(name: &str, database_name: &str) -> (PathBuf, PathBuf, PathBuf, ShadowVault) {
    let root = temp_root(name);
    let sqlite = root.join("vault.db");
    let vault = root.join("vault.calyx");
    std::fs::create_dir_all(&root).unwrap();
    seed_sqlite(&sqlite, database_name, 5);
    replay_existing_sqlite(&sqlite, &vault).unwrap();
    let shadow = ShadowVault::open(&sqlite, &vault).unwrap();
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

fn sqlite_top(path: &Path, query: &[f32], top_k: usize) -> Vec<String> {
    let conn = open_sqlite(path).unwrap();
    let mut rows = stream_rows(&conn).unwrap();
    rows.sort_by(|left, right| {
        cosine(query, &right.embedding)
            .partial_cmp(&cosine(query, &left.embedding))
            .unwrap_or(Ordering::Equal)
            .then_with(|| left.chunk_id.cmp(&right.chunk_id))
    });
    rows.into_iter()
        .take(top_k)
        .map(|row| row.chunk_id)
        .collect()
}

fn ids(hits: &[super::read_flip::Hit]) -> Vec<String> {
    hits.iter().map(|hit| hit.chunk_id.clone()).collect()
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

fn cosine(left: &[f32], right: &[f32]) -> f64 {
    let dot = left
        .iter()
        .zip(right)
        .map(|(left, right)| f64::from(*left) * f64::from(*right))
        .sum::<f64>();
    dot / (norm(left) * norm(right)).max(f64::MIN_POSITIVE)
}

fn norm(vector: &[f32]) -> f64 {
    vector
        .iter()
        .map(|value| f64::from(*value) * f64::from(*value))
        .sum::<f64>()
        .sqrt()
}

fn insert_query_row(path: &Path, database_name: &str, query: &str) {
    Connection::open(path)
        .unwrap()
        .execute(
            "INSERT INTO queries VALUES(2,?1,?2)",
            params![database_name, query],
        )
        .unwrap();
}

fn sqlite_count(path: &Path) -> i64 {
    Connection::open(path)
        .unwrap()
        .query_row("SELECT COUNT(*) FROM chunks", [], |row| row.get(0))
        .unwrap()
}

fn temp_root(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "calyx-read-flip-{name}-{}-{}",
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
