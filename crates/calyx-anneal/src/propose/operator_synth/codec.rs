use calyx_core::{Result, Ts};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{
    ANNEAL_OPERATOR_PROPOSAL_TAG, OperatorProposalRecord, invalid_record, validate_record,
};

const OPERATOR_PREFIX: &[u8] = b"operator/v1/";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OperatorProposalReadback {
    pub key: Vec<u8>,
    pub value: Vec<u8>,
    pub record: OperatorProposalRecord,
}

pub fn operator_proposal_key(ts: Ts, proposal_id: &str) -> Vec<u8> {
    let mut key = Vec::with_capacity(OPERATOR_PREFIX.len() + 8 + proposal_id.len());
    key.extend_from_slice(OPERATOR_PREFIX);
    key.extend_from_slice(&ts.to_be_bytes());
    key.extend_from_slice(proposal_id.as_bytes());
    key
}

pub fn encode_operator_proposal(record: &OperatorProposalRecord) -> Result<Vec<u8>> {
    validate_record(record)?;
    serde_json::to_vec_pretty(&json!({
        "tag": ANNEAL_OPERATOR_PROPOSAL_TAG,
        "record": record,
    }))
    .map_err(|error| invalid_record(format!("encode operator proposal row: {error}")))
}

pub fn decode_operator_proposal(bytes: &[u8]) -> Result<OperatorProposalRecord> {
    let row: Value = serde_json::from_slice(bytes)
        .map_err(|error| invalid_record(format!("decode operator proposal row: {error}")))?;
    match row.get("tag").and_then(Value::as_str) {
        Some(ANNEAL_OPERATOR_PROPOSAL_TAG) => {}
        Some(other) => return Err(invalid_record(format!("unexpected operator tag {other}"))),
        None => return Err(invalid_record("operator proposal row missing tag")),
    }
    let record = serde_json::from_value::<OperatorProposalRecord>(
        row.get("record")
            .ok_or_else(|| invalid_record("operator proposal row missing record"))?
            .clone(),
    )
    .map_err(|error| invalid_record(format!("decode operator proposal record: {error}")))?;
    validate_record(&record)?;
    Ok(record)
}

pub fn decode_operator_proposal_rows(
    rows: Vec<(Vec<u8>, Vec<u8>)>,
) -> Result<Vec<OperatorProposalReadback>> {
    let mut out = Vec::with_capacity(rows.len());
    for (key, value) in rows {
        out.push(OperatorProposalReadback {
            record: decode_operator_proposal(&value)?,
            key,
            value,
        });
    }
    out.sort_by(|left, right| left.key.cmp(&right.key));
    Ok(out)
}
