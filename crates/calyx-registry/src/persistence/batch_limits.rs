use super::*;

pub fn set_vault_registry_batch_limits(
    vault_dir: impl AsRef<Path>,
    updates: &[RegistryBatchLimitUpdate],
) -> Result<VaultRegistryBatchLimitWrite> {
    let vault_dir = vault_dir.as_ref();
    let state = load_vault_panel_state(vault_dir)?;
    let mut snapshot = state.registry_snapshot.ok_or_else(|| {
        CalyxError::aster_corrupt_shard(
            "vault has no persisted registry snapshot; cannot update lens batch limits",
        )
    })?;
    let changes = apply_registry_snapshot_batch_limits(&mut snapshot, updates)?;
    if changes.iter().all(|change| !change.changed) {
        let manifest = ManifestStore::open(vault_dir).load_current()?;
        let registry_ref = manifest.registry_ref.clone().ok_or_else(|| {
            CalyxError::aster_corrupt_shard(
                "vault manifest has no registry_ref after loading registry snapshot",
            )
        })?;
        return Ok(VaultRegistryBatchLimitWrite {
            manifest_seq: manifest.manifest_seq,
            durable_seq: manifest.durable_seq,
            panel_ref: manifest.panel_ref,
            registry_ref,
            wrote_manifest: false,
            changes,
        });
    }
    let registry = rebuild_registry(&snapshot)?;
    let write = persist_vault_panel_state(vault_dir, &state.panel, &registry)?;
    Ok(VaultRegistryBatchLimitWrite {
        manifest_seq: write.manifest_seq,
        durable_seq: write.durable_seq,
        panel_ref: write.panel_ref,
        registry_ref: write.registry_ref,
        wrote_manifest: true,
        changes,
    })
}

pub fn apply_registry_snapshot_batch_limits(
    snapshot: &mut VaultRegistrySnapshot,
    updates: &[RegistryBatchLimitUpdate],
) -> Result<Vec<RegistryBatchLimitChange>> {
    if updates.is_empty() {
        return Err(registry_batch_limit_invalid(
            "at least one lens batch limit update is required",
        ));
    }
    let mut seen = BTreeSet::new();
    for update in updates {
        if update.max_batch == 0 {
            return Err(registry_batch_limit_invalid(format!(
                "lens {} max_batch must be > 0",
                update.lens_id
            )));
        }
        if !seen.insert(update.lens_id) {
            return Err(registry_batch_limit_invalid(format!(
                "duplicate batch limit update for lens {}",
                update.lens_id
            )));
        }
    }

    let mut changes = Vec::with_capacity(updates.len());
    for update in updates {
        let lens = snapshot
            .lenses
            .iter_mut()
            .find(|lens| lens.lens_id == update.lens_id)
            .ok_or_else(|| {
                CalyxError::lens_unreachable(format!(
                    "lens {} is not present in persisted registry snapshot",
                    update.lens_id
                ))
            })?;
        let spec = lens.spec.as_mut().ok_or_else(|| {
            CalyxError::lens_unreachable(format!(
                "lens {} is persisted without LensSpec metadata; cannot update max_batch",
                update.lens_id
            ))
        })?;
        let before = spec.max_batch;
        let changed = before != Some(update.max_batch);
        spec.max_batch = Some(update.max_batch);
        changes.push(RegistryBatchLimitChange {
            lens_id: update.lens_id,
            name: spec.name.clone(),
            before,
            after: update.max_batch,
            changed,
        });
    }
    Ok(changes)
}

fn registry_batch_limit_invalid(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: "CALYX_REGISTRY_BATCH_LIMIT_INVALID",
        message: message.into(),
        remediation: "pass positive, unique lens batch limits that match lenses in the persisted vault registry",
    }
}
