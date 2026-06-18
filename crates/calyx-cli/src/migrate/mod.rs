pub(crate) mod adapter;
pub(crate) mod backfill;
pub(crate) mod errors;
pub(crate) mod manifest;
pub(crate) mod reader;
pub(crate) mod temporal;
#[cfg(test)]
mod tests;
pub(crate) mod verifier;
#[cfg(test)]
mod verify_tests;

use std::collections::BTreeMap;
use std::path::Path;

use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{CalyxError, LensId, Result, SlotId, VaultStore};
use serde::{Deserialize, Serialize};
use serde_json::json;

use adapter::{VaultSqliteAdapter, default_base_lens_id, default_panel_version};
use backfill::{BackfillMode, BackfillSummary, backfill_default_panel};
use manifest::{MigrationManifest, hex_encode};
use reader::{open_sqlite, read_chunk, row_count, stream_rows};
use verifier::{
    StatusReport, VerifyError, VerifyReport, readback_chunk, row_exists_and_matches, status,
    verify_migration,
};

use crate::error::CliResult;
use crate::output::print_json;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct MigrateVaultReport {
    source_rows: usize,
    migrated_rows: usize,
    written_rows: usize,
    skipped_rows: usize,
    batches_completed: usize,
    dry_run: bool,
    manifest: String,
    gte_lens_id: String,
    gte_endpoint: Option<String>,
    backfill: Option<BackfillSummary>,
    verify: Option<VerifyReport>,
    verify_summary: Option<String>,
    status: Option<StatusReport>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct MigrationOptions {
    verify: bool,
    backfill: bool,
    require_backfill: bool,
    dry_run: bool,
    batch_size: usize,
    gte_lens_id: Option<String>,
    gte_endpoint: Option<String>,
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
            print_verify_report(&report)
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
) -> CliResult<MigrateVaultReport> {
    let conn = open_sqlite(sqlite_path)?;
    let source_rows = usize::try_from(row_count(&conn)?)
        .map_err(|_| errors::schema("SQLite row count exceeds usize"))?;
    eprintln!("migrating {source_rows} rows...");
    let rows = stream_rows(&conn)?;
    if rows.len() != source_rows {
        return Err(errors::schema(format!(
            "streamed {} rows but row_count reported {source_rows}",
            rows.len()
        ))
        .into());
    }
    let base_lens_id = options
        .gte_lens_id
        .clone()
        .unwrap_or_else(default_base_lens_id);
    validate_lens_id(&base_lens_id)?;
    let mut manifest = MigrationManifest::load_or_create(
        vault_dir,
        sqlite_path,
        &rows,
        base_lens_id,
        default_panel_version(),
    )?;
    ensure_manifest_matches_options(&manifest, &options)?;
    let adapter = adapter(&manifest)?;
    ensure_unique_cx_ids(&adapter, &rows)?;
    let batch_size = options.batch_size.max(1);
    let batches_planned = rows.len().div_ceil(batch_size);
    if options.dry_run {
        for row in &rows {
            adapter.constellation(row)?;
        }
        eprintln!(
            "dry-run: would migrate {} rows in {batches_planned} batches",
            rows.len()
        );
        return Ok(MigrateVaultReport {
            source_rows,
            migrated_rows: rows.len(),
            written_rows: 0,
            skipped_rows: 0,
            batches_completed: batches_planned,
            dry_run: true,
            manifest: manifest::manifest_path(vault_dir).display().to_string(),
            gte_lens_id: manifest.base_lens_id.clone(),
            gte_endpoint: options.gte_endpoint.clone(),
            backfill: None,
            verify: None,
            verify_summary: None,
            status: None,
        });
    }
    if let Some(endpoint) = &options.gte_endpoint {
        eprintln!("gte endpoint configured: {endpoint}");
    }
    let vault = open_vault(vault_dir, &manifest)?;
    let mut written_rows = 0;
    let mut skipped_rows = 0;
    let mut processed_rows = 0;
    let mut batches_completed = 0;
    for batch in rows.chunks(batch_size) {
        for row in batch {
            if row_exists_and_matches(&vault, row, &adapter)? {
                skipped_rows += 1;
            } else {
                vault.put(adapter.constellation(row)?)?;
                written_rows += 1;
            }
            processed_rows += 1;
        }
        batches_completed += 1;
        if processed_rows == source_rows || processed_rows % 1000 == 0 {
            eprintln!("migrated {processed_rows}/{source_rows}...");
        }
    }
    vault.flush()?;
    manifest.source_rows = source_rows;
    manifest.migrated_rows = written_rows + skipped_rows;
    manifest.write(vault_dir)?;
    eprintln!("migration complete: {written_rows} new, {skipped_rows} duplicate");
    let backfill = if options.backfill {
        Some(backfill_default_panel(
            &vault,
            vault_dir,
            &rows,
            &adapter,
            options.mode.unwrap_or(BackfillMode::RealTei),
            batch_size,
        )?)
    } else {
        None
    };
    let verify = if options.verify {
        let report = verify_migration(
            &vault,
            &rows,
            &adapter,
            options.require_backfill || options.backfill,
        )?;
        if report.mismatched > 0 {
            return Err(verify_failed_error(&report).into());
        }
        Some(report)
    } else {
        None
    };
    let verify_summary = verify.as_ref().map(verify_success_summary);
    let status = status(&vault, vault_dir)?;
    Ok(MigrateVaultReport {
        source_rows,
        migrated_rows: written_rows + skipped_rows,
        written_rows,
        skipped_rows,
        batches_completed,
        dry_run: false,
        manifest: manifest::manifest_path(vault_dir).display().to_string(),
        gte_lens_id: manifest.base_lens_id.clone(),
        gte_endpoint: options.gte_endpoint.clone(),
        backfill,
        verify,
        verify_summary,
        status: Some(status),
    })
}

fn run_backfill(
    sqlite_path: &Path,
    vault_dir: &Path,
    options: MigrationOptions,
) -> CliResult<BackfillSummary> {
    let manifest = MigrationManifest::load(vault_dir)?;
    let conn = open_sqlite(sqlite_path)?;
    let rows = stream_rows(&conn)?;
    let vault = open_vault(vault_dir, &manifest)?;
    let adapter = adapter(&manifest)?;
    Ok(backfill_default_panel(
        &vault,
        vault_dir,
        &rows,
        &adapter,
        options.mode.unwrap_or(BackfillMode::RealTei),
        options.batch_size.max(1),
    )?)
}

fn run_verify(
    sqlite_path: &Path,
    vault_dir: &Path,
    require_backfill: bool,
) -> CliResult<VerifyReport> {
    let manifest = MigrationManifest::load(vault_dir)?;
    let conn = open_sqlite(sqlite_path)?;
    let rows = stream_rows(&conn)?;
    let vault = open_vault(vault_dir, &manifest)?;
    Ok(verify_migration(
        &vault,
        &rows,
        &adapter(&manifest)?,
        require_backfill,
    )?)
}

fn print_verify_report(report: &VerifyReport) -> CliResult {
    if report.mismatched == 0 {
        println!("{}", verify_success_summary(report));
        return Ok(());
    }
    for error in &report.errors {
        println!("{}", verify_error_line(error));
    }
    eprintln!("FAILED: {} mismatches", report.mismatched);
    Err(verify_failed_error(report).into())
}

fn verify_success_summary(report: &VerifyReport) -> String {
    format!(
        "verified {}/{} rows: byte-exact on content",
        report.matched, report.total
    )
}

fn verify_error_line(error: &VerifyError) -> String {
    format!(
        "MISMATCH row={} chunk_id={} expected={} actual={}",
        error.row_num,
        error.chunk_id,
        hex_encode(&error.expected_hash),
        hex_encode(&error.actual_hash)
    )
}

fn verify_failed_error(report: &VerifyReport) -> CalyxError {
    CalyxError::aster_corrupt_shard(format!(
        "{} migration content hash mismatches",
        report.mismatched
    ))
}

fn run_status(vault_dir: &Path) -> CliResult<StatusReport> {
    let manifest = MigrationManifest::load(vault_dir)?;
    let vault = open_vault(vault_dir, &manifest)?;
    Ok(status(&vault, vault_dir)?)
}

fn run_readback(
    sqlite_path: &Path,
    vault_dir: &Path,
    chunk_id: &str,
) -> CliResult<serde_json::Value> {
    let manifest = MigrationManifest::load(vault_dir)?;
    let conn = open_sqlite(sqlite_path)?;
    let row = read_chunk(&conn, chunk_id)?;
    let vault = open_vault(vault_dir, &manifest)?;
    Ok(readback_chunk(&vault, &row, &adapter(&manifest)?)?)
}

pub(crate) fn open_vault(vault_dir: &Path, manifest: &MigrationManifest) -> Result<AsterVault> {
    AsterVault::new_durable(
        vault_dir,
        manifest.vault_id()?,
        manifest.vault_salt()?,
        VaultOptions::default(),
    )
}

pub(crate) fn adapter(manifest: &MigrationManifest) -> Result<VaultSqliteAdapter> {
    let lens_id = manifest
        .base_lens_id
        .parse::<LensId>()
        .map_err(|err| errors::manifest(format!("invalid base_lens_id: {err}")))?;
    Ok(VaultSqliteAdapter::new_with_lens_slot(
        manifest.vault_id()?,
        manifest.panel_version,
        lens_id,
        SlotId::new(manifest.base_slot_id),
    ))
}

fn validate_lens_id(value: &str) -> CliResult {
    value
        .parse::<LensId>()
        .map(|_| ())
        .map_err(|err| errors::manifest(format!("invalid --gte-lens-id: {err}")).into())
}

fn ensure_manifest_matches_options(
    manifest: &MigrationManifest,
    options: &MigrationOptions,
) -> CliResult {
    if let Some(requested) = &options.gte_lens_id
        && manifest.base_lens_id != *requested
    {
        return Err(errors::manifest(format!(
            "existing manifest base_lens_id {} does not match --gte-lens-id {requested}",
            manifest.base_lens_id
        ))
        .into());
    }
    Ok(())
}

pub(crate) fn ensure_unique_cx_ids(
    adapter: &VaultSqliteAdapter,
    rows: &[reader::ChunkRow],
) -> CliResult {
    let mut seen = BTreeMap::new();
    for row in rows {
        let cx_id = adapter.cx_id(row);
        if let Some(first_row_num) = seen.insert(cx_id, row.row_num) {
            return Err(errors::schema(format!(
                "rows {first_row_num} and {} map to the same content-addressed cx_id {cx_id}; duplicate content cannot preserve distinct SQLite metadata",
                row.row_num
            ))
            .into());
        }
    }
    Ok(())
}

fn parse_vault(rest: &[String]) -> std::result::Result<(&Path, &Path, MigrationOptions), String> {
    if rest.len() < 2 {
        return Err(migrate_usage());
    }
    let mut options = MigrationOptions {
        batch_size: 100,
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
            "--dry-run" if allow_verify => options.dry_run = true,
            "--backfill-default-panel" if allow_verify => options.backfill = true,
            "--offline-backfill" => options.mode = Some(BackfillMode::OfflineDeterministic),
            "--gte-lens-id" if allow_verify && idx + 1 < flags.len() => {
                idx += 1;
                let value = flags[idx].clone();
                value
                    .parse::<LensId>()
                    .map_err(|err| format!("invalid --gte-lens-id: {err}"))?;
                options.gte_lens_id = Some(value);
            }
            "--gte-endpoint" if allow_verify && idx + 1 < flags.len() => {
                idx += 1;
                options.gte_endpoint = Some(flags[idx].clone());
            }
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
            "calyx migrate vault <sqlite.db> <vault.calyx> [--verify] [--dry-run] [--gte-lens-id <hex16>] [--gte-endpoint <url>] [--backfill-default-panel] [--offline-backfill] [--batch-size <n>]",
            "calyx migrate backfill <sqlite.db> <vault.calyx> [--offline-backfill] [--batch-size <n>]",
            "calyx migrate verify <sqlite.db> <vault.calyx> [--require-backfill]",
            "calyx migrate status <vault.calyx>",
            "calyx migrate readback <sqlite.db> <vault.calyx> <chunk_id>"
        ]
    })
    .to_string()
}
