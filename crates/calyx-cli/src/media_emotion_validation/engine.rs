use std::collections::{BTreeMap, BTreeSet};

use calyx_assay::{EstimatorKind, MiEstimate, TrustTag, ksg_mi_continuous_discrete_with_anchor};
use calyx_core::{Anchor, AnchorKind, AnchorValue, SlotId};
use serde::Serialize;

use super::data::{EmotionSample, ValidationData};
use super::request::EmotionRequest;

pub(crate) const MEDIA_PANEL_VERSION: u32 = 10;
pub(crate) const AUDIO_EMOTION_SLOT: SlotId = SlotId::new(3);

#[derive(Clone, Debug, Serialize)]
pub(crate) struct EmotionReport {
    pub(crate) sample_rows: usize,
    pub(crate) dataset_counts: BTreeMap<String, usize>,
    pub(crate) source_sha256_count: usize,
    pub(crate) emotion_sample_count: usize,
    pub(crate) emotion_label_count: usize,
    pub(crate) audio_feature_dim: usize,
    pub(crate) emotion_bits: MiEstimate,
    pub(crate) min_bits: f32,
    pub(crate) panel_version: u32,
    pub(crate) audio_emotion_slot: u16,
    pub(crate) trigger: String,
    pub(crate) intended_outcome: String,
}

pub(crate) fn evaluate_emotion(
    data: &ValidationData,
    request: &EmotionRequest,
) -> Result<EmotionReport, String> {
    let view = emotion_view(&data.samples)?;
    let anchor = grounded_anchor();
    let emotion_bits =
        ksg_mi_continuous_discrete_with_anchor(&view.features, &view.labels, request.k, &anchor)
            .map_err(|error| error.to_string())?;
    if emotion_bits.bits + f32::EPSILON < request.min_bits {
        return Err(format!(
            "CALYX_FSV_MEDIA_EMOTION_BITS_BELOW_THRESHOLD: bits={:.6} threshold={:.6}",
            emotion_bits.bits, request.min_bits
        ));
    }
    Ok(EmotionReport {
        sample_rows: data.total_rows,
        dataset_counts: data.dataset_counts.clone(),
        source_sha256_count: data.source_sha256_count,
        emotion_sample_count: view.features.len(),
        emotion_label_count: view.label_count,
        audio_feature_dim: view.feature_dim,
        emotion_bits,
        min_bits: request.min_bits,
        panel_version: MEDIA_PANEL_VERSION,
        audio_emotion_slot: AUDIO_EMOTION_SLOT.get(),
        trigger: "calyx media emotion-validate on verified audio-emotion samples".to_string(),
        intended_outcome: "persist trusted audio-emotion lens bits against emotion labels"
            .to_string(),
    })
}

pub(crate) fn panel_estimate(report: &EmotionReport) -> MiEstimate {
    MiEstimate::point(
        report.emotion_bits.bits,
        report.emotion_sample_count,
        EstimatorKind::PanelSufficiency,
        TrustTag::Trusted,
    )
}

fn emotion_view(samples: &[EmotionSample]) -> Result<EmotionView, String> {
    if samples.len() < 50 {
        return Err(format!(
            "CALYX_FSV_MEDIA_EMOTION_LABEL_DOMAIN_MISMATCH: need at least 50 samples; got {}",
            samples.len()
        ));
    }
    let feature_dim = rectangular_dim(samples.iter().map(|sample| sample.features.as_slice()))?;
    let labels = samples
        .iter()
        .map(|sample| sample.label)
        .collect::<Vec<_>>();
    let label_count = labels.iter().copied().collect::<BTreeSet<_>>().len();
    if label_count < 2 {
        return Err(
            "CALYX_FSV_MEDIA_EMOTION_LABEL_DOMAIN_MISMATCH: emotion labels must contain at least two values"
                .to_string(),
        );
    }
    Ok(EmotionView {
        features: samples
            .iter()
            .map(|sample| sample.features.clone())
            .collect(),
        labels,
        label_count,
        feature_dim,
    })
}

fn rectangular_dim<'a>(mut rows: impl Iterator<Item = &'a [f32]>) -> Result<usize, String> {
    let Some(first) = rows.next() else {
        return Err("CALYX_FSV_MEDIA_EMOTION_INVALID_FEATURE: no feature rows".to_string());
    };
    let dim = first.len();
    if dim == 0 {
        return Err("CALYX_FSV_MEDIA_EMOTION_INVALID_FEATURE: feature dim is zero".to_string());
    }
    for row in rows {
        if row.len() != dim {
            return Err(
                "CALYX_FSV_MEDIA_EMOTION_INVALID_FEATURE: feature rows are not rectangular"
                    .to_string(),
            );
        }
    }
    Ok(dim)
}

fn grounded_anchor() -> Anchor {
    Anchor {
        kind: AnchorKind::Label("media_audio_emotion".to_string()),
        value: AnchorValue::Text("PH69 verified audio-emotion labels".to_string()),
        source: "PH69 dataset MANIFEST and verified audio-emotion labels".to_string(),
        observed_at: 70,
        confidence: 1.0,
    }
}

struct EmotionView {
    features: Vec<Vec<f32>>,
    labels: Vec<usize>,
    label_count: usize,
    feature_dim: usize,
}
