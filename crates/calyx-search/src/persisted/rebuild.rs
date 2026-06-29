use calyx_aster::cf::ColumnFamily;
use calyx_core::{CalyxError, VaultStore};

use super::*;

pub fn rebuild_for_vault(vault_dir: &Path, vault: &AsterVault) -> CliResult {
    let docs = load_docs(vault)?;
    let summary = rebuild_from_docs(vault_dir, &docs, vault.latest_seq())?;
    let _ = (summary.slots, summary.total_rows, &summary.manifest_path);
    Ok(())
}

pub(super) fn rebuild_from_docs(
    vault_dir: &Path,
    docs: &BTreeMap<CxId, Constellation>,
    base_seq: u64,
) -> CliResult<RebuildSummary> {
    let root = vault_dir.join(INDEX_ROOT);
    fs::create_dir_all(&root)?;
    let mut entries = Vec::new();
    let mut total_rows = 0usize;
    for (slot, rows) in dense::collect(docs)? {
        total_rows += rows.len();
        entries.push(dense::write(vault_dir, &root, slot, rows, base_seq)?);
    }
    for (slot, rows) in sparse::collect(docs)? {
        total_rows += rows.len();
        entries.push(sparse::write(vault_dir, &root, slot, rows, base_seq)?);
    }
    for (slot, rows) in multi::collect(docs)? {
        total_rows += rows.len();
        entries.push(multi::write(vault_dir, &root, slot, rows, base_seq)?);
    }
    entries.sort_by_key(|entry| entry.slot);
    let manifest = SearchIndexManifest {
        format: MANIFEST_FORMAT.to_string(),
        base_seq,
        filter: Some(filter::write(vault_dir, &root, docs, base_seq)?),
        slots: entries,
    };
    let manifest_path = manifest_path(vault_dir);
    write_json_atomic(&manifest_path, &manifest)?;
    prune_stale_index_artifacts(vault_dir, &root, &manifest)?;
    Ok(RebuildSummary {
        slots: manifest.slots.len(),
        total_rows,
        manifest_path,
    })
}

pub fn load_docs(vault: &AsterVault) -> CliResult<BTreeMap<CxId, Constellation>> {
    let snapshot = vault.snapshot();
    let mut docs = BTreeMap::new();
    for (key, _) in vault.scan_cf_at(snapshot, ColumnFamily::Base)? {
        let bytes: [u8; 16] = key.as_slice().try_into().map_err(|_| {
            CalyxError::vault_access_denied(format!("base CF key has {} bytes", key.len()))
        })?;
        let cx_id = CxId::from_bytes(bytes);
        docs.insert(cx_id, vault.get(cx_id, snapshot)?);
    }
    Ok(docs)
}

fn prune_stale_index_artifacts(
    vault_dir: &Path,
    root: &Path,
    manifest: &SearchIndexManifest,
) -> CliResult {
    let keep = referenced_index_artifacts(vault_dir, root, manifest)?;
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if !is_prunable_index_artifact(&name) || keep.iter().any(|item| item == &path) {
            continue;
        }
        if entry.file_type()?.is_dir() {
            fs::remove_dir_all(&path)?;
        } else {
            fs::remove_file(&path)?;
        }
    }
    Ok(())
}

fn referenced_index_artifacts(
    vault_dir: &Path,
    root: &Path,
    manifest: &SearchIndexManifest,
) -> CliResult<Vec<PathBuf>> {
    let mut keep = vec![manifest_path(vault_dir)];
    if let Some(filter) = &manifest.filter {
        keep.push(vault_dir.join(&filter.index_rel));
    }
    for entry in &manifest.slots {
        if let Some(index_rel) = &entry.index_rel {
            keep.push(vault_dir.join(index_rel));
        }
        if let Some(graph_rel) = &entry.graph_rel {
            let graph = vault_dir.join(graph_rel);
            let ann_dir = graph.parent().ok_or_else(|| {
                stale(format!(
                    "persistent slot {} graph path has no parent directory",
                    entry.slot
                ))
            })?;
            if ann_dir.parent().is_some_and(|parent| parent == root) {
                keep.push(ann_dir.to_path_buf());
            } else {
                keep.push(graph);
            }
        }
        if let Some(id_map_rel) = &entry.id_map_rel {
            keep.push(vault_dir.join(id_map_rel));
        }
    }
    keep.sort();
    keep.dedup();
    Ok(keep)
}

fn is_prunable_index_artifact(name: &str) -> bool {
    name.starts_with("slot_") || name.starts_with("filter_") || name.starts_with("filters_")
}
