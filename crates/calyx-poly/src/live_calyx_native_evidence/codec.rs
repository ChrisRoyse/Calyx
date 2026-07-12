use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::*;

const STRING_CHUNK_CHARS: usize = 32;
const CALIBRATION_FIELD: &str = "calibration";
const VERSION_FIELD: &str = "version";
const VERSION_CHUNKS_FIELD: &str = "version_chunks";
const PREVIOUS_VERSION_FIELD: &str = "previous_version";
const PREVIOUS_VERSION_CHUNKS_FIELD: &str = "previous_version_chunks";
const PANEL_FIELD: &str = "panel";
const ASSAY_CARD_FIELD: &str = "assay_card";
const A37_DIVERSITY_FIELD: &str = "a37_diversity";
const TEMPORAL_LANE_ROLE_FIELD: &str = "temporal_lane_role";
const TEMPORAL_LANE_ROLE_CHUNKS_FIELD: &str = "temporal_lane_role_chunks";
const REDUNDANCY_METHOD_FIELD: &str = "redundancy_method";
const METRIC_FIELD: &str = "metric";
const METRIC_CHUNKS_FIELD: &str = "metric_chunks";
const TUPLE_DESIGN_FIELD: &str = "tuple_design";
const TUPLE_DESIGN_CHUNKS_FIELD: &str = "tuple_design_chunks";
const UNCERTAINTY_METHOD_FIELD: &str = "uncertainty_method";
const UNCERTAINTY_METHOD_CHUNKS_FIELD: &str = "uncertainty_method_chunks";
const GATE_SCORE_METHOD_FIELD: &str = "gate_score_method";
const GATE_SCORE_METHOD_CHUNKS_FIELD: &str = "gate_score_method_chunks";

#[derive(Serialize, Deserialize)]
struct EvidencePayload {
    schema_version: String,
    event: String,
    evidence: Value,
}

pub(super) fn encode_payload(evidence: &LiveCalyxNativeEvidence) -> Result<Vec<u8>> {
    let mut evidence = serde_json::to_value(evidence)
        .map_err(|error| invalid_error(format!("encode live evidence value: {error}")))?;
    encode_calibration_versions(&mut evidence)?;
    encode_temporal_lane_role(&mut evidence)?;
    encode_redundancy_method(&mut evidence)?;
    serde_json::to_vec(&EvidencePayload {
        schema_version: LIVE_CALYX_NATIVE_EVIDENCE_SCHEMA_VERSION.to_string(),
        event: LIVE_CALYX_NATIVE_EVIDENCE_EVENT.to_string(),
        evidence,
    })
    .map_err(|error| invalid_error(format!("encode live evidence payload: {error}")))
}

pub(super) fn decode_payload(bytes: &[u8], ledger_seq: u64) -> Result<LiveCalyxNativeEvidence> {
    let mut payload: EvidencePayload = serde_json::from_slice(bytes).map_err(|error| {
        readback_error(format!(
            "decode evidence payload at ledger seq {ledger_seq}: {error}"
        ))
    })?;
    if payload.schema_version != LIVE_CALYX_NATIVE_EVIDENCE_SCHEMA_VERSION
        || payload.event != LIVE_CALYX_NATIVE_EVIDENCE_EVENT
    {
        return Err(readback_error(format!(
            "ledger row {ledger_seq} has an unknown live-evidence schema or event"
        )));
    }
    decode_calibration_versions(&mut payload.evidence, ledger_seq)?;
    decode_temporal_lane_role(&mut payload.evidence, ledger_seq)?;
    decode_redundancy_method(&mut payload.evidence, ledger_seq)?;
    serde_json::from_value(payload.evidence).map_err(|error| {
        readback_error(format!(
            "decode typed evidence at ledger seq {ledger_seq}: {error}"
        ))
    })
}

fn encode_calibration_versions(evidence: &mut Value) -> Result<()> {
    let calibration = calibration_object_mut(evidence)
        .map_err(|message| invalid_error(format!("encode live evidence: {message}")))?;
    encode_chunked_field(
        calibration,
        VERSION_FIELD,
        VERSION_CHUNKS_FIELD,
        false,
        "calibration version",
    )?;
    encode_chunked_field(
        calibration,
        PREVIOUS_VERSION_FIELD,
        PREVIOUS_VERSION_CHUNKS_FIELD,
        true,
        "previous calibration version",
    )
}

