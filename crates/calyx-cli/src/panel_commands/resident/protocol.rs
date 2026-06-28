use std::net::SocketAddr;
use std::path::PathBuf;

use calyx_core::{AbsentReason, Modality, Placement, SlotVector};
use serde::{Deserialize, Serialize};

pub(super) const READY_SCHEMA: &str = "calyx-panel-resident-readiness-v1";
pub(super) const MEASURE_SCHEMA: &str = "calyx-panel-resident-measure-v1";

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
}

#[derive(Serialize)]
pub(super) struct ReadyResponse {
    pub(super) schema: &'static str,
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
    pub(super) content_lens_count: usize,
    pub(super) registry_lens_count: usize,
    pub(super) warmed_lens_count: usize,
    pub(super) gpu_content_lens_count: usize,
    pub(super) cpu_content_lens_count: usize,
}

#[derive(Serialize)]
pub(super) struct MeasureResponse {
    pub(super) schema: &'static str,
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

#[derive(Serialize)]
pub(super) struct ResidentSlotMeasure {
    pub(super) slot: u16,
    pub(super) key: String,
    pub(super) lens_id: String,
    pub(super) modality: Modality,
    pub(super) placement: Placement,
    pub(super) measured: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) vector: Option<SlotVector>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) absent_reason: Option<AbsentReason>,
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
