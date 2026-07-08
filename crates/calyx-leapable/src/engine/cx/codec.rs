use std::collections::BTreeMap;

use calyx_aster::cf::{ColumnFamily, base_key, full_content_hash};
use calyx_aster::dedup::EpochSecs;
use calyx_aster::recurrence::{OccurrenceContext, RetentionPolicy, append_occurrence};
use calyx_core::{
    CalyxError, Constellation, CxFlags, CxId, InputRef, LedgerRef, SlotId, SlotVector, Ts,
    VaultStore,
};
use calyx_ledger::{decode as decode_ledger, tombstone_from_entry};
use serde_json::{Value, json};

use super::super::{EngineResult, VaultHandle};
use super::params::{CxInput, CxPutItem, CxSlotParam};

const CALYX_LEAPABLE_CX_INPUT_INVALID: &str = "CALYX_LEAPABLE_CX_INPUT_INVALID";
const CALYX_LEAPABLE_CX_ID_INVALID: &str = "CALYX_LEAPABLE_CX_ID_INVALID";
const CALYX_LEAPABLE_CX_SCAN_LIMIT_INVALID: &str = "CALYX_LEAPABLE_CX_SCAN_LIMIT_INVALID";

const INPUT_HEX_METADATA: &str = "leapable.input_hex";
const INPUT_ENCODING_METADATA: &str = "leapable.input_encoding";
const INPUT_TEXT_ENCODING: &str = "utf8";
const INPUT_BYTES_ENCODING: &str = "bytes";
const DEFAULT_SCAN_LIMIT: usize = 100;
const MAX_SCAN_LIMIT: usize = 1000;

pub(super) struct PreparedPut {
    pub(super) constellation: Constellation,
    pub(super) deduped: bool,
    recurrence_context: OccurrenceContext,
}

pub(super) fn prepare_put(
    handle: &VaultHandle,
    item: CxPutItem,
    batch_ts: Ts,
    within_batch_duplicate: bool,
) -> EngineResult<PreparedPut> {
    let (input_bytes, encoding) = decode_input(&item.input)?;
    reject_reserved_metadata(&item.metadata)?;
    let id = handle
        .vault
        .cx_id_for_input(&input_bytes, item.panel_version);
    let input_hash = full_content_hash([input_bytes.as_slice()]);
    let mut metadata = item.metadata;
    metadata.insert(INPUT_HEX_METADATA.to_string(), hex(&input_bytes));
    metadata.insert(INPUT_ENCODING_METADATA.to_string(), encoding.to_string());
    let slots = slots_from_params(item.slots)?;
    let created_at = item.ts.unwrap_or(batch_ts);
    let recurrence_context = recurrence_context(item.panel_version, &input_hash, &metadata)?;
    let deduped = within_batch_duplicate || base_exists(handle, id)?;
    let constellation = Constellation {
        cx_id: id,
        vault_id: handle.vault.vault_id(),
        panel_version: item.panel_version,
        created_at,
        input_ref: InputRef {
            hash: input_hash,
            pointer: item.input.pointer,
            redacted: item.input.redacted,
        },
        modality: item.modality,
        slots,
        scalars: item.scalars,
        metadata,
        anchors: item.anchors,
        provenance: LedgerRef {
            seq: 0,
            hash: [0; 32],
        },
        flags: CxFlags {
            ungrounded: false,
            degraded: false,
            novel_region: false,
            redacted_input: item.input.redacted,
        },
    };
    let mut constellation = constellation;
    constellation.flags.ungrounded = constellation.anchors.is_empty();
    Ok(PreparedPut {
        constellation,
        deduped,
        recurrence_context,
    })
}

pub(super) fn predicted_id(handle: &VaultHandle, item: &CxPutItem) -> EngineResult<CxId> {
    let (input_bytes, _) = decode_input(&item.input)?;
    Ok(handle
        .vault
        .cx_id_for_input(&input_bytes, item.panel_version))
}

