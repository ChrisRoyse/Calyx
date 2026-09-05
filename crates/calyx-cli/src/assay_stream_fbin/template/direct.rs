use std::collections::BTreeSet;
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};

use calyx_core::{Asymmetry, Modality, QuantPolicy};
use calyx_registry::{
    AlgorithmicLens, FrozenLensContract, LensDType, LensRuntime, LensSpec, NormPolicy,
    StructuredBinding, read_tei_service_info, tei_endpoint_identity,
};
use sha2::{Digest, Sha256};

use crate::a35_signal::lens_spec_signal_kind_name;
use crate::assay_corpus_build::lens::projection::projected_slot_dim;
use crate::error::{CliError, CliResult};
use crate::lens_commands::support::dim;

use super::super::local_error;
use super::{
    FORMAT, LensTemplateDescriptor, LensTemplateRecord, MODE, activation_status, hex_from_digest,
    roster_sha256, spec_sha256,
};

#[derive(Clone, Debug)]
pub(super) enum DirectLensSource {
    Tei {
        name: String,
        endpoint: String,
        dim: u32,
        weights_path: PathBuf,
    },
    Algorithmic {
        name: String,
        kind: String,
        dim: u32,
    },
    /// Structured-v1 algorithmic lens bound to a frozen binding JSON. The binding
    /// carries the exact schema/selector/encoder, so its runtime kind, shape, and
    /// frozen contract are derived from the binding artifact rather than a hand
    /// passed `<kind> <dim>` pair. Routes `Modality::Structured` so `structured-v1`
    /// kinds are admitted instead of rejected as text (issue #1914).
    Structured { name: String, binding_path: PathBuf },
}

pub(super) fn record_from_direct_lenses(
    sources: &[DirectLensSource],
) -> CliResult<LensTemplateRecord> {
    if sources.is_empty() {
        return Err(CliError::usage(
            "lens template direct import requires at least one --tei or --algorithmic lens",
        ));
    }
    let mut names = BTreeSet::new();
    let mut descriptors = Vec::with_capacity(sources.len());
    for (slot, source) in sources.iter().enumerate() {
        let descriptor = descriptor_from_direct_source(slot, source)?;
        if !names.insert(descriptor.name.clone()) {
            return Err(local_error(
                "CALYX_FSV_ASSAY_STREAM_FBIN_LENS_TEMPLATE_DUPLICATE",
                format!("duplicate lens {} in template import", descriptor.name),
                "deduplicate the stream-fbin lens template roster",
            ));
        }
        descriptors.push(descriptor);
    }
    let roster_sha256 = roster_sha256(&descriptors);
    Ok(LensTemplateRecord {
        format: FORMAT.to_string(),
        mode: MODE.to_string(),
        roster_sha256,
        descriptors,
    })
}

fn descriptor_from_direct_source(
    slot: usize,
    source: &DirectLensSource,
) -> CliResult<LensTemplateDescriptor> {
    let DirectDescriptorParts {
        spec,
        runtime,
        model_id,
        endpoint,
        dtype,
        source_path,
        source_sha256,
    } = direct_descriptor_parts(source)?;
    let spec_sha256 = spec_sha256(&spec)?;
    Ok(LensTemplateDescriptor {
        slot: u16::try_from(slot).map_err(|_| CliError::usage("lens template slot exceeds u16"))?,
        name: spec.name.clone(),
        lens_id: spec.lens_id().to_string(),
        weights_sha256: hex_from_digest(&spec.weights_sha256),
        signal_kind: lens_spec_signal_kind_name(&spec).to_string(),
        dim: projected_slot_dim(spec.output) as usize,
        native_dim: dim(spec.output) as usize,
        runtime,
        model_id,
        endpoint,
        dtype,
        quantization: format!("{:?}", spec.quant_default),
        max_batch: spec.max_batch,
        activation_status: activation_status(&spec.runtime).to_string(),
        admission_status: "requires_a37_db_gate".to_string(),
        source_path,
        import_manifest_sha256: source_sha256,
        spec_sha256,
        spec,
    })
}

