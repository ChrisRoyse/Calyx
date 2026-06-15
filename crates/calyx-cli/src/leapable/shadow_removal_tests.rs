use std::path::{Path, PathBuf};

use proptest::prelude::*;
use rusqlite::{Connection, params};

use super::dual_write::replay_existing_sqlite;
use super::panel_guard_enable::{PanelGuardEnable, PanelSpec};
use super::read_flip::ReadFlip;
use super::shadow_harness::{ShadowVault, VaultMode, read_shadow_manifest};
use super::shadow_removal::{
    CALYX_ROLLBACK_GATE_ALREADY_PASSED, CALYX_SHADOW_REMOVAL_FAILED, CALYX_VAULT_FLIP_REQUIRED,
    DefaultPanelOptions, DefaultPanels, ShadowRemoval, VaultType,
};

#[test]
fn remove_shadow_installs_text_panel_and_routes_calyx_only() {
    let (root, sqlite, vault, mut shadow) = prepared_flipped("happy", "remove_db", 5);
    let sqlite_hash = file_hash(&sqlite);

    let panel = DefaultPanels::install(&mut shadow, VaultType::Text).unwrap();
    let receipt = ShadowRemoval::execute(&mut shadow).unwrap();
    let ask = shadow.ask(&vector(3.0), 2).unwrap();
    let manifest = read_shadow_manifest(&vault).unwrap();

    assert_eq!(panel.template, "text-default");
    assert_eq!(panel.lens_count, calyx_registry::text_default().slots.len());
    assert_eq!(panel.backfill_pending, 0);
    assert_eq!(receipt.database_name, "remove_db");
    assert!(receipt.calyx_only_at_seq > 0);
    assert_eq!(manifest.mode, VaultMode::CalyxOnly);
    assert_eq!(manifest.mode_byte, 2);
    assert_eq!(manifest.features["read_path"], "calyx-only");
    assert_eq!(manifest.features["sqlite_shadow_archived"], "true");
    assert_eq!(manifest.features["default_panel_template"], "text-default");
    assert_eq!(
        manifest.features["default_panel_lens_count"],
        calyx_registry::text_default().slots.len().to_string()
    );
    assert!(!sqlite.exists());
    assert_eq!(file_hash(&archive_path(&sqlite)), sqlite_hash);
    assert_eq!(ask.mode, VaultMode::CalyxOnly);
    assert_eq!(ask.hits.len(), 2);
    shadow.close().unwrap();
    cleanup(root);
}

#[test]
fn rollback_before_gate_restores_sqlite_hash_and_calyx_mode() {
    let (root, sqlite, vault, mut shadow) = prepared_flipped("rollback", "rollback_db", 3);
    let sqlite_hash = file_hash(&sqlite);
    DefaultPanels::install(&mut shadow, VaultType::Text).unwrap();
    let receipt = ShadowRemoval::execute(&mut shadow).unwrap();
    shadow.close().unwrap();

    ShadowRemoval::rollback(&receipt).unwrap();
    let manifest = read_shadow_manifest(&vault).unwrap();

    assert_eq!(manifest.mode, VaultMode::Calyx);
    assert_eq!(manifest.mode_byte, 1);
    assert_eq!(manifest.features["read_path"], "calyx");
    assert_eq!(manifest.features["sqlite_shadow_archived"], "false");
    assert!(sqlite.exists());
    assert!(!archive_path(&sqlite).exists());
    assert_eq!(file_hash(&sqlite), sqlite_hash);
    cleanup(root);
}

#[test]
fn execute_on_shadow_mode_preserves_sqlite_and_manifest() {
    let root = temp_root("shadow-precondition");
    let sqlite = root.join("vault.db");
    let vault = root.join("vault.calyx");
    std::fs::create_dir_all(&root).unwrap();
    seed_sqlite(&sqlite, "shadow_db", 2);
    replay_existing_sqlite(&sqlite, &vault).unwrap();
    let mut shadow = ShadowVault::open(&sqlite, &vault).unwrap();
    let before_hash = file_hash(&sqlite);
    let before_manifest = std::fs::read(vault.join("MANIFEST")).unwrap();

    let error = ShadowRemoval::execute(&mut shadow).unwrap_err();

    assert_eq!(error.code, CALYX_VAULT_FLIP_REQUIRED);
    assert_eq!(
        std::fs::read(vault.join("MANIFEST")).unwrap(),
        before_manifest
    );
    assert_eq!(
        read_shadow_manifest(&vault).unwrap().mode,
        VaultMode::Shadow
    );
    assert_eq!(file_hash(&sqlite), before_hash);
    assert!(!archive_path(&sqlite).exists());
    shadow.close().unwrap();
    cleanup(root);
}

