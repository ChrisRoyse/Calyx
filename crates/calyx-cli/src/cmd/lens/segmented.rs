use calyx_core::{Modality, SlotShape};
use calyx_registry::{
    DEFAULT_TEI_SEGMENTATION_POLICY, LensRuntime, TeiSegmentedLens, pooled_policy_for_endpoint,
};

use super::*;

pub(super) fn build_segmented_tei_lens(
    name: &str,
    endpoint: Option<&str>,
    shape: Option<&str>,
    modality: Modality,
) -> CliResult<BuiltLens> {
    let output = shape
        .map(parse_shape)
        .transpose()?
        .unwrap_or(SlotShape::Multi { token_dim: 768 });
    let dim = match output {
        SlotShape::Multi { token_dim } => token_dim,
        other => {
            return Err(CliError::usage(format!(
                "tei-http-segmented requires Multi(<token_dim>), got {other:?}"
            )));
        }
    };
    let endpoint = endpoint.unwrap_or(calyx_registry::DEFAULT_TEI_ENDPOINT);
    let lens = TeiSegmentedLens::new(
        name,
        endpoint,
        modality,
        dim,
        DEFAULT_TEI_SEGMENTATION_POLICY,
    )?;
    let contract = lens.contract(name);
    let spec = spec_from_contract(
        name,
        LensRuntime::TeiHttpSegmented {
            endpoint: endpoint.to_string(),
            policy: DEFAULT_TEI_SEGMENTATION_POLICY,
        },
        &contract,
    );
    Ok(BuiltLens {
        lens_id: contract.lens_id(),
        spec,
        runtime: BuiltRuntime::TeiSegmented(lens, contract),
    })
}

/// Builds a pooled, paragraph-aware segmented TEI lane (#1980).
///
/// The per-segment window is read from the live endpoint's own
/// `max_input_length`, so each lane packs to its own context limit. The lane
/// emits one dense vector per document and therefore declares a dense shape.
pub(super) fn build_pooled_segmented_tei_lens(
    name: &str,
    endpoint: Option<&str>,
    shape: Option<&str>,
    modality: Modality,
) -> CliResult<BuiltLens> {
    let output = shape
        .map(parse_shape)
        .transpose()?
        .unwrap_or(SlotShape::Dense(768));
    let dim = match output {
        SlotShape::Dense(dim) => dim,
        other => {
            return Err(CliError::usage(format!(
                "tei-http-segmented-pooled emits one vector per document and requires Dense(<dim>), got {other:?}"
            )));
        }
    };
    let endpoint = endpoint.unwrap_or(calyx_registry::DEFAULT_TEI_ENDPOINT);
    let policy = pooled_policy_for_endpoint(endpoint)?;
    let lens = TeiSegmentedLens::new(name, endpoint, modality, dim, policy)?;
    let contract = lens.contract(name);
    let spec = spec_from_contract(
        name,
        LensRuntime::TeiHttpSegmented {
            endpoint: endpoint.to_string(),
            policy,
        },
        &contract,
    );
    Ok(BuiltLens {
        lens_id: contract.lens_id(),
        spec,
        runtime: BuiltRuntime::TeiSegmented(lens, contract),
    })
}
