//! Vault-owned retained source bytes used for deterministic replay.

use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use calyx_core::{CalyxError, Input, InputRef, Modality, Result};

use crate::cf::full_content_hash;

pub const VAULT_INPUT_POINTER_PREFIX: &str = "calyx-vault://inputs/";
pub const CALYX_INPUT_POINTER_INVALID: &str = "CALYX_INPUT_POINTER_INVALID";
pub const CALYX_INPUT_BLOB_WRITE_FAILED: &str = "CALYX_INPUT_BLOB_WRITE_FAILED";
pub const CALYX_INPUT_BLOB_UNAVAILABLE: &str = "CALYX_INPUT_BLOB_UNAVAILABLE";
pub const CALYX_INPUT_BLOB_HASH_MISMATCH: &str = "CALYX_INPUT_BLOB_HASH_MISMATCH";

const INPUT_DIR: &str = "inputs";
static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

/// Retains text bytes under their raw BLAKE3 digest and returns a replayable input.
pub fn retain_text_input(vault_dir: &Path, text: &str) -> Result<Input> {
    let bytes = text.as_bytes();
    let hash = raw_input_hash(bytes);
    let pointer = canonical_text_pointer(&hash);
    let path = retained_pointer_path(vault_dir, &pointer)?;
    install_blob(&path, bytes)?;
    let readback = read_retained_bytes(vault_dir, &pointer, &hash)?;
    if readback != bytes {
        return Err(input_error(
            CALYX_INPUT_BLOB_HASH_MISMATCH,
            format!(
                "retained input {} has hash-compatible but byte-distinct content",
                path.display()
            ),
            "quarantine the retained input blob and re-ingest from authoritative source bytes",
        ));
    }
    Ok(Input::new(Modality::Text, readback).with_pointer(pointer))
}

/// Resolves and verifies the retained bytes named by a stored input reference.
pub fn input_from_ref(vault_dir: &Path, modality: Modality, input_ref: &InputRef) -> Result<Input> {
    if input_ref.redacted {
        return Err(input_error(
            CALYX_INPUT_BLOB_UNAVAILABLE,
            "retained input is intentionally redacted",
            "use a non-redacted constellation with authoritative retained source bytes",
        ));
    }
    let pointer = input_ref.pointer.as_deref().ok_or_else(|| {
        input_error(
            CALYX_INPUT_BLOB_UNAVAILABLE,
            "input reference has no retained pointer",
            "re-ingest the exact authoritative source bytes to backfill a retained pointer",
        )
    })?;
    let bytes = read_retained_bytes(vault_dir, pointer, &input_ref.hash)?;
    Ok(Input::new(modality, bytes).with_pointer(pointer.to_string()))
}

/// Reads a vault-owned pointer and verifies its bytes against the stored input hash.
pub fn read_retained_bytes(
    vault_dir: &Path,
    pointer: &str,
    expected_hash: &[u8; 32],
) -> Result<Vec<u8>> {
    let path = retained_pointer_path(vault_dir, pointer)?;
    let bytes = fs::read(&path).map_err(|error| {
        input_error(
            CALYX_INPUT_BLOB_UNAVAILABLE,
            format!("read retained input {}: {error}", path.display()),
            "restore the named vault input blob from authoritative source bytes",
        )
    })?;
    verify_input_hash(&bytes, expected_hash, pointer, &path)?;
    Ok(bytes)
}

/// Returns the canonical pointer for newly retained text bytes.
pub fn canonical_text_pointer(hash: &[u8; 32]) -> String {
    format!("{VAULT_INPUT_POINTER_PREFIX}{}.bin", hex(hash))
}

/// Resolves a vault input pointer without allowing absolute or parent components.
pub fn retained_pointer_path(vault_dir: &Path, pointer: &str) -> Result<PathBuf> {
    let relative = validate_pointer_syntax(pointer)?;
    let input_dir = vault_dir.join(INPUT_DIR);
    reject_symlink(&input_dir, pointer)?;
    let path = input_dir.join(relative);
    reject_symlink(&path, pointer)?;
    Ok(path)
}

/// Validates that a pointer is a canonical vault-owned input location.
pub fn validate_vault_input_pointer(pointer: &str) -> Result<()> {
    validate_pointer_syntax(pointer).map(|_| ())
}

fn validate_pointer_syntax(pointer: &str) -> Result<&str> {
    let relative = pointer
        .strip_prefix(VAULT_INPUT_POINTER_PREFIX)
        .ok_or_else(|| invalid_pointer(pointer, "missing vault input prefix"))?;
    if !is_canonical_blob_name(relative) {
        return Err(invalid_pointer(
            pointer,
            "expected one lowercase 64-hex .bin blob name",
        ));
    }
    let relative = Path::new(relative);
    if relative
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(invalid_pointer(
            pointer,
            "path escapes the vault input directory",
        ));
    }
    Ok(relative.to_str().expect("canonical input pointer is ASCII"))
}

