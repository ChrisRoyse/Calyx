use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

use calyx_core::{CalyxError, VaultId, content_address};
use rand::{RngCore, rngs::OsRng};
use ulid::Ulid;

use super::EngineResult;

pub(super) const SALT_FILE_NAME: &str = "salt";
const SALT_LEN: usize = 32;
const LEGACY_SALT_LEN: usize = 16;
const CALYX_LEAPABLE_SALT_INVALID: &str = "CALYX_LEAPABLE_SALT_INVALID";
const CALYX_LEAPABLE_SALT_IO: &str = "CALYX_LEAPABLE_SALT_IO";

pub(super) fn vault_id_for(vault_ref: &str) -> VaultId {
    VaultId::from_ulid(Ulid::from_bytes(content_address([vault_ref.as_bytes()])))
}

pub(super) fn salt_for(vault_ref: &str) -> Vec<u8> {
    content_address([
        b"calyx-leapable-vault-salt".as_slice(),
        vault_ref.as_bytes(),
    ])
    .to_vec()
}

pub(super) fn salt_for_dir(dir: &Path, vault_ref: &str) -> EngineResult<Vec<u8>> {
    let path = dir.join(SALT_FILE_NAME);
    match fs::read(&path) {
        Ok(bytes) => return validate_salt(bytes, &path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(salt_error(
                CALYX_LEAPABLE_SALT_IO,
                format!("read salt file {}: {error}", path.display()),
                "check vault directory permissions and retry",
            )
            .into());
        }
    }
    if legacy_vault_bytes_exist(dir) {
        eprintln!(
            "calyx-leapable: CALYX_LEAPABLE_LEGACY_SALT: deterministic salt fallback vault_ref={vault_ref}"
        );
        return Ok(salt_for(vault_ref));
    }
    let mut salt = vec![0_u8; SALT_LEN];
    OsRng.fill_bytes(&mut salt);
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|error| {
            salt_error(
                CALYX_LEAPABLE_SALT_IO,
                format!("create salt file {}: {error}", path.display()),
                "ensure the vault directory is writable and retry vault.create",
            )
        })?;
    file.write_all(&salt).map_err(|error| {
        salt_error(
            CALYX_LEAPABLE_SALT_IO,
            format!("write salt file {}: {error}", path.display()),
            "ensure the vault directory is writable and retry vault.create",
        )
    })?;
    file.sync_all().map_err(|error| {
        salt_error(
            CALYX_LEAPABLE_SALT_IO,
            format!("sync salt file {}: {error}", path.display()),
            "ensure the vault directory supports durable file sync and retry vault.create",
        )
    })?;
    Ok(salt)
}

fn validate_salt(bytes: Vec<u8>, path: &Path) -> EngineResult<Vec<u8>> {
    if bytes.len() == SALT_LEN {
        return Ok(bytes);
    }
    if bytes.len() == LEGACY_SALT_LEN {
        eprintln!(
            "calyx-leapable: CALYX_LEAPABLE_LEGACY_SALT: legacy salt file {} has {LEGACY_SALT_LEN} bytes",
            path.display()
        );
        return Ok(bytes);
    }
    Err(salt_error(
        CALYX_LEAPABLE_SALT_INVALID,
        format!(
            "salt file {} has {} bytes, expected {SALT_LEN} or legacy {LEGACY_SALT_LEN}",
            path.display(),
            bytes.len()
        ),
        "restore the original vault salt file from backup; changing it orphans content ids",
    )
    .into())
}

fn legacy_vault_bytes_exist(dir: &Path) -> bool {
    dir.join("cf").exists() || dir.join("wal").exists()
}

fn salt_error(
    code: &'static str,
    message: impl Into<String>,
    remediation: &'static str,
) -> CalyxError {
    CalyxError {
        code,
        message: message.into(),
        remediation,
    }
}
