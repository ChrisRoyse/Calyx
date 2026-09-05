//! Field-level validation primitives shared by every configuration rule:
//! normalized-path, mode, account-name, and required-value checks. These carry
//! no broker policy of their own; policy lives in [`super::rules`].

use std::path::Path;

use super::ConfigError;

pub(super) fn validate_absolute_dir(field: &str, path: &Path) -> Result<(), ConfigError> {
    validate_path(field, path)?;
    if path.parent().is_none() {
        return invalid(field, "filesystem root is not a valid authority directory");
    }
    Ok(())
}

pub(super) fn validate_absolute_file(field: &str, path: &Path) -> Result<(), ConfigError> {
    validate_path(field, path)?;
    if path.file_name().is_none() || path.parent().is_none() {
        return invalid(field, "must include an absolute parent and filename");
    }
    Ok(())
}

fn validate_path(field: &str, path: &Path) -> Result<(), ConfigError> {
    let value = path.to_str().ok_or_else(|| ConfigError::InvalidField {
        field: field.into(),
        reason: "must be valid UTF-8 for the Linux broker".into(),
    })?;
    if !value.starts_with('/')
        || value.contains("//")
        || (value.len() > 1 && value.ends_with('/'))
        || value
            .split('/')
            .skip(1)
            .any(|part| matches!(part, "." | ".."))
    {
        return invalid(
            field,
            "must be a normalized absolute Linux path without traversal",
        );
    }
    Ok(())
}

pub(super) fn contains(parent: &Path, child: &Path) -> bool {
    child.starts_with(parent)
}

pub(super) fn parse_mode(field: &str, value: &str) -> Result<u32, ConfigError> {
    if value.len() != 4
        || !value.starts_with('0')
        || !value.bytes().all(|b| matches!(b, b'0'..=b'7'))
    {
        return invalid(field, "must be four octal digits such as 0700");
    }
    u32::from_str_radix(value, 8).map_err(|error| ConfigError::InvalidField {
        field: field.into(),
        reason: error.to_string(),
    })
}

pub(super) fn validate_account(field: &str, value: &str) -> Result<(), ConfigError> {
    if value.is_empty()
        || value.len() > 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return invalid(field, "must be a nonempty account name of at most 64 bytes");
    }
    Ok(())
}

pub(super) fn validate_unit_prefix(value: &str) -> Result<(), ConfigError> {
    if value.is_empty()
        || value.len() > 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
    {
        return invalid("unit_prefix", "contains a forbidden systemd unit character");
    }
    Ok(())
}

pub(super) fn bounded_nonzero(
    field: &str,
    value: usize,
    maximum: usize,
) -> Result<(), ConfigError> {
    if value == 0 || value > maximum {
        return invalid(field, format!("expected 1..={maximum}"));
    }
    Ok(())
}

pub(super) fn require_eq(
    field: &str,
    actual: &str,
    required: &'static str,
) -> Result<(), ConfigError> {
    if actual != required {
        return Err(ConfigError::UnsafeSetting {
            field: field.into(),
            required,
        });
    }
    Ok(())
}

pub(super) fn require_true(field: &str, value: bool) -> Result<(), ConfigError> {
    if !value {
        return Err(ConfigError::UnsafeSetting {
            field: field.into(),
            required: "true",
        });
    }
    Ok(())
}

pub(super) fn require_false(field: &str, value: bool) -> Result<(), ConfigError> {
    if value {
        return Err(ConfigError::UnsafeSetting {
            field: field.into(),
            required: "false",
        });
    }
    Ok(())
}

pub(super) fn invalid<T>(
    field: impl Into<String>,
    reason: impl Into<String>,
) -> Result<T, ConfigError> {
    Err(ConfigError::InvalidField {
        field: field.into(),
        reason: reason.into(),
    })
}