struct DirectDescriptorParts {
    spec: LensSpec,
    runtime: String,
    model_id: String,
    endpoint: Option<String>,
    dtype: String,
    source_path: String,
    source_sha256: String,
}

fn direct_descriptor_parts(source: &DirectLensSource) -> CliResult<DirectDescriptorParts> {
    match source {
        DirectLensSource::Tei {
            name,
            endpoint,
            dim,
            weights_path,
        } => {
            validate_direct_name(name)?;
            validate_direct_dim(*dim)?;
            if !endpoint.starts_with("http://") {
                return Err(CliError::usage("--tei endpoint must be http://"));
            }
            if endpoint.contains("#rerank_query=") && *dim != 2 {
                return Err(CliError::usage(
                    "--tei reranker endpoint requires dim=2 for the frozen score-plus-bias projection",
                ));
            }
            let (physical_weights_path, weights_sha256) = physical_weights(weights_path)?;
            let service = read_tei_service_info(endpoint).map_err(|error| {
                tei_attestation_error(
                    format!("lens {name} live /info failed: {}", error.message),
                    "restore the resident TEI service before importing its DB-native lens",
                )
            })?;
            let endpoint_identity = tei_endpoint_identity(endpoint).map_err(|error| {
                tei_attestation_error(
                    format!("lens {name} endpoint identity failed: {}", error.message),
                    "fix and freeze the TEI endpoint and reranker query",
                )
            })?;
            require_revision_bound_path(&physical_weights_path, &service.model_sha, name)?;
            let corpus_hash = digest(&[b"calyx-tei-measurement-v1", endpoint.as_bytes()]);
            let contract = FrozenLensContract::new(
                name,
                weights_sha256,
                corpus_hash,
                calyx_core::SlotShape::Dense(*dim),
                Modality::Text,
                LensDType::F32,
                NormPolicy::unit(),
            );
            let mut spec = spec_from_contract(
                name,
                LensRuntime::TeiHttp {
                    endpoint: endpoint.clone(),
                },
                &contract,
            );
            spec.max_batch = Some(64);
            let model_identity = service.model_identity();
            let runtime_identity = service.runtime_identity();
            let weights_hex = hex_digest(&weights_sha256);
            let attestation_sha256 = direct_source_sha256(&[
                "tei-http-attested-v1",
                name,
                endpoint,
                &dim.to_string(),
                &physical_weights_path.display().to_string(),
                &weights_hex,
                &model_identity,
                &runtime_identity,
                &service.model_dtype,
                &endpoint_identity.endpoint_sha256,
                endpoint_identity.prompt_sha256.as_deref().unwrap_or("none"),
            ]);
            Ok(DirectDescriptorParts {
                spec,
                runtime: runtime_identity,
                model_id: model_identity,
                endpoint: Some(endpoint.clone()),
                dtype: service.model_dtype,
                source_path: physical_weights_path.display().to_string(),
                source_sha256: attestation_sha256,
            })
        }
        DirectLensSource::Algorithmic { name, kind, dim } => {
            validate_direct_name(name)?;
            validate_direct_dim(*dim)?;
            let kind = kind.replace('_', "-");
            let lens = algorithmic_lens(name, &kind, *dim)?;
            let contract = lens.contract().clone();
            let spec = spec_from_contract(
                name,
                LensRuntime::Algorithmic { kind: kind.clone() },
                &contract,
            );
            Ok(DirectDescriptorParts {
                spec,
                runtime: "algorithmic".to_string(),
                model_id: kind.clone(),
                endpoint: None,
                dtype: "f32".to_string(),
                source_path: format!("calyx-db-direct:algorithmic:{name}:{kind}"),
                source_sha256: direct_source_sha256(&[
                    "algorithmic",
                    name,
                    &kind,
                    &dim.to_string(),
                ]),
            })
        }
        DirectLensSource::Structured { name, binding_path } => {
            validate_direct_name(name)?;
            let (physical_binding_path, binding_file_sha256, bytes) =
                read_structured_binding(binding_path)?;
            let binding = StructuredBinding::from_json_bytes(&bytes).map_err(CliError::from)?;
            // The binding is authoritative: derive the canonical runtime kind and
            // the emitted slot shape from it, then build the frozen structured
            // contract exactly as `calyx lens add --modality structured` does, so a
            // template-imported lens is byte-identical to a directly registered one.
            let kind = binding.runtime_kind().map_err(CliError::from)?;
            let shape = binding.shape();
            let lens = AlgorithmicLens::from_runtime_kind(name, Modality::Structured, &kind, shape)
                .map_err(CliError::from)?;
            let contract = lens.contract().clone();
            let spec = spec_from_contract(
                name,
                LensRuntime::Algorithmic { kind: kind.clone() },
                &contract,
            );
            let binding_hex = hex_digest(&binding_file_sha256);
            Ok(DirectDescriptorParts {
                spec,
                runtime: "algorithmic".to_string(),
                model_id: format!("structured-v1:{}", binding.schema_id()),
                endpoint: None,
                dtype: "f32".to_string(),
                source_path: physical_binding_path.display().to_string(),
                source_sha256: direct_source_sha256(&[
                    "structured-v1",
                    name,
                    binding.schema_id(),
                    &binding_hex,
                ]),
            })
        }
    }
}

