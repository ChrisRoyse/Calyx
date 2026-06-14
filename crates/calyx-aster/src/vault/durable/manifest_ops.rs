use super::{DurableVault, storage_error};
use crate::manifest::{ImmutableRef, ManifestStore, VaultManifest};
use crate::timetravel::RetentionHorizon;
use calyx_core::{CalyxError, Panel, Result};
use std::fs::{self, File};
use std::io;
use std::io::Write;
use std::path::Path;

impl DurableVault {
    pub(super) fn retention_horizon(&self) -> RetentionHorizon {
        self.retention_horizon
            .lock()
            .map(|guard| guard.clone())
            .unwrap_or_default()
    }

    pub(in crate::vault) fn write_retention_horizon_manifest(
        &self,
        horizon: &RetentionHorizon,
    ) -> Result<()> {
        horizon.validate()?;
        let current = self.current_manifest()?;
        let manifest_seq = current
            .as_ref()
            .map_or(1, |manifest| manifest.manifest_seq.saturating_add(1));
        let durable_seq = current.as_ref().map_or(0, |manifest| manifest.durable_seq);
        self.write_manifest_with_seq_and_horizon(manifest_seq, durable_seq, horizon)?;
        *self
            .retention_horizon
            .lock()
            .map_err(|_| CalyxError::backpressure("retention horizon lock poisoned"))? =
            horizon.clone();
        Ok(())
    }

    pub(super) fn write_manifest(&self, seq: u64) -> Result<()> {
        let manifest_seq = self.current_manifest()?.map_or(seq.max(1), |manifest| {
            manifest.manifest_seq.saturating_add(1)
        });
        self.write_manifest_with_seq(manifest_seq, seq)
    }

    pub(super) fn write_manifest_with_seq(
        &self,
        manifest_seq: u64,
        durable_seq: u64,
    ) -> Result<()> {
        let horizon = self.retention_horizon();
        self.write_manifest_with_seq_and_horizon(manifest_seq, durable_seq, &horizon)
    }

    fn write_manifest_with_seq_and_horizon(
        &self,
        manifest_seq: u64,
        durable_seq: u64,
        horizon: &RetentionHorizon,
    ) -> Result<()> {
        let current = self.current_manifest()?;
        let (panel_ref, codebook_refs) = match (&self.panel, current.as_ref()) {
            (Some(panel), _) => ensure_manifest_assets(&self.root, Some(panel))?,
            (None, Some(manifest)) => (manifest.panel_ref.clone(), manifest.codebook_refs.clone()),
            (None, None) => ensure_manifest_assets(&self.root, None)?,
        };
        let mut manifest = VaultManifest::new_with_policies(
            manifest_seq,
            durable_seq,
            panel_ref,
            codebook_refs,
            self.temporal_policy,
            self.dedup_policy.clone(),
        )?;
        manifest.retention_horizon = horizon.clone();
        manifest.registry_ref = current.and_then(|manifest| manifest.registry_ref);
        manifest.validate()?;
        ManifestStore::open(&self.root).write_current(&manifest)?;
        Ok(())
    }

    fn current_manifest(&self) -> Result<Option<VaultManifest>> {
        if self.root.join("CURRENT").exists() {
            ManifestStore::open(&self.root).load_current().map(Some)
        } else {
            Ok(None)
        }
    }
}

fn ensure_manifest_assets(
    root: &Path,
    panel: Option<&Panel>,
) -> Result<(ImmutableRef, Vec<ImmutableRef>)> {
    let codebook_path = root.join("codebooks/default.bin");
    let codebook_bytes = b"calyx-stage1-codebook";
    let panel_ref = if let Some(panel) = panel {
        let panel_bytes = serde_json::to_vec_pretty(panel).map_err(|error| {
            CalyxError::aster_corrupt_shard(format!("encode durable panel asset: {error}"))
        })?;
        let hash = blake3::hash(&panel_bytes).to_hex().to_string();
        let logical = format!("panel/panel-v{:08}-{}.json", panel.version, &hash[..16]);
        write_asset(&root.join(&logical), &panel_bytes)?;
        ImmutableRef::from_bytes(logical, &panel_bytes)?
    } else {
        let panel_bytes = b"calyx-stage1-panel";
        write_asset(&root.join("panel/current.bin"), panel_bytes)?;
        ImmutableRef::from_bytes("panel/current.bin", panel_bytes)?
    };
    write_asset(&codebook_path, codebook_bytes)?;
    Ok((
        panel_ref,
        vec![ImmutableRef::from_bytes(
            "codebooks/default.bin",
            codebook_bytes,
        )?],
    ))
}

fn write_asset(path: &Path, bytes: &[u8]) -> Result<()> {
    match fs::read(path) {
        Ok(existing) if existing == bytes => return Ok(()),
        Ok(_) => {
            return Err(CalyxError::aster_corrupt_shard(format!(
                "manifest immutable asset {} hash mismatch",
                path.display()
            )));
        }
        Err(error) if error.kind() != io::ErrorKind::NotFound => {
            return Err(storage_error("read manifest asset", error));
        }
        Err(_) => {}
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| storage_error("create manifest asset dir", error))?;
    }
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("manifest-asset");
    let tmp = path.with_file_name(format!(
        ".{file_name}.{:?}.tmp",
        std::thread::current().id()
    ));
    {
        let mut file =
            File::create(&tmp).map_err(|error| storage_error("create manifest asset", error))?;
        file.write_all(bytes)
            .map_err(|error| storage_error("write manifest asset", error))?;
        file.sync_all()
            .map_err(|error| storage_error("fsync manifest asset", error))?;
    }
    fs::rename(&tmp, path).map_err(|error| storage_error("install manifest asset", error))?;
    Ok(())
}
