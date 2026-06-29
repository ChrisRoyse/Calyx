use calyx_aster::cf::ColumnFamily;
use calyx_aster::vault::encode::{decode_constellation_base, decode_slot_vector};
use calyx_core::{CalyxError, CxId, SlotId, VaultStore};
use rayon::prelude::*;

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
    let previous_manifest = previous_manifest(vault_dir)?;
    let mut entries = Vec::new();
    let mut total_rows = 0usize;
    for slot in dense::slots(docs) {
        let rows = dense::collect_slot(docs, slot)?;
        total_rows += rows.len();
        entries.push(dense::write(vault_dir, &root, slot, rows, base_seq)?);
    }
    for (slot, rows) in sparse::collect(docs)? {
        total_rows += rows.len();
        entries.push(sparse::write(vault_dir, &root, slot, rows, base_seq)?);
    }
    for (slot, rows) in multi::collect(docs)? {
        total_rows += rows.len();
        let previous = previous_manifest
            .as_ref()
            .and_then(|manifest| manifest.slots.iter().find(|entry| entry.slot == slot.get()));
        entries.push(multi::write(
            vault_dir, &root, slot, rows, base_seq, previous,
        )?);
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

fn previous_manifest(vault_dir: &Path) -> CliResult<Option<SearchIndexManifest>> {
    let path = manifest_path(vault_dir);
    if !path.exists() {
        return Ok(None);
    }
    let manifest: SearchIndexManifest =
        serde_json::from_slice(&fs::read(&path)?).map_err(|err| {
            stale(format!(
                "persistent search index manifest {} is unreadable before rebuild: {err}",
                path.display()
            ))
        })?;
    if manifest.format != MANIFEST_FORMAT {
        return Err(stale(format!(
            "persistent search index manifest {} has format {}; expected {MANIFEST_FORMAT}",
            path.display(),
            manifest.format
        )));
    }
    Ok(Some(manifest))
}

pub fn load_docs(vault: &AsterVault) -> CliResult<BTreeMap<CxId, Constellation>> {
    let snapshot = vault.snapshot();
    let base_rows = vault.scan_cf_at(snapshot, ColumnFamily::Base)?;
    let decoded_base = base_rows
        .into_par_iter()
        .map(|(key, bytes)| {
            let cx_id = cx_id_from_cf_key(&key, "base CF")?;
            let cx = decode_constellation_base(&bytes)?;
            if cx.cx_id != cx_id {
                return Err(CalyxError::aster_corrupt_shard(format!(
                    "base CF key {cx_id} contains constellation {}",
                    cx.cx_id
                )));
            }
            Ok((cx_id, cx))
        })
        .collect::<calyx_core::Result<Vec<_>>>()?;
    let mut docs = decoded_base.into_iter().collect::<BTreeMap<_, _>>();
    let slots = indexed_slots(&docs);
    for slot in slots {
        load_slot_rows(vault, snapshot, slot, &mut docs)?;
    }
    Ok(docs)
}

fn indexed_slots(docs: &BTreeMap<CxId, Constellation>) -> Vec<SlotId> {
    let mut slots = docs
        .values()
        .flat_map(|cx| cx.slots.keys().copied())
        .collect::<Vec<_>>();
    slots.sort();
    slots.dedup();
    slots
}

fn load_slot_rows(
    vault: &AsterVault,
    snapshot: calyx_core::Seq,
    slot: SlotId,
    docs: &mut BTreeMap<CxId, Constellation>,
) -> CliResult {
    let expected = docs
        .iter()
        .filter_map(|(cx_id, cx)| cx.slots.contains_key(&slot).then_some(*cx_id))
        .collect::<std::collections::BTreeSet<_>>();
    let rows = vault.scan_cf_at(snapshot, ColumnFamily::slot(slot))?;
    let decoded = rows
        .into_par_iter()
        .map(|(key, bytes)| {
            let cx_id = cx_id_from_cf_key(&key, "slot CF")?;
            let vector = decode_slot_vector(&bytes)?;
            Ok((cx_id, vector))
        })
        .collect::<calyx_core::Result<Vec<_>>>()?;
    let mut found = std::collections::BTreeSet::new();
    for (cx_id, vector) in decoded {
        if !expected.contains(&cx_id) {
            continue;
        }
        let Some(cx) = docs.get_mut(&cx_id) else {
            continue;
        };
        cx.slots.insert(slot, vector);
        found.insert(cx_id);
    }
    if found.len() != expected.len() {
        let missing = expected
            .difference(&found)
            .next()
            .map(ToString::to_string)
            .unwrap_or_else(|| "<unknown>".to_string());
        return Err(CalyxError::aster_corrupt_shard(format!(
            "slot CF row missing for slot {slot} cx_id {missing}"
        ))
        .into());
    }
    Ok(())
}

fn cx_id_from_cf_key(key: &[u8], cf_name: &str) -> calyx_core::Result<CxId> {
    let bytes: [u8; 16] = key.try_into().map_err(|_| {
        CalyxError::vault_access_denied(format!("{cf_name} key has {} bytes", key.len()))
    })?;
    Ok(CxId::from_bytes(bytes))
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
            if entry.kind == "multi_maxsim_segments" {
                keep.extend(multi::referenced_segment_artifacts(
                    vault_dir,
                    entry,
                    SlotId::new(entry.slot),
                )?);
            }
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
