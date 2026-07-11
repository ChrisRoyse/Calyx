use std::fs;
use std::path::{Path, PathBuf};

use calyx_core::Result;

use super::invalid_config;

pub(super) fn atomic_write_text(path: &Path, text: &str) -> Result<()> {
    let tmp = temp_path(path)?;
    fs::write(&tmp, text)
        .map_err(|error| invalid_config(format!("write {}: {error}", tmp.display())))?;
    fs::rename(&tmp, path).map_err(|error| {
        let _ = fs::remove_file(&tmp);
        invalid_config(format!(
            "rename {} -> {}: {error}",
            tmp.display(),
            path.display()
        ))
    })
}

fn temp_path(path: &Path) -> Result<PathBuf> {
    let file_name = path
        .file_name()
        .ok_or_else(|| invalid_config("tripwire config path must include a file name"))?;
    let mut tmp_name = file_name.to_os_string();
    tmp_name.push(format!(".tmp-{}", std::process::id()));
    Ok(path.with_file_name(tmp_name))
}
