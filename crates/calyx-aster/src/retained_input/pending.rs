//! Crash-recoverable publication for retained input objects.

mod names;

use std::collections::BTreeMap;
use std::fs;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use calyx_core::{CalyxError, Clock, InputRef, Result, VaultStore};

use crate::cf::ColumnFamily;
use crate::vault::{AsterVault, encode};

use super::permissions;
use super::{
    CALYX_INPUT_BLOB_HASH_MISMATCH, CALYX_INPUT_RETENTION_RECONCILIATION_REQUIRED,
    CALYX_INPUT_RETENTION_ROLLBACK_FAILED, canonical_text_pointer, input_error, raw_input_hash,
    retained_pointer_path, validate_vault_input_pointer, verify_exact_existing,
};
use names::{hash_from_blob_name, hex, is_pending_temp_name};

const INPUT_DIR: &str = "inputs";
const PENDING_DIR: &str = ".pending";
static NEXT_PENDING_TEMP: AtomicU64 = AtomicU64::new(0);

#[derive(Debug)]
struct PendingItem {
    pointer: String,
    pending_path: PathBuf,
    final_path: PathBuf,
}

/// Exact source objects staged before an Aster commit and published afterward.
#[derive(Debug)]
pub(crate) struct PendingRetentionBatch {
    pending_dir: PathBuf,
    items: Vec<PendingItem>,
}

impl PendingRetentionBatch {
    pub(crate) fn stage<'a>(
        vault_dir: &Path,
        sources: impl IntoIterator<Item = (&'a InputRef, &'a [u8])>,
    ) -> Result<Self> {
        permissions::prepare_store_for_write(vault_dir)?;
        let mut unique = BTreeMap::<String, (&InputRef, &[u8])>::new();
        for (input_ref, bytes) in sources {
            let pointer = validate_source(input_ref, bytes)?;
            match unique.entry(pointer.to_string()) {
                std::collections::btree_map::Entry::Vacant(entry) => {
                    entry.insert((input_ref, bytes));
                }
                std::collections::btree_map::Entry::Occupied(entry) if entry.get().1 != bytes => {
                    return Err(input_error(
                        CALYX_INPUT_BLOB_HASH_MISMATCH,
                        format!("retained input batch repeats {pointer} with different bytes"),
                        "supply one exact authoritative byte sequence for each retained pointer",
                    ));
                }
                std::collections::btree_map::Entry::Occupied(_) => {}
            }
        }

        let pending_dir = vault_dir.join(INPUT_DIR).join(PENDING_DIR);
        if pending_entries_exist(vault_dir)? {
            return Err(reconciliation_error(format!(
                "retained-input transaction found unreconciled state in {}",
                pending_dir.display()
            )));
        }
        let mut batch = Self {
            pending_dir: pending_dir.clone(),
            items: Vec::new(),
        };
        for (pointer, (_, bytes)) in unique {
            match stage_item(vault_dir, &pending_dir, pointer, bytes) {
                Ok(Some(item)) => batch.items.push(item),
                Ok(None) => {}
                Err(error) => {
                    return Err(match batch.rollback() {
                        Ok(()) => error,
                        Err(rollback) => rollback_error(&error, &rollback),
                    });
                }
            }
        }
        Ok(batch)
    }

    pub(crate) fn finalize(self) -> Result<()> {
        for item in &self.items {
            publish_pending(item)?;
        }
        if self.pending_dir.is_dir() {
            crate::fsync::sync_dir(&self.pending_dir, "retained input pending directory")?;
        }
        Ok(())
    }

    pub(crate) fn rollback(self) -> Result<()> {
        let mut failures = Vec::new();
        for item in &self.items {
            if let Err(error) = fs::remove_file(&item.pending_path)
                && error.kind() != ErrorKind::NotFound
            {
                failures.push(format!("remove {}: {error}", item.pending_path.display()));
            }
        }
        if self.pending_dir.is_dir()
            && let Err(error) = crate::fsync::sync_dir(
                &self.pending_dir,
                "retained input rollback pending directory",
            )
        {
            failures.push(error.message);
        }
        if failures.is_empty() {
            Ok(())
        } else {
            Err(input_error(
                CALYX_INPUT_RETENTION_ROLLBACK_FAILED,
                failures.join("; "),
                "repair the retained-input pending directory, reopen the vault for reconciliation, and retry",
            ))
        }
    }
}

fn stage_item(
    vault_dir: &Path,
    pending_dir: &Path,
    pointer: String,
    bytes: &[u8],
) -> Result<Option<PendingItem>> {
    let final_path = retained_pointer_path(vault_dir, &pointer)?;
    if let Some(existing) = permissions::read_private_file_if_exists(&final_path, true)? {
        verify_exact_existing(&final_path, &existing, bytes)?;
        return Ok(None);
    }
    ensure_pending_dir(pending_dir)?;
    let file_name = final_path.file_name().ok_or_else(|| {
        stage_error(format!(
            "retained input {} has no filename",
            final_path.display()
        ))
    })?;
    let pending_path = pending_dir.join(file_name);
    if pending_path.exists() {
        return Err(reconciliation_error(format!(
            "unexpected retained-input pending object {}; reopen the vault to reconcile it before retrying",
            pending_path.display()
        )));
    }
    write_pending(&pending_path, bytes)?;
    Ok(Some(PendingItem {
        pointer,
        pending_path,
        final_path,
    }))
}

