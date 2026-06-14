use std::fs;
use std::path::Path;

use calyx_anneal::{read_tripwire_config_from_vault, tripwire_config_path};
use serde_json::json;

pub fn readback_tripwire_config(vault: &Path) -> crate::error::CliResult {
    let config_path = tripwire_config_path(vault);
    let bytes = fs::read(&config_path)
        .map_err(|error| format!("read {}: {error}", config_path.display()))?;
    let parsed = read_tripwire_config_from_vault(vault).map_err(|error| error.to_string())?;
    let readback = json!({
        "surface": "config.tripwire",
        "source_of_truth": "vault .anneal/tripwire.toml",
        "config_path": display_path(&parsed.config_path),
        "config_len": bytes.len(),
        "config_blake3": blake3::hash(&bytes).to_hex().to_string(),
        "thresholds": parsed.thresholds,
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
