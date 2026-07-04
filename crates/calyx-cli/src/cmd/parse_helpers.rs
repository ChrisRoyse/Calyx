use super::PANEL_TEMPLATES;
use crate::error::{CliError, CliResult};

pub(crate) fn validate_vault_name(name: &str) -> CliResult {
    if name.is_empty() {
        return Err(CliError::usage("vault name must not be empty"));
    }
    if name.contains(['/', '\\']) || name == "." || name == ".." {
        return Err(CliError::usage(
            "vault name must be a name, not a filesystem path",
        ));
    }
    if name.chars().any(char::is_whitespace) {
        return Err(CliError::usage("vault name must not contain spaces"));
    }
    Ok(())
}

pub(crate) fn validate_panel_template_name(value: &str) -> CliResult {
    if value.is_empty()
        || value.contains(['/', '\\'])
        || value == "."
        || value == ".."
        || value.chars().any(char::is_whitespace)
    {
        Err(CliError::usage(format!(
            "invalid --panel-template {value}; use a built-in template ({}) or a saved path-safe template name",
            PANEL_TEMPLATES.join(", ")
        )))
    } else {
        Ok(())
    }
}

pub(crate) fn value<'a>(args: &'a [String], index: usize, flag: &str) -> CliResult<&'a str> {
    args.get(index)
        .map(String::as_str)
        .ok_or_else(|| CliError::usage(format!("{flag} requires a value")))
}