#[test]
fn cli_on_shadow_mode_preserves_sqlite_and_manifest() {
    let root = temp_root("shadow-cli-precondition");
    let sqlite = root.join("vault.db");
    let vault = root.join("vault.calyx");
    std::fs::create_dir_all(&root).unwrap();
    seed_sqlite(&sqlite, "shadow_cli_db", 2);
    replay_existing_sqlite(&sqlite, &vault).unwrap();
    let before_hash = file_hash(&sqlite);
    let before_manifest = std::fs::read(vault.join("MANIFEST")).unwrap();

    let error = super::shadow_removal::run_remove_shadow(&[
        "--sqlite".to_string(),
        sqlite.display().to_string(),
        "--calyx".to_string(),
        vault.display().to_string(),
        "--vault-type".to_string(),
        "text".to_string(),
    ])
    .unwrap_err();

    assert_eq!(error.code(), CALYX_VAULT_FLIP_REQUIRED);
    assert_eq!(
        std::fs::read(vault.join("MANIFEST")).unwrap(),
        before_manifest
    );
    assert_eq!(
        read_shadow_manifest(&vault).unwrap().mode,
        VaultMode::Shadow
    );
    assert_eq!(file_hash(&sqlite), before_hash);
    assert!(!archive_path(&sqlite).exists());
    cleanup(root);
}

#[test]
fn prior_partial_archive_skips_rename_and_writes_calyx_only() {
    let (root, sqlite, vault, mut shadow) = prepared_flipped("partial", "partial_db", 2);
    let sqlite_hash = file_hash(&sqlite);
    let archive = archive_path(&sqlite);
    DefaultPanels::install(&mut shadow, VaultType::Text).unwrap();
    shadow.release_sqlite_for_archive().unwrap();
    std::fs::rename(&sqlite, &archive).unwrap();

    let receipt = ShadowRemoval::execute(&mut shadow).unwrap();

    assert_eq!(receipt.archived_path, archive);
    assert!(!sqlite.exists());
    assert_eq!(file_hash(&archive), sqlite_hash);
    assert_eq!(
        read_shadow_manifest(&vault).unwrap().mode,
        VaultMode::CalyxOnly
    );
    shadow.close().unwrap();
    cleanup(root);
}

#[test]
fn cli_prior_partial_archive_skips_rename_and_writes_calyx_only() {
    let (root, sqlite, vault, shadow) = prepared_flipped("cli-partial", "cli_partial_db", 2);
    shadow.close().unwrap();
    let sqlite_hash = file_hash(&sqlite);
    let archive = archive_path(&sqlite);
    std::fs::rename(&sqlite, &archive).unwrap();

    super::shadow_removal::run_remove_shadow(&[
        "--sqlite".to_string(),
        sqlite.display().to_string(),
        "--calyx".to_string(),
        vault.display().to_string(),
        "--vault-type".to_string(),
        "text".to_string(),
    ])
    .unwrap();

    assert!(!sqlite.exists());
    assert!(archive.exists());
    assert_eq!(file_hash(&archive), sqlite_hash);
    assert_eq!(
        read_shadow_manifest(&vault).unwrap().mode,
        VaultMode::CalyxOnly
    );
    cleanup(root);
}

#[test]
fn archive_collision_fails_without_mode_change_or_overwrite() {
    let (root, sqlite, vault, mut shadow) = prepared_flipped("collision", "collision_db", 2);
    let archive = archive_path(&sqlite);
    std::fs::write(&archive, b"existing archive").unwrap();
    let before_hash = file_hash(&sqlite);

    let error = ShadowRemoval::execute(&mut shadow).unwrap_err();

    assert_eq!(error.code, CALYX_SHADOW_REMOVAL_FAILED);
    assert_eq!(read_shadow_manifest(&vault).unwrap().mode, VaultMode::Calyx);
    assert_eq!(file_hash(&sqlite), before_hash);
    assert_eq!(std::fs::read(&archive).unwrap(), b"existing archive");
    shadow.close().unwrap();
    cleanup(root);
}