/// Reads and hashes a frozen structured binding JSON artifact for a direct
/// template import. Fails closed (never fabricates a binding) if the artifact is
/// missing or unreadable.
fn read_structured_binding(path: &Path) -> CliResult<(PathBuf, [u8; 32], Vec<u8>)> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|error| {
                structured_binding_error(format!(
                    "resolve current directory for {} failed: {error}",
                    path.display()
                ))
            })?
            .join(path)
    };
    let bytes = std::fs::read(&absolute).map_err(|error| {
        structured_binding_error(format!(
            "read structured binding {} failed: {error}",
            absolute.display()
        ))
    })?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    Ok((absolute, hasher.finalize().into(), bytes))
}

fn structured_binding_error(message: impl Into<String>) -> crate::error::CliError {
    local_error(
        "CALYX_FSV_ASSAY_STREAM_FBIN_STRUCTURED_BINDING_INVALID",
        message,
        "pass --structured <name> <binding.json> pointing at the frozen structured binding artifact",
    )
}

pub(super) fn validate_attested_tei_descriptor(descriptor: &LensTemplateDescriptor) -> CliResult {
    let LensRuntime::TeiHttp { endpoint } = &descriptor.spec.runtime else {
        return Ok(());
    };
    if descriptor.endpoint.as_deref() != Some(endpoint.as_str()) {
        return Err(tei_attestation_error(
            format!(
                "lens {} stored endpoint disagrees with its frozen spec",
                descriptor.name
            ),
            "re-import the TEI lens from its physical artifact and live service",
        ));
    }
    let (physical_weights_path, weights_sha256) =
        physical_weights(Path::new(&descriptor.source_path))?;
    let weights_hex = hex_digest(&weights_sha256);
    if descriptor.weights_sha256 != weights_hex || descriptor.spec.weights_sha256 != weights_sha256
    {
        return Err(tei_attestation_error(
            format!(
                "lens {} physical weights {} hash {} disagrees with DB descriptor {}",
                descriptor.name,
                physical_weights_path.display(),
                weights_hex,
                descriptor.weights_sha256
            ),
            "restore the exact physical model artifact or import it as a new frozen lens",
        ));
    }
    let service = read_tei_service_info(endpoint).map_err(|error| {
        tei_attestation_error(
            format!(
                "lens {} live /info failed: {}",
                descriptor.name, error.message
            ),
            "restore the attested TEI service and retry the gate",
        )
    })?;
    let endpoint_identity = tei_endpoint_identity(endpoint).map_err(|error| {
        tei_attestation_error(
            format!(
                "lens {} endpoint identity failed: {}",
                descriptor.name, error.message
            ),
            "fix the frozen TEI endpoint and reranker query",
        )
    })?;
    require_revision_bound_path(&physical_weights_path, &service.model_sha, &descriptor.name)?;
    let expected_model = service.model_identity();
    let expected_runtime = service.runtime_identity();
    let expected_corpus = digest(&[b"calyx-tei-measurement-v1", endpoint.as_bytes()]);
    let expected_attestation = direct_source_sha256(&[
        "tei-http-attested-v1",
        &descriptor.name,
        endpoint,
        &descriptor.native_dim.to_string(),
        &physical_weights_path.display().to_string(),
        &weights_hex,
        &expected_model,
        &expected_runtime,
        &service.model_dtype,
        &endpoint_identity.endpoint_sha256,
        endpoint_identity.prompt_sha256.as_deref().unwrap_or("none"),
    ]);
    if descriptor.model_id != expected_model
        || descriptor.runtime != expected_runtime
        || descriptor.dtype != service.model_dtype
        || descriptor.spec.corpus_hash != expected_corpus
        || descriptor.import_manifest_sha256 != expected_attestation
    {
        return Err(tei_attestation_error(
            format!(
                "lens {} TEI attestation disagrees: model={} runtime={} dtype={} endpoint_sha256={} prompt_sha256={}",
                descriptor.name,
                expected_model,
                expected_runtime,
                service.model_dtype,
                endpoint_identity.endpoint_sha256,
                endpoint_identity.prompt_sha256.as_deref().unwrap_or("none")
            ),
            "do not admit this lens; restore the attested service or import the changed runtime as a new lens",
        ));
    }
    Ok(())
}

