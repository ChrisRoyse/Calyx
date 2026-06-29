use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

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

pub fn measure_registry_snapshot_lens_batch(
    snapshot: &RegistryLensSnapshot,
    inputs: &[Input],
) -> Result<Vec<SlotVector>> {
    if snapshot.lens_id != snapshot.contract.lens_id() {
        return Err(CalyxError::lens_frozen_violation(format!(
            "registry lens {} does not match frozen contract {}",
            snapshot.lens_id,
            snapshot.contract.lens_id()
        )));
    }
    for input in inputs {
        if input.modality != snapshot.contract.modality() {
            return Err(CalyxError::lens_dim_mismatch(format!(
                "lens {} accepts {:?}, got {:?}",
                snapshot.lens_id,
                snapshot.contract.modality(),
                input.modality
            )));
        }
    }
    let runtime = load_runtime_lens(snapshot)?;
    let vectors = runtime.measure_batch(inputs)?;
    if vectors.len() != inputs.len() {
        return Err(CalyxError::lens_dim_mismatch(format!(
            "lens {} returned {} vectors for {} inputs",
            snapshot.lens_id,
            vectors.len(),
            inputs.len()
        )));
    }
    for vector in &vectors {
        snapshot.contract.verify_vector(snapshot.lens_id, vector)?;
    }
    Ok(vectors)
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
        let runtime = Arc::new(LazyPersistedLens::new(lens.clone()));
        registry.register_persisted_arc(
            runtime,
            lens.contract.clone(),
            lens.spec.clone(),
            lens.determinism,
        )?;
    }
    Ok(registry)
}

fn load_runtime_lens(snapshot: &RegistryLensSnapshot) -> Result<Arc<dyn Lens>> {
    let spec = snapshot.spec.as_ref().ok_or_else(|| {
        CalyxError::lens_unreachable(format!(
            "persisted lens {} has no LensSpec, so its runtime cannot be reconstructed",
            snapshot.lens_id
        ))
    })?;
    let lens: Arc<dyn Lens> = match &spec.runtime {
        LensRuntime::Algorithmic { kind } => {
            Arc::new(algorithmic_lens(spec, kind).ok_or_else(|| {
                lens_config_invalid(format!(
                    "unsupported algorithmic lens kind {kind} for persisted lens {} ({})",
                    snapshot.lens_id, spec.name
                ))
            })?)
        }
        LensRuntime::TeiHttp { endpoint } => Arc::new(TeiHttpLens::new(
            &spec.name,
            endpoint,
            spec.modality,
            dense_dim(spec.output).ok_or_else(|| {
                lens_config_invalid(format!(
                    "TEI lens {} ({}) requires dense output shape, got {:?}",
                    snapshot.lens_id, spec.name, spec.output
                ))
            })?,
        )),
        LensRuntime::ExternalCmd { cmd, args } => Arc::new(ExternalCmdLens::new(
            &spec.name,
            cmd,
            args.clone(),
            spec.modality,
            dense_dim(spec.output).ok_or_else(|| {
                lens_config_invalid(format!(
                    "external command lens {} ({}) requires dense output shape, got {:?}",
                    snapshot.lens_id, spec.name, spec.output
                ))
            })?,
        )),
        LensRuntime::CandleLocal { .. } => Arc::new(CandleLens::from_lens_spec(spec)?),
        LensRuntime::Onnx { .. } => Arc::new(OnnxLens::from_lens_spec(spec)?),
        LensRuntime::OnnxColbert { .. } => Arc::new(OnnxColbertLens::from_lens_spec(spec)?),
        LensRuntime::FastembedSparse { .. } => Arc::new(FastembedSparseLens::from_lens_spec(spec)?),
        LensRuntime::FastembedBgem3 { .. } => Arc::new(FastembedBgem3Lens::from_lens_spec(spec)?),
        LensRuntime::FastembedReranker { .. } => {
            Arc::new(FastembedRerankerLens::from_lens_spec(spec)?)
        }
        LensRuntime::FastembedQwen3 { .. } => Arc::new(FastembedQwen3Lens::from_lens_spec(spec)?),
        LensRuntime::StaticLookup { .. } => Arc::new(StaticLookupLens::from_lens_spec(spec)?),
        LensRuntime::MultimodalAdapter { .. } => {
            Arc::new(MultimodalAdapterLens::from_lens_spec(spec)?)
        }
    };
    snapshot.contract.verify_registration(lens.as_ref())?;
    Ok(lens)
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

fn lens_config_invalid(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: "CALYX_LENS_CONFIG_INVALID",
        message: message.into(),
        remediation: "fix persisted LensSpec runtime fields or re-register the lens",
    }
}

struct LazyPersistedLens {
    snapshot: RegistryLensSnapshot,
    runtime: Mutex<Option<LazyRuntimeCache>>,
}

enum LazyRuntimeCache {
    Loaded(Arc<dyn Lens>),
    Failed(String),
}

impl LazyPersistedLens {
    fn new(snapshot: RegistryLensSnapshot) -> Self {
        Self {
            snapshot,
            runtime: Mutex::new(None),
        }
    }

    fn runtime(&self) -> Result<Arc<dyn Lens>> {
        let mut guard = self.runtime.lock().map_err(|_| {
            CalyxError::lens_unreachable(format!(
                "lazy persisted lens {} runtime mutex was poisoned",
                self.snapshot.lens_id
            ))
        })?;
        match guard.as_ref() {
            Some(LazyRuntimeCache::Loaded(runtime)) => return Ok(runtime.clone()),
            Some(LazyRuntimeCache::Failed(load_error)) => {
                return Err(self.error(load_error.clone()));
            }
            None => {}
        }
        match load_runtime_lens(&self.snapshot) {
            Ok(runtime) => {
                *guard = Some(LazyRuntimeCache::Loaded(runtime.clone()));
                Ok(runtime)
            }
            Err(error) => {
                let load_error = format!(
                    "{}: {} (remediation: {})",
                    error.code, error.message, error.remediation
                );
                *guard = Some(LazyRuntimeCache::Failed(load_error.clone()));
                Err(self.error(load_error))
            }
        }
    }

    fn error(&self, load_error: String) -> CalyxError {
        CalyxError::lens_unreachable(format!(
            "lens {} is persisted but its runtime failed to load in this process: {}",
            self.snapshot.lens_id, load_error
        ))
    }
}

impl Lens for LazyPersistedLens {
    fn id(&self) -> LensId {
        self.snapshot.lens_id
    }

    fn shape(&self) -> SlotShape {
        self.snapshot.contract.shape()
    }

    fn modality(&self) -> Modality {
        self.snapshot.contract.modality()
    }

    fn measure(&self, input: &Input) -> Result<SlotVector> {
        self.runtime()?.measure(input)
    }

    fn measure_batch(&self, inputs: &[Input]) -> Result<Vec<SlotVector>> {
        self.runtime()?.measure_batch(inputs)
    }
}
