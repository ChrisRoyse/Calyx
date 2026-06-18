use std::fs;

use calyx_assay::{AssayCacheKey, AssayStore, AssaySubject};
use calyx_aster::cf::ColumnFamily;
use calyx_aster::vault::AsterVault;
use calyx_core::AnchorKind;
use serde::Serialize;

use super::engine::{AUDIO_EMOTION_SLOT, EmotionReport, MEDIA_PANEL_VERSION, panel_estimate};
use super::request::EmotionRequest;

const METRIC_KEY: &[u8] = b"ph70/media/audio_emotion";

#[derive(Clone, Debug, Serialize)]
pub(crate) struct MetricEvidence {
    pub(crate) emotion_bits_path: String,
    pub(crate) summary_path: String,
    pub(crate) metric_cf: String,
    pub(crate) metric_cf_key_hex: String,
    pub(crate) metric_cf_value_bytes: usize,
    pub(crate) metric_cf_seq: u64,
    pub(crate) assay_cf: String,
    pub(crate) assay_rows_persisted: usize,
    pub(crate) assay_rows_loaded: usize,
    pub(crate) report: EmotionReport,
}

pub(crate) fn write_metric_outputs(
    vault: &AsterVault,
    request: &EmotionRequest,
    report: EmotionReport,
) -> Result<MetricEvidence, String> {
    fs::create_dir_all(&request.metrics_dir).map_err(|error| error.to_string())?;
    let emotion_bits = request.metrics_dir.join("media_audio_emotion_bits.txt");
    let summary = request.metrics_dir.join("media_audio_emotion_summary.json");
    write_float(&emotion_bits, report.emotion_bits.bits)?;
    let value = serde_json::to_vec_pretty(&report).map_err(|error| error.to_string())?;
    fs::write(&summary, &value).map_err(|error| error.to_string())?;
    let assay_rows_persisted = persist_assay_rows(vault, &report)?;
    let assay_rows_loaded = AssayStore::load_from_vault(vault)
        .map_err(|error| error.to_string())?
        .len();
    let seq = vault
        .write_cf(ColumnFamily::Online, METRIC_KEY.to_vec(), value.clone())
        .map_err(|error| error.to_string())?;
    vault.flush().map_err(|error| error.to_string())?;
    Ok(MetricEvidence {
        emotion_bits_path: emotion_bits.display().to_string(),
        summary_path: summary.display().to_string(),
        metric_cf: "online".to_string(),
        metric_cf_key_hex: hex(METRIC_KEY),
        metric_cf_value_bytes: value.len(),
        metric_cf_seq: seq,
        assay_cf: "assay".to_string(),
        assay_rows_persisted,
        assay_rows_loaded,
        report,
    })
}

fn persist_assay_rows(vault: &AsterVault, report: &EmotionReport) -> Result<usize, String> {
    let key = AssayCacheKey::scoped(
        MEDIA_PANEL_VERSION,
        "ph70-media-audio-emotion",
        vault.vault_id(),
        AnchorKind::Label("media_audio_emotion".to_string()),
    );
    let mut store = AssayStore::default();
    store.put(
        key.clone(),
        AssaySubject::Lens {
            slot: AUDIO_EMOTION_SLOT,
        },
        report.emotion_bits.clone(),
        "PH70 media audio-emotion lens bits vs verified labels",
        6060,
    );
    store.put(
        key,
        AssaySubject::Panel,
        panel_estimate(report),
        "PH70 media panel sufficiency summary for verified audio-emotion anchors",
        6061,
    );
    store
        .persist_to_vault(vault)
        .map_err(|error| error.to_string())
}

fn write_float(path: &std::path::Path, value: f32) -> std::result::Result<(), String> {
    if !value.is_finite() {
        return Err(format!(
            "CALYX_FSV_MEDIA_EMOTION_NONFINITE_METRIC: {}",
            path.display()
        ));
    }
    fs::write(path, format!("{value:.6}\n")).map_err(|error| error.to_string())
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(hex_digit(byte >> 4));
        out.push(hex_digit(byte & 0x0f));
    }
    out
}

fn hex_digit(value: u8) -> char {
    match value {
        0..=9 => char::from(b'0' + value),
        10..=15 => char::from(b'a' + value - 10),
        _ => unreachable!("nibble out of range"),
    }
}