fn physical_weights(path: &Path) -> CliResult<(PathBuf, [u8; 32])> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|error| {
                tei_attestation_error(
                    format!(
                        "resolve current directory for {} failed: {error}",
                        path.display()
                    ),
                    "pass an absolute physical model artifact path",
                )
            })?
            .join(path)
    };
    if !absolute.exists() {
        return Err(tei_attestation_error(
            format!("physical TEI weights {} do not exist", absolute.display()),
            "pass --tei <name> <endpoint> <dim> <physical-model-artifact>",
        ));
    }
    let file = File::open(&absolute).map_err(|error| {
        tei_attestation_error(
            format!(
                "open physical TEI weights {} failed: {error}",
                absolute.display()
            ),
            "restore the physical model artifact before importing the lens",
        )
    })?;
    let mut reader = BufReader::with_capacity(1024 * 1024, file);
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let read = reader.read(&mut buffer).map_err(|error| {
            tei_attestation_error(
                format!(
                    "hash physical TEI weights {} failed: {error}",
                    absolute.display()
                ),
                "restore the physical model artifact before importing the lens",
            )
        })?;
        if read == 0 {
            return Ok((absolute, hasher.finalize().into()));
        }
        hasher.update(&buffer[..read]);
    }
}

fn require_revision_bound_path(path: &Path, revision: &str, lens_name: &str) -> CliResult {
    if path
        .components()
        .any(|component| component.as_os_str().to_string_lossy() == revision)
    {
        return Ok(());
    }
    Err(tei_attestation_error(
        format!(
            "lens {lens_name} physical artifact path {} does not bind live model revision {revision}",
            path.display()
        ),
        "use the immutable model artifact under its exact snapshot/revision directory",
    ))
}

fn digest(parts: &[&[u8]]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update((part.len() as u64).to_be_bytes());
        hasher.update(part);
    }
    hasher.finalize().into()
}

