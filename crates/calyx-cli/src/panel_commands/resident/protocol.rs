use std::net::SocketAddr;
use std::path::PathBuf;

use calyx_core::{AbsentReason, Modality, Placement, SlotVector};
use serde::{Deserialize, Serialize};

pub(super) const READY_SCHEMA: &str = "calyx-panel-resident-readiness-v1";
pub(super) const MEASURE_SCHEMA: &str = "calyx-panel-resident-measure-v1";
pub(super) const MEASURE_BATCH_SCHEMA: &str = "calyx-panel-resident-measure-batch-v1";
/// v2 (#1002): measure_batch responses stream as one header frame, one frame
/// per measured input row, and one end frame — never a single response frame
/// carrying every multi-vector payload at once.
pub(super) const RESIDENT_BINARY_PROTOCOL_VERSION: u16 = 2;

#[derive(Debug)]
pub(super) enum ClientMeasureInput {
    Utf8(String),
    Hex(String),
}

#[derive(Deserialize)]
pub(super) struct ResidentRequest {
    pub(super) op: String,
    pub(super) modality: Option<Modality>,
    pub(super) input: Option<String>,
    pub(super) input_hex: Option<String>,
    pub(super) inputs_hex: Option<Vec<String>>,
    pub(super) runtime_batch_limit: Option<usize>,
}

#[derive(Debug, Deserialize, Serialize)]
pub(super) struct ReadyResponse {
    pub(super) schema: String,
    pub(super) ready: bool,
    pub(super) residency_scope: &'static str,
    pub(super) process_id: u32,
    pub(super) bind: SocketAddr,
    pub(super) uptime_ms: u128,
    pub(super) source_of_truth: String,
    pub(super) home: PathBuf,
    pub(super) template_selector: String,
    pub(super) template_source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) ready_out: Option<PathBuf>,
    pub(super) max_resident_vram_mib: u64,
    pub(super) declared_template_vram_mib: u64,
    pub(super) resident_overhead_multiplier: f32,
    pub(super) estimated_resident_vram_mib: u64,
    pub(super) max_load_secs: u64,
    pub(super) load_parallelism: usize,
    pub(super) load_ms: u128,
    pub(super) probe_ms: u128,
    pub(super) slot_count: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(super) slot_scope: Vec<u16>,
    pub(super) content_lens_count: usize,
    pub(super) registry_lens_count: usize,
    pub(super) warmed_lens_count: usize,
    pub(super) gpu_content_lens_count: usize,
    pub(super) cpu_content_lens_count: usize,
}

#[derive(Debug, Deserialize, Serialize)]
pub(super) struct MeasureResponse {
    pub(super) schema: String,
    pub(super) ready: bool,
    pub(super) process_id: u32,
    pub(super) template_source: String,
    pub(super) modality: Modality,
    pub(super) input_len: usize,
    pub(super) elapsed_ms: u128,
    pub(super) measured_slot_count: usize,
    pub(super) absent_slot_count: usize,
    pub(super) slots: Vec<ResidentSlotMeasure>,
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct MeasureBatchResponse {
    pub(crate) schema: String,
    pub(crate) ready: bool,
    pub(crate) process_id: u32,
    pub(crate) template_source: String,
    pub(crate) modality: Modality,
    pub(crate) input_count: usize,
    pub(crate) elapsed_ms: u128,
    pub(crate) runtime_batch_limit: Option<usize>,
    pub(crate) rows: Vec<ResidentMeasuredInput>,
}

#[derive(Debug)]
pub(crate) struct MeasureBatchAtResponse {
    pub(crate) response: MeasureBatchResponse,
    pub(crate) request_bytes: usize,
    pub(crate) response_bytes: usize,
}

#[derive(Debug, Serialize)]
pub(crate) struct MeasureBatchSummaryResponse {
    pub(crate) schema: String,
    pub(crate) ready: bool,
    pub(crate) process_id: u32,
    pub(crate) template_source: String,
    pub(crate) modality: Modality,
    pub(crate) input_count: usize,
    pub(crate) elapsed_ms: u128,
    pub(crate) runtime_batch_limit: Option<usize>,
    pub(crate) row_count: usize,
    pub(crate) measured_slot_counts: Vec<usize>,
    pub(crate) absent_slot_counts: Vec<usize>,
    pub(crate) response_rows_sha256: String,
    pub(crate) request_bytes: usize,
    pub(crate) response_bytes: usize,
}

#[derive(Debug, Deserialize, Serialize)]
pub(super) struct ResidentMeasureBatchBinaryRequest {
    pub(super) protocol_version: u16,
    pub(super) modality: Modality,
    pub(super) inputs: Vec<Vec<u8>>,
    pub(super) runtime_batch_limit: Option<usize>,
}

/// One length-prefixed bincode frame of the streamed measure_batch response.
#[derive(Debug, Deserialize, Serialize)]
pub(super) enum ResidentMeasureBatchStreamFrame {
    Header(ResidentMeasureBatchStreamHeader),
    Row(Box<ResidentMeasuredInput>),
    End(ResidentMeasureBatchStreamEnd),
    Err {
        code: String,
        message: String,
        remediation: String,
    },
}

#[derive(Debug, Deserialize, Serialize)]
pub(super) struct ResidentMeasureBatchStreamHeader {
    pub(super) protocol_version: u16,
    pub(super) schema: String,
    pub(super) ready: bool,
    pub(super) process_id: u32,
    pub(super) template_source: String,
    pub(super) modality: Modality,
    pub(super) input_count: usize,
    pub(super) runtime_batch_limit: Option<usize>,
}

#[derive(Debug, Deserialize, Serialize)]
pub(super) struct ResidentMeasureBatchStreamEnd {
    pub(super) row_count: usize,
    pub(super) elapsed_ms: u128,
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct ResidentMeasuredInput {
    pub(crate) input_index: usize,
    pub(crate) input_len: usize,
    pub(crate) measured_slot_count: usize,
    pub(crate) absent_slot_count: usize,
    pub(crate) slots: Vec<ResidentSlotMeasure>,
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct ResidentSlotMeasure {
    pub(crate) slot: u16,
    pub(crate) key: String,
    pub(crate) lens_id: String,
    pub(crate) modality: Modality,
    pub(crate) placement: Placement,
    pub(crate) measured: bool,
    pub(crate) vector: Option<SlotVector>,
    pub(crate) absent_reason: Option<AbsentReason>,
}

pub(super) fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

pub(super) fn hex_decode(raw: &str) -> Result<Vec<u8>, String> {
    if !raw.len().is_multiple_of(2) {
        return Err(format!(
            "input_hex length {} is odd; expected complete bytes",
            raw.len()
        ));
    }
    let mut bytes = Vec::with_capacity(raw.len() / 2);
    let raw = raw.as_bytes();
    let mut idx = 0;
    while idx < raw.len() {
        let hi = hex_nibble(raw[idx]).ok_or_else(|| invalid_hex(idx, raw[idx]))?;
        let lo = hex_nibble(raw[idx + 1]).ok_or_else(|| invalid_hex(idx + 1, raw[idx + 1]))?;
        bytes.push((hi << 4) | lo);
        idx += 2;
    }
    Ok(bytes)
}

fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn invalid_hex(index: usize, byte: u8) -> String {
    format!("input_hex contains non-hex byte 0x{byte:02x} at character index {index}")
}
