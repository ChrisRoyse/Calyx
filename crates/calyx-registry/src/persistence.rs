use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use calyx_aster::manifest::{ImmutableRef, ManifestStore};
use calyx_core::{CalyxError, Input, Lens, LensId, Modality, Panel, Result, SlotShape, SlotVector};
use serde::{Deserialize, Serialize};

use crate::{
    AlgorithmicLens, CandleLens, ExternalCmdLens, FastembedBgem3Lens, FastembedQwen3Lens,
    FastembedRerankerLens, FastembedSparseLens, LensRuntime, LensSpec, MultimodalAdapterLens,
    OnnxColbertLens, OnnxLens, Registry, RegistryLensSnapshot, StaticLookupLens, TeiHttpLens,
};

const SNAPSHOT_VERSION: u16 = 1;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VaultRegistrySnapshot {
    pub version: u16,
    pub panel_ref: ImmutableRef,
    pub lenses: Vec<RegistryLensSnapshot>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VaultPanelWrite {
    pub manifest_seq: u64,
    pub durable_seq: u64,
    pub panel_ref: ImmutableRef,
    pub registry_ref: ImmutableRef,
}

#[derive(Clone)]
pub struct VaultPanelState {
    pub panel: Panel,
    pub registry: Registry,
    pub registry_snapshot: Option<VaultRegistrySnapshot>,
}

pub fn persist_vault_panel_state(
    vault_dir: impl AsRef<Path>,
    panel: &Panel,
    registry: &Registry,
) -> Result<VaultPanelWrite> {
    let vault_dir = vault_dir.as_ref();
    let store = ManifestStore::open(vault_dir);
    let mut manifest = store.load_current()?;
    let panel_ref = write_panel_asset(vault_dir, panel)?;
    let registry_ref = write_registry_asset(vault_dir, &panel_ref, registry)?;
    manifest.manifest_seq = manifest
        .manifest_seq
        .checked_add(1)
        .ok_or_else(|| CalyxError::aster_corrupt_shard("manifest sequence exhausted"))?;
    manifest.panel_ref = panel_ref.clone();
    manifest.registry_ref = Some(registry_ref.clone());
    manifest.validate()?;
    let durable_seq = manifest.durable_seq;
    let manifest_seq = manifest.manifest_seq;
    store.write_current(&manifest)?;
    Ok(VaultPanelWrite {
        manifest_seq,
        durable_seq,
        panel_ref,
        registry_ref,
    })
}

pub fn load_vault_panel_state(vault_dir: impl AsRef<Path>) -> Result<VaultPanelState> {
    let vault_dir = vault_dir.as_ref();
    let manifest = ManifestStore::open(vault_dir).load_current()?;
    let panel_bytes = read_ref(vault_dir, &manifest.panel_ref)?;
    let panel: Panel = serde_json::from_slice(&panel_bytes)
        .map_err(|error| CalyxError::aster_corrupt_shard(format!("decode panel: {error}")))?;
    let snapshot = manifest
        .registry_ref
        .as_ref()
        .map(|reference| read_registry_snapshot(vault_dir, reference, &manifest.panel_ref))
        .transpose()?;
    let registry = snapshot
        .as_ref()
        .map_or_else(|| Ok(Registry::new()), rebuild_registry)?;
    Ok(VaultPanelState {
        panel,
        registry,
        registry_snapshot: snapshot,
    })
}

fn write_panel_asset(vault_dir: &Path, panel: &Panel) -> Result<ImmutableRef> {
    let bytes = serde_json::to_vec_pretty(panel)
        .map_err(|error| CalyxError::aster_corrupt_shard(format!("encode panel: {error}")))?;
    let hash = blake3::hash(&bytes).to_hex().to_string();
    let logical = format!("panel/panel-v{:08}-{}.json", panel.version, &hash[..16]);
    write_asset(&vault_dir.join(&logical), &bytes)?;
    ImmutableRef::from_bytes(logical, &bytes)
}

fn write_registry_asset(
    vault_dir: &Path,
    panel_ref: &ImmutableRef,
    registry: &Registry,
) -> Result<ImmutableRef> {
    let snapshot = VaultRegistrySnapshot {
        version: SNAPSHOT_VERSION,
        panel_ref: panel_ref.clone(),
        lenses: registry.lens_snapshots(),
    };
    let bytes = serde_json::to_vec_pretty(&snapshot)
        .map_err(|error| CalyxError::aster_corrupt_shard(format!("encode registry: {error}")))?;
    let hash = blake3::hash(&bytes).to_hex().to_string();
    let logical = format!("registry/registry-{}.json", &hash[..16]);
    write_asset(&vault_dir.join(&logical), &bytes)?;
    ImmutableRef::from_bytes(logical, &bytes)
}

fn read_registry_snapshot(
    vault_dir: &Path,
    reference: &ImmutableRef,
    panel_ref: &ImmutableRef,
) -> Result<VaultRegistrySnapshot> {
    let bytes = read_ref(vault_dir, reference)?;
    let snapshot: VaultRegistrySnapshot = serde_json::from_slice(&bytes)
        .map_err(|error| CalyxError::aster_corrupt_shard(format!("decode registry: {error}")))?;
    if snapshot.version != SNAPSHOT_VERSION {
        return Err(CalyxError::aster_corrupt_shard(format!(
            "unsupported registry snapshot version {}",
            snapshot.version
        )));
    }
    if &snapshot.panel_ref != panel_ref {
        return Err(CalyxError::aster_corrupt_shard(
            "registry snapshot panel_ref does not match manifest panel_ref",
        ));
    }
    Ok(snapshot)
}

fn rebuild_registry(snapshot: &VaultRegistrySnapshot) -> Result<Registry> {
    let mut registry = Registry::new();
    for lens in &snapshot.lenses {
        if lens.lens_id != lens.contract.lens_id() {
            return Err(CalyxError::lens_frozen_violation(format!(
                "registry lens {} does not match frozen contract {}",
                lens.lens_id,
                lens.contract.lens_id()
            )));
        }
        let runtime = load_runtime_lens(lens).unwrap_or_else(|| {
            Arc::new(PersistedUnavailableLens {
                id: lens.lens_id,
                shape: lens.contract.shape(),
                modality: lens.contract.modality(),
            })
        });
        registry.register_persisted_arc(
            runtime,
            lens.contract.clone(),
            lens.spec.clone(),
            lens.determinism,
        )?;
    }
    Ok(registry)
}

fn load_runtime_lens(snapshot: &RegistryLensSnapshot) -> Option<Arc<dyn Lens>> {
    let spec = snapshot.spec.as_ref()?;
    let lens: Arc<dyn Lens> = match &spec.runtime {
        LensRuntime::Algorithmic { kind } => Arc::new(algorithmic_lens(spec, kind)?),
        LensRuntime::TeiHttp { endpoint } => Arc::new(TeiHttpLens::new(
            &spec.name,
            endpoint,
            spec.modality,
            dense_dim(spec.output)?,
        )),
        LensRuntime::ExternalCmd { cmd, args } => Arc::new(ExternalCmdLens::new(
            &spec.name,
            cmd,
            args.clone(),
            spec.modality,
            dense_dim(spec.output)?,
        )),
        LensRuntime::CandleLocal { .. } => Arc::new(CandleLens::from_lens_spec(spec).ok()?),
        LensRuntime::Onnx { .. } => Arc::new(OnnxLens::from_lens_spec(spec).ok()?),
        LensRuntime::OnnxColbert { .. } => Arc::new(OnnxColbertLens::from_lens_spec(spec).ok()?),
        LensRuntime::FastembedSparse { .. } => {
            Arc::new(FastembedSparseLens::from_lens_spec(spec).ok()?)
        }
        LensRuntime::FastembedBgem3 { .. } => {
            Arc::new(FastembedBgem3Lens::from_lens_spec(spec).ok()?)
        }
        LensRuntime::FastembedReranker { .. } => {
            Arc::new(FastembedRerankerLens::from_lens_spec(spec).ok()?)
        }
        LensRuntime::FastembedQwen3 { .. } => {
            Arc::new(FastembedQwen3Lens::from_lens_spec(spec).ok()?)
        }
        LensRuntime::StaticLookup { .. } => Arc::new(StaticLookupLens::from_lens_spec(spec).ok()?),
        LensRuntime::MultimodalAdapter { .. } => {
            Arc::new(MultimodalAdapterLens::from_lens_spec(spec).ok()?)
        }
    };
    if snapshot.contract.verify_registration(lens.as_ref()).is_ok() {
        Some(lens)
    } else {
        None
    }
}

fn algorithmic_lens(spec: &LensSpec, kind: &str) -> Option<AlgorithmicLens> {
    match kind {
        "byte_features" | "byte-features" | "byte" => {
            Some(AlgorithmicLens::byte_features(&spec.name, spec.modality))
        }
        "scalar" => Some(AlgorithmicLens::scalar(&spec.name, spec.modality)),
        "ast_style" | "ast-style" => Some(AlgorithmicLens::ast_style(&spec.name, spec.modality)),
        "sparse" | "sparse_keywords" | "sparse-keywords" => Some(AlgorithmicLens::sparse_keywords(
            &spec.name,
            spec.modality,
            sparse_dim(spec.output)?,
        )),
        "token_hash" | "token-hash" | "multi_hash" | "multi-hash" => Some(
            AlgorithmicLens::token_hash(&spec.name, spec.modality, token_dim(spec.output)?),
        ),
        "one_hot" | "one-hot" => Some(AlgorithmicLens::one_hot(
            &spec.name,
            spec.modality,
            dense_dim(spec.output)?,
        )),
        value => {
            if let Some(dim) = value
                .strip_prefix("sparse_keywords:")
                .or_else(|| value.strip_prefix("sparse-keywords:"))
                .and_then(|dim| dim.parse().ok())
            {
                return Some(AlgorithmicLens::sparse_keywords(
                    &spec.name,
                    spec.modality,
                    dim,
                ));
            }
            if let Some(dim) = value
                .strip_prefix("token_hash:")
                .or_else(|| value.strip_prefix("token-hash:"))
                .or_else(|| value.strip_prefix("multi_hash:"))
                .or_else(|| value.strip_prefix("multi-hash:"))
                .and_then(|dim| dim.parse().ok())
            {
                return Some(AlgorithmicLens::token_hash(&spec.name, spec.modality, dim));
            }
            value
                .strip_prefix("one_hot:")
                .or_else(|| value.strip_prefix("one-hot:"))
                .and_then(|buckets| buckets.parse().ok())
                .map(|buckets| AlgorithmicLens::one_hot(&spec.name, spec.modality, buckets))
        }
    }
}

fn dense_dim(shape: SlotShape) -> Option<u32> {
    match shape {
        SlotShape::Dense(dim) => Some(dim),
        SlotShape::Sparse(_) | SlotShape::Multi { .. } => None,
    }
}

fn sparse_dim(shape: SlotShape) -> Option<u32> {
    match shape {
        SlotShape::Sparse(dim) => Some(dim),
        SlotShape::Dense(_) | SlotShape::Multi { .. } => None,
    }
}

fn token_dim(shape: SlotShape) -> Option<u32> {
    match shape {
        SlotShape::Multi { token_dim } => Some(token_dim),
        SlotShape::Dense(_) | SlotShape::Sparse(_) => None,
    }
}

fn read_ref(vault_dir: &Path, reference: &ImmutableRef) -> Result<Vec<u8>> {
    fs::read(vault_dir.join(&reference.logical_path)).map_err(|error| {
        CalyxError::aster_corrupt_shard(format!(
            "manifest ref {} unreadable: {error}",
            reference.logical_path
        ))
    })
}

fn write_asset(path: &Path, bytes: &[u8]) -> Result<()> {
    match fs::read(path) {
        Ok(existing) if existing == bytes => return Ok(()),
        Ok(_) => {
            return Err(CalyxError::aster_corrupt_shard(format!(
                "registry immutable asset {} hash mismatch",
                path.display()
            )));
        }
        Err(error) if error.kind() != io::ErrorKind::NotFound => {
            return Err(storage_error("read registry asset", error));
        }
        Err(_) => {}
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| storage_error("create registry asset dir", error))?;
    }
    let tmp = tmp_path(path);
    {
        let mut file =
            File::create(&tmp).map_err(|error| storage_error("create registry asset", error))?;
        file.write_all(bytes)
            .map_err(|error| storage_error("write registry asset", error))?;
        file.sync_all()
            .map_err(|error| storage_error("fsync registry asset", error))?;
    }
    fs::rename(&tmp, path).map_err(|error| storage_error("install registry asset", error))
}

fn tmp_path(path: &Path) -> PathBuf {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("registry-asset");
    path.with_file_name(format!(
        ".{file_name}.{:?}.tmp",
        std::thread::current().id()
    ))
}

fn storage_error(context: &str, error: io::Error) -> CalyxError {
    CalyxError::disk_pressure(format!("{context}: {error}"))
}

struct PersistedUnavailableLens {
    id: LensId,
    shape: SlotShape,
    modality: Modality,
}

impl Lens for PersistedUnavailableLens {
    fn id(&self) -> LensId {
        self.id
    }

    fn shape(&self) -> SlotShape {
        self.shape
    }

    fn modality(&self) -> Modality {
        self.modality
    }

    fn measure(&self, _input: &Input) -> Result<SlotVector> {
        Err(CalyxError::lens_unreachable(format!(
            "lens {} is persisted but its runtime is unavailable in this process",
            self.id
        )))
    }
}
