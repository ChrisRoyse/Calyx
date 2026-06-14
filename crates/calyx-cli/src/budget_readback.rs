use std::fs;
use std::path::Path;

use calyx_anneal::{budget_config_path, read_budget_config_from_vault};
use serde_json::json;

pub fn readback_budget_config(vault: &Path) -> crate::error::CliResult {
    let config_path = budget_config_path(vault);
    let bytes = fs::read(&config_path)
        .map_err(|error| format!("read {}: {error}", config_path.display()))?;
    let parsed = read_budget_config_from_vault(vault).map_err(|error| error.to_string())?;
    let readback = json!({
        "surface": "config.budget",
        "source_of_truth": "vault .anneal/budget.toml",
        "config_path": display_path(&parsed.config_path),
        "config_len": bytes.len(),
        "config_blake3": blake3::hash(&bytes).to_hex().to_string(),
        "config": parsed.config,
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&readback).map_err(|error| error.to_string())?
    );
    Ok(())
}

fn display_path(path: &Path) -> String {
    path.canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .display()
        .to_string()
}
