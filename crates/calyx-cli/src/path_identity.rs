use std::path::Path;

use crate::error::{CliError, CliResult};

pub(crate) fn vault_template_source(path: &Path) -> CliResult<String> {
    Ok(format!("vault:{}", canonical_protocol_path(path)?))
}

fn canonical_protocol_path(path: &Path) -> CliResult<String> {
    let canonical = path.canonicalize()?;
    let raw = canonical.to_str().ok_or_else(|| {
        CliError::io(format!(
            "canonical path {} is not valid UTF-8",
            canonical.display()
        ))
    })?;
    Ok(normalize_windows_extended_prefix(raw).replace('\\', "/"))
}

fn normalize_windows_extended_prefix(raw: &str) -> &str {
    raw.strip_prefix(r"\\?\").unwrap_or(raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_paths_strip_windows_extended_prefix_and_normalize_separators() {
        let normalized =
            normalize_windows_extended_prefix(r"\\?\C:\tmp\vaults\01ABC").replace('\\', "/");

        assert_eq!(normalized, "C:/tmp/vaults/01ABC");
    }
}