fn hex_digest(digest: &[u8; 32]) -> String {
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn tei_attestation_error(
    message: impl Into<String>,
    remediation: &'static str,
) -> crate::error::CliError {
    local_error(
        "CALYX_FSV_ASSAY_STREAM_FBIN_TEI_ATTESTATION_MISMATCH",
        message,
        remediation,
    )
}

fn algorithmic_lens(name: &str, kind: &str, dim: u32) -> CliResult<AlgorithmicLens> {
    AlgorithmicLens::from_runtime_kind_dim(name, Modality::Text, kind, dim).map_err(CliError::from)
}

fn spec_from_contract(name: &str, runtime: LensRuntime, contract: &FrozenLensContract) -> LensSpec {
    LensSpec {
        name: name.to_string(),
        runtime,
        output: contract.shape(),
        modality: contract.modality(),
        weights_sha256: contract.weights_sha256(),
        corpus_hash: contract.corpus_hash(),
        norm_policy: contract.norm_policy(),
        max_batch: None,
        axis: Some(name.to_string()),
        asymmetry: Asymmetry::None,
        quant_default: QuantPolicy::turboquant_default(),
        truncate_dim: None,
        recall_delta: calyx_registry::spec::default_recall_delta(),
        retrieval_only: false,
        excluded_from_dedup: false,
    }
}

fn validate_direct_name(name: &str) -> CliResult {
    if name.is_empty() || name.chars().any(char::is_whitespace) || name.contains(['/', '\\']) {
        return Err(CliError::usage(
            "direct lens name must be non-empty and path-safe",
        ));
    }
    Ok(())
}

fn validate_direct_dim(dim: u32) -> CliResult {
    if dim == 0 {
        return Err(CliError::usage("direct lens dimension must be > 0"));
    }
    Ok(())
}

fn direct_source_sha256(parts: &[&str]) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update((part.len() as u64).to_be_bytes());
        hasher.update(part.as_bytes());
    }
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("{name}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_numeric_binding(dir: &Path) -> PathBuf {
        // Roster lane 5: /citation_count Log1pSigned -> Dense(1).
        let json = serde_json::json!({
            "schema_id": "courtlistener-opinion-v1",
            "selector": {"kind": "field", "path": "/citation_count"},
            "source_type": "integer",
            "encoder": {"kind": "numeric", "scaling": {"kind": "log1p_signed"}}
        });
        let path = dir.join("citation_binding.json");
        std::fs::write(&path, serde_json::to_vec(&json).unwrap()).unwrap();
        path
    }

    #[test]
    fn structured_direct_source_routes_modality_structured() {
        let dir = temp_dir("stream-fbin-direct-structured");
        let binding_path = write_numeric_binding(&dir);

        let record = record_from_direct_lenses(&[DirectLensSource::Structured {
            name: "citation-count-v1".to_string(),
            binding_path,
        }])
        .expect("structured direct source must build a template descriptor");

        assert_eq!(record.descriptors.len(), 1);
        let descriptor = &record.descriptors[0];
        assert_eq!(descriptor.name, "citation-count-v1");
        assert_eq!(descriptor.runtime, "algorithmic");
        assert_eq!(descriptor.spec.modality, Modality::Structured);
        assert_eq!(descriptor.native_dim, 1);
        match &descriptor.spec.runtime {
            LensRuntime::Algorithmic { kind } => {
                assert!(
                    kind.starts_with("structured-v1:"),
                    "structured direct source must freeze a structured-v1 runtime kind, got {kind}"
                );
            }
            other => panic!("expected an algorithmic structured runtime, got {other:?}"),
        }
        assert!(descriptor.model_id.contains("courtlistener-opinion-v1"));

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn structured_direct_source_missing_binding_fails_closed() {
        let dir = temp_dir("stream-fbin-direct-structured-missing");
        let error = record_from_direct_lenses(&[DirectLensSource::Structured {
            name: "citation-count-v1".to_string(),
            binding_path: dir.join("does-not-exist.json"),
        }])
        .expect_err("a missing structured binding artifact must fail closed");
        assert!(
            error
                .to_string()
                .contains("CALYX_FSV_ASSAY_STREAM_FBIN_STRUCTURED_BINDING_INVALID"),
            "expected structured binding invalid code, got: {error}"
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