#[test]
fn cli_archive_collision_preserves_manifest_sqlite_and_archive() {
    let (root, sqlite, vault, shadow) = prepared_flipped("cli-collision", "cli_collision_db", 2);
    shadow.close().unwrap();
    let archive = archive_path(&sqlite);
    std::fs::write(&archive, b"existing archive").unwrap();
    let before_hash = file_hash(&sqlite);
    let before_manifest = std::fs::read(vault.join("MANIFEST")).unwrap();

    let error = super::shadow_removal::run_remove_shadow(&[
        "--sqlite".to_string(),
        sqlite.display().to_string(),
        "--calyx".to_string(),
        vault.display().to_string(),
        "--vault-type".to_string(),
        "text".to_string(),
    ])
    .unwrap_err();

    assert_eq!(error.code(), CALYX_SHADOW_REMOVAL_FAILED);
    assert_eq!(
        std::fs::read(vault.join("MANIFEST")).unwrap(),
        before_manifest
    );
    assert_eq!(read_shadow_manifest(&vault).unwrap().mode, VaultMode::Calyx);
    assert_eq!(file_hash(&sqlite), before_hash);
    assert_eq!(std::fs::read(&archive).unwrap(), b"existing archive");
    cleanup(root);
}

#[test]
fn lens_mismatch_during_panel_install_leaves_v1_and_sqlite_intact() {
    let (root, sqlite, vault, mut shadow) = prepared_flipped("lens", "lens_db", 2);
    let before_hash = file_hash(&sqlite);

    let error = DefaultPanels::install_with_options(
        &mut shadow,
        VaultType::Text,
        &DefaultPanelOptions {
            expected_base_lens_id: Some("not-the-frozen-lens".to_string()),
            ..DefaultPanelOptions::default()
        },
    )
    .unwrap_err();

    assert_eq!(error.code, "CALYX_LENS_FROZEN_VIOLATION");
    assert_eq!(read_shadow_manifest(&vault).unwrap().mode, VaultMode::Calyx);
    assert!(sqlite.exists());
    assert_eq!(file_hash(&sqlite), before_hash);
    assert!(!archive_path(&sqlite).exists());
    shadow.close().unwrap();
    cleanup(root);
}

#[test]
fn rollback_after_gate_fails_closed() {
    let (root, _sqlite, _vault, mut shadow) = prepared_flipped("gate", "gate_db", 1);
    DefaultPanels::install(&mut shadow, VaultType::Text).unwrap();
    let mut receipt = ShadowRemoval::execute(&mut shadow).unwrap();
    receipt.rollback_gate_passed = true;

    let error = ShadowRemoval::rollback(&receipt).unwrap_err();

    assert_eq!(error.code, CALYX_ROLLBACK_GATE_ALREADY_PASSED);
    shadow.close().unwrap();
    cleanup(root);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(50))]
    #[test]
    fn default_panel_install_is_idempotent_for_any_vault_type(kind in 0u8..4) {
        let vault_type = match kind {
            0 => VaultType::Text,
            1 => VaultType::Code,
            2 => VaultType::Civic,
            _ => VaultType::Media,
        };
        let (root, _sqlite, _vault, mut shadow) = prepared_flipped("panel-prop", "panel_db", 1);
        let options = DefaultPanelOptions {
            backfill: false,
            ..DefaultPanelOptions::default()
        };

        let first = DefaultPanels::install_with_options(&mut shadow, vault_type, &options).unwrap();
        let second = DefaultPanels::install_with_options(&mut shadow, vault_type, &options).unwrap();

        prop_assert_eq!(second, first);
        shadow.close().unwrap();
        cleanup(root);
    }
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

fn file_hash(path: &Path) -> blake3::Hash {
    blake3::hash(&std::fs::read(path).unwrap())
}

fn archive_path(sqlite_path: &Path) -> PathBuf {
    let mut archive = sqlite_path.as_os_str().to_os_string();
    archive.push(".archive");
    PathBuf::from(archive)
}

fn temp_root(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "calyx-shadow-removal-{name}-{}-{}",
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