fn decode_calibration_versions(evidence: &mut Value, ledger_seq: u64) -> Result<()> {
    let calibration = calibration_object_mut(evidence).map_err(|message| {
        readback_error(format!(
            "decode evidence payload at ledger seq {ledger_seq}: {message}"
        ))
    })?;
    decode_chunked_field(calibration, VERSION_CHUNKS_FIELD, VERSION_FIELD, false)
        .and_then(|()| {
            decode_chunked_field(
                calibration,
                PREVIOUS_VERSION_CHUNKS_FIELD,
                PREVIOUS_VERSION_FIELD,
                true,
            )
        })
        .map_err(|message| {
            readback_error(format!(
                "decode evidence payload at ledger seq {ledger_seq}: {message}"
            ))
        })
}

fn encode_temporal_lane_role(evidence: &mut Value) -> Result<()> {
    let a37 = a37_object_mut(evidence)
        .map_err(|message| invalid_error(format!("encode live evidence: {message}")))?;
    encode_chunked_field(
        a37,
        TEMPORAL_LANE_ROLE_FIELD,
        TEMPORAL_LANE_ROLE_CHUNKS_FIELD,
        false,
        "A37 temporal lane role",
    )
}

fn decode_temporal_lane_role(evidence: &mut Value, ledger_seq: u64) -> Result<()> {
    let a37 = a37_object_mut(evidence).map_err(|message| {
        readback_error(format!(
            "decode evidence payload at ledger seq {ledger_seq}: {message}"
        ))
    })?;
    decode_chunked_field(
        a37,
        TEMPORAL_LANE_ROLE_CHUNKS_FIELD,
        TEMPORAL_LANE_ROLE_FIELD,
        false,
    )
    .map_err(|message| {
        readback_error(format!(
            "decode evidence payload at ledger seq {ledger_seq}: {message}"
        ))
    })
}

fn encode_redundancy_method(evidence: &mut Value) -> Result<()> {
    let Some(method) = redundancy_method_object_mut(evidence)
        .map_err(|message| invalid_error(format!("encode live evidence: {message}")))?
    else {
        return Ok(());
    };
    encode_method_fields(method)
}

fn decode_redundancy_method(evidence: &mut Value, ledger_seq: u64) -> Result<()> {
    let Some(method) = redundancy_method_object_mut(evidence).map_err(|message| {
        readback_error(format!(
            "decode evidence payload at ledger seq {ledger_seq}: {message}"
        ))
    })?
    else {
        return Ok(());
    };
    decode_method_fields(method).map_err(|message| {
        readback_error(format!(
            "decode evidence payload at ledger seq {ledger_seq}: {message}"
        ))
    })
}

fn encode_method_fields(method: &mut Map<String, Value>) -> Result<()> {
    for (source, destination, label) in [
        (METRIC_FIELD, METRIC_CHUNKS_FIELD, "redundancy metric"),
        (
            TUPLE_DESIGN_FIELD,
            TUPLE_DESIGN_CHUNKS_FIELD,
            "redundancy tuple design",
        ),
        (
            UNCERTAINTY_METHOD_FIELD,
            UNCERTAINTY_METHOD_CHUNKS_FIELD,
            "redundancy uncertainty method",
        ),
        (
            GATE_SCORE_METHOD_FIELD,
            GATE_SCORE_METHOD_CHUNKS_FIELD,
            "redundancy gate score method",
        ),
    ] {
        encode_chunked_field(method, source, destination, false, label)?;
    }
    Ok(())
}

fn decode_method_fields(method: &mut Map<String, Value>) -> std::result::Result<(), String> {
    for (source, destination) in [
        (METRIC_CHUNKS_FIELD, METRIC_FIELD),
        (TUPLE_DESIGN_CHUNKS_FIELD, TUPLE_DESIGN_FIELD),
        (UNCERTAINTY_METHOD_CHUNKS_FIELD, UNCERTAINTY_METHOD_FIELD),
        (GATE_SCORE_METHOD_CHUNKS_FIELD, GATE_SCORE_METHOD_FIELD),
    ] {
        decode_chunked_field(method, source, destination, false)?;
    }
    Ok(())
}

fn calibration_object_mut(
    evidence: &mut Value,
) -> std::result::Result<&mut Map<String, Value>, String> {
    evidence
        .as_object_mut()
        .and_then(|object| object.get_mut(CALIBRATION_FIELD))
        .and_then(Value::as_object_mut)
        .ok_or_else(|| "calibration object is missing".to_string())
}

