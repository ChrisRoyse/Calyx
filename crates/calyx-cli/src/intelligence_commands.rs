use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::error::{CliError, CliResult};
use crate::output;

pub(crate) fn abundance(vault: &Path) -> CliResult {
    let report = read_abundance_report(vault)?;
    output::print_json(&report)
}

pub(crate) fn read_abundance_report(vault: &Path) -> CliResult<Value> {
    let path = abundance_report_path(vault);
    let bytes = fs::read(&path).map_err(|error| {
        CliError::io(format!(
            "read abundance report {} failed: {error}",
            path.display()
        ))
    })?;
    serde_json::from_slice(&bytes).map_err(|error| {
        CliError::usage(format!(
            "parse abundance report {} failed: {error}",
            path.display()
        ))
    })
}

fn abundance_report_path(vault: &Path) -> PathBuf {
    vault.join("intelligence").join("abundance.json")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::CALYX_CLI_IO_ERROR;

    #[test]
    fn abundance_report_path_round_trips_json() {
        let root = temp_root("abundance-ok");
        let intelligence = root.join("intelligence");
        fs::create_dir_all(&intelligence).unwrap();
        fs::write(
            intelligence.join("abundance.json"),
            br#"{"n_lenses":7,"materialized":147}"#,
        )
        .unwrap();

        let readback = read_abundance_report(&root).unwrap();
        assert_eq!(readback["n_lenses"], 7);
        assert_eq!(readback["materialized"], 147);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn missing_abundance_report_is_io_error() {
        let root = temp_root("abundance-missing");
        let error = read_abundance_report(&root).unwrap_err();
        assert_eq!(error.code(), CALYX_CLI_IO_ERROR);
    }

    fn temp_root(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("calyx-cli-{name}-{}", std::process::id()))
    }
}
