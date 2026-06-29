use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistrySnapshotMeasureStats {
    pub input_count: usize,
    pub runtime_batch_limit: Option<usize>,
    pub effective_chunk_size: usize,
    pub chunk_count: usize,
    pub runtime_load_ms: u128,
    pub measure_ms: u128,
    pub total_ms: u128,
}

#[derive(Clone)]
pub struct LoadedRegistrySnapshotLens {
    snapshot: RegistryLensSnapshot,
    runtime: Arc<dyn Lens>,
    runtime_load_ms: u128,
}

impl LoadedRegistrySnapshotLens {
    pub fn load(snapshot: RegistryLensSnapshot) -> Result<Self> {
        verify_registry_snapshot_contract(&snapshot)?;
        let load_start = Instant::now();
        let runtime = load_runtime_lens(&snapshot)?;
        let runtime_load_ms = load_start.elapsed().as_millis();
        Ok(Self {
            snapshot,
            runtime,
            runtime_load_ms,
        })
    }

    pub fn lens_id(&self) -> LensId {
        self.snapshot.lens_id
    }

    pub fn runtime_load_ms(&self) -> u128 {
        self.runtime_load_ms
    }

    pub fn measure_batch_with_stats(
        &self,
        inputs: &[Input],
        runtime_batch_limit: Option<usize>,
    ) -> Result<(Vec<SlotVector>, RegistrySnapshotMeasureStats)> {
        measure_loaded_snapshot_lens_batch_with_stats(
            &self.snapshot,
            self.runtime.as_ref(),
            inputs,
            runtime_batch_limit,
            0,
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistryBatchLimitUpdate {
    pub lens_id: LensId,
    pub max_batch: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistryBatchLimitChange {
    pub lens_id: LensId,
    pub name: String,
    pub before: Option<usize>,
    pub after: usize,
    pub changed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VaultRegistryBatchLimitWrite {
    pub manifest_seq: u64,
    pub durable_seq: u64,
    pub panel_ref: ImmutableRef,
    pub registry_ref: ImmutableRef,
    pub wrote_manifest: bool,
    pub changes: Vec<RegistryBatchLimitChange>,
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
    let (vectors, _) = measure_registry_snapshot_lens_batch_with_stats(snapshot, inputs, None)?;
    Ok(vectors)
}

pub fn measure_registry_snapshot_lens_batch_with_stats(
    snapshot: &RegistryLensSnapshot,
    inputs: &[Input],
    runtime_batch_limit: Option<usize>,
) -> Result<(Vec<SlotVector>, RegistrySnapshotMeasureStats)> {
    let total_start = Instant::now();
    verify_registry_snapshot_inputs(snapshot, inputs)?;
    let load_start = Instant::now();
    let runtime = load_runtime_lens(snapshot)?;
    let runtime_load_ms = load_start.elapsed().as_millis();
    let (vectors, mut stats) = measure_loaded_snapshot_lens_batch_with_stats(
        snapshot,
        runtime.as_ref(),
        inputs,
        runtime_batch_limit,
        runtime_load_ms,
    )?;
    stats.total_ms = total_start.elapsed().as_millis();
    Ok((vectors, stats))
}

fn verify_registry_snapshot_contract(snapshot: &RegistryLensSnapshot) -> Result<()> {
    if snapshot.lens_id != snapshot.contract.lens_id() {
        return Err(CalyxError::lens_frozen_violation(format!(
            "registry lens {} does not match frozen contract {}",
            snapshot.lens_id,
            snapshot.contract.lens_id()
        )));
    }
    Ok(())
}

fn verify_registry_snapshot_inputs(
    snapshot: &RegistryLensSnapshot,
    inputs: &[Input],
) -> Result<()> {
    verify_registry_snapshot_contract(snapshot)?;
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
    Ok(())
}

fn measure_loaded_snapshot_lens_batch_with_stats(
    snapshot: &RegistryLensSnapshot,
    runtime: &dyn Lens,
    inputs: &[Input],
    runtime_batch_limit: Option<usize>,
    runtime_load_ms: u128,
) -> Result<(Vec<SlotVector>, RegistrySnapshotMeasureStats)> {
    let total_start = Instant::now();
    verify_registry_snapshot_inputs(snapshot, inputs)?;
    let effective_chunk_size =
        effective_runtime_chunk_size(snapshot, inputs.len(), runtime_batch_limit)?;
    let chunk_count = if inputs.is_empty() {
        0
    } else {
        inputs.len().div_ceil(effective_chunk_size)
    };
    let measure_start = Instant::now();
    let mut vectors = Vec::with_capacity(inputs.len());
    if !inputs.is_empty() {
        for chunk in inputs.chunks(effective_chunk_size) {
            let chunk_vectors = runtime.measure_batch(chunk)?;
            if chunk_vectors.len() != chunk.len() {
                return Err(CalyxError::lens_dim_mismatch(format!(
                    "lens {} returned {} vectors for {} input chunk rows",
                    snapshot.lens_id,
                    chunk_vectors.len(),
                    chunk.len()
                )));
            }
            vectors.extend(chunk_vectors);
        }
    }
    let measure_ms = measure_start.elapsed().as_millis();
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
    let stats = RegistrySnapshotMeasureStats {
        input_count: inputs.len(),
        runtime_batch_limit,
        effective_chunk_size,
        chunk_count,
        runtime_load_ms,
        measure_ms,
        total_ms: total_start.elapsed().as_millis(),
    };
    Ok((vectors, stats))
}

fn effective_runtime_chunk_size(
    snapshot: &RegistryLensSnapshot,
    input_count: usize,
    runtime_batch_limit: Option<usize>,
) -> Result<usize> {
    if runtime_batch_limit == Some(0) {
        return Err(CalyxError::lens_unreachable(
            "runtime batch limit must be > 0 when supplied",
        ));
    }
    let spec_limit = snapshot.spec.as_ref().and_then(|spec| spec.max_batch);
    if spec_limit == Some(0) {
        return Err(lens_config_invalid("LensSpec max_batch must be > 0"));
    }
    let limit = match (runtime_batch_limit, spec_limit) {
        (Some(runtime), Some(spec)) => runtime.min(spec),
        (Some(runtime), None) => runtime,
        (None, Some(spec)) => spec,
        (None, None) => input_count.max(1),
    };
    Ok(limit.max(1))
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

fn registry_batch_limit_invalid(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: "CALYX_REGISTRY_BATCH_LIMIT_INVALID",
        message: message.into(),
        remediation: "pass positive, unique lens batch limits that match lenses in the persisted vault registry",
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::time::{SystemTime, UNIX_EPOCH};

    use calyx_aster::vault::{AsterVault, VaultOptions};
    use calyx_core::{
        Asymmetry, Input, Modality, Panel, QuantPolicy, Slot, SlotId, SlotKey, SlotState, VaultId,
    };

    use super::*;
    use crate::{AlgorithmicLens, DeterminismProof, LensRuntime, LensSpec};

    #[test]
    fn snapshot_measurement_chunks_by_runtime_limit_and_reports_stats() {
        let snapshot = algorithmic_snapshot(Some(3));
        let inputs = text_inputs(["alpha", "beta", "gamma", "delta", "epsilon"]);

        let (vectors, stats) =
            measure_registry_snapshot_lens_batch_with_stats(&snapshot, &inputs, Some(2)).unwrap();

        assert_eq!(vectors.len(), 5);
        assert!(
            vectors
                .iter()
                .all(|vector| matches!(vector, SlotVector::Dense { dim: 16, .. }))
        );
        assert_eq!(stats.input_count, 5);
        assert_eq!(stats.runtime_batch_limit, Some(2));
        assert_eq!(stats.effective_chunk_size, 2);
        assert_eq!(stats.chunk_count, 3);
    }

    #[test]
    fn snapshot_measurement_empty_input_reports_zero_chunks() {
        let snapshot = algorithmic_snapshot(Some(3));

        let (vectors, stats) =
            measure_registry_snapshot_lens_batch_with_stats(&snapshot, &[], Some(2)).unwrap();

        assert!(vectors.is_empty());
        assert_eq!(stats.input_count, 0);
        assert_eq!(stats.effective_chunk_size, 2);
        assert_eq!(stats.chunk_count, 0);
    }

    #[test]
    fn snapshot_measurement_rejects_zero_runtime_limit() {
        let snapshot = algorithmic_snapshot(Some(3));
        let inputs = text_inputs(["alpha"]);

        let error = measure_registry_snapshot_lens_batch_with_stats(&snapshot, &inputs, Some(0))
            .unwrap_err();

        assert_eq!(error.code, "CALYX_LENS_UNREACHABLE");
        assert!(error.message.contains("runtime batch limit must be > 0"));
    }

    #[test]
    fn loaded_snapshot_lens_reuses_runtime_and_reports_zero_per_request_load() {
        let snapshot = algorithmic_snapshot(Some(2));
        let loaded = LoadedRegistrySnapshotLens::load(snapshot).unwrap();
        let first_inputs = text_inputs(["alpha", "beta", "gamma"]);
        let second_inputs = text_inputs(["delta", "epsilon"]);

        let (first_vectors, first_stats) = loaded
            .measure_batch_with_stats(&first_inputs, Some(2))
            .unwrap();
        let (second_vectors, second_stats) = loaded
            .measure_batch_with_stats(&second_inputs, Some(2))
            .unwrap();

        println!(
            "loaded_snapshot_lens_state load_ms={} first_stats={first_stats:?} second_stats={second_stats:?}",
            loaded.runtime_load_ms()
        );
        assert_eq!(first_vectors.len(), 3);
        assert_eq!(second_vectors.len(), 2);
        assert_eq!(first_stats.runtime_load_ms, 0);
        assert_eq!(second_stats.runtime_load_ms, 0);
        assert_eq!(first_stats.effective_chunk_size, 2);
        assert_eq!(first_stats.chunk_count, 2);
        assert_eq!(second_stats.effective_chunk_size, 2);
        assert_eq!(second_stats.chunk_count, 1);
    }

    #[test]
    fn vault_batch_limit_update_persists_manifest_backed_registry() {
        let (vault, lens_id) = test_vault_with_batch_lens("happy", Some(1));
        let before_manifest = ManifestStore::open(&vault).load_current().unwrap();
        let before_max = manifest_registry_max_batch(&vault, lens_id);
        println!(
            "before batch-limit happy path: manifest_seq={} registry_ref={:?} max_batch={before_max:?}",
            before_manifest.manifest_seq,
            before_manifest
                .registry_ref
                .as_ref()
                .map(|reference| reference.logical_path.as_str())
        );

        let write = set_vault_registry_batch_limits(
            &vault,
            &[RegistryBatchLimitUpdate {
                lens_id,
                max_batch: 8,
            }],
        )
        .unwrap();
        let after_manifest = ManifestStore::open(&vault).load_current().unwrap();
        let after_state = load_vault_panel_state(&vault).unwrap();
        let after_max = manifest_registry_max_batch(&vault, lens_id);
        println!(
            "after batch-limit happy path: manifest_seq={} registry_ref={:?} max_batch={after_max:?} wrote_manifest={} registry_file_exists={}",
            after_manifest.manifest_seq,
            after_manifest
                .registry_ref
                .as_ref()
                .map(|reference| reference.logical_path.as_str()),
            write.wrote_manifest,
            vault.join(&write.registry_ref.logical_path).is_file()
        );

        assert!(write.wrote_manifest);
        assert_eq!(write.changes.len(), 1);
        assert_eq!(write.changes[0].before, Some(1));
        assert_eq!(write.changes[0].after, 8);
        assert!(write.changes[0].changed);
        assert_ne!(before_manifest.registry_ref, after_manifest.registry_ref);
        assert_eq!(before_max, Some(1));
        assert_eq!(after_max, Some(8));
        assert_eq!(
            after_state
                .registry
                .lens_spec(lens_id)
                .and_then(|spec| spec.max_batch),
            Some(8)
        );
    }

    #[test]
    fn vault_batch_limit_update_rejects_empty_without_manifest_write() {
        let (vault, lens_id) = test_vault_with_batch_lens("empty", Some(1));
        let before_manifest = ManifestStore::open(&vault).load_current().unwrap();
        let before_max = manifest_registry_max_batch(&vault, lens_id);
        println!(
            "before empty edge: manifest_seq={} registry_ref={:?} max_batch={before_max:?}",
            before_manifest.manifest_seq,
            before_manifest
                .registry_ref
                .as_ref()
                .map(|reference| reference.logical_path.as_str())
        );

        let error = set_vault_registry_batch_limits(&vault, &[]).unwrap_err();
        let after_manifest = ManifestStore::open(&vault).load_current().unwrap();
        let after_max = manifest_registry_max_batch(&vault, lens_id);
        println!(
            "after empty edge: error_code={} manifest_seq={} registry_ref={:?} max_batch={after_max:?}",
            error.code,
            after_manifest.manifest_seq,
            after_manifest
                .registry_ref
                .as_ref()
                .map(|reference| reference.logical_path.as_str())
        );

        assert_eq!(error.code, "CALYX_REGISTRY_BATCH_LIMIT_INVALID");
        assert_eq!(before_manifest.manifest_seq, after_manifest.manifest_seq);
        assert_eq!(before_manifest.registry_ref, after_manifest.registry_ref);
        assert_eq!(before_max, after_max);
    }

    #[test]
    fn vault_batch_limit_update_rejects_zero_without_manifest_write() {
        let (vault, lens_id) = test_vault_with_batch_lens("zero", Some(1));
        let before_manifest = ManifestStore::open(&vault).load_current().unwrap();
        let before_max = manifest_registry_max_batch(&vault, lens_id);
        println!(
            "before zero edge: manifest_seq={} registry_ref={:?} max_batch={before_max:?}",
            before_manifest.manifest_seq,
            before_manifest
                .registry_ref
                .as_ref()
                .map(|reference| reference.logical_path.as_str())
        );

        let error = set_vault_registry_batch_limits(
            &vault,
            &[RegistryBatchLimitUpdate {
                lens_id,
                max_batch: 0,
            }],
        )
        .unwrap_err();
        let after_manifest = ManifestStore::open(&vault).load_current().unwrap();
        let after_max = manifest_registry_max_batch(&vault, lens_id);
        println!(
            "after zero edge: error_code={} manifest_seq={} registry_ref={:?} max_batch={after_max:?}",
            error.code,
            after_manifest.manifest_seq,
            after_manifest
                .registry_ref
                .as_ref()
                .map(|reference| reference.logical_path.as_str())
        );

        assert_eq!(error.code, "CALYX_REGISTRY_BATCH_LIMIT_INVALID");
        assert_eq!(before_manifest.manifest_seq, after_manifest.manifest_seq);
        assert_eq!(before_manifest.registry_ref, after_manifest.registry_ref);
        assert_eq!(before_max, after_max);
    }

    #[test]
    fn vault_batch_limit_update_rejects_missing_lens_without_manifest_write() {
        let (vault, lens_id) = test_vault_with_batch_lens("missing", Some(1));
        let missing = calyx_core::LensId::from_bytes([0xA5; 16]);
        let before_manifest = ManifestStore::open(&vault).load_current().unwrap();
        let before_max = manifest_registry_max_batch(&vault, lens_id);
        println!(
            "before missing edge: manifest_seq={} registry_ref={:?} existing_max_batch={before_max:?}",
            before_manifest.manifest_seq,
            before_manifest
                .registry_ref
                .as_ref()
                .map(|reference| reference.logical_path.as_str())
        );

        let error = set_vault_registry_batch_limits(
            &vault,
            &[RegistryBatchLimitUpdate {
                lens_id: missing,
                max_batch: 8,
            }],
        )
        .unwrap_err();
        let after_manifest = ManifestStore::open(&vault).load_current().unwrap();
        let after_max = manifest_registry_max_batch(&vault, lens_id);
        println!(
            "after missing edge: error_code={} manifest_seq={} registry_ref={:?} existing_max_batch={after_max:?}",
            error.code,
            after_manifest.manifest_seq,
            after_manifest
                .registry_ref
                .as_ref()
                .map(|reference| reference.logical_path.as_str())
        );

        assert_eq!(error.code, "CALYX_LENS_UNREACHABLE");
        assert_eq!(before_manifest.manifest_seq, after_manifest.manifest_seq);
        assert_eq!(before_manifest.registry_ref, after_manifest.registry_ref);
        assert_eq!(before_max, after_max);
    }

    fn algorithmic_snapshot(max_batch: Option<usize>) -> RegistryLensSnapshot {
        let lens = AlgorithmicLens::byte_features("issue999-byte", Modality::Text);
        let contract = lens.contract().clone();
        let spec = LensSpec {
            name: contract.name().to_string(),
            runtime: LensRuntime::Algorithmic {
                kind: "byte-features".to_string(),
            },
            output: contract.shape(),
            modality: contract.modality(),
            weights_sha256: contract.weights_sha256(),
            corpus_hash: contract.corpus_hash(),
            norm_policy: contract.norm_policy(),
            max_batch,
            axis: None,
            asymmetry: Asymmetry::None,
            quant_default: QuantPolicy::turboquant_default(),
            truncate_dim: None,
            recall_delta: crate::spec::default_recall_delta(),
            retrieval_only: false,
            excluded_from_dedup: false,
        };
        RegistryLensSnapshot {
            lens_id: contract.lens_id(),
            contract,
            spec: Some(spec),
            determinism: DeterminismProof::ContractOnlyExemption,
        }
    }

    fn text_inputs<const N: usize>(values: [&str; N]) -> Vec<Input> {
        values
            .into_iter()
            .map(|value| Input::new(Modality::Text, value.as_bytes().to_vec()))
            .collect()
    }

    fn test_vault_with_batch_lens(
        name: &str,
        max_batch: Option<usize>,
    ) -> (PathBuf, calyx_core::LensId) {
        let vault = temp_vault_dir(name);
        let vault_id: VaultId = "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap();
        let mut registry = Registry::new();
        let lens = AlgorithmicLens::byte_features("batch-limit-lens", Modality::Text);
        let contract = lens.contract().clone();
        let lens_id = contract.lens_id();
        let spec = LensSpec {
            name: contract.name().to_string(),
            runtime: LensRuntime::Algorithmic {
                kind: "byte-features".to_string(),
            },
            output: contract.shape(),
            modality: contract.modality(),
            weights_sha256: contract.weights_sha256(),
            corpus_hash: contract.corpus_hash(),
            norm_policy: contract.norm_policy(),
            max_batch,
            axis: None,
            asymmetry: Asymmetry::None,
            quant_default: QuantPolicy::turboquant_default(),
            truncate_dim: None,
            recall_delta: crate::spec::default_recall_delta(),
            retrieval_only: false,
            excluded_from_dedup: false,
        };
        registry
            .register_frozen_with_spec(lens, contract, spec)
            .unwrap();
        let panel = panel_with_lens(lens_id);
        AsterVault::new_durable(
            &vault,
            vault_id,
            [0x5A; 32],
            VaultOptions {
                panel: Some(panel.clone()),
                ..VaultOptions::default()
            },
        )
        .unwrap();
        persist_vault_panel_state(&vault, &panel, &registry).unwrap();
        (vault, lens_id)
    }

    fn panel_with_lens(lens_id: calyx_core::LensId) -> Panel {
        let slot = SlotId::new(0);
        Panel {
            version: 1,
            slots: vec![Slot {
                slot_id: slot,
                slot_key: SlotKey::new(slot, "batch-limit-lens"),
                lens_id,
                shape: SlotShape::Dense(16),
                modality: Modality::Text,
                asymmetry: Asymmetry::None,
                quant: QuantPolicy::None,
                resource: Default::default(),
                axis: Some("batch-limit-lens".to_string()),
                retrieval_only: false,
                excluded_from_dedup: false,
                bits_about: BTreeMap::new(),
                state: SlotState::Active,
                added_at_panel_version: 1,
            }],
            created_at: 1,
            kernel_ref: None,
            guard_ref: None,
        }
    }

    fn manifest_registry_max_batch(vault: &Path, lens_id: calyx_core::LensId) -> Option<usize> {
        let manifest = ManifestStore::open(vault).load_current().unwrap();
        let registry_ref = manifest.registry_ref.as_ref().unwrap();
        let bytes = fs::read(vault.join(&registry_ref.logical_path)).unwrap();
        let snapshot: VaultRegistrySnapshot = serde_json::from_slice(&bytes).unwrap();
        snapshot
            .lenses
            .iter()
            .find(|lens| lens.lens_id == lens_id)
            .and_then(|lens| lens.spec.as_ref())
            .and_then(|spec| spec.max_batch)
    }

    fn temp_vault_dir(name: &str) -> PathBuf {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "calyx-registry-batch-limit-{name}-{}-{now}",
            std::process::id()
        ))
    }
}