pub(crate) fn pending_entries_exist(vault_dir: &Path) -> Result<bool> {
    if !permissions::validate_store_for_read(vault_dir)? {
        return Ok(false);
    }
    let pending_dir = vault_dir.join(INPUT_DIR).join(PENDING_DIR);
    let Some(mut entries) = read_pending_dir(&pending_dir)? else {
        return Ok(false);
    };
    if let Some(entry) = entries.next() {
        entry.map_err(|error| {
            stage_error(format!("read {} entry: {error}", pending_dir.display()))
        })?;
        return Ok(true);
    }
    Ok(false)
}

pub(crate) fn reconcile_pending_retained_inputs<C: Clock>(
    vault: &AsterVault<C>,
    vault_dir: &Path,
) -> Result<()> {
    permissions::prepare_store_for_write(vault_dir)?;
    let pending_dir = vault_dir.join(INPUT_DIR).join(PENDING_DIR);
    let Some(entries) = read_pending_dir(&pending_dir)? else {
        return Ok(());
    };
    let mut paths = entries
        .map(|entry| {
            entry.map(|entry| entry.path()).map_err(|error| {
                stage_error(format!("read {} entry: {error}", pending_dir.display()))
            })
        })
        .collect::<Result<Vec<_>>>()?;
    paths.sort();
    if paths.is_empty() {
        return Ok(());
    }

    let mut referenced = BTreeMap::<String, InputRef>::new();
    for (_, bytes) in vault.scan_cf_at(vault.snapshot(), ColumnFamily::Base)? {
        let cx = encode::decode_constellation_base(&bytes)?;
        let Some(pointer) = cx.input_ref.pointer.as_ref() else {
            continue;
        };
        match referenced.entry(pointer.clone()) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(cx.input_ref);
            }
            std::collections::btree_map::Entry::Occupied(entry) if entry.get() != &cx.input_ref => {
                return Err(reconciliation_error(format!(
                    "Base rows disagree on retained input identity for pointer {pointer}"
                )));
            }
            std::collections::btree_map::Entry::Occupied(_) => {}
        }
    }

    for path in paths {
        let metadata = fs::symlink_metadata(&path)
            .map_err(|error| stage_error(format!("inspect {}: {error}", path.display())))?;
        if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
            return Err(reconciliation_error(format!(
                "retained-input pending entry {} is not a regular file",
                path.display()
            )));
        }
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| {
                reconciliation_error(format!(
                    "pending entry {} has a non-UTF8 name",
                    path.display()
                ))
            })?;
        if is_pending_temp_name(name) {
            fs::remove_file(&path).map_err(|error| {
                reconciliation_error(format!("remove {}: {error}", path.display()))
            })?;
            continue;
        }
        let hash = hash_from_blob_name(name)?;
        let pointer = canonical_text_pointer(&hash);
        let bytes = permissions::read_private_file(&path, true)?;
        validate_source(
            &InputRef {
                hash,
                pointer: Some(pointer.clone()),
                redacted: false,
            },
            &bytes,
        )?;
        if let Some(expected) = referenced.get(&pointer) {
            validate_source(expected, &bytes)?;
            publish_pending(&PendingItem {
                pointer,
                pending_path: path.clone(),
                final_path: retained_pointer_path(vault_dir, &canonical_text_pointer(&hash))?,
            })?;
        } else {
            fs::remove_file(&path).map_err(|error| {
                reconciliation_error(format!("remove uncommitted {}: {error}", path.display()))
            })?;
        }
    }
    crate::fsync::sync_dir(&pending_dir, "retained input recovery pending directory")?;
    Ok(())
}

pub(crate) fn rollback_error(original: &CalyxError, rollback: &CalyxError) -> CalyxError {
    input_error(
        CALYX_INPUT_RETENTION_ROLLBACK_FAILED,
        format!(
            "operation failed with {}: {}; retained-input rollback also failed with {}: {}",
            original.code, original.message, rollback.code, rollback.message
        ),
        "reopen the vault for retained-input reconciliation before retrying the failed operation",
    )
}

fn validate_source<'a>(input_ref: &'a InputRef, bytes: &[u8]) -> Result<&'a str> {
    if input_ref.redacted {
        return Err(stage_error(
            "cannot retain bytes for a redacted input reference",
        ));
    }
    let pointer = input_ref
        .pointer
        .as_deref()
        .ok_or_else(|| stage_error("retained input source has no pointer"))?;
    validate_vault_input_pointer(pointer)?;
    let actual = raw_input_hash(bytes);
    if input_ref.hash != actual || pointer != canonical_text_pointer(&actual) {
        return Err(input_error(
            CALYX_INPUT_BLOB_HASH_MISMATCH,
            format!(
                "retained source bytes require pointer {}, but input_ref has hash={} pointer={pointer}",
                canonical_text_pointer(&actual),
                hex(&input_ref.hash)
            ),
            "supply the exact authoritative source bytes matching the constellation input reference",
        ));
    }
    Ok(pointer)
}

