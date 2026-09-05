use calyx_core::{CalyxError, Modality};
use calyx_registry::{AlgorithmicLens, LensRuntime, StructuredBinding};

use super::*;

pub(super) fn build_structured_lens(
    name: &str,
    binding_path: Option<&str>,
    shape: Option<&str>,
    modality: Modality,
) -> ToolResult<BuiltLens> {
    if modality != Modality::Structured {
        return Err(CalyxError {
            code: calyx_registry::CALYX_STRUCTURED_BINDING_INVALID,
            message: format!(
                "structured algorithmic lens requires modality Structured, got {modality:?}"
            ),
            remediation: "use modality=structured or omit modality, then register a new LensId",
        }
        .into());
    }
    let path = binding_path.ok_or_else(|| CalyxError {
        code: calyx_registry::CALYX_STRUCTURED_BINDING_INVALID,
        message: "algorithmic:structured requires weights=<binding.json>".to_string(),
        remediation: "provide the frozen structured binding JSON path and retry",
    })?;
    let bytes = std::fs::read(path).map_err(|error| {
        CalyxError::lens_unreachable(format!("read structured binding {path}: {error}"))
    })?;
    let binding = StructuredBinding::from_json_bytes(&bytes)?;
    let requested = shape.map(parse_shape).transpose()?;
    if requested.is_some_and(|shape| shape != binding.shape()) {
        return Err(CalyxError::lens_dim_mismatch(format!(
            "structured binding emits {:?}, but requested shape is {requested:?}",
            binding.shape()
        ))
        .into());
    }
    let kind = binding.runtime_kind()?;
    let lens = AlgorithmicLens::from_runtime_kind(name, modality, &kind, binding.shape())?;
    let contract = lens.contract().clone();
    let spec = spec_from_contract(name, LensRuntime::Algorithmic { kind }, &contract);
    Ok(BuiltLens {
        lens_id: contract.lens_id(),
        spec,
        runtime: BuiltRuntime::Algorithmic(lens, contract),
    })
}
