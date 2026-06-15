use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::json;

use super::ShadowVault;
use super::shadow_harness::read_shadow_manifest;
use crate::error::{CliError, CliResult};
use crate::output::print_json;

pub(crate) fn run_shadow_open(args: &[String]) -> CliResult {
    let (sqlite, vault) = parse_sqlite_vault_args(args)?;
    let mut shadow = ShadowVault::open(sqlite, vault)?;
    let before = sqlite_state(sqlite)?;
    shadow.set_mode(shadow.mode())?;
    let contract = shadow.verify_pg_contract()?;
    let readback = shadow.manifest_readback()?;
    let name = shadow.vault_name().to_string();
    let mode: super::VaultMode = shadow.mode();
    shadow.close()?;
    let after = sqlite_state(sqlite)?;
    print_json(&json!({
        "database_name": name,
        "mode": mode,
        "manifest": readback,
        "pg_contract": contract,
        "sqlite_before": before,
        "sqlite_after": after,
        "sqlite_mtime_unchanged": before == after
    }))
}

pub(crate) fn run_shadow_readback(args: &[String]) -> CliResult {
    let vault = parse_vault_only_args(args)?;
    let readback = read_shadow_manifest(vault)?;
    print_json(&readback)
}

pub(crate) fn readback_shadow_manifest_cli(vault: &Path) -> CliResult {
    print_json(&read_shadow_manifest(vault)?)
}

fn parse_sqlite_vault_args(args: &[String]) -> CliResult<(&Path, &Path)> {
    match args {
        [sqlite_flag, sqlite, vault_flag, vault]
            if sqlite_flag == "--sqlite" && vault_flag == "--vault" =>
        {
            Ok((Path::new(sqlite), Path::new(vault)))
        }
        _ => Err(CliError::usage(
            "usage: calyx leapable shadow-open --sqlite <db> --vault <dir>",
        )),
    }
}

fn parse_vault_only_args(args: &[String]) -> CliResult<&Path> {
    match args {
        [vault_flag, vault] if vault_flag == "--vault" => Ok(Path::new(vault)),
        _ => Err(CliError::usage(
            "usage: calyx leapable shadow-readback --vault <dir>",
        )),
    }
}

fn sqlite_state(path: &Path) -> CliResult<serde_json::Value> {
    let metadata = fs::metadata(path)
        .map_err(|error| CliError::io(format!("stat sqlite {}: {error}", path.display())))?;
    Ok(json!({
        "path": path,
        "bytes": metadata.len(),
        "modified_ms": modified_ms(&metadata)
    }))
}

fn modified_ms(metadata: &fs::Metadata) -> u64 {
    metadata
        .modified()
        .ok()
        .and_then(system_time_ms)
        .unwrap_or(0)
}

fn system_time_ms(time: SystemTime) -> Option<u64> {
    time.duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_millis() as u64)
}
