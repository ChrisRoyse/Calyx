use std::collections::BTreeSet;

use calyx_core::{Asymmetry, Modality, QuantPolicy};
use calyx_registry::{AlgorithmicLens, FrozenLensContract, LensRuntime, LensSpec};
use sha2::{Digest, Sha256};

use crate::a35_signal::lens_spec_signal_kind_name;
use crate::assay_corpus_build::lens::projection::projected_slot_dim;
use crate::error::{CliError, CliResult};
use crate::lens_commands::support::dim;

use super::super::local_error;
use super::{
    FORMAT, LensTemplateDescriptor, LensTemplateRecord, MODE, activation_status, hex_sha256,
    roster_sha256, spec_sha256,
};

#[derive(Clone, Debug)]
pub(super) enum DirectLensSource {
    Tei {
        name: String,
        endpoint: String,
        dim: u32,
    },
    Algorithmic {
        name: String,
        kind: String,
        dim: u32,
    },
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
        weights_sha256: hex_sha256(&spec.weights_sha256),
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
        } => {
            validate_direct_name(name)?;
            validate_direct_dim(*dim)?;
            if !endpoint.starts_with("http://") {
                return Err(CliError::usage("--tei endpoint must be http://"));
            }
            let contract = FrozenLensContract::tei_http(name, endpoint, Modality::Text, *dim);
            let mut spec = spec_from_contract(
                name,
                LensRuntime::TeiHttp {
                    endpoint: endpoint.clone(),
                },
                &contract,
            );
            spec.max_batch = Some(64);
            Ok(DirectDescriptorParts {
                spec,
                runtime: "tei_http".to_string(),
                model_id: name.clone(),
                endpoint: Some(endpoint.clone()),
                dtype: "float16".to_string(),
                source_path: format!("calyx-db-direct:tei-http:{name}"),
                source_sha256: direct_source_sha256(&[
                    "tei-http",
                    name,
                    endpoint,
                    &dim.to_string(),
                ]),
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
    }
}

fn algorithmic_lens(name: &str, kind: &str, dim: u32) -> CliResult<AlgorithmicLens> {
    let lens = match kind {
        "byte" | "byte-features" => fixed_algorithmic_dim(
            kind,
            dim,
            16,
            AlgorithmicLens::byte_features(name, Modality::Text),
        )?,
        "scalar" => {
            fixed_algorithmic_dim(kind, dim, 1, AlgorithmicLens::scalar(name, Modality::Text))?
        }
        "ast-style" => fixed_algorithmic_dim(
            kind,
            dim,
            8,
            AlgorithmicLens::ast_style(name, Modality::Text),
        )?,
        "gdelt-cameo" => fixed_algorithmic_dim(
            kind,
            dim,
            16,
            AlgorithmicLens::gdelt_cameo(name, Modality::Text),
        )?,
        "gdelt-actor-geo" => AlgorithmicLens::gdelt_actor_geo(name, Modality::Text, dim),
        "gdelt-source-domain" => AlgorithmicLens::gdelt_source_domain(name, Modality::Text, dim),
        "gdelt-event-geo" => AlgorithmicLens::gdelt_event_geo(name, Modality::Text, dim),
        "gdelt-actor-pair" => AlgorithmicLens::gdelt_actor_pair(name, Modality::Text, dim),
        "gdelt-event-actor" => AlgorithmicLens::gdelt_event_actor(name, Modality::Text, dim),
        "gdelt-tone-signal" => AlgorithmicLens::gdelt_tone_signal(name, Modality::Text, dim),
        "gdelt-source-event" => AlgorithmicLens::gdelt_source_event(name, Modality::Text, dim),
        "sparse" | "sparse-keywords" => AlgorithmicLens::sparse_keywords(name, Modality::Text, dim),
        "token-hash" | "multi-hash" => AlgorithmicLens::token_hash(name, Modality::Text, dim),
        "one-hot" => AlgorithmicLens::one_hot(name, Modality::Text, dim),
        other => {
            return Err(CliError::usage(format!(
                "unknown algorithmic runtime kind {other}"
            )));
        }
    };
    Ok(lens)
}

fn fixed_algorithmic_dim(
    kind: &str,
    actual: u32,
    expected: u32,
    lens: AlgorithmicLens,
) -> CliResult<AlgorithmicLens> {
    if actual != expected {
        return Err(CliError::usage(format!(
            "{kind} direct lens dimension must be {expected}, got {actual}"
        )));
    }
    Ok(lens)
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