fn install_blob(path: &Path, bytes: &[u8]) -> Result<()> {
    match fs::read(path) {
        Ok(existing) => return verify_exact_existing(path, &existing, bytes),
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => return Err(write_error(format!("read {}: {error}", path.display()))),
    }
    let parent = path
        .parent()
        .ok_or_else(|| write_error(format!("retained input {} has no parent", path.display())))?;
    fs::create_dir_all(parent)
        .map_err(|error| write_error(format!("create {}: {error}", parent.display())))?;
    crate::fsync::sync_parent(parent, "retained input directory")
        .map_err(|error| write_error(error.to_string()))?;

    let temp = path.with_extension(format!(
        "bin.tmp-{}-{}",
        std::process::id(),
        NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
    ));
    let write_result = (|| {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp)
            .map_err(|error| write_error(format!("create {}: {error}", temp.display())))?;
        file.write_all(bytes)
            .map_err(|error| write_error(format!("write {}: {error}", temp.display())))?;
        file.sync_all()
            .map_err(|error| write_error(format!("sync {}: {error}", temp.display())))?;
        match fs::hard_link(&temp, path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {
                let existing = fs::read(path).map_err(|read_error| {
                    write_error(format!("read raced blob {}: {read_error}", path.display()))
                })?;
                verify_exact_existing(path, &existing, bytes)
            }
            Err(error) => Err(write_error(format!(
                "install retained input {} -> {}: {error}",
                temp.display(),
                path.display()
            ))),
        }
    })();
    let cleanup_result = fs::remove_file(&temp);
    write_result?;
    if let Err(error) = cleanup_result
        && error.kind() != ErrorKind::NotFound
    {
        return Err(write_error(format!("remove {}: {error}", temp.display())));
    }
    crate::fsync::sync_parent(path, "retained input blob")
        .map_err(|error| write_error(error.to_string()))?;
    Ok(())
}

fn verify_exact_existing(path: &Path, existing: &[u8], expected: &[u8]) -> Result<()> {
    if existing == expected {
        return Ok(());
    }
    Err(input_error(
        CALYX_INPUT_BLOB_HASH_MISMATCH,
        format!(
            "retained input {} exists with different bytes",
            path.display()
        ),
        "quarantine the conflicting blob and restore it from authoritative source bytes",
    ))
}

fn verify_input_hash(bytes: &[u8], expected: &[u8; 32], pointer: &str, path: &Path) -> Result<()> {
    let raw = raw_input_hash(bytes);
    let legacy = full_content_hash([bytes]);
    let canonical = canonical_text_pointer(&raw);
    let legacy_pointer = canonical_text_pointer(&legacy);
    if pointer != canonical && !(expected == &legacy && pointer == legacy_pointer) {
        return Err(input_error(
            CALYX_INPUT_BLOB_HASH_MISMATCH,
            format!(
                "retained input {} is named by {pointer}, but its bytes require {canonical}",
                path.display()
            ),
            "quarantine the misnamed blob and restore it under its raw BLAKE3 pointer",
        ));
    }
    if expected == &raw || expected == &legacy {
        return Ok(());
    }
    Err(input_error(
        CALYX_INPUT_BLOB_HASH_MISMATCH,
        format!(
            "retained input {} does not match stored input hash {}",
            path.display(),
            hex(expected)
        ),
        "restore the exact authoritative bytes or repair the corrupt input reference",
    ))
}

fn is_canonical_blob_name(relative: &str) -> bool {
    let Some(hash) = relative.strip_suffix(".bin") else {
        return false;
    };
    hash.len() == 64
        && hash
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn reject_symlink(path: &Path, pointer: &str) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(invalid_pointer(
            pointer,
            "retained input path contains a symbolic link",
        )),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(invalid_pointer(
            pointer,
            &format!("cannot inspect retained input path: {error}"),
        )),
    }
}

fn raw_input_hash(bytes: &[u8]) -> [u8; 32] {
    *blake3::hash(bytes).as_bytes()
}

fn invalid_pointer(pointer: &str, reason: &str) -> CalyxError {
    input_error(
        CALYX_INPUT_POINTER_INVALID,
        format!("invalid retained input pointer {pointer:?}: {reason}"),
        "use a vault-owned calyx-vault://inputs/<content-hash>.bin pointer",
    )
}

fn write_error(message: impl Into<String>) -> CalyxError {
    input_error(
        CALYX_INPUT_BLOB_WRITE_FAILED,
        message,
        "repair the vault input directory and retry retention",
    )
}

fn input_error(
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

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests;