pub(super) fn append_recurrence_if_needed(
    handle: &VaultHandle,
    id: CxId,
    ts: Ts,
    prepared: PreparedPut,
    deduped: bool,
) -> EngineResult<Option<u64>> {
    if !deduped {
        return Ok(None);
    }
    let occurrence = append_occurrence(
        &handle.vault,
        id,
        epoch_secs(ts)?,
        prepared.recurrence_context,
        epoch_secs(ts)?,
        RetentionPolicy::default(),
    )?;
    Ok(Some(occurrence.0))
}

pub(super) fn append_duplicate_anchors(
    handle: &VaultHandle,
    constellation: &Constellation,
) -> EngineResult<()> {
    for anchor in &constellation.anchors {
        handle.vault.anchor(constellation.cx_id, anchor.clone())?;
    }
    Ok(())
}

pub(super) fn validate_scan_limit(limit: Option<usize>) -> EngineResult<usize> {
    let limit = limit.unwrap_or(DEFAULT_SCAN_LIMIT);
    if (1..=MAX_SCAN_LIMIT).contains(&limit) {
        return Ok(limit);
    }
    Err(cx_error(
        CALYX_LEAPABLE_CX_SCAN_LIMIT_INVALID,
        format!("cx.scan limit {limit} is outside 1..={MAX_SCAN_LIMIT}"),
        "choose a positive limit no larger than the engine maximum",
    )
    .into())
}

pub(super) fn parse_cx_id(value: &str) -> EngineResult<CxId> {
    value.parse::<CxId>().map_err(|error| {
        cx_error(
            CALYX_LEAPABLE_CX_ID_INVALID,
            format!("invalid cx_id {value:?}: {error}"),
            "send the 32-character lowercase hex CxId returned by cx.put",
        )
        .into()
    })
}

pub(super) fn cx_id_from_key(key: &[u8]) -> EngineResult<CxId> {
    let bytes: [u8; 16] = key.try_into().map_err(|_| {
        cx_error(
            CALYX_LEAPABLE_CX_ID_INVALID,
            format!("base CF key has {} bytes, expected 16", key.len()),
            "inspect the vault for Base CF key corruption",
        )
    })?;
    Ok(CxId::from_bytes(bytes))
}

pub(super) fn constellation_value(cx: &Constellation) -> Value {
    let input_hex = cx.metadata.get(INPUT_HEX_METADATA).cloned();
    let input_text = if cx.metadata.get(INPUT_ENCODING_METADATA).map(String::as_str)
        == Some(INPUT_TEXT_ENCODING)
    {
        input_hex
            .as_deref()
            .and_then(|value| decode_hex(value).ok())
            .and_then(|bytes| String::from_utf8(bytes).ok())
    } else {
        None
    };
    json!({
        "cx_id": cx.cx_id.to_string(),
        "input_hex": input_hex,
        "input_text": input_text,
        "constellation": cx,
    })
}

pub(super) fn scan_tombstones(handle: &VaultHandle, snapshot: u64) -> EngineResult<Vec<Value>> {
    let mut out = Vec::new();
    for (_, bytes) in handle.vault.scan_cf_at(snapshot, ColumnFamily::Ledger)? {
        let entry = decode_ledger(&bytes)?;
        if let Some(tombstone) = tombstone_from_entry(&entry)? {
            out.push(json!({
                "seq": tombstone.seq,
                "scope": tombstone.scope,
                "actor": tombstone.actor,
                "erased_at": tombstone.erased_at,
                "records_deleted": tombstone.records_deleted,
                "compact": tombstone.as_json_value(),
            }));
        }
    }
    Ok(out)
}

