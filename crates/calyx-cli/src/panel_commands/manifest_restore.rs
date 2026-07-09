use std::fs;
use std::path::{Component, Path, PathBuf};

use calyx_aster::manifest::{ImmutableRef, ManifestStore, VaultManifest};
use calyx_core::{CalyxError, Panel};
use calyx_registry::{
    VaultRegistrySnapshot, load_vault_panel_state, require_vault_registry_contracts,
};
use serde::Serialize;

use crate::error::{CliError, CliResult};
use crate::output::print_json;

#[derive(Debug, Serialize)]
struct ManifestRestoreReport {
    status: &'static str,
    source_of_truth: &'static str,
    vault: PathBuf,
    manifest_seq_before: u64,
    manifest_seq_after: u64,
    durable_seq: u64,
    derived_content_seq: Option<u64>,
    old_panel_ref: String,
    old_registry_ref: Option<String>,
    new_panel_ref: String,
    new_panel_blake3: String,
    new_registry_ref: String,
    new_registry_blake3: String,
    manifest_pointer: String,
    reloaded_panel_version: u32,
    reloaded_slot_count: usize,
    reloaded_registry_lens_count: usize,
    registry_checked_count: usize,
}

#[derive(Debug)]
struct ManifestRestoreFlags {
    vault: PathBuf,
    panel_asset: String,
    registry_asset: String,
}

pub(super) fn manifest_restore(args: &[String]) -> CliResult {
    let flags = ManifestRestoreFlags::parse(args)?;
    let report =
        restore_manifest_from_assets(&flags.vault, &flags.panel_asset, &flags.registry_asset)?;
    print_json(&report)
}

fn restore_manifest_from_assets(
    vault: &Path,
    panel_asset: &str,
    registry_asset: &str,
) -> CliResult<ManifestRestoreReport> {
    let store = ManifestStore::open(vault);
    let before = store.load_current()?;
    let old_panel_ref = before.panel_ref.logical_path.clone();
    let old_registry_ref = before
        .registry_ref
        .as_ref()
        .map(|reference| reference.logical_path.clone());
    let (panel_ref, panel) = read_panel_asset(vault, panel_asset)?;
    let (registry_ref, snapshot) = read_registry_asset(vault, registry_asset)?;
    if snapshot.panel_ref != panel_ref {
        return Err(CliError::from(CalyxError {
            code: "CALYX_MANIFEST_RESTORE_ASSET_MISMATCH",
            message: format!(
                "registry asset {} points at panel {}, but --panel-asset supplied {}",
                registry_ref.logical_path, snapshot.panel_ref.logical_path, panel_ref.logical_path
            ),
            remediation: "choose a registry snapshot whose panel_ref exactly matches the supplied panel asset",
        }));
    }
    if snapshot.lenses.is_empty() {
        return Err(CliError::from(CalyxError {
            code: "CALYX_MANIFEST_RESTORE_EMPTY_REGISTRY",
            message: format!("registry asset {} has no lenses", registry_ref.logical_path),
            remediation: "supply a real registry snapshot asset with persisted lens contracts",
        }));
    }
    let after = manifest_with_assets(before, panel_ref.clone(), registry_ref.clone())?;
    let write = store.write_current(&after)?;
    let reloaded = load_vault_panel_state(vault)?;
    if reloaded.panel != panel {
        return Err(CliError::from(CalyxError {
            code: "CALYX_MANIFEST_RESTORE_VERIFY_FAILED",
            message: "reloaded panel bytes do not match the supplied panel asset".to_string(),
            remediation: "inspect CURRENT, MANIFEST, and the supplied panel asset before retrying",
        }));
    }
    let audit = require_vault_registry_contracts(vault)?;
    Ok(ManifestRestoreReport {
        status: "manifest_panel_registry_restored",
        source_of_truth: "operator-supplied immutable panel/registry assets plus MANIFEST readback via load_vault_panel_state",
        vault: vault.to_path_buf(),
        manifest_seq_before: after.manifest_seq - 1,
        manifest_seq_after: after.manifest_seq,
        durable_seq: after.durable_seq,
        derived_content_seq: after.derived_content_seq,
        old_panel_ref,
        old_registry_ref,
        new_panel_ref: panel_ref.logical_path,
        new_panel_blake3: panel_ref.blake3_hex,
        new_registry_ref: registry_ref.logical_path,
        new_registry_blake3: registry_ref.blake3_hex,
        manifest_pointer: write.pointer,
        reloaded_panel_version: reloaded.panel.version,
        reloaded_slot_count: reloaded.panel.slots.len(),
        reloaded_registry_lens_count: reloaded
            .registry_snapshot
            .as_ref()
            .map_or(0, |snapshot| snapshot.lenses.len()),
        registry_checked_count: audit.checked_count,
    })
}

