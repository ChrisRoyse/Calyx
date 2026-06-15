use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use calyx_core::{CalyxError, Clock, LensId, Modality, Result, Ts};
use serde::{Deserialize, Serialize};

pub const CALYX_ASSAY_UNAVAILABLE: &str = "CALYX_ASSAY_UNAVAILABLE";
pub const CALYX_ASSAY_INVALID_METRIC: &str = "CALYX_ASSAY_INVALID_METRIC";
pub const CALYX_ANNEAL_DEFICIT_INVALID_CONFIG: &str = "CALYX_ANNEAL_DEFICIT_INVALID_CONFIG";
pub const DEFAULT_DEFICIT_THRESHOLD_BITS: f64 = 0.5;
pub const MODALITY_COVERAGE_THRESHOLD_BITS: f64 = 0.10;

const METRIC_EPSILON: f64 = 1e-12;

pub type ModalityId = Modality;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AnchorId(String);

impl AnchorId {
    pub fn new(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(invalid_config("anchor id must not be empty"));
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for AnchorId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AnchorGap {
    pub anchor_class: String,
    pub entropy_h: f64,
    pub mutual_info_i: f64,
    pub gap: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DeficitMap {
    pub computed_at: Ts,
    pub top_gaps: Vec<AnchorGap>,
    pub underrepresented_modalities: Vec<ModalityId>,
    pub total_bits_deficit: f64,
}

pub trait AssayAttribution {
    fn per_sensor_bits(&self, anchor: &AnchorId) -> Result<Vec<(LensId, f64)>>;
    fn panel_sufficiency(&self, anchor: &AnchorId) -> Result<f64>;
    fn entropy(&self, anchor: &AnchorId) -> Result<f64>;

    fn expected_modalities(&self, _anchor: &AnchorId) -> Result<Vec<ModalityId>> {
        Ok(Vec::new())
    }

    fn lens_modality(&self, _lens: &LensId) -> Result<Option<ModalityId>> {
        Ok(None)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DeficitLocalizerConfig {
    pub deficit_threshold_bits: f64,
    pub modality_min_bits: f64,
}

impl Default for DeficitLocalizerConfig {
    fn default() -> Self {
        Self {
            deficit_threshold_bits: DEFAULT_DEFICIT_THRESHOLD_BITS,
            modality_min_bits: MODALITY_COVERAGE_THRESHOLD_BITS,
        }
    }
}

pub struct DeficitLocalizer<'a> {
    clock: &'a dyn Clock,
    config: DeficitLocalizerConfig,
}

impl<'a> DeficitLocalizer<'a> {
    pub fn new(clock: &'a dyn Clock) -> Self {
        Self {
            clock,
            config: DeficitLocalizerConfig::default(),
        }
    }

    pub fn with_config(clock: &'a dyn Clock, config: DeficitLocalizerConfig) -> Result<Self> {
        validate_metric("deficit_threshold_bits", config.deficit_threshold_bits)?;
        validate_metric("modality_min_bits", config.modality_min_bits)?;
        Ok(Self { clock, config })
    }

    pub fn config(&self) -> DeficitLocalizerConfig {
        self.config
    }

    pub fn localize(
        &self,
        assay: &dyn AssayAttribution,
        anchor: &AnchorId,
        panel: &[LensId],
    ) -> Result<DeficitMap> {
        self.localize_many(assay, std::slice::from_ref(anchor), panel)
    }

    pub fn localize_many(
        &self,
        assay: &dyn AssayAttribution,
        anchors: &[AnchorId],
        panel: &[LensId],
    ) -> Result<DeficitMap> {
        let panel_set: BTreeSet<LensId> = panel.iter().copied().collect();
        let mut gaps = Vec::with_capacity(anchors.len());
        let mut modality_union = Vec::new();
        let mut total_bits_deficit = 0.0;

        for anchor in anchors {
            let entropy_h = assay_metric(assay.entropy(anchor), "entropy", anchor)?;
            let sufficiency =
                assay_metric(assay.panel_sufficiency(anchor), "panel_sufficiency", anchor)?;
            let mutual_info_i = if panel_set.is_empty() {
                0.0
            } else {
                validate_dpi(anchor, entropy_h, sufficiency)?
            };
            let gap = gap_bits(entropy_h, mutual_info_i);
            total_bits_deficit += gap;
            gaps.push(AnchorGap {
                anchor_class: anchor.as_str().to_string(),
                entropy_h,
                mutual_info_i,
                gap,
            });

            if gap > 0.0 {
                let bits = assay_metric(assay.per_sensor_bits(anchor), "per_sensor_bits", anchor)?;
                let missing = self.underrepresented_modalities(assay, anchor, &panel_set, bits)?;
                append_unique(&mut modality_union, missing);
            }
        }

        gaps.sort_by(|left, right| {
            right
                .gap
                .total_cmp(&left.gap)
                .then_with(|| left.anchor_class.cmp(&right.anchor_class))
        });
        Ok(DeficitMap {
            computed_at: self.clock.now(),
            top_gaps: gaps,
            underrepresented_modalities: modality_union,
            total_bits_deficit: normalize_zero(total_bits_deficit),
        })
    }

    fn underrepresented_modalities(
        &self,
        assay: &dyn AssayAttribution,
        anchor: &AnchorId,
        panel: &BTreeSet<LensId>,
        bits: Vec<(LensId, f64)>,
    ) -> Result<Vec<ModalityId>> {
        let expected = assay_metric(
            assay.expected_modalities(anchor),
            "expected_modalities",
            anchor,
        )?;
        if expected.is_empty() {
            return Ok(Vec::new());
        }
        let bits_by_lens = collect_bits(bits)?;
        let mut covered = Vec::new();
        for lens in panel {
            let Some(modality) = assay_metric(assay.lens_modality(lens), "lens_modality", anchor)?
            else {
                continue;
            };
            let bits = *bits_by_lens.get(lens).unwrap_or(&0.0);
            if bits > self.config.modality_min_bits {
                push_unique(&mut covered, modality);
            }
        }
        Ok(expected
            .into_iter()
            .filter(|modality| !covered.contains(modality))
            .collect())
    }
}

pub fn has_deficit(map: &DeficitMap, threshold: f64) -> bool {
    threshold.is_finite() && map.total_bits_deficit > threshold
}

pub fn top_gap_description(map: &DeficitMap) -> String {
    let Some(top) = map.top_gaps.first() else {
        return "no anchor deficit localized".to_string();
    };
    let modality_clause = if top.gap <= 0.0 {
        "no positive deficit localized".to_string()
    } else if map.underrepresented_modalities.is_empty() {
        format!(
            "all expected modalities have >{:.3} bits",
            MODALITY_COVERAGE_THRESHOLD_BITS
        )
    } else {
        let names = map
            .underrepresented_modalities
            .iter()
            .map(|modality| format!("'{}'", modality_name(*modality)))
            .collect::<Vec<_>>()
            .join(", ");
        format!("no lens covers {names} modality")
    };
    format!(
        "Anchor class '{}' has gap {:.3} bits; {}",
        top.anchor_class, top.gap, modality_clause
    )
}

fn collect_bits(bits: Vec<(LensId, f64)>) -> Result<BTreeMap<LensId, f64>> {
    let mut out = BTreeMap::new();
    for (lens, value) in bits {
        let value = validate_metric("per_sensor_bits", value)?;
        out.insert(lens, value);
    }
    Ok(out)
}

fn assay_metric<T>(result: Result<T>, metric: &'static str, anchor: &AnchorId) -> Result<T> {
    result.map_err(|error| CalyxError {
        code: CALYX_ASSAY_UNAVAILABLE,
        message: format!(
            "Assay attribution unavailable while reading {metric} for anchor {anchor}: {}: {}",
            error.code, error.message
        ),
        remediation: "restore Assay attribution data before proposing a lens",
    })
}

fn validate_metric(metric: &'static str, value: f64) -> Result<f64> {
    if !value.is_finite() || value < -METRIC_EPSILON {
        return Err(CalyxError {
            code: CALYX_ASSAY_INVALID_METRIC,
            message: format!("{metric} must be finite and non-negative, got {value}"),
            remediation: "re-measure Assay attribution metrics before localizing deficits",
        });
    }
    Ok(normalize_zero(value))
}

fn validate_dpi(anchor: &AnchorId, entropy_h: f64, mutual_info_i: f64) -> Result<f64> {
    if mutual_info_i > entropy_h + METRIC_EPSILON {
        return Err(CalyxError {
            code: CALYX_ASSAY_INVALID_METRIC,
            message: format!(
                "panel_sufficiency for anchor {anchor} violates DPI: I={mutual_info_i} > H={entropy_h}"
            ),
            remediation: "recompute Assay sufficiency; MI must not exceed entropy",
        });
    }
    Ok(mutual_info_i.min(entropy_h))
}

fn gap_bits(entropy_h: f64, mutual_info_i: f64) -> f64 {
    normalize_zero((entropy_h - mutual_info_i).max(0.0))
}

fn append_unique(target: &mut Vec<ModalityId>, values: Vec<ModalityId>) {
    for value in values {
        push_unique(target, value);
    }
}

fn push_unique(target: &mut Vec<ModalityId>, value: ModalityId) {
    if !target.contains(&value) {
        target.push(value);
    }
}

fn normalize_zero(value: f64) -> f64 {
    if value.abs() <= METRIC_EPSILON {
        0.0
    } else {
        value
    }
}

fn invalid_config(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_ANNEAL_DEFICIT_INVALID_CONFIG,
        message: message.into(),
        remediation: "fix the Anneal deficit localization request before running",
    }
}

fn modality_name(modality: ModalityId) -> &'static str {
    match modality {
        Modality::Text => "text",
        Modality::Code => "code",
        Modality::Image => "image",
        Modality::Audio => "audio",
        Modality::Video => "video",
        Modality::Protein => "protein",
        Modality::Dna => "dna",
        Modality::Molecule => "molecule",
        Modality::Structured => "structured",
        Modality::Mixed => "mixed",
    }
}
