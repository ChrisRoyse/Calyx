use std::path::Path;

use calyx_core::{CALYX_STRUCTURED_FIELD_MISSING, CalyxError, Input, Modality, StructuredRecord};
use calyx_registry::{LensRuntime, ProfileProbe, StructuredBinding};

use super::*;

pub(super) fn build_structured_lens(
    name: &str,
    binding_path: Option<&Path>,
    shape: Option<&str>,
    modality: Modality,
) -> CliResult<BuiltLens> {
    if modality != Modality::Structured {
        return Err(CalyxError {
            code: calyx_registry::CALYX_STRUCTURED_BINDING_INVALID,
            message: format!(
                "structured algorithmic lens requires modality Structured, got {modality:?}"
            ),
            remediation: "use --modality structured or omit --modality, then register a new LensId",
        }
        .into());
    }
    let path = binding_path.ok_or_else(|| CalyxError {
        code: calyx_registry::CALYX_STRUCTURED_BINDING_INVALID,
        message: "algorithmic:structured requires --weights <binding.json>".to_string(),
        remediation: "provide the frozen structured binding JSON as --weights and retry",
    })?;
    let binding = StructuredBinding::from_json_bytes(&std::fs::read(path)?)?;
    let declared = shape
        .map(parse_shape)
        .transpose()?
        .unwrap_or_else(|| binding.shape());
    if declared != binding.shape() {
        return Err(CalyxError::lens_dim_mismatch(format!(
            "structured binding emits {:?}, but --shape declares {declared:?}",
            binding.shape()
        ))
        .into());
    }
    let kind = binding.runtime_kind()?;
    let lens = AlgorithmicLens::from_runtime_kind(name, modality, &kind, declared)?;
    let contract = lens.contract().clone();
    let spec = spec_from_contract(name, LensRuntime::Algorithmic { kind }, &contract);
    Ok(BuiltLens {
        lens_id: contract.lens_id(),
        spec,
        runtime: BuiltRuntime::Algorithmic(lens, contract),
    })
}

pub(super) fn structured_profile_probes(
    path: Option<&Path>,
    runtime_kind: &str,
) -> CliResult<Vec<ProfileProbe>> {
    let path = path.ok_or_else(|| CalyxError {
        code: CALYX_STRUCTURED_FIELD_MISSING,
        message: "structured profile requires --probe <records.jsonl>".to_string(),
        remediation: "provide one canonical JSON object per probe line and retry",
    })?;
    let binding = StructuredBinding::from_runtime_kind(runtime_kind)?;
    let raw = std::fs::read_to_string(path)?;
    let mut probes = Vec::new();
    for (index, line) in raw.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let value = serde_json::from_str(line).map_err(|error| CalyxError {
            code: calyx_core::CALYX_STRUCTURED_RECORD_INVALID,
            message: format!(
                "structured profile line {} is invalid JSON: {error}",
                index + 1
            ),
            remediation: "provide one complete canonical JSON object per probe line",
        })?;
        let record = StructuredRecord::from_json_value(binding.schema_id(), &value)?;
        probes.push(ProfileProbe::new(
            Input::new(Modality::Structured, Vec::new()).with_structured(record),
        ));
    }
    if probes.is_empty() {
        return Err(CalyxError {
            code: CALYX_STRUCTURED_FIELD_MISSING,
            message: "structured profile probe set contains no records".to_string(),
            remediation: "provide at least one canonical JSON object and retry",
        }
        .into());
    }
    Ok(probes)
}