fn manifest_with_assets(
    mut manifest: VaultManifest,
    panel_ref: ImmutableRef,
    registry_ref: ImmutableRef,
) -> CliResult<VaultManifest> {
    manifest.manifest_seq = manifest.manifest_seq.checked_add(1).ok_or_else(|| {
        CliError::from(CalyxError::ledger_chain_broken(
            "manifest sequence exhausted",
        ))
    })?;
    manifest.panel_ref = panel_ref;
    manifest.registry_ref = Some(registry_ref);
    manifest.validate()?;
    Ok(manifest)
}

fn read_panel_asset(vault: &Path, logical: &str) -> CliResult<(ImmutableRef, Panel)> {
    require_asset_prefix(logical, "panel/")?;
    let bytes = read_validated_asset(vault, logical)?;
    let panel: Panel = serde_json::from_slice(&bytes).map_err(|error| {
        CliError::from(CalyxError::aster_corrupt_shard(format!(
            "decode supplied panel asset {logical}: {error}"
        )))
    })?;
    if panel.version == 0 || panel.slots.is_empty() {
        return Err(CliError::from(CalyxError {
            code: "CALYX_MANIFEST_RESTORE_INVALID_PANEL",
            message: format!(
                "panel asset {logical} has version {} and {} slots",
                panel.version,
                panel.slots.len()
            ),
            remediation: "supply a real persisted panel JSON asset with version > 0 and at least one slot",
        }));
    }
    Ok((ImmutableRef::from_bytes(logical, &bytes)?, panel))
}

fn read_registry_asset(
    vault: &Path,
    logical: &str,
) -> CliResult<(ImmutableRef, VaultRegistrySnapshot)> {
    require_asset_prefix(logical, "registry/")?;
    let bytes = read_validated_asset(vault, logical)?;
    let snapshot: VaultRegistrySnapshot = serde_json::from_slice(&bytes).map_err(|error| {
        CliError::from(CalyxError::aster_corrupt_shard(format!(
            "decode supplied registry asset {logical}: {error}"
        )))
    })?;
    Ok((ImmutableRef::from_bytes(logical, &bytes)?, snapshot))
}

fn read_validated_asset(vault: &Path, logical: &str) -> CliResult<Vec<u8>> {
    validate_manifest_asset_logical_path(logical)?;
    fs::read(vault.join(logical)).map_err(|error| {
        CliError::from(CalyxError::aster_corrupt_shard(format!(
            "manifest restore asset {logical} unreadable: {error}"
        )))
    })
}

fn require_asset_prefix(logical: &str, prefix: &str) -> CliResult {
    if logical.starts_with(prefix) {
        return Ok(());
    }
    Err(CliError::from(CalyxError {
        code: "CALYX_MANIFEST_RESTORE_BAD_ASSET_PATH",
        message: format!("asset path {logical} must be under {prefix}"),
        remediation: "pass vault-relative immutable asset paths such as panel/panel-vNNNN.json and registry/registry-xxxx.json",
    }))
}

fn validate_manifest_asset_logical_path(logical: &str) -> CliResult {
    let path = Path::new(logical);
    let invalid = logical.is_empty()
        || logical.contains('\\')
        || logical.contains(':')
        || path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
        || matches!(logical, "CURRENT" | "MANIFEST")
        || logical.ends_with(".tmp");
    if invalid {
        return Err(bad_asset_path(logical));
    }
    Ok(())
}

fn bad_asset_path(logical: &str) -> CliError {
    CliError::from(CalyxError {
        code: "CALYX_MANIFEST_RESTORE_BAD_ASSET_PATH",
        message: format!("asset path {logical} must be a vault-relative immutable asset path"),
        remediation: "pass vault-relative immutable asset paths such as panel/panel-vNNNN.json and registry/registry-xxxx.json",
    })
}

