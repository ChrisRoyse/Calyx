use std::fs;

use calyx_assay::{AssayCacheKey, AssayStore, AssaySubject};
use calyx_aster::cf::ColumnFamily;
use calyx_aster::vault::AsterVault;
use calyx_core::AnchorKind;
use serde::Serialize;

use super::engine::{
    IMAGE_CLIP_SLOT, MEDIA_PANEL_VERSION, MediaImageReport, TRANSCRIPT_SLOT, panel_estimate,
};
use super::request::MediaImageRequest;

const METRIC_KEY: &[u8] = b"ph70/media/image/cross_modal";

#[derive(Clone, Debug, Serialize)]
pub(crate) struct MetricEvidence {
    pub(crate) image_bits_path: String,
    pub(crate) cross_modal_bits_path: String,
    pub(crate) agreement_path: String,
    pub(crate) summary_path: String,
    pub(crate) metric_cf: String,
    pub(crate) metric_cf_key_hex: String,
    pub(crate) metric_cf_value_bytes: usize,
    pub(crate) metric_cf_seq: u64,
    pub(crate) assay_cf: String,
    pub(crate) assay_rows_persisted: usize,
    pub(crate) assay_rows_loaded: usize,
    pub(crate) report: MediaImageReport,
}

pub(crate) fn write_metric_outputs(
    vault: &AsterVault,
    request: &MediaImageRequest,
    report: MediaImageReport,
) -> Result<MetricEvidence, String> {
    fs::create_dir_all(&request.metrics_dir).map_err(|error| error.to_string())?;
    let image_bits = request.metrics_dir.join("media_image_class_bits.txt");
    let cross_bits = request.metrics_dir.join("media_cross_modal_bits.txt");
    let agreement = request.metrics_dir.join("media_cross_modal_agreement.txt");
    let summary = request.metrics_dir.join("media_image_summary.json");
    write_float(&image_bits, report.image_class_bits.bits)?;
    write_float(&cross_bits, report.cross_modal_bits.bits)?;
    write_float(
        &agreement,
        report.cross_modal_agreement.dominant_axis_match_rate,
    )?;
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
        image_bits_path: image_bits.display().to_string(),
        cross_modal_bits_path: cross_bits.display().to_string(),
        agreement_path: agreement.display().to_string(),
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

fn persist_assay_rows(vault: &AsterVault, report: &MediaImageReport) -> Result<usize, String> {
    let key = AssayCacheKey::scoped(
        MEDIA_PANEL_VERSION,
        "ph70-media-image-cross-modal",
        vault.vault_id(),
        AnchorKind::Label("media_image_class_caption".to_string()),
    );
    let mut store = AssayStore::default();
    store.put(
        key.clone(),
        AssaySubject::Lens {
            slot: IMAGE_CLIP_SLOT,
        },
        report.image_class_bits.clone(),
        "PH70 media image lens bits vs ImageNet/CIFAR class anchors",
        6070,
    );
    store.put(
        key.clone(),
        AssaySubject::Pair {
            a: IMAGE_CLIP_SLOT,
            b: TRANSCRIPT_SLOT,
        },
        report.cross_modal_bits.clone(),
        "PH70 media image-caption cross-modal agreement on COCO",
        6071,
    );
    store.put(
        key,
        AssaySubject::Panel,
        panel_estimate(report),
        "PH70 media panel sufficiency summary for image/caption anchors",
        6072,
    );
    store
        .persist_to_vault(vault)
        .map_err(|error| error.to_string())
}

fn write_float(path: &std::path::Path, value: f32) -> std::result::Result<(), String> {
    if !value.is_finite() {
        return Err(format!(
            "CALYX_FSV_MEDIA_NONFINITE_METRIC: {}",
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
