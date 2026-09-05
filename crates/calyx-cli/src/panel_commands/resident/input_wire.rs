use calyx_core::StructuredRecord;
use serde::{Deserialize, Serialize};

use super::codec::{decode_binary, encode_binary};
use super::*;

pub(super) const INPUT_CONTRACT_MISMATCH: &str = "CALYX_PANEL_RESIDENT_INPUT_CONTRACT_MISMATCH";
pub(super) const INPUT_CONTRACT_REMEDIATION: &str = "restart the resident service from the same Calyx build and send each complete typed Input without changing its modality or structured record";

/// Owned positional bincode wire image of [`Input`], used to decode a complete
/// typed input received from the resident binary protocol.
///
/// [`Input`] carries `#[serde(default, skip_serializing_if = "Option::is_none")]`
/// on `structured` so its canonical JSON stays compact and byte-stable for
/// persisted/manifest artifacts. The resident wire, however, uses
/// non-self-describing bincode where a skipped field desynchronizes the
/// positional decoder: for the common `structured: None` case the encoder omits
/// the field entirely while the derived positional decoder still tries to read
/// its `Option` tag, producing `UnexpectedEnd { additional: 1 }` (#1945).
///
/// This wire image mirrors every [`Input`] field with **no** skip attributes so
/// the encoder and decoder agree on the exact byte layout for `structured:
/// None` and `Some(..)` alike. It is transient wire-only state — never persisted
/// — so splitting it out from [`Input`]'s shared `Serialize` impl fixes the
/// codec without changing any durable JSON representation.
#[derive(Deserialize)]
struct InputWire {
    modality: Modality,
    bytes: Vec<u8>,
    pointer: Option<String>,
    structured: Option<StructuredRecord>,
}

impl From<InputWire> for Input {
    fn from(wire: InputWire) -> Self {
        let InputWire {
            modality,
            bytes,
            pointer,
            structured,
        } = wire;
        Self {
            modality,
            bytes,
            pointer,
            structured,
        }
    }
}

/// Borrowing counterpart of [`InputWire`] for the encode path. Serializes to a
/// byte-identical layout (bincode encodes `&[u8]`/`&Option<..>` exactly as their
/// owned forms) while avoiding a copy of the potentially large primary payload.
#[derive(Serialize)]
struct InputWireRef<'a> {
    modality: Modality,
    bytes: &'a [u8],
    pointer: &'a Option<String>,
    structured: &'a Option<StructuredRecord>,
}

impl<'a> From<&'a Input> for InputWireRef<'a> {
    fn from(input: &'a Input) -> Self {
        Self {
            modality: input.modality,
            bytes: &input.bytes,
            pointer: &input.pointer,
            structured: &input.structured,
        }
    }
}

/// Encodes each complete measurement input inside the streamed request. The
/// outer request deliberately retains `Vec<Vec<u8>>` for wire compatibility,
/// but protocol v3 defines each element as a bincode-encoded `Input`, not raw
/// content bytes. This preserves hybrid structured state across the process
/// boundary instead of silently reducing a constellation to one byte vector.
pub(super) fn encode_inputs(modality: Modality, inputs: &[Input]) -> CliResult<Vec<Vec<u8>>> {
    validate_inputs(modality, inputs)?;
    inputs
        .iter()
        .map(|input| encode_binary(&InputWireRef::from(input)).map_err(CliError::from))
        .collect()
}

pub(super) fn decode_inputs(modality: Modality, encoded: Vec<Vec<u8>>) -> CliResult<Vec<Input>> {
    let inputs = encoded
        .iter()
        .enumerate()
        .map(|(index, bytes)| {
            decode_binary::<InputWire>(bytes)
                .map(Input::from)
                .map_err(|error| {
                    CliError::from(CalyxError {
                        code: error.code,
                        message: format!(
                            "decode resident typed input {index} failed: {}",
                            error.message
                        ),
                        remediation: error.remediation,
                    })
                })
        })
        .collect::<CliResult<Vec<_>>>()?;
    validate_inputs(modality, &inputs)?;
    Ok(inputs)
}

pub(super) fn validate_inputs(modality: Modality, inputs: &[Input]) -> CliResult {
    for (index, input) in inputs.iter().enumerate() {
        if input.modality != modality {
            return Err(contract_error(format!(
                "resident request declares {modality:?}, but typed input {index} declares {:?}",
                input.modality
            )));
        }
        if input.bytes.is_empty() && input.structured().is_none() {
            return Err(CliError::from(CalyxError {
                code: super::stream::EMPTY_INPUT,
                message: format!(
                    "resident measure input {index} for modality {modality:?} has neither primary bytes nor a canonical structured record"
                ),
                remediation: super::stream::EMPTY_INPUT_REMEDIATION,
            }));
        }
        if let Some(record) = input.structured() {
            record.validate()?;
        }
    }
    Ok(())
}

pub(super) fn contract_error(message: impl Into<String>) -> CliError {
    CliError::from(CalyxError {
        code: INPUT_CONTRACT_MISMATCH,
        message: message.into(),
        remediation: INPUT_CONTRACT_REMEDIATION,
    })
}

/// Closed structured-error catalog admitted across the resident boundary.
/// Exact `(code, remediation)` matching prevents a remote process from
/// inventing error identities while preserving the original actionable cause.
pub(super) fn declared_error(
    code: &str,
    remediation: &str,
) -> Option<(&'static str, &'static str)> {
    const CANONICAL: &str = "supply a canonical supported structured record and retry";
    const PAIRS: &[(&str, &str)] = &[
        (INPUT_CONTRACT_MISMATCH, INPUT_CONTRACT_REMEDIATION),
        (
            "CALYX_STRUCTURED_SCHEMA_INVALID",
            "declare a non-empty stable schema id and a supported typed JSON object",
        ),
        (
            "CALYX_STRUCTURED_SELECTOR_INVALID",
            "use a valid non-root RFC 6901 field pointer and register a new LensId",
        ),
        (
            "CALYX_STRUCTURED_FIELD_MISSING",
            "supply the required selected field under the frozen record schema",
        ),
        (
            "CALYX_STRUCTURED_TYPE_MISMATCH",
            "supply the declared source type without coercion or register a new binding",
        ),
        (
            "CALYX_STRUCTURED_NON_FINITE",
            "supply a finite representable number and preserve the original source value",
        ),
        (
            "CALYX_STRUCTURED_SCHEMA_MISMATCH",
            "use the exact frozen schema/binding for ingest, query, replay, and backfill",
        ),
        ("CALYX_STRUCTURED_RECORD_INVALID", CANONICAL),
        (
            "CALYX_STRUCTURED_RESERVED_METADATA",
            "remove calyx.structured.* user metadata; those keys are engine-owned",
        ),
        (
            "CALYX_STRUCTURED_BINDING_INVALID",
            "repair the frozen selector/type/encoder contract and register a new LensId",
        ),
        (
            "CALYX_STRUCTURED_CATEGORY_UNKNOWN",
            "add the category to a newly registered frozen vocabulary or repair the input",
        ),
        (
            "CALYX_STRUCTURED_DATE_INVALID",
            "supply an exact Gregorian YYYY-MM-DD value or register a different source type",
        ),
        (
            "CALYX_STRUCTURED_DERIVATION_INVALID",
            "repair the declared numeric operands/operation and register a new LensId",
        ),
    ];
    PAIRS
        .iter()
        .copied()
        .find(|candidate| candidate.0 == code && candidate.1 == remediation)
}