fn ensure_pending_dir(pending_dir: &Path) -> Result<()> {
    permissions::ensure_private_directory(pending_dir)
}

fn read_pending_dir(pending_dir: &Path) -> Result<Option<fs::ReadDir>> {
    match fs::symlink_metadata(pending_dir) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err(reconciliation_error(format!(
                "retained input pending path {} is not a real directory",
                pending_dir.display()
            )));
        }
        Ok(_) => {}
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(stage_error(format!(
                "inspect {}: {error}",
                pending_dir.display()
            )));
        }
    }
    fs::read_dir(pending_dir)
        .map(Some)
        .map_err(|error| stage_error(format!("read {}: {error}", pending_dir.display())))
}

fn write_pending(path: &Path, bytes: &[u8]) -> Result<()> {
    let temp = path.with_extension(format!(
        "bin.tmp-{}-{}",
        std::process::id(),
        NEXT_PENDING_TEMP.fetch_add(1, Ordering::Relaxed)
    ));
    let mut linked = false;
    let result = (|| {
        let mut file = permissions::create_private_file(&temp)?;
        file.write_all(bytes)
            .map_err(|error| stage_error(format!("write {}: {error}", temp.display())))?;
        file.sync_all()
            .map_err(|error| stage_error(format!("sync {}: {error}", temp.display())))?;
        fs::hard_link(&temp, path).map_err(|error| {
            if error.kind() == ErrorKind::AlreadyExists {
                reconciliation_error(format!(
                    "pending retained input {} appeared while staging {}",
                    path.display(),
                    temp.display()
                ))
            } else {
                stage_error(format!(
                    "publish pending retained input {} -> {}: {error}",
                    temp.display(),
                    path.display()
                ))
            }
        })?;
        linked = true;
        crate::fsync::sync_parent(path, "pending retained input")
    })();
    if let Err(error) = result {
        return match cleanup_pending_write(&temp, path, linked) {
            Ok(()) => Err(error),
            Err(rollback) => Err(rollback_error(&error, &rollback)),
        };
    }
    if let Err(error) = fs::remove_file(&temp)
        && error.kind() != ErrorKind::NotFound
    {
        let operation = stage_error(format!("remove {}: {error}", temp.display()));
        return match cleanup_pending_write(&temp, path, true) {
            Ok(()) => Err(operation),
            Err(rollback) => Err(rollback_error(&operation, &rollback)),
        };
    }
    Ok(())
}

fn cleanup_pending_write(temp: &Path, path: &Path, remove_path: bool) -> Result<()> {
    let mut failures = Vec::new();
    if remove_path
        && let Err(error) = fs::remove_file(path)
        && error.kind() != ErrorKind::NotFound
    {
        failures.push(format!("remove {}: {error}", path.display()));
    }
    if let Err(error) = fs::remove_file(temp)
        && error.kind() != ErrorKind::NotFound
    {
        failures.push(format!("remove {}: {error}", temp.display()));
    }
    if let Some(parent) = path.parent()
        && let Err(error) = crate::fsync::sync_dir(parent, "retained input stage rollback")
    {
        failures.push(error.message);
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(input_error(
            CALYX_INPUT_RETENTION_ROLLBACK_FAILED,
            failures.join("; "),
            "repair the retained-input pending directory, reopen the vault for reconciliation, and retry",
        ))
    }
}

fn publish_pending(item: &PendingItem) -> Result<()> {
    let bytes = permissions::read_private_file(&item.pending_path, true)?;
    match fs::hard_link(&item.pending_path, &item.final_path) {
        Ok(()) => {}
        Err(error) if error.kind() == ErrorKind::AlreadyExists => {
            let existing = permissions::read_private_file(&item.final_path, true)?;
            verify_exact_existing(&item.final_path, &existing, &bytes)?;
        }
        Err(error) => {
            return Err(reconciliation_error(format!(
                "publish retained input {} -> {} for {}: {error}",
                item.pending_path.display(),
                item.final_path.display(),
                item.pointer
            )));
        }
    }
    crate::fsync::sync_parent(&item.final_path, "retained input commit")?;
    fs::remove_file(&item.pending_path).map_err(|error| {
        reconciliation_error(format!("remove {}: {error}", item.pending_path.display()))
    })?;
    Ok(())
}

fn stage_error(message: impl Into<String>) -> CalyxError {
    input_error(
        super::CALYX_INPUT_BLOB_WRITE_FAILED,
        message,
        "repair the vault input directory and retry the retained-input transaction",
    )
}

fn reconciliation_error(message: impl Into<String>) -> CalyxError {
    input_error(
        CALYX_INPUT_RETENTION_RECONCILIATION_REQUIRED,
        message,
        "reopen the write-capable vault to reconcile pending retained inputs before retrying",
    )
}
