use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

use calyx_core::{CalyxError, Input};

use crate::server::{ToolError, ToolResult};
use crate::tools::vault::store::ResolvedVault;

pub(super) const INPUT_POINTER_PREFIX: &str = "calyx-vault://";

pub(super) fn retained_text_input(resolved: &ResolvedVault, text: &str) -> ToolResult<Input> {
    Ok(calyx_aster::retained_input::retain_text_input(
        &resolved.path,
        text,
    )?)
}

pub(super) fn input_hash(bytes: &[u8]) -> [u8; 32] {
    *blake3::hash(bytes).as_bytes()
}

pub(super) fn write_input_blob(path: &Path, bytes: &[u8]) -> ToolResult<()> {
    if let Ok(existing) = fs::read(path) {
        if existing == bytes {
            return Ok(());
        }
        return Err(CalyxError::aster_corrupt_shard(format!(
            "input blob {} exists with different bytes",
            path.display()
        ))
        .into());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| input_blob_error(format!("create {}: {error}", parent.display())))?;
    }
    let tmp = path.with_extension(format!("bin.tmp-{}", std::process::id()));
    {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&tmp)
            .map_err(|error| input_blob_error(format!("create {}: {error}", tmp.display())))?;
        file.write_all(bytes)
            .map_err(|error| input_blob_error(format!("write {}: {error}", tmp.display())))?;
        file.sync_all()
            .map_err(|error| input_blob_error(format!("sync {}: {error}", tmp.display())))?;
    }
    fs::rename(&tmp, path).map_err(|error| {
        input_blob_error(format!(
            "install input blob {} -> {}: {error}",
            tmp.display(),
            path.display()
        ))
    })?;
    Ok(())
}

fn input_blob_error(message: impl Into<String>) -> ToolError {
    CalyxError {
        code: "CALYX_INPUT_BLOB_WRITE_FAILED",
        message: message.into(),
        remediation: "repair the vault input blob directory before ingesting retained source bytes",
    }
    .into()
}