fn decode_input(input: &CxInput) -> EngineResult<(Vec<u8>, &'static str)> {
    let present = usize::from(input.text.is_some())
        + usize::from(input.bytes.is_some())
        + usize::from(input.hex.is_some());
    if present != 1 {
        return Err(cx_error(
            CALYX_LEAPABLE_CX_INPUT_INVALID,
            "cx input must contain exactly one of text, bytes, or hex",
            "send one canonical raw chunk representation",
        )
        .into());
    }
    if let Some(text) = &input.text {
        return Ok((text.as_bytes().to_vec(), INPUT_TEXT_ENCODING));
    }
    if let Some(bytes) = &input.bytes {
        return Ok((bytes.clone(), INPUT_BYTES_ENCODING));
    }
    let hex = input.hex.as_deref().expect("present checked");
    decode_hex(hex).map(|bytes| (bytes, INPUT_BYTES_ENCODING))
}

fn reject_reserved_metadata(metadata: &BTreeMap<String, String>) -> EngineResult<()> {
    if let Some(key) = metadata
        .keys()
        .find(|key| key.as_str() == INPUT_HEX_METADATA || key.as_str() == INPUT_ENCODING_METADATA)
    {
        return Err(cx_error(
            CALYX_LEAPABLE_CX_INPUT_INVALID,
            format!("metadata key {key:?} is reserved for input readback"),
            "remove leapable.input_* metadata and let the engine stamp byte readback fields",
        )
        .into());
    }
    Ok(())
}

fn slots_from_params(params: Vec<CxSlotParam>) -> EngineResult<BTreeMap<SlotId, SlotVector>> {
    let mut slots = BTreeMap::new();
    for param in params {
        let slot = SlotId::new(param.slot_id);
        if slots.insert(slot, param.vector).is_some() {
            return Err(cx_error(
                CALYX_LEAPABLE_CX_INPUT_INVALID,
                format!("slot {slot} appears more than once"),
                "send each slot_id at most once",
            )
            .into());
        }
    }
    Ok(slots)
}

fn base_exists(handle: &VaultHandle, id: CxId) -> EngineResult<bool> {
    Ok(handle
        .vault
        .read_cf_at(handle.vault.snapshot(), ColumnFamily::Base, &base_key(id))?
        .is_some())
}

fn recurrence_context(
    panel_version: u32,
    input_hash: &[u8; 32],
    metadata: &BTreeMap<String, String>,
) -> EngineResult<OccurrenceContext> {
    let mut value = format!("pv={panel_version};hash={}", hex(input_hash));
    if let Some(chunk_id) = metadata.get("chunk_id")
        && value.len() + chunk_id.len() + 8 <= 256
    {
        value.push_str(";chunk=");
        value.push_str(chunk_id);
    }
    Ok(OccurrenceContext::new(value.into_bytes())?)
}

fn epoch_secs(ts: Ts) -> EngineResult<EpochSecs> {
    let secs = ts / 1_000;
    let secs = i64::try_from(secs).map_err(|_| {
        cx_error(
            CALYX_LEAPABLE_CX_INPUT_INVALID,
            format!("timestamp {ts} does not fit recurrence epoch seconds"),
            "send a Unix millisecond timestamp within the signed epoch range",
        )
    })?;
    Ok(EpochSecs(secs))
}

fn decode_hex(value: &str) -> EngineResult<Vec<u8>> {
    if !value.len().is_multiple_of(2) {
        return Err(cx_error(
            CALYX_LEAPABLE_CX_INPUT_INVALID,
            "hex input must contain an even number of characters",
            "send lowercase hexadecimal bytes without separators",
        )
        .into());
    }
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|chunk| Ok((hex_value(chunk[0])? << 4) | hex_value(chunk[1])?))
        .collect()
}

fn hex_value(byte: u8) -> EngineResult<u8> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(cx_error(
            CALYX_LEAPABLE_CX_INPUT_INVALID,
            "hex input contains a non-hex character",
            "send hexadecimal characters 0-9, a-f, or A-F",
        )
        .into()),
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn cx_error(
    code: &'static str,
    message: impl Into<String>,
    remediation: &'static str,
) -> CalyxError {
    CalyxError {
        code,
        message: message.into(),
        remediation,
    }
}
