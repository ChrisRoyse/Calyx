use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

use calyx_aster::cf::full_content_hash;
use calyx_core::{CalyxError, Input, Modality};

use crate::server::{ToolError, ToolResult};
use crate::tools::vault::store::ResolvedVault;

pub(super) const INPUT_POINTER_PREFIX: &str = "calyx-vault://";

pub(super) fn retained_text_input(resolved: &ResolvedVault, text: &str) -> ToolResult<Input> {
    let bytes = text.as_bytes().to_vec();
    let hash = full_content_hash([bytes.as_slice()]);
    let rel = format!("inputs/{}.bin", hex32(&hash));
    write_input_blob(&resolved.path.join(&rel), &bytes)?;
    Ok(Input::new(Modality::Text, bytes).with_pointer(format!("{INPUT_POINTER_PREFIX}{rel}")))
}

pub(super) fn input_hash(bytes: &[u8]) -> [u8; 32] {
    full_content_hash([bytes])
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

fn hex32(bytes: &[u8; 32]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn input_blob_error(message: impl Into<String>) -> ToolError {
    CalyxError {
        code: "CALYX_INPUT_BLOB_WRITE_FAILED",
        message: message.into(),
        remediation: "repair the vault input blob directory before ingesting retained source bytes",
    }
    .into()
}
