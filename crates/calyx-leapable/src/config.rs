//! Runtime configuration for the `calyx-leapable` stdio engine.

use std::ffi::OsString;
use std::fs;
use std::path::PathBuf;

use calyx_core::{CalyxError, Result};

/// Fail-closed local error for malformed process arguments.
pub const CALYX_LEAPABLE_CONFIG_INVALID: &str = "CALYX_LEAPABLE_CONFIG_INVALID";
/// Environment variable carrying the vault master key as 32 bytes of lowercase
/// or uppercase hex.
pub const CALYX_LEAPABLE_MASTER_KEY_ENV: &str = "CALYX_LEAPABLE_MASTER_KEY_HEX";
const MASTER_KEY_HEX_LEN: usize = 64;

/// Engine process configuration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EngineConfig {
    /// Canonical Leapable-owned data directory containing all vault refs.
    pub data_dir: PathBuf,
    /// Explicit process-provided vault master key.
    pub master_key: Vec<u8>,
}

impl EngineConfig {
    /// Parses process args. Supported:
    ///
    /// - `--data-dir <path>`: required unless `CALYX_LEAPABLE_DATA_DIR` is set.
    /// - `--master-key-hex <64-hex>`: required unless
    ///   `CALYX_LEAPABLE_MASTER_KEY_HEX` is set.
    pub fn from_args(args: &[String]) -> Result<Self> {
        Self::from_args_with_env(
            args,
            std::env::var_os("CALYX_LEAPABLE_DATA_DIR"),
            std::env::var_os(CALYX_LEAPABLE_MASTER_KEY_ENV),
        )
    }

    fn from_args_with_env(
        args: &[String],
        data_dir_env: Option<OsString>,
        master_key_env: Option<OsString>,
    ) -> Result<Self> {
        let mut data_dir = data_dir_env.map(PathBuf::from);
        let mut master_key_hex = master_key_env
            .map(|value| value.into_string())
            .transpose()
            .map_err(|_| config_error(format!("{CALYX_LEAPABLE_MASTER_KEY_ENV} is not UTF-8")))?;
        let mut index = 0;
        while index < args.len() {
            match args[index].as_str() {
                "--data-dir" => {
                    index += 1;
                    let Some(value) = args.get(index) else {
                        return Err(config_error("--data-dir requires a value"));
                    };
                    data_dir = Some(PathBuf::from(value));
                }
                "--master-key-hex" => {
                    index += 1;
                    let Some(value) = args.get(index) else {
                        return Err(config_error("--master-key-hex requires a value"));
                    };
                    master_key_hex = Some(value.clone());
                }
                other => {
                    return Err(config_error(format!("unknown argument: {other}")));
                }
            }
            index += 1;
        }
        let data_dir = data_dir.ok_or_else(|| {
            config_error("set --data-dir or CALYX_LEAPABLE_DATA_DIR for vault confinement")
        })?;
        let master_key = parse_master_key_hex(master_key_hex.as_deref().ok_or_else(|| {
            config_error(format!(
                "set --master-key-hex or {CALYX_LEAPABLE_MASTER_KEY_ENV} for vault key derivation"
            ))
        })?)?;
        fs::create_dir_all(&data_dir).map_err(|error| {
            config_error(format!("create data dir {}: {error}", data_dir.display()))
        })?;
        let data_dir = data_dir.canonicalize().map_err(|error| {
            config_error(format!(
                "canonicalize data dir {}: {error}",
                data_dir.display()
            ))
        })?;
        Ok(Self {
            data_dir,
            master_key,
        })
    }
}

fn parse_master_key_hex(value: &str) -> Result<Vec<u8>> {
    let value = value.trim();
    if value.len() != MASTER_KEY_HEX_LEN {
        return Err(config_error(format!(
            "master key must be {MASTER_KEY_HEX_LEN} hex characters"
        )));
    }
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(MASTER_KEY_HEX_LEN / 2);
    for (index, pair) in bytes.chunks_exact(2).enumerate() {
        let hi = hex_nibble(pair[0], index * 2)?;
        let lo = hex_nibble(pair[1], index * 2 + 1)?;
        out.push((hi << 4) | lo);
    }
    Ok(out)
}

fn hex_nibble(value: u8, index: usize) -> Result<u8> {
    match value {
        b'0'..=b'9' => Ok(value - b'0'),
        b'a'..=b'f' => Ok(value - b'a' + 10),
        b'A'..=b'F' => Ok(value - b'A' + 10),
        _ => Err(config_error(format!(
            "master key contains non-hex byte at index {index}"
        ))),
    }
}

fn config_error(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_LEAPABLE_CONFIG_INVALID,
        message: message.into(),
        remediation: "launch calyx-leapable with --data-dir and explicit vault key material from the Leapable sidecar",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_args_rejects_unknown_arg() {
        let err = EngineConfig::from_args_with_env(
            &["--socket".to_string()],
            None,
            Some(OsString::from(test_key_hex())),
        )
        .unwrap_err();
        assert_eq!(err.code, CALYX_LEAPABLE_CONFIG_INVALID);
    }

    #[test]
    fn from_args_requires_master_key() {
        let dir = std::env::temp_dir().join("calyx-leapable-config-missing-key");
        let err =
            EngineConfig::from_args_with_env(&[], Some(dir.into_os_string()), None).unwrap_err();
        assert_eq!(err.code, CALYX_LEAPABLE_CONFIG_INVALID);
    }

    #[test]
    fn from_args_rejects_non_hex_master_key() {
        let dir = std::env::temp_dir().join("calyx-leapable-config-bad-key");
        let err = EngineConfig::from_args_with_env(
            &[],
            Some(dir.into_os_string()),
            Some(OsString::from("z".repeat(MASTER_KEY_HEX_LEN))),
        )
        .unwrap_err();
        assert_eq!(err.code, CALYX_LEAPABLE_CONFIG_INVALID);
    }

    fn test_key_hex() -> String {
        "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff".to_string()
    }
}