impl ManifestRestoreFlags {
    fn parse(args: &[String]) -> CliResult<Self> {
        let mut vault = None;
        let mut panel_asset = None;
        let mut registry_asset = None;
        let mut idx = 0;
        while idx < args.len() {
            match args[idx].as_str() {
                "--vault" => {
                    idx += 1;
                    vault = Some(super::value(args, idx, "--vault")?.into());
                }
                "--panel-asset" => {
                    idx += 1;
                    panel_asset = Some(super::value(args, idx, "--panel-asset")?.to_string());
                }
                "--registry-asset" => {
                    idx += 1;
                    registry_asset = Some(super::value(args, idx, "--registry-asset")?.to_string());
                }
                other => {
                    return Err(CliError::usage(format!(
                        "unexpected panel manifest-restore flag {other}"
                    )));
                }
            }
            idx += 1;
        }
        Ok(Self {
            vault: vault
                .ok_or_else(|| CliError::usage("panel manifest-restore requires --vault"))?,
            panel_asset: panel_asset
                .ok_or_else(|| CliError::usage("panel manifest-restore requires --panel-asset"))?,
            registry_asset: registry_asset.ok_or_else(|| {
                CliError::usage("panel manifest-restore requires --registry-asset")
            })?,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use calyx_aster::manifest::{ImmutableRef, ManifestStore, VaultManifest};
    use calyx_registry::{
        VaultRegistrySnapshot, materialize_panel_template, persist_vault_panel_state, text_default,
    };

    use super::*;

    #[test]
    fn explicit_manifest_restore_recovers_bootstrap_panel_ref() {
        let root = temp_root("manifest-restore");
        fs::create_dir_all(root.join("panel")).unwrap();
        fs::create_dir_all(root.join("codebooks")).unwrap();
        let bootstrap_panel = b"calyx-stage1-panel";
        let codebook = b"calyx-stage1-codebook";
        fs::write(root.join("panel/current.bin"), bootstrap_panel).unwrap();
        fs::write(root.join("codebooks/default.bin"), codebook).unwrap();
        let manifest = VaultManifest::new(
            1,
            7,
            ImmutableRef::from_bytes("panel/current.bin", bootstrap_panel).unwrap(),
            vec![ImmutableRef::from_bytes("codebooks/default.bin", codebook).unwrap()],
        )
        .unwrap();
        ManifestStore::open(&root).write_current(&manifest).unwrap();
        let materialized = materialize_panel_template(&text_default(), 42).unwrap();
        let panel = materialized.panel.clone();
        let write = persist_vault_panel_state(&root, &panel, &materialized.registry).unwrap();
        ManifestStore::open(&root).write_current(&manifest).unwrap();
        assert!(load_vault_panel_state(&root).is_err());

        let report = restore_manifest_from_assets(
            &root,
            &write.panel_ref.logical_path,
            &write.registry_ref.logical_path,
        )
        .unwrap();

        assert_eq!(report.status, "manifest_panel_registry_restored");
        assert_eq!(report.manifest_seq_before, 1);
        assert_eq!(report.manifest_seq_after, 2);
        assert_eq!(report.reloaded_panel_version, panel.version);
        assert_eq!(report.reloaded_slot_count, panel.slots.len());
        assert!(report.reloaded_registry_lens_count > 0);
        let reloaded = load_vault_panel_state(&root).unwrap();
        assert_eq!(reloaded.panel, panel);
    }

    #[test]
    fn mismatched_registry_panel_ref_fails_closed() {
        let root = temp_root("manifest-restore-mismatch");
        fs::create_dir_all(root.join("panel")).unwrap();
        fs::create_dir_all(root.join("registry")).unwrap();
        fs::create_dir_all(root.join("codebooks")).unwrap();
        let bootstrap_panel = b"calyx-stage1-panel";
        let codebook = b"calyx-stage1-codebook";
        fs::write(root.join("panel/current.bin"), bootstrap_panel).unwrap();
        fs::write(root.join("codebooks/default.bin"), codebook).unwrap();
        let manifest = VaultManifest::new(
            1,
            7,
            ImmutableRef::from_bytes("panel/current.bin", bootstrap_panel).unwrap(),
            vec![ImmutableRef::from_bytes("codebooks/default.bin", codebook).unwrap()],
        )
        .unwrap();
        ManifestStore::open(&root).write_current(&manifest).unwrap();
        let materialized = materialize_panel_template(&text_default(), 42).unwrap();
        let panel = materialized.panel.clone();
        let write = persist_vault_panel_state(&root, &panel, &materialized.registry).unwrap();
        let other_panel = materialize_panel_template(&text_default(), 43)
            .unwrap()
            .panel;
        let other_bytes = serde_json::to_vec_pretty(&other_panel).unwrap();
        let other_panel_ref = ImmutableRef::from_bytes("panel/other.json", &other_bytes).unwrap();
        fs::write(root.join("panel/other.json"), other_bytes).unwrap();
        let mismatch = VaultRegistrySnapshot {
            version: 1,
            panel_ref: other_panel_ref,
            lenses: materialized.registry.lens_snapshots(),
        };
        let mismatch_bytes = serde_json::to_vec_pretty(&mismatch).unwrap();
        fs::write(root.join("registry/mismatch.json"), mismatch_bytes).unwrap();
        ManifestStore::open(&root).write_current(&manifest).unwrap();

        let error = restore_manifest_from_assets(
            &root,
            &write.panel_ref.logical_path,
            "registry/mismatch.json",
        )
        .unwrap_err();

        assert_eq!(error.code(), "CALYX_MANIFEST_RESTORE_ASSET_MISMATCH");
    }

    #[test]
    fn manifest_restore_rejects_asset_path_traversal() {
        let root = temp_root("manifest-restore-traversal");

        let error = read_panel_asset(&root, "panel/../CURRENT").unwrap_err();

        assert_eq!(error.code(), "CALYX_MANIFEST_RESTORE_BAD_ASSET_PATH");
        fs::remove_dir_all(root).ok();
    }

    fn temp_root(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("calyx-{name}-{}-{nonce}", std::process::id()))
    }
}