fn a37_object_mut(evidence: &mut Value) -> std::result::Result<&mut Map<String, Value>, String> {
    evidence
        .as_object_mut()
        .and_then(|object| object.get_mut(PANEL_FIELD))
        .and_then(Value::as_object_mut)
        .and_then(|panel| panel.get_mut(ASSAY_CARD_FIELD))
        .and_then(Value::as_object_mut)
        .and_then(|card| card.get_mut(A37_DIVERSITY_FIELD))
        .and_then(Value::as_object_mut)
        .ok_or_else(|| "panel Assay A37 object is missing".to_string())
}

fn redundancy_method_object_mut(
    evidence: &mut Value,
) -> std::result::Result<Option<&mut Map<String, Value>>, String> {
    let card = evidence
        .as_object_mut()
        .and_then(|object| object.get_mut(PANEL_FIELD))
        .and_then(Value::as_object_mut)
        .and_then(|panel| panel.get_mut(ASSAY_CARD_FIELD))
        .and_then(Value::as_object_mut)
        .ok_or_else(|| "panel Assay card object is missing".to_string())?;
    match card.get_mut(REDUNDANCY_METHOD_FIELD) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Object(method)) => Ok(Some(method)),
        Some(_) => Err("panel Assay redundancy method is malformed".to_string()),
    }
}

fn encode_chunked_field(
    object: &mut Map<String, Value>,
    source: &str,
    destination: &str,
    optional: bool,
    label: &str,
) -> Result<()> {
    let value = object
        .remove(source)
        .ok_or_else(|| invalid_error(format!("{label} is missing")))?;
    let encoded = match value {
        Value::String(value) => Value::Array(
            chunk_string(&value)
                .into_iter()
                .map(Value::String)
                .collect(),
        ),
        Value::Null if optional => Value::Null,
        _ => {
            return Err(invalid_error(format!("{label} is not a string")));
        }
    };
    object.insert(destination.to_string(), encoded);
    Ok(())
}

fn decode_chunked_field(
    object: &mut Map<String, Value>,
    source: &str,
    destination: &str,
    optional: bool,
) -> std::result::Result<(), String> {
    let value = object
        .remove(source)
        .ok_or_else(|| format!("chunked field {source} is missing"))?;
    let decoded = match value {
        Value::Array(chunks) => Value::String(join_chunks(chunks)?),
        Value::Null if optional => Value::Null,
        _ => return Err(format!("chunked field {source} is malformed")),
    };
    object.insert(destination.to_string(), decoded);
    Ok(())
}

fn chunk_string(value: &str) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current = String::new();
    let mut count = 0;
    for character in value.chars() {
        if count == STRING_CHUNK_CHARS {
            chunks.push(std::mem::take(&mut current));
            count = 0;
        }
        current.push(character);
        count += 1;
    }
    if !current.is_empty() || chunks.is_empty() {
        chunks.push(current);
    }
    chunks
}

fn join_chunks(chunks: Vec<Value>) -> std::result::Result<String, String> {
    let mut joined = String::new();
    for chunk in chunks {
        let Value::String(chunk) = chunk else {
            return Err("wire string chunk is not a string".to_string());
        };
        if chunk.chars().count() > STRING_CHUNK_CHARS {
            return Err("wire string chunk exceeds the limit".to_string());
        }
        joined.push_str(&chunk);
    }
    Ok(joined)
}

#[cfg(test)]
mod tests {
    use calyx_ledger::RedactionPolicy;
    use serde_json::json;

    use super::*;

    #[test]
    fn redundancy_metadata_round_trips_through_ledger_safe_chunks() {
        let mut evidence = json!({
            "panel": {"assay_card": {"redundancy_method": {
                "metric": "debiased_linear_cka_hsic1_u4_v1",
                "tuple_design": "blake3_counter_uniform_four_distinct_with_replacement_v1",
                "tuple_plan_blake3": "ab".repeat(32),
                "uncertainty_method": "delete_32_group_jackknife_ratio_v1",
                "gate_score_method": "max_0_raw_plus_4_mc_se_clamped_1_fail_closed_v1"
            }}}
        });
        let original = evidence.clone();

        encode_redundancy_method(&mut evidence).unwrap();
        let bytes = serde_json::to_vec(&evidence).unwrap();
        RedactionPolicy::check_payload(&bytes).unwrap();
        decode_redundancy_method(&mut evidence, 7).unwrap();

        assert_eq!(evidence, original);
    }
}
