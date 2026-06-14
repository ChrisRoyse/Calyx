mod adapter;
mod backfill;
mod errors;
mod manifest;
mod reader;
mod verifier;

use std::path::Path;

use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{Result, VaultStore};
use serde::{Deserialize, Serialize};
use serde_json::json;

use adapter::{VaultSqliteAdapter, default_base_lens_id, default_panel_version};
use backfill::{BackfillMode, BackfillSummary, backfill_default_panel};
use manifest::MigrationManifest;
use reader::{open_sqlite, read_chunk, stream_rows};
use verifier::{StatusReport, VerifyReport, readback_chunk, status, verify_migration};

use crate::error::CliResult;
use crate::output::print_json;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct MigrateVaultReport {
    source_rows: usize,
    migrated_rows: usize,
    manifest: String,
    backfill: Option<BackfillSummary>,
    verify: Option<VerifyReport>,
    status: StatusReport,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct MigrationOptions {
    verify: bool,
    backfill: bool,
    require_backfill: bool,
    batch_size: usize,
    mode: Option<BackfillMode>,
}

pub(crate) fn run(topic: &str, rest: &[String]) -> CliResult {
    match topic {
        "vault" => {
            let (sqlite, vault, options) = parse_vault(rest)?;
            let report = migrate_vault(sqlite, vault, options)?;
            print_json(&report)
        }
        "backfill" => {
            let (sqlite, vault, options) = parse_backfill(rest)?;
            let report = run_backfill(sqlite, vault, options)?;
            print_json(&report)
        }
        "verify" => {
            let (sqlite, vault, require_backfill) = parse_verify(rest)?;
            let report = run_verify(sqlite, vault, require_backfill)?;
            print_json(&report)
        }
        "status" if rest.len() == 1 => {
            let report = run_status(Path::new(&rest[0]))?;
            print_json(&report)
        }
        "readback" if rest.len() == 3 => {
            let value = run_readback(Path::new(&rest[0]), Path::new(&rest[1]), &rest[2])?;
            print_json(&value)
        }
        _ => Err(migrate_usage().into()),
    }
}

fn migrate_vault(
    sqlite_path: &Path,
    vault_dir: &Path,
    options: MigrationOptions,
) -> Result<MigrateVaultReport> {
    let conn = open_sqlite(sqlite_path)?;
    let rows = stream_rows(&conn)?;
    let mut manifest = MigrationManifest::load_or_create(
        vault_dir,
        sqlite_path,
        &rows,
        default_base_lens_id(),
        default_panel_version(),
    )?;
    let vault = open_vault(vault_dir, &manifest)?;
    let adapter = adapter(&manifest)?;
    for row in &rows {
        vault.put(adapter.constellation(row))?;
    }
    vault.flush()?;
    manifest.source_rows = rows.len();
    manifest.migrated_rows = rows.len();
    manifest.write(vault_dir)?;
    let backfill = if options.backfill {
        Some(backfill_default_panel(
            &vault,
            vault_dir,
            &rows,
            &adapter,
            options.mode.unwrap_or(BackfillMode::RealTei),
            options.batch_size.max(1),
        )?)
    } else {
        None
    };
    let verify = if options.verify {
        Some(verify_migration(
            &vault,
            &rows,
            &adapter,
            options.require_backfill || options.backfill,
        )?)
    } else {
        None
    };
    let status = status(&vault)?;
    Ok(MigrateVaultReport {
        source_rows: rows.len(),
        migrated_rows: rows.len(),
        manifest: manifest::manifest_path(vault_dir).display().to_string(),
        backfill,
        verify,
        status,
    })
}

fn run_backfill(
    sqlite_path: &Path,
    vault_dir: &Path,
    options: MigrationOptions,
) -> Result<BackfillSummary> {
    let manifest = MigrationManifest::load(vault_dir)?;
    let conn = open_sqlite(sqlite_path)?;
    let rows = stream_rows(&conn)?;
    let vault = open_vault(vault_dir, &manifest)?;
    let adapter = adapter(&manifest)?;
    backfill_default_panel(
        &vault,
        vault_dir,
        &rows,
        &adapter,
        options.mode.unwrap_or(BackfillMode::RealTei),
        options.batch_size.max(1),
    )
}

fn run_verify(
    sqlite_path: &Path,
    vault_dir: &Path,
    require_backfill: bool,
) -> Result<VerifyReport> {
    let manifest = MigrationManifest::load(vault_dir)?;
    let conn = open_sqlite(sqlite_path)?;
    let rows = stream_rows(&conn)?;
    let vault = open_vault(vault_dir, &manifest)?;
    verify_migration(&vault, &rows, &adapter(&manifest)?, require_backfill)
}

fn run_status(vault_dir: &Path) -> Result<StatusReport> {
    let manifest = MigrationManifest::load(vault_dir)?;
    let vault = open_vault(vault_dir, &manifest)?;
    status(&vault)
}

fn run_readback(sqlite_path: &Path, vault_dir: &Path, chunk_id: &str) -> Result<serde_json::Value> {
    let manifest = MigrationManifest::load(vault_dir)?;
    let conn = open_sqlite(sqlite_path)?;
    let row = read_chunk(&conn, chunk_id)?;
    let vault = open_vault(vault_dir, &manifest)?;
    readback_chunk(&vault, &row, &adapter(&manifest)?)
}

fn open_vault(vault_dir: &Path, manifest: &MigrationManifest) -> Result<AsterVault> {
    AsterVault::new_durable(
        vault_dir,
        manifest.vault_id()?,
        manifest.vault_salt()?,
        VaultOptions::default(),
    )
}

fn adapter(manifest: &MigrationManifest) -> Result<VaultSqliteAdapter> {
    Ok(VaultSqliteAdapter::new(
        manifest.vault_id()?,
        manifest.vault_salt()?,
        manifest.panel_version,
    ))
}

fn parse_vault(rest: &[String]) -> std::result::Result<(&Path, &Path, MigrationOptions), String> {
    if rest.len() < 2 {
        return Err(migrate_usage());
    }
    let mut options = MigrationOptions {
        batch_size: 16,
        ..MigrationOptions::default()
    };
    parse_options(&rest[2..], &mut options, true)?;
    Ok((Path::new(&rest[0]), Path::new(&rest[1]), options))
}

fn parse_backfill(
    rest: &[String],
) -> std::result::Result<(&Path, &Path, MigrationOptions), String> {
    if rest.len() < 2 {
        return Err(migrate_usage());
    }
    let mut options = MigrationOptions {
        backfill: true,
        batch_size: 16,
        ..MigrationOptions::default()
    };
    parse_options(&rest[2..], &mut options, false)?;
    Ok((Path::new(&rest[0]), Path::new(&rest[1]), options))
}

fn parse_verify(rest: &[String]) -> std::result::Result<(&Path, &Path, bool), String> {
    if rest.len() < 2 {
        return Err(migrate_usage());
    }
    let require_backfill = match &rest[2..] {
        [] => false,
        [flag] if flag == "--require-backfill" => true,
        _ => return Err(migrate_usage()),
    };
    Ok((Path::new(&rest[0]), Path::new(&rest[1]), require_backfill))
}

fn parse_options(
    flags: &[String],
    options: &mut MigrationOptions,
    allow_verify: bool,
) -> std::result::Result<(), String> {
    let mut idx = 0;
    while idx < flags.len() {
        match flags[idx].as_str() {
            "--verify" if allow_verify => options.verify = true,
            "--backfill-default-panel" if allow_verify => options.backfill = true,
            "--offline-backfill" => options.mode = Some(BackfillMode::OfflineDeterministic),
            "--batch-size" if idx + 1 < flags.len() => {
                idx += 1;
                options.batch_size = flags[idx]
                    .parse::<usize>()
                    .map_err(|err| format!("invalid --batch-size: {err}"))?;
            }
            _ => return Err(migrate_usage()),
        }
        idx += 1;
    }
    Ok(())
}

fn migrate_usage() -> String {
    json!({
        "usage": [
            "calyx migrate vault <sqlite.db> <vault.calyx> [--verify] [--backfill-default-panel] [--offline-backfill] [--batch-size <n>]",
            "calyx migrate backfill <sqlite.db> <vault.calyx> [--offline-backfill] [--batch-size <n>]",
            "calyx migrate verify <sqlite.db> <vault.calyx> [--require-backfill]",
            "calyx migrate status <vault.calyx>",
            "calyx migrate readback <sqlite.db> <vault.calyx> <chunk_id>"
        ]
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    use super::*;

    #[test]
    fn migrates_and_offline_backfills_default_panel() {
        let root = std::env::temp_dir().join(format!(
            "calyx-migrate-offline-{}-{}",
            std::process::id(),
            manifest::now_ms()
        ));
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
        assert_eq!(
            report.verify.unwrap().missing_backfill,
            Vec::<String>::new()
        );
        assert!(report.status.slot_rows.values().all(|count| *count == 2));
        std::fs::remove_dir_all(root).unwrap();
    }

    fn seed_sqlite(path: &Path) {
        let conn = Connection::open(path).unwrap();
        conn.execute(
            "CREATE TABLE chunks(chunk_id TEXT,database_name TEXT,content TEXT,embedding BLOB)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO chunks VALUES('kernel-1','db','alpha beta',?1)",
            [embedding(&[1.0, 0.0])],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO chunks VALUES('hot-2','db','gamma delta',?1)",
            [embedding(&[0.0, 1.0])],
        )
        .unwrap();
    }

    fn embedding(values: &[f32]) -> Vec<u8> {
        values
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect()
    }
}
