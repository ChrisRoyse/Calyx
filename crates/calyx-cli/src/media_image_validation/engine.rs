use std::collections::{BTreeMap, BTreeSet};

use calyx_assay::{
    EstimatorKind, MiEstimate, NmiReport, TrustTag, ksg_mi_continuous_discrete_with_anchor,
    ksg_mi_continuous_with_anchor, partitioned_histogram_nmi,
};
use calyx_core::{Anchor, AnchorKind, AnchorValue, SlotId};
use serde::Serialize;

use super::data::{ClassSample, CrossModalSample, ValidationData};
use super::request::MediaImageRequest;

pub(crate) const MEDIA_PANEL_VERSION: u32 = 10;
pub(crate) const IMAGE_CLIP_SLOT: SlotId = SlotId::new(1);
pub(crate) const TRANSCRIPT_SLOT: SlotId = SlotId::new(5);

#[derive(Clone, Debug, Serialize)]
pub(crate) struct MediaImageReport {
    pub(crate) sample_rows: usize,
    pub(crate) dataset_counts: BTreeMap<String, usize>,
    pub(crate) source_sha256_count: usize,
    pub(crate) class_sample_count: usize,
    pub(crate) class_label_count: usize,
    pub(crate) cross_modal_sample_count: usize,
    pub(crate) image_feature_dim: usize,
    pub(crate) cross_image_feature_dim: usize,
    pub(crate) caption_feature_dim: usize,
    pub(crate) image_class_bits: MiEstimate,
    pub(crate) cross_modal_bits: MiEstimate,
    pub(crate) cross_modal_agreement: CrossModalAgreement,
    pub(crate) min_image_bits: f32,
    pub(crate) min_cross_modal_bits: f32,
    pub(crate) panel_version: u32,
    pub(crate) image_slot: u16,
    pub(crate) transcript_slot: u16,
    pub(crate) trigger: String,
    pub(crate) intended_outcome: String,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct CrossModalAgreement {
    pub(crate) dominant_axis_match_rate: f32,
    pub(crate) dominant_axis_nmi: NmiReport,
}

pub(crate) fn evaluate_media_image(
    data: &ValidationData,
    request: &MediaImageRequest,
) -> Result<MediaImageReport, String> {
    let class = class_view(&data.class_samples)?;
    let cross = cross_view(&data.cross_modal_samples)?;
    let anchor = grounded_anchor();
    let image_class_bits =
        ksg_mi_continuous_discrete_with_anchor(&class.features, &class.labels, request.k, &anchor)
            .map_err(|error| error.to_string())?;
    if image_class_bits.bits + f32::EPSILON < request.min_image_bits {
        return Err(format!(
            "CALYX_FSV_MEDIA_IMAGE_BITS_BELOW_THRESHOLD: bits={:.6} threshold={:.6}",
            image_class_bits.bits, request.min_image_bits
        ));
    }
    let cross_modal_bits =
        ksg_mi_continuous_with_anchor(&cross.image, &cross.caption, request.k, &anchor)
            .map_err(|error| error.to_string())?;
    if cross_modal_bits.bits + f32::EPSILON < request.min_cross_modal_bits {
        return Err(format!(
            "CALYX_FSV_MEDIA_CROSS_MODAL_BELOW_THRESHOLD: bits={:.6} threshold={:.6}",
            cross_modal_bits.bits, request.min_cross_modal_bits
        ));
    }
    Ok(MediaImageReport {
        sample_rows: data.total_rows,
        dataset_counts: data.dataset_counts.clone(),
        source_sha256_count: data.source_sha256_count,
        class_sample_count: class.features.len(),
        class_label_count: class.label_count,
        cross_modal_sample_count: cross.image.len(),
        image_feature_dim: class.feature_dim,
        cross_image_feature_dim: cross.image_dim,
        caption_feature_dim: cross.caption_dim,
        image_class_bits,
        cross_modal_bits,
        cross_modal_agreement: cross.agreement,
        min_image_bits: request.min_image_bits,
        min_cross_modal_bits: request.min_cross_modal_bits,
        panel_version: MEDIA_PANEL_VERSION,
        image_slot: IMAGE_CLIP_SLOT.get(),
        transcript_slot: TRANSCRIPT_SLOT.get(),
        trigger: "calyx media image-validate on PH69 ImageNet/CIFAR/COCO samples".to_string(),
        intended_outcome: "persist trusted image-lens bits and image-caption cross-term agreement"
            .to_string(),
    })
}

pub(crate) fn panel_estimate(report: &MediaImageReport) -> MiEstimate {
    let bits = report
        .image_class_bits
        .bits
        .max(report.cross_modal_bits.bits);
    MiEstimate::point(
        bits,
        report
            .class_sample_count
            .min(report.cross_modal_sample_count),
        EstimatorKind::PanelSufficiency,
        TrustTag::Trusted,
    )
}

fn class_view(samples: &[ClassSample]) -> Result<ClassView, String> {
    if samples.len() < 50 {
        return Err(format!(
            "CALYX_FSV_MEDIA_LABEL_DOMAIN_MISMATCH: need at least 50 class samples; got {}",
            samples.len()
        ));
    }
    let feature_dim = rectangular_dim(
        samples
            .iter()
            .map(|sample| sample.image_features.as_slice()),
        "image class features",
    )?;
    let labels = samples
        .iter()
        .map(|sample| sample.class_label)
        .collect::<Vec<_>>();
    let label_count = labels.iter().copied().collect::<BTreeSet<_>>().len();
    if label_count < 2 {
        return Err(
            "CALYX_FSV_MEDIA_LABEL_DOMAIN_MISMATCH: class labels must contain at least two values"
                .to_string(),
        );
    }
    Ok(ClassView {
        features: samples
            .iter()
            .map(|sample| sample.image_features.clone())
            .collect(),
        labels,
        label_count,
        feature_dim,
    })
}

fn cross_view(samples: &[CrossModalSample]) -> Result<CrossView, String> {
    if samples.len() < 50 {
        return Err(format!(
            "CALYX_FSV_MEDIA_CAPTION_INTEGRITY_MISMATCH: need at least 50 cross-modal samples; got {}",
            samples.len()
        ));
    }
    let image_dim = rectangular_dim(
        samples
            .iter()
            .map(|sample| sample.image_features.as_slice()),
        "cross image features",
    )?;
    let caption_dim = rectangular_dim(
        samples
            .iter()
            .map(|sample| sample.caption_features.as_slice()),
        "caption features",
    )?;
    if image_dim != caption_dim {
        return Err(format!(
            "CALYX_FSV_MEDIA_CAPTION_INTEGRITY_MISMATCH: image dim {image_dim} != caption dim {caption_dim}"
        ));
    }
    let image = samples
        .iter()
        .map(|sample| sample.image_features.clone())
        .collect::<Vec<_>>();
    let caption = samples
        .iter()
        .map(|sample| sample.caption_features.clone())
        .collect::<Vec<_>>();
    let image_axis = dominant_axes(&image);
    let caption_axis = dominant_axes(&caption);
    let matches = image_axis
        .iter()
        .zip(&caption_axis)
        .filter(|(left, right)| left == right)
        .count();
    let nmi = partitioned_histogram_nmi(&image_axis, &caption_axis, 20)
        .map_err(|error| error.to_string())?;
    Ok(CrossView {
        image,
        caption,
        image_dim,
        caption_dim,
        agreement: CrossModalAgreement {
            dominant_axis_match_rate: matches as f32 / samples.len() as f32,
            dominant_axis_nmi: nmi,
        },
    })
}

fn rectangular_dim<'a>(
    mut rows: impl Iterator<Item = &'a [f32]>,
    name: &str,
) -> Result<usize, String> {
    let Some(first) = rows.next() else {
        return Err(format!("CALYX_FSV_MEDIA_INVALID_FEATURE: {name} is empty"));
    };
    let dim = first.len();
    if dim == 0 {
        return Err(format!(
            "CALYX_FSV_MEDIA_INVALID_FEATURE: {name} dim is zero"
        ));
    }
    for row in rows {
        if row.len() != dim {
            return Err(format!(
                "CALYX_FSV_MEDIA_INVALID_FEATURE: {name} is not rectangular"
            ));
        }
    }
    Ok(dim)
}

fn dominant_axes(rows: &[Vec<f32>]) -> Vec<f32> {
    rows.iter()
        .map(|row| {
            row.iter()
                .enumerate()
                .max_by(|(_, left), (_, right)| left.total_cmp(right))
                .map(|(idx, _)| idx as f32)
                .unwrap_or(0.0)
        })
        .collect()
}

fn grounded_anchor() -> Anchor {
    Anchor {
        kind: AnchorKind::Label("media_image_class_caption".to_string()),
        value: AnchorValue::Text("PH69 verified image class/caption anchors".to_string()),
        source: "PH69 dataset MANIFEST and COCO/ImageNet/CIFAR labels".to_string(),
        observed_at: 70,
        confidence: 1.0,
    }
}

struct ClassView {
    features: Vec<Vec<f32>>,
    labels: Vec<usize>,
    label_count: usize,
    feature_dim: usize,
}

struct CrossView {
    image: Vec<Vec<f32>>,
    caption: Vec<Vec<f32>>,
    image_dim: usize,
    caption_dim: usize,
    agreement: CrossModalAgreement,
}
