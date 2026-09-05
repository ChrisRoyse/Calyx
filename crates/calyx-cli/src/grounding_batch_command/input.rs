use std::fs::{Metadata, OpenOptions};
use std::io::{Read, Seek, SeekFrom};
#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::Path;

use calyx_aster::vault::GroundingBatchEvent;
use calyx_core::{Anchor, AnchorKind, AnchorValue, CxId};
use serde::Deserialize;
use serde_json::Value;
use serde_json_canonicalizer::to_vec as jcs_to_vec;
use sha2::{Digest, Sha256};

const SCHEMA: &str = "calyx.grounding_batch.v1";
const MAX_INPUT_BYTES: u64 = 256 * 1024 * 1024;
const MAX_LABEL_BYTES: usize = 192;
const EVENT_SCHEMA: &str = "kernelweights.m3.grounding_event.v5";
const BINDING_CANONICALIZATION: &str = "RFC8785";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Plan {
    schema: String,
    events: Vec<EventSpec>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EventSpec {
    event_id: String,
    targets: Vec<String>,
    anchors: Vec<Value>,
    payload: Value,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AnchorSpec {
    kind: String,
    value: Value,
    source: String,
    observed_at: u64,
    confidence: f32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PayloadSpec {
    schema: String,
    binding_canonicalization: String,
    supersedes_event_id: String,
    campaign: String,
    family: String,
    index: usize,
    receipt_sha256: String,
    registry_sha256: String,
    node_map_sha256: String,
    archive_sha256: String,
    ordered_shards_sha256: String,
    intervention_sha256: String,
    selection_sha256: String,
    protocol_sha256: String,
    panel_sha256: String,
    model_sha256: String,
    engine: String,
    intervention_id_sha256: String,
    intervention_kind: String,
    schedule_sha256: Option<String>,
    schedule_entry_sha256: Option<String>,
    targets: usize,
    targets_sha256: String,
    anchors: usize,
    anchors_sha256: String,
    observed_at_unix_ms: u64,
}

pub(super) fn read_plan(path: &Path) -> Result<(Vec<GroundingBatchEvent>, String, u64), String> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    let mut input = options
        .open(path)
        .map_err(|error| format!("open physical input {}: {error}", path.display()))?;
    let metadata = input
        .metadata()
        .map_err(|error| format!("inspect open input {}: {error}", path.display()))?;
    if !metadata.file_type().is_file() {
        return Err(format!(
            "input is not a physical regular file: {}",
            path.display()
        ));
    }
    #[cfg(unix)]
    {
        if metadata.nlink() != 1 || metadata.mode() & 0o022 != 0 {
            return Err(format!(
                "input must be single-link and not group/world writable: {} nlink={} mode={:o}",
                path.display(),
                metadata.nlink(),
                metadata.mode() & 0o7777
            ));
        }
    }
    if metadata.len() == 0 || metadata.len() > MAX_INPUT_BYTES {
        return Err(format!(
            "input bytes={} outside 1..={MAX_INPUT_BYTES}",
            metadata.len()
        ));
    }
    let mut raw = Vec::with_capacity(metadata.len() as usize);
    input
        .read_to_end(&mut raw)
        .map_err(|error| format!("read open input {}: {error}", path.display()))?;
    if raw.len() as u64 != metadata.len() {
        return Err(format!(
            "input length changed during first read: {} metadata={} read={}",
            path.display(),
            metadata.len(),
            raw.len()
        ));
    }
    let input_sha256 = hex_sha256(&raw);
    let plan: Plan = serde_json::from_slice(&raw)
        .map_err(|error| format!("parse input {}: {error}", path.display()))?;
    if plan.schema != SCHEMA {
        return Err(format!(
            "input schema {:?}, expected {SCHEMA:?}",
            plan.schema
        ));
    }
    let events = plan
        .events
        .into_iter()
        .enumerate()
        .map(|(index, event)| parse_event(index, event))
        .collect::<Result<Vec<_>, _>>()?;
    input
        .seek(SeekFrom::Start(0))
        .map_err(|error| format!("rewind open input {}: {error}", path.display()))?;
    let mut readback = Vec::with_capacity(raw.len());
    input
        .read_to_end(&mut readback)
        .map_err(|error| format!("read back open input {}: {error}", path.display()))?;
    if readback != raw {
        return Err(format!(
            "input changed during preflight: {}",
            path.display()
        ));
    }
    let final_metadata = input
        .metadata()
        .map_err(|error| format!("reinspect open input {}: {error}", path.display()))?;
    if physical_identity_changed(&metadata, &final_metadata) {
        return Err(format!(
            "input identity changed during preflight: {}",
            path.display()
        ));
    }
    Ok((events, input_sha256, metadata.len()))
}

fn physical_identity_changed(before: &Metadata, after: &Metadata) -> bool {
    if before.len() != after.len() || !after.file_type().is_file() {
        return true;
    }
    #[cfg(unix)]
    {
        before.dev() != after.dev()
            || before.ino() != after.ino()
            || before.nlink() != after.nlink()
            || before.mode() != after.mode()
            || before.mtime() != after.mtime()
            || before.mtime_nsec() != after.mtime_nsec()
            || before.ctime() != after.ctime()
            || before.ctime_nsec() != after.ctime_nsec()
    }
    #[cfg(not(unix))]
    {
        false
    }
}

fn parse_event(index: usize, event: EventSpec) -> Result<GroundingBatchEvent, String> {
    let payload = jcs_to_vec(&event.payload)
        .map_err(|error| format!("event {index} payload JCS serialization: {error}"))?;
    let event_id = decode_sha256(&event.event_id, &format!("event {index} event_id"))?;
    if Sha256::digest(&payload).as_slice() != event_id {
        return Err(format!("event {index} event_id is not SHA-256(payload)"));
    }
    let payload_spec = serde_json::from_value::<PayloadSpec>(event.payload.clone())
        .map_err(|error| format!("event {index} payload schema: {error}"))?;
    let anchor_specs = event
        .anchors
        .iter()
        .enumerate()
        .map(|(anchor_index, anchor)| {
            serde_json::from_value::<AnchorSpec>(anchor.clone())
                .map_err(|error| format!("event {index} anchor {anchor_index} schema: {error}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    validate_payload(
        index,
        &payload_spec,
        &event.targets,
        &event.anchors,
        &anchor_specs,
    )?;
    let targets = event
        .targets
        .into_iter()
        .map(|value| {
            value
                .parse::<CxId>()
                .map_err(|error| format!("event {index} target {value:?}: {error}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let anchors = anchor_specs
        .into_iter()
        .map(|anchor| parse_anchor(index, anchor))
        .collect::<Result<Vec<_>, _>>()?;
    let supersedes_event_id = decode_sha256(
        &payload_spec.supersedes_event_id,
        &format!("event {index} payload supersedes_event_id"),
    )?;
    Ok(GroundingBatchEvent {
        event_id,
        supersedes_event_id: Some(supersedes_event_id),
        targets,
        anchors,
        payload,
    })
}

fn validate_payload(
    index: usize,
    payload: &PayloadSpec,
    targets: &[String],
    anchor_values: &[Value],
    anchors: &[AnchorSpec],
) -> Result<(), String> {
    if payload.schema != EVENT_SCHEMA
        || payload.binding_canonicalization != BINDING_CANONICALIZATION
        || payload.campaign.is_empty()
        || !matches!(payload.family.as_str(), "experts" | "pairs" | "neurons")
        || payload.engine.is_empty()
    {
        return Err(format!("event {index} payload identity fields are invalid"));
    }
    for (label, value) in [
        ("archive_sha256", &payload.archive_sha256),
        ("receipt_sha256", &payload.receipt_sha256),
        ("registry_sha256", &payload.registry_sha256),
        ("node_map_sha256", &payload.node_map_sha256),
        ("ordered_shards_sha256", &payload.ordered_shards_sha256),
        ("intervention_sha256", &payload.intervention_sha256),
        ("intervention_id_sha256", &payload.intervention_id_sha256),
        ("selection_sha256", &payload.selection_sha256),
        ("protocol_sha256", &payload.protocol_sha256),
        ("panel_sha256", &payload.panel_sha256),
        ("model_sha256", &payload.model_sha256),
        ("targets_sha256", &payload.targets_sha256),
        ("anchors_sha256", &payload.anchors_sha256),
    ] {
        decode_sha256(value, &format!("event {index} payload {label}"))?;
    }
    let target_bytes = jcs_to_vec(&targets)
        .map_err(|error| format!("event {index} target JCS serialization: {error}"))?;
    let anchor_bytes = jcs_to_vec(&anchor_values)
        .map_err(|error| format!("event {index} anchor JCS serialization: {error}"))?;
    let observed_targets_sha256 = hex_sha256(&target_bytes);
    let observed_anchors_sha256 = hex_sha256(&anchor_bytes);
    let mut binding_errors = Vec::new();
    if payload.targets != targets.len() {
        binding_errors.push(format!(
            "targets count declared={} observed={}",
            payload.targets,
            targets.len()
        ));
    }
    if payload.targets_sha256 != observed_targets_sha256 {
        binding_errors.push(format!(
            "targets_sha256 declared={} observed={observed_targets_sha256}",
            payload.targets_sha256
        ));
    }
    if payload.anchors != anchors.len() {
        binding_errors.push(format!(
            "anchors count declared={} observed={}",
            payload.anchors,
            anchors.len()
        ));
    }
    if payload.anchors_sha256 != observed_anchors_sha256 {
        binding_errors.push(format!(
            "anchors_sha256 declared={} observed={observed_anchors_sha256}",
            payload.anchors_sha256
        ));
    }
    if payload.observed_at_unix_ms == 0 {
        binding_errors.push("observed_at_unix_ms must be nonzero".to_owned());
    }
    for (anchor_index, anchor) in anchors.iter().enumerate() {
        decode_sha256(
            &anchor.source,
            &format!("event {index} anchor {anchor_index} source"),
        )?;
        if anchor.source != payload.archive_sha256 {
            binding_errors.push(format!(
                "anchor {anchor_index} source declared={:?} expected={:?}",
                anchor.source, payload.archive_sha256
            ));
        }
        if anchor.observed_at != payload.observed_at_unix_ms {
            binding_errors.push(format!(
                "anchor {anchor_index} observed_at declared={} expected={}",
                anchor.observed_at, payload.observed_at_unix_ms
            ));
        }
    }
    if !binding_errors.is_empty() {
        return Err(format!(
            "event {index} payload binding failed: {}",
            binding_errors.join("; ")
        ));
    }
    let family_limit = match payload.family.as_str() {
        "experts" => 768,
        "pairs" => 1_152,
        "neurons" => 6_912,
        _ => unreachable!("family checked above"),
    };
    if payload.index >= family_limit {
        return Err(format!(
            "event {index} payload family index {} outside 0..{family_limit}",
            payload.index
        ));
    }
    let expected_kind = match payload.family.as_str() {
        "experts" => "expert",
        "pairs" => "expert_pair",
        "neurons" => "neuron_group",
        _ => unreachable!("family checked above"),
    };
    let expected_scope = match payload.family.as_str() {
        "experts" => "expert",
        "pairs" => "pair",
        "neurons" => "neuron_group",
        _ => unreachable!("family checked above"),
    };
    let anchor_prefix = format!("label:m3.scope_{expected_scope}.{:04}.", payload.index);
    if payload.intervention_kind != expected_kind {
        return Err(format!(
            "event {index} payload intervention identity conflicts with family"
        ));
    }
    if anchors
        .iter()
        .any(|anchor| !anchor.kind.starts_with(&anchor_prefix))
    {
        return Err(format!(
            "event {index} anchor scope conflicts with payload family/index"
        ));
    }
    match payload.family.as_str() {
        "experts"
            if payload.schedule_sha256.is_none() && payload.schedule_entry_sha256.is_none() => {}
        "pairs" if payload.schedule_sha256.is_some() && payload.schedule_entry_sha256.is_none() => {
        }
        "neurons"
            if payload.schedule_sha256.is_some()
                && payload
                    .schedule_entry_sha256
                    .as_ref()
                    .is_some_and(|digest| !digest.is_empty()) => {}
        _ => {
            return Err(format!(
                "event {index} payload schedule identity conflicts with family"
            ));
        }
    }
    if let Some(schedule_sha256) = &payload.schedule_sha256 {
        decode_sha256(
            schedule_sha256,
            &format!("event {index} payload schedule_sha256"),
        )?;
    }
    if let Some(schedule_entry_sha256) = &payload.schedule_entry_sha256 {
        decode_sha256(
            schedule_entry_sha256,
            &format!("event {index} payload schedule_entry_sha256"),
        )?;
    }
    Ok(())
}

fn parse_anchor(index: usize, spec: AnchorSpec) -> Result<Anchor, String> {
    let label = spec
        .kind
        .strip_prefix("label:")
        .ok_or_else(|| format!("event {index} anchor kind must use an explicit label: scope"))?;
    if label.is_empty()
        || label.len() > MAX_LABEL_BYTES
        || !label.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"._-".contains(&byte)
        })
    {
        return Err(format!(
            "event {index} anchor label must be 1..={MAX_LABEL_BYTES} lowercase ASCII [a-z0-9._-]"
        ));
    }
    let value = match spec.value {
        Value::Bool(value) => AnchorValue::Bool(value),
        Value::Number(value) => value
            .as_f64()
            .filter(|value| value.is_finite())
            .map(AnchorValue::Number)
            .ok_or_else(|| format!("event {index} anchor number is not finite f64"))?,
        Value::String(value) if !value.is_empty() => AnchorValue::Enum(value),
        _ => {
            return Err(format!(
                "event {index} anchor value must be bool, finite number, or nonempty string"
            ));
        }
    };
    decode_sha256(&spec.source, &format!("event {index} anchor source"))?;
    if spec.observed_at == 0 {
        return Err(format!("event {index} anchor observed_at must be positive"));
    }
    if !spec.confidence.is_finite() || !(0.0..=1.0).contains(&spec.confidence) {
        return Err(format!("event {index} anchor confidence must be in [0,1]"));
    }
    Ok(Anchor {
        kind: AnchorKind::Label(label.to_string()),
        value,
        source: spec.source,
        observed_at: spec.observed_at,
        confidence: spec.confidence,
    })
}

fn decode_sha256(value: &str, label: &str) -> Result<[u8; 32], String> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(format!("{label} must be 64 lowercase hex characters"));
    }
    let mut out = [0u8; 32];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        out[index] = u8::from_str_radix(
            std::str::from_utf8(pair).map_err(|error| format!("{label}: {error}"))?,
            16,
        )
        .map_err(|error| format!("{label}: {error}"))?;
    }
    Ok(out)
}

pub(super) fn hex_sha256(bytes: &[u8]) -> String {
    encode_hex(&Sha256::digest(bytes))
}

pub(super) fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}
