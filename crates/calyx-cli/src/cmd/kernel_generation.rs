//! Durable current-generation contract for kernel artifacts.

use std::fs;
use std::io::{BufReader, Read};
use std::path::{Component, Path, PathBuf};

use calyx_core::{CxId, LedgerRef, SlotId};
use calyx_lodestar::{
    FsKernelStore, PANEL_ASTER_ASSOC_COLLECTION, PANEL_RRF_K, TypedPanelKernelIndex,
    load_typed_panel_kernel_index_verified, read_kernel_artifact,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::durable_write::{DurableWriteLockGuard, write_bytes_atomic};
use crate::error::{CliError, CliResult};

const SCHEMA_VERSION: u32 = 3;
const CURRENT_FILE: &str = "idx/kernel/CURRENT";
const MANIFEST_DIR: &str = "idx/kernel/manifests";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct KernelArtifactRef {
    pub relative_path: String,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct KernelGraphContract {
    pub collection: String,
    pub nodes: usize,
    pub edges: usize,
    pub source_seq: u64,
    pub embedding_slots: Vec<SlotId>,
    pub fusion: String,
    pub rrf_k: u32,
    pub panel_version: u64,
    pub knn: usize,
    pub edge_score_threshold: f32,
    pub metadata_sha256: String,
    pub node_props_sha256: String,
    pub csr_sha256: String,
    pub physical_contract_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct KernelAdmissionContract {
    pub schema_version: u32,
    pub method: String,
    pub corpus_count: usize,
    pub sample_count: usize,
    pub sample_limit: usize,
    pub sample_seed: u64,
    pub lower_tail_quantile: f32,
    pub threshold: f32,
    pub min_score: f32,
    pub median_score: f32,
    pub max_score: f32,
    pub sample_ids_sha256: String,
    pub observations_sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub calibration_queries: Option<KernelArtifactRef>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct KernelJurisdictionScope {
    pub source_dataset: String,
    pub court_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub county: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub appellate_district: Option<String>,
    pub source_rows: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct KernelJurisdictionContract {
    pub schema_version: u32,
    pub country: String,
    pub court_system: String,
    pub state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub county: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub appellate_district: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scopes: Vec<KernelJurisdictionScope>,
    pub source_rows: usize,
    pub metadata_contract_sha256: String,
}

pub(crate) fn jurisdiction_metadata_sha256(
    jurisdiction: &KernelJurisdictionContract,
) -> Option<String> {
    let mut digest = Sha256::new();
    match jurisdiction.schema_version {
        1 => {
            digest.update(b"calyx-kernel-jurisdiction-v1");
            let fields = [
                (
                    "appellate_district",
                    jurisdiction.appellate_district.as_deref()?,
                ),
                ("county", jurisdiction.county.as_deref()?),
                ("country", jurisdiction.country.as_str()),
                ("court_system", jurisdiction.court_system.as_str()),
                ("state", jurisdiction.state.as_str()),
            ];
            for (key, value) in fields {
                digest.update(key.as_bytes());
                digest.update([0]);
                digest.update(value.as_bytes());
                digest.update([0]);
            }
        }
        2 => {
            digest.update(b"calyx-kernel-jurisdiction-v2");
            for (key, value) in [
                ("country", jurisdiction.country.as_str()),
                ("court_system", jurisdiction.court_system.as_str()),
                ("state", jurisdiction.state.as_str()),
            ] {
                digest.update(key.as_bytes());
                digest.update([0]);
                digest.update(value.as_bytes());
                digest.update([0]);
            }
            for scope in &jurisdiction.scopes {
                digest.update(scope.source_dataset.as_bytes());
                digest.update([0]);
                for court_id in &scope.court_ids {
                    digest.update(court_id.as_bytes());
                    digest.update([0]);
                }
                digest.update([0xff]);
                if let Some(county) = &scope.county {
                    digest.update(county.as_bytes());
                }
                digest.update([0]);
                if let Some(appellate_district) = &scope.appellate_district {
                    digest.update(appellate_district.as_bytes());
                }
                digest.update([0]);
                digest.update(scope.source_rows.to_le_bytes());
            }
        }
        _ => return None,
    }
    digest.update(jurisdiction.source_rows.to_le_bytes());
    Some(hex32(&digest.finalize().into()))
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct KernelGenerationManifest {
    pub schema_version: u32,
    pub kernel_id: CxId,
    pub kernel_json: KernelArtifactRef,
    pub index_json: KernelArtifactRef,
    pub graph: KernelGraphContract,
    pub admission: KernelAdmissionContract,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jurisdiction: Option<KernelJurisdictionContract>,
    pub build_ledger_seq: u64,
    pub build_ledger_hash: String,
}

#[derive(Clone, Debug)]
pub(crate) struct LoadedKernelGeneration {
    pub manifest: KernelGenerationManifest,
    pub manifest_path: PathBuf,
    pub manifest_sha256: String,
}

pub(crate) fn acquire_build_lock(vault: &Path) -> CliResult<DurableWriteLockGuard> {
    DurableWriteLockGuard::acquire(&vault.join("idx/kernel/BUILD"), "kernel generation build")
}

pub(crate) fn artifact_ref(vault: &Path, path: &Path) -> CliResult<KernelArtifactRef> {
    let relative = path.strip_prefix(vault).map_err(|error| {
        CliError::runtime(format!(
            "kernel artifact {} is outside vault {}: {error}",
            path.display(),
            vault.display()
        ))
    })?;
    let bytes = fs::metadata(path)
        .map_err(|error| CliError::io(format!("stat kernel artifact {}: {error}", path.display())))?
        .len();
    Ok(KernelArtifactRef {
        relative_path: relative_path(relative)?,
        bytes,
        sha256: sha256_file(path)?,
    })
}

pub(crate) fn artifact_ref_from_physical_readback(
    vault: &Path,
    path: &Path,
    bytes: u64,
    sha256: String,
) -> CliResult<KernelArtifactRef> {
    let relative = path.strip_prefix(vault).map_err(|error| {
        CliError::runtime(format!(
            "kernel artifact {} is outside vault {}: {error}",
            path.display(),
            vault.display()
        ))
    })?;
    let metadata = fs::metadata(path).map_err(|error| {
        CliError::io(format!("stat kernel artifact {}: {error}", path.display()))
    })?;
    let _ = decode_sha256(&sha256, "kernel artifact physical readback sha256")?;
    if !metadata.is_file() || metadata.len() != bytes {
        return Err(CliError::runtime(format!(
            "kernel artifact changed after physical readback path={} expected_bytes={} actual_bytes={} is_file={}",
            path.display(),
            bytes,
            metadata.len(),
            metadata.is_file()
        )));
    }
    Ok(KernelArtifactRef {
        relative_path: relative_path(relative)?,
        bytes,
        sha256,
    })
}

pub(crate) fn publish_current_generation(
    vault: &Path,
    manifest: KernelGenerationManifest,
) -> CliResult<LoadedKernelGeneration> {
    if manifest.schema_version != SCHEMA_VERSION {
        return Err(CliError::runtime("kernel manifest schema version mismatch"));
    }
    let mut bytes = serde_json::to_vec_pretty(&manifest)
        .map_err(|error| CliError::runtime(format!("serialize kernel manifest: {error}")))?;
    bytes.push(b'\n');
    let manifest_sha256 = sha256_bytes(&bytes);
    let relative = format!("{MANIFEST_DIR}/{manifest_sha256}.json");
    let manifest_path = vault.join(&relative);
    install_immutable(&manifest_path, &bytes, "kernel generation manifest")?;
    let physical = fs::read(&manifest_path).map_err(|error| {
        CliError::io(format!(
            "read back kernel manifest {}: {error}",
            manifest_path.display()
        ))
    })?;
    if physical != bytes || sha256_bytes(&physical) != manifest_sha256 {
        return Err(CliError::runtime(format!(
            "kernel manifest physical readback mismatch at {}",
            manifest_path.display()
        )));
    }
    write_bytes_atomic(
        &vault.join(CURRENT_FILE),
        format!("{relative}\n").as_bytes(),
        "kernel CURRENT pointer",
    )?;
    let loaded = load_current_generation(vault)?;
    if loaded.manifest_sha256 != manifest_sha256 || loaded.manifest != manifest {
        return Err(CliError::runtime(
            "kernel CURRENT independent readback differs from the published generation",
        ));
    }
    Ok(loaded)
}

pub(crate) fn load_current_generation(vault: &Path) -> CliResult<LoadedKernelGeneration> {
    load_current_generation_with_index(vault).map(|(loaded, _)| loaded)
}

pub(crate) fn load_current_generation_with_index(
    vault: &Path,
) -> CliResult<(LoadedKernelGeneration, TypedPanelKernelIndex)> {
    let current_path = vault.join(CURRENT_FILE);
    let pointer = fs::read_to_string(&current_path).map_err(|error| {
        CliError::io(format!(
            "read kernel CURRENT pointer {}: {error}; run `calyx kernel-build <vault>`",
            current_path.display()
        ))
    })?;
    let relative = pointer.strip_suffix('\n').ok_or_else(|| {
        CliError::runtime(format!(
            "kernel CURRENT {} must contain one newline-terminated relative manifest path",
            current_path.display()
        ))
    })?;
    validate_manifest_pointer(relative)?;
    load_generation_path_with_index(vault, &vault.join(relative))
}

/// Read and structurally validate the content-addressed CURRENT manifest
/// without cold-loading its multi-GB index. A caller may use this only when a
/// separately verified resident proves it has pinned this exact manifest,
/// kernel id, artifact digest, and row count.
pub(crate) fn load_current_generation_reference(vault: &Path) -> CliResult<LoadedKernelGeneration> {
    let current_path = vault.join(CURRENT_FILE);
    let pointer = fs::read_to_string(&current_path).map_err(|error| {
        CliError::io(format!(
            "read kernel CURRENT pointer {}: {error}; run `calyx kernel-build <vault>`",
            current_path.display()
        ))
    })?;
    let relative = pointer.strip_suffix('\n').ok_or_else(|| {
        CliError::runtime(format!(
            "kernel CURRENT {} must contain one newline-terminated relative manifest path",
            current_path.display()
        ))
    })?;
    validate_manifest_pointer(relative)?;
    load_generation_path_reference(vault, &vault.join(relative))
}

pub(crate) fn load_generation_by_sha256(
    vault: &Path,
    manifest_sha256: &str,
) -> CliResult<LoadedKernelGeneration> {
    load_generation_by_sha256_with_index(vault, manifest_sha256).map(|(loaded, _)| loaded)
}

pub(crate) fn load_generation_by_sha256_with_index(
    vault: &Path,
    manifest_sha256: &str,
) -> CliResult<(LoadedKernelGeneration, TypedPanelKernelIndex)> {
    let _ = decode_sha256(manifest_sha256, "kernel_manifest_sha256")?;
    load_generation_path_with_index(
        vault,
        &vault
            .join(MANIFEST_DIR)
            .join(format!("{manifest_sha256}.json")),
    )
}

pub(crate) fn load_generation_by_sha256_reference(
    vault: &Path,
    manifest_sha256: &str,
) -> CliResult<LoadedKernelGeneration> {
    let _ = decode_sha256(manifest_sha256, "kernel_manifest_sha256")?;
    load_generation_path_reference(
        vault,
        &vault
            .join(MANIFEST_DIR)
            .join(format!("{manifest_sha256}.json")),
    )
}

fn load_generation_path_with_index(
    vault: &Path,
    manifest_path: &Path,
) -> CliResult<(LoadedKernelGeneration, TypedPanelKernelIndex)> {
    let loaded = load_generation_path_reference(vault, manifest_path)?;
    let index = validate_manifest_index(vault, &loaded.manifest)?;
    Ok((loaded, index))
}

fn load_generation_path_reference(
    vault: &Path,
    manifest_path: &Path,
) -> CliResult<LoadedKernelGeneration> {
    let bytes = fs::read(manifest_path).map_err(|error| {
        CliError::io(format!(
            "read CURRENT kernel manifest {}: {error}",
            manifest_path.display()
        ))
    })?;
    let manifest_sha256 = sha256_bytes(&bytes);
    let expected_name = format!("{manifest_sha256}.json");
    if manifest_path.file_name().and_then(|name| name.to_str()) != Some(&expected_name) {
        return Err(CliError::runtime(format!(
            "kernel manifest digest {} does not match filename {}",
            manifest_sha256,
            manifest_path.display()
        )));
    }
    let manifest: KernelGenerationManifest = serde_json::from_slice(&bytes).map_err(|error| {
        CliError::runtime(format!(
            "decode CURRENT kernel manifest {}: {error}",
            manifest_path.display()
        ))
    })?;
    validate_manifest_reference(vault, &manifest)?;
    Ok(LoadedKernelGeneration {
        manifest,
        manifest_path: manifest_path.to_path_buf(),
        manifest_sha256,
    })
}

fn validate_manifest_reference(vault: &Path, manifest: &KernelGenerationManifest) -> CliResult {
    if manifest.schema_version != SCHEMA_VERSION {
        return Err(CliError::runtime(format!(
            "unsupported kernel manifest schema {}",
            manifest.schema_version
        )));
    }
    if manifest.graph.collection != PANEL_ASTER_ASSOC_COLLECTION
        || manifest.graph.embedding_slots.len() < 2
        || !manifest
            .graph
            .embedding_slots
            .windows(2)
            .all(|pair| pair[0] < pair[1])
        || manifest.graph.fusion != "typed_block_rrf"
        || manifest.graph.rrf_k != PANEL_RRF_K
        || !manifest.graph.edge_score_threshold.is_finite()
        || !(0.0..=1.0).contains(&manifest.graph.edge_score_threshold)
    {
        return Err(CliError::runtime(
            "kernel manifest contains an invalid no-flatten panel graph contract",
        ));
    }
    validate_admission(
        vault,
        &manifest.admission,
        manifest.graph.nodes,
        manifest.jurisdiction.is_some(),
    )?;
    if let Some(jurisdiction) = &manifest.jurisdiction {
        validate_jurisdiction(jurisdiction, manifest.graph.nodes)?;
    }
    validate_artifact(vault, &manifest.kernel_json)?;
    validate_artifact_metadata(vault, &manifest.index_json)?;
    let store = FsKernelStore::new(vault);
    let kernel = read_kernel_artifact(manifest.kernel_id, &store)?;
    let expected_kernel = relative_path(
        store
            .kernel_file_path(manifest.kernel_id)
            .strip_prefix(vault)
            .map_err(|error| {
                CliError::runtime(format!("resolve kernel artifact contract: {error}"))
            })?,
    )?;
    let expected_index = relative_path(
        store
            .index_file_path(manifest.kernel_id)
            .strip_prefix(vault)
            .map_err(|error| {
                CliError::runtime(format!("resolve kernel index contract: {error}"))
            })?,
    )?;
    if manifest.kernel_json.relative_path != expected_kernel
        || manifest.index_json.relative_path != expected_index
        || kernel.kernel_id != manifest.kernel_id
    {
        return Err(CliError::runtime(format!(
            "CURRENT kernel artifact paths or physical kernel id differ from manifest {}",
            manifest.kernel_id
        )));
    }
    Ok(())
}

fn validate_manifest_index(
    vault: &Path,
    manifest: &KernelGenerationManifest,
) -> CliResult<TypedPanelKernelIndex> {
    let store = FsKernelStore::new(vault);
    let kernel = read_kernel_artifact(manifest.kernel_id, &store)?;
    // One physical pass simultaneously parses every typed row, validates its
    // contract, counts bytes, and verifies the immutable manifest SHA-256.
    let index = load_typed_panel_kernel_index_verified(
        manifest.kernel_id,
        &store,
        manifest.index_json.bytes,
        &manifest.index_json.sha256,
    )?;
    if kernel.members.len() != index.rows().len() {
        return Err(CliError::runtime(format!(
            "CURRENT kernel {} has {} members but {} index rows",
            manifest.kernel_id,
            kernel.members.len(),
            index.rows().len()
        )));
    }
    Ok(index)
}

fn validate_jurisdiction(
    jurisdiction: &KernelJurisdictionContract,
    graph_nodes: usize,
) -> CliResult {
    let global_fields = [
        jurisdiction.country.as_str(),
        jurisdiction.court_system.as_str(),
        jurisdiction.state.as_str(),
    ];
    let common_invalid = jurisdiction.source_rows != graph_nodes
        || global_fields
            .iter()
            .any(|field| field.trim().is_empty() || field.trim() != *field)
        || decode_sha256(
            &jurisdiction.metadata_contract_sha256,
            "jurisdiction metadata_contract_sha256",
        )
        .is_err()
        || jurisdiction_metadata_sha256(jurisdiction).as_deref()
            != Some(jurisdiction.metadata_contract_sha256.as_str());
    let version_invalid = match jurisdiction.schema_version {
        1 => {
            !jurisdiction.scopes.is_empty()
                || jurisdiction
                    .county
                    .as_deref()
                    .is_none_or(|value| value.trim().is_empty() || value.trim() != value)
                || jurisdiction
                    .appellate_district
                    .as_deref()
                    .is_none_or(|value| value.trim().is_empty() || value.trim() != value)
        }
        2 => {
            jurisdiction.county.is_some()
                || jurisdiction.appellate_district.is_some()
                || jurisdiction.scopes.is_empty()
                || jurisdiction_scope_rows(&jurisdiction.scopes) != Some(jurisdiction.source_rows)
                || jurisdiction.scopes.iter().any(invalid_jurisdiction_scope)
                || !jurisdiction.scopes.windows(2).all(|pair| {
                    jurisdiction_scope_sort_key(&pair[0]) < jurisdiction_scope_sort_key(&pair[1])
                })
        }
        _ => true,
    };
    if common_invalid || version_invalid {
        return Err(CliError::runtime(
            "kernel manifest contains an invalid jurisdiction contract",
        ));
    }
    Ok(())
}

fn invalid_jurisdiction_scope(scope: &KernelJurisdictionScope) -> bool {
    !scope.source_dataset.starts_with("courtlistener-")
        || scope.source_dataset.trim() != scope.source_dataset
        || scope.source_rows == 0
        || scope.court_ids.is_empty()
        || scope
            .court_ids
            .iter()
            .any(|court_id| court_id.trim().is_empty() || court_id.trim() != court_id)
        || !scope.court_ids.windows(2).all(|pair| pair[0] < pair[1])
        || scope
            .county
            .as_deref()
            .is_some_and(|value| value.trim().is_empty() || value.trim() != value)
        || scope
            .appellate_district
            .as_deref()
            .is_some_and(|value| value.trim().is_empty() || value.trim() != value)
}

fn jurisdiction_scope_rows(scopes: &[KernelJurisdictionScope]) -> Option<usize> {
    scopes
        .iter()
        .try_fold(0_usize, |total, scope| total.checked_add(scope.source_rows))
}

fn jurisdiction_scope_sort_key(
    scope: &KernelJurisdictionScope,
) -> (&str, Option<&str>, Option<&str>) {
    (
        scope.source_dataset.as_str(),
        scope.county.as_deref(),
        scope.appellate_district.as_deref(),
    )
}

fn validate_admission(
    vault: &Path,
    admission: &KernelAdmissionContract,
    graph_nodes: usize,
    has_jurisdiction: bool,
) -> CliResult {
    let stats = [
        admission.lower_tail_quantile,
        admission.threshold,
        admission.min_score,
        admission.median_score,
        admission.max_score,
    ];
    let common_invalid = admission.schema_version != 3
        || admission.corpus_count != graph_nodes
        || admission.sample_count < 2
        || admission.sample_count > admission.sample_limit
        || !stats.iter().all(|value| value.is_finite())
        || admission.lower_tail_quantile.to_bits() != 0.05_f32.to_bits()
        || stats.iter().any(|value| !(0.0..=1.0).contains(value))
        || admission.min_score > admission.threshold
        || admission.threshold > admission.median_score
        || admission.median_score > admission.max_score
        || decode_sha256(&admission.sample_ids_sha256, "admission sample_ids_sha256").is_err()
        || decode_sha256(
            &admission.observations_sha256,
            "admission observations_sha256",
        )
        .is_err();
    let method_invalid = if has_jurisdiction {
        admission.method != "real_query_typed_block_rrf_p05_v3"
            || admission.sample_count < 20
            || admission.sample_limit != admission.sample_count
            || admission.sample_seed != 0
            || admission.calibration_queries.is_none()
    } else {
        admission.method != "loo_typed_block_rrf_p05_v3"
            || admission.sample_count > admission.corpus_count
            || admission.sample_seed == 0
            || admission.calibration_queries.is_some()
    };
    if common_invalid || method_invalid {
        return Err(CliError::runtime(
            "kernel manifest contains an invalid admission calibration contract",
        ));
    }
    if let Some(source) = &admission.calibration_queries {
        validate_artifact(vault, source)?;
    }
    Ok(())
}

fn validate_artifact(vault: &Path, artifact: &KernelArtifactRef) -> CliResult {
    let (path, actual_bytes) = validate_artifact_metadata(vault, artifact)?;
    let sha256 = sha256_file(&path)?;
    if sha256 != artifact.sha256 {
        return Err(CliError::runtime(format!(
            "kernel artifact readback mismatch path={} expected_bytes={} actual_bytes={} expected_sha256={} actual_sha256={}",
            path.display(),
            artifact.bytes,
            actual_bytes,
            artifact.sha256,
            sha256
        )));
    }
    Ok(())
}

fn validate_artifact_metadata(
    vault: &Path,
    artifact: &KernelArtifactRef,
) -> CliResult<(PathBuf, u64)> {
    let relative = Path::new(&artifact.relative_path);
    if relative.is_absolute()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(CliError::runtime(format!(
            "kernel artifact path {:?} is not a strict relative path",
            artifact.relative_path
        )));
    }
    let path = vault.join(relative);
    let metadata = fs::metadata(&path).map_err(|error| {
        CliError::io(format!("stat kernel artifact {}: {error}", path.display()))
    })?;
    let _ = decode_sha256(&artifact.sha256, "kernel artifact sha256")?;
    if !metadata.is_file() || metadata.len() != artifact.bytes {
        return Err(CliError::runtime(format!(
            "kernel artifact metadata mismatch path={} expected_bytes={} actual_bytes={} is_file={}",
            path.display(),
            artifact.bytes,
            metadata.len(),
            metadata.is_file()
        )));
    }
    Ok((path, metadata.len()))
}

fn validate_manifest_pointer(relative: &str) -> CliResult {
    let path = Path::new(relative);
    let components = path.components().collect::<Vec<_>>();
    if path.is_absolute()
        || components.len() != 4
        || components
            .iter()
            .any(|component| !matches!(component, Component::Normal(_)))
        || !relative.starts_with(&format!("{MANIFEST_DIR}/"))
        || !relative.ends_with(".json")
    {
        return Err(CliError::runtime(format!(
            "kernel CURRENT pointer {relative:?} violates the immutable manifest contract"
        )));
    }
    Ok(())
}

fn install_immutable(path: &Path, bytes: &[u8], label: &str) -> CliResult {
    if path.exists() {
        let existing = fs::read(path)
            .map_err(|error| CliError::io(format!("read existing {label}: {error}")))?;
        if existing == bytes {
            return Ok(());
        }
        return Err(CliError::runtime(format!(
            "refusing to replace immutable {label} {}",
            path.display()
        )));
    }
    write_bytes_atomic(path, bytes, label)
}

fn relative_path(path: &Path) -> CliResult<String> {
    if path
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(CliError::runtime(format!(
            "kernel artifact has non-normal relative path {}",
            path.display()
        )));
    }
    Ok(path.to_string_lossy().replace('\\', "/"))
}

pub(crate) fn sha256_file(path: &Path) -> CliResult<String> {
    let file = fs::File::open(path)
        .map_err(|error| CliError::io(format!("open {} for sha256: {error}", path.display())))?;
    let mut reader = BufReader::with_capacity(1024 * 1024, file);
    let mut digest = Sha256::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let read = reader.read(&mut buffer).map_err(|error| {
            CliError::io(format!("read {} for sha256: {error}", path.display()))
        })?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(hex32(&digest.finalize().into()))
}

pub(crate) fn sha256_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub(crate) fn physical_graph_contract(
    metadata: &[u8],
    node_props: &[(CxId, Vec<u8>)],
    csr: &[u8],
) -> ([u8; 32], String) {
    let mut props_hasher = Sha256::new();
    props_hasher.update(b"calyx-kernel-node-props-v1");
    for (id, bytes) in node_props {
        props_hasher.update(id.as_bytes());
        props_hasher.update((bytes.len() as u64).to_be_bytes());
        props_hasher.update(bytes);
    }
    let props_hash: [u8; 32] = props_hasher.finalize().into();
    let mut contract = Sha256::new();
    contract.update(b"calyx-kernel-physical-graph-contract-v1");
    contract.update((metadata.len() as u64).to_be_bytes());
    contract.update(metadata);
    contract.update(props_hash);
    contract.update((csr.len() as u64).to_be_bytes());
    contract.update(csr);
    let contract_hash: [u8; 32] = contract.finalize().into();
    (contract_hash, hex32(&props_hash))
}

pub(crate) fn decode_sha256(raw: &str, field: &str) -> CliResult<[u8; 32]> {
    if raw.len() != 64 || !raw.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(CliError::runtime(format!(
            "{field} is not a 64-digit SHA-256"
        )));
    }
    let mut out = [0_u8; 32];
    for (index, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&raw[index * 2..index * 2 + 2], 16)
            .map_err(|error| CliError::runtime(format!("decode {field}: {error}")))?;
    }
    Ok(out)
}

pub(crate) fn ledger_hash(reference: &LedgerRef) -> String {
    reference
        .hash
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub(crate) fn hex32(bytes: &[u8; 32]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
