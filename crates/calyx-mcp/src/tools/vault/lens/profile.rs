use calyx_core::{CalyxError, Input, LensId, Modality, StructuredRecord};
use calyx_registry::{LensRuntime, ProfileProbe, Registry, StructuredBinding};
use serde_json::Value;

use crate::server::{ToolError, ToolResult};

pub(super) fn profile_probes(
    path: Option<&str>,
    registry: &Registry,
    lens_id: LensId,
) -> ToolResult<Vec<ProfileProbe>> {
    let spec = registry
        .lens_spec(lens_id)
        .ok_or_else(|| CalyxError::registry_unavailable(format!("lens {lens_id} missing spec")))?;
    let path = path
        .ok_or_else(|| ToolError::invalid_params("profile_lens requires an explicit probe set"))?;
    let text = std::fs::read_to_string(path)
        .map_err(|error| CalyxError::lens_unreachable(format!("read probe set failed: {error}")))?;
    let binding = match &spec.runtime {
        LensRuntime::Algorithmic { kind } if StructuredBinding::is_runtime_kind(kind) => {
            Some(StructuredBinding::from_runtime_kind(kind)?)
        }
        _ => None,
    };
    let mut probes = Vec::new();
    for line in text.lines().map(str::trim).filter(|line| !line.is_empty()) {
        probes.push(match &binding {
            Some(binding) => structured_probe(line, binding)?,
            None => profile_probe_from_line(line, spec.modality)?,
        });
    }
    if probes.is_empty() {
        Err(ToolError::invalid_params(
            "profile_lens probe set must not be empty",
        ))
    } else {
        Ok(probes)
    }
}

pub(super) fn profile_probe_from_line(line: &str, modality: Modality) -> ToolResult<ProfileProbe> {
    match serde_json::from_str::<Value>(line) {
        Ok(Value::String(text)) => Ok(ProfileProbe::new(Input::new(modality, text.into_bytes()))),
        Ok(Value::Object(map)) => {
            let text = map
                .get("input")
                .or_else(|| map.get("text"))
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    ToolError::invalid_params("profile probe object requires string input or text")
                })?;
            let label = map
                .get("label")
                .and_then(Value::as_str)
                .map(ToString::to_string);
            Ok(ProfileProbe {
                input: Input::new(modality, text.as_bytes().to_vec()),
                label,
            })
        }
        Ok(_) => Err(ToolError::invalid_params(
            "profile probe JSONL must be a string or object",
        )),
        Err(error) if starts_like_json(line) => Err(ToolError::invalid_params(format!(
            "parse profile probe JSONL: {error}"
        ))),
        Err(_) => Ok(ProfileProbe::new(Input::new(
            modality,
            line.as_bytes().to_vec(),
        ))),
    }
}

fn structured_probe(line: &str, binding: &StructuredBinding) -> ToolResult<ProfileProbe> {
    let value = serde_json::from_str(line).map_err(|error| CalyxError {
        code: calyx_core::CALYX_STRUCTURED_RECORD_INVALID,
        message: format!("parse structured profile JSONL: {error}"),
        remediation: "provide one complete typed JSON object per probe line",
    })?;
    let record = StructuredRecord::from_json_value(binding.schema_id(), &value)?;
    Ok(ProfileProbe::new(
        Input::new(Modality::Structured, Vec::new()).with_structured(record),
    ))
}

fn starts_like_json(line: &str) -> bool {
    matches!(
        line.as_bytes().first(),
        Some(b'{') | Some(b'[') | Some(b'"')
    )
}
