use std::path::Path;

use calyx_aster::manifest::ManifestStore;

pub fn readback_vault_manifest_field(vault: &Path, field: &str) -> Result<(), String> {
    let manifest = ManifestStore::open(vault)
        .load_current()
        .map_err(|error| error.to_string())?;
    let manifest_json = serde_json::to_value(&manifest).map_err(|error| error.to_string())?;
    let value = manifest_json
        .get(field)
        .ok_or_else(|| format!("manifest field `{field}` not found"))?;
    println!(
        "{}",
        serde_json::to_string_pretty(value).map_err(|error| error.to_string())?
    );
    Ok(())
}
