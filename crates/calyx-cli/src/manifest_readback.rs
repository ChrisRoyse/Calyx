use std::path::Path;

use calyx_aster::manifest::ManifestStore;

use crate::error::{CliError, CliResult};
use crate::output::print_json;

pub fn readback_vault_manifest_field(vault: &Path, field: &str) -> CliResult {
    let manifest = ManifestStore::open(vault).load_current()?;
    let manifest_json = serde_json::to_value(&manifest)?;
    let value = manifest_json
        .get(field)
        .ok_or_else(|| CliError::usage(format!("manifest field `{field}` not found")))?;
    print_json(value)
}
