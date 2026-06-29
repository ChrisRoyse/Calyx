use std::path::Component;

use super::*;

pub(super) fn checked_segment_path(
    vault_dir: &Path,
    index_rel: &str,
    slot: SlotId,
) -> CliResult<PathBuf> {
    checked_rel(index_rel)?;
    let path = vault_dir.join(index_rel);
    if !path.is_file() {
        return Err(stale(format!(
            "persistent segmented multi sidecar missing for slot {slot} at {}; rebuild the vault search indexes",
            path.display()
        )));
    }
    Ok(path)
}

pub(super) fn checked_rel(index_rel: &str) -> CliResult {
    let path = Path::new(index_rel);
    let bad = path.components().any(|component| {
        matches!(
            component,
            Component::Prefix(_) | Component::RootDir | Component::ParentDir
        )
    });
    if path.as_os_str().is_empty() || bad {
        return Err(stale(format!(
            "persistent segmented multi sidecar path {index_rel:?} is not a vault-relative path"
        )));
    }
    Ok(())
}
