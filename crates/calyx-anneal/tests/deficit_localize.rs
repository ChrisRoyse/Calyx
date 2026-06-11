use std::collections::BTreeMap;

use calyx_anneal::{
    AnchorId, AssayAttribution, CALYX_ASSAY_UNAVAILABLE, DEFAULT_DEFICIT_THRESHOLD_BITS,
    DeficitLocalizer, has_deficit, top_gap_description,
};
use calyx_core::{CalyxError, FixedClock, LensId, Modality, Result};
use proptest::prelude::*;

const TEST_TS: u64 = 1_785_500_418;

#[test]
fn low_sufficiency_reports_expected_gap_and_deficit() {
    let anchor = anchor("outcome_positive");
    let lens = lens(1);
    let assay = assay(vec![FixtureAnchor::new(
        anchor.clone(),
        2.0,
        0.4,
        vec![Modality::Text],
        vec![(lens, 0.4, Modality::Text)],
    )]);

    let map = localizer().localize(&assay, &anchor, &[lens]).unwrap();

    assert_eq!(map.computed_at, TEST_TS);
    assert_eq!(map.total_bits_deficit, 1.6);
    assert_eq!(map.top_gaps[0].gap, 1.6);
    assert_eq!(map.top_gaps[0].entropy_h, 2.0);
    assert_eq!(map.top_gaps[0].mutual_info_i, 0.4);
    assert!(has_deficit(&map, DEFAULT_DEFICIT_THRESHOLD_BITS));
}

#[test]
fn well_covered_panel_stays_below_default_deficit_threshold() {
    let anchor = anchor("outcome_ok");
    let lens = lens(2);
    let assay = assay(vec![FixtureAnchor::new(
        anchor.clone(),
        1.0,
        0.95,
        vec![Modality::Text],
        vec![(lens, 0.95, Modality::Text)],
    )]);

    let map = localizer().localize(&assay, &anchor, &[lens]).unwrap();

    assert!((map.total_bits_deficit - 0.05).abs() < 1e-12);
    assert!((map.top_gaps[0].gap - 0.05).abs() < 1e-12);
    assert!(!has_deficit(&map, DEFAULT_DEFICIT_THRESHOLD_BITS));
}

#[test]
fn empty_panel_deficit_equals_entropy_and_all_expected_modalities_are_missing() {
    let anchor = anchor("outcome_empty");
    let lens = lens(3);
    let assay = assay(vec![FixtureAnchor::new(
        anchor.clone(),
        1.25,
        0.9,
        vec![Modality::Text, Modality::Audio],
        vec![(lens, 0.9, Modality::Text)],
    )]);

    let map = localizer().localize(&assay, &anchor, &[]).unwrap();

    assert_eq!(map.top_gaps[0].mutual_info_i, 0.0);
    assert_eq!(map.total_bits_deficit, 1.25);
    assert_eq!(
        map.underrepresented_modalities,
        vec![Modality::Text, Modality::Audio]
    );
}

#[test]
fn single_lens_only_covers_its_modality_above_point_one_bits() {
    let anchor = anchor("outcome_audio");
    let text = lens(4);
    let audio = lens(5);
    let assay = assay(vec![FixtureAnchor::new(
        anchor.clone(),
        2.0,
        0.3,
        vec![Modality::Text, Modality::Audio],
        vec![(text, 0.3, Modality::Text), (audio, 0.10, Modality::Audio)],
    )]);

    let map = localizer()
        .localize(&assay, &anchor, &[text, audio])
        .unwrap();

    assert_eq!(map.top_gaps[0].anchor_class, "outcome_audio");
    assert_eq!(map.underrepresented_modalities, vec![Modality::Audio]);
    assert!(top_gap_description(&map).contains("'audio'"));
}

#[test]
fn zero_entropy_has_zero_gap_and_no_deficit() {
    let anchor = anchor("outcome_zero");
    let lens = lens(6);
    let assay = assay(vec![FixtureAnchor::new(
        anchor.clone(),
        0.0,
        0.0,
        vec![Modality::Structured],
        vec![(lens, 0.0, Modality::Structured)],
    )]);

    let map = localizer().localize(&assay, &anchor, &[lens]).unwrap();

    assert_eq!(map.total_bits_deficit, 0.0);
    assert!(map.underrepresented_modalities.is_empty());
    assert!(!has_deficit(&map, DEFAULT_DEFICIT_THRESHOLD_BITS));
    assert!(top_gap_description(&map).contains("no positive deficit localized"));
}

#[test]
fn assay_error_fails_closed_without_zero_gap_fake() {
    let anchor = anchor("missing_anchor");
    let lens = lens(7);
    let error = localizer()
        .localize(&FailingAssay, &anchor, &[lens])
        .unwrap_err();

    assert_eq!(error.code, CALYX_ASSAY_UNAVAILABLE);
    assert!(error.message.contains("entropy"));
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(32))]

    #[test]
    fn dpi_bounded_metrics_never_emit_negative_gap(
        entropy in 0.0f64..8.0,
        ratio in 0.0f64..1.0,
    ) {
        let anchor = anchor("proptest_outcome");
        let lens = lens(8);
        let sufficiency = entropy * ratio;
        let assay = assay(vec![FixtureAnchor::new(
            anchor.clone(),
            entropy,
            sufficiency,
            vec![Modality::Text],
            vec![(lens, sufficiency, Modality::Text)],
        )]);

        let map = localizer().localize(&assay, &anchor, &[lens]).unwrap();

        prop_assert!(map.top_gaps[0].gap >= 0.0);
        prop_assert!(map.total_bits_deficit >= 0.0);
    }
}

fn localizer() -> DeficitLocalizer<'static> {
    static CLOCK: FixedClock = FixedClock::new(TEST_TS);
    DeficitLocalizer::new(&CLOCK)
}

fn anchor(value: &str) -> AnchorId {
    AnchorId::new(value).unwrap()
}

fn lens(byte: u8) -> LensId {
    LensId::from_bytes([byte; 16])
}

fn assay(anchors: Vec<FixtureAnchor>) -> FixtureAssay {
    FixtureAssay::new(anchors)
}

struct FixtureAssay {
    anchors: BTreeMap<AnchorId, FixtureAnchor>,
    modalities: BTreeMap<LensId, Modality>,
}

impl FixtureAssay {
    fn new(anchors: Vec<FixtureAnchor>) -> Self {
        let mut map = BTreeMap::new();
        let mut modalities = BTreeMap::new();
        for anchor in anchors {
            for (lens, _bits, modality) in &anchor.bits {
                modalities.insert(*lens, *modality);
            }
            map.insert(anchor.anchor_id.clone(), anchor);
        }
        Self {
            anchors: map,
            modalities,
        }
    }

    fn anchor(&self, anchor: &AnchorId) -> Result<&FixtureAnchor> {
        self.anchors
            .get(anchor)
            .ok_or_else(|| assay_unavailable("missing fixture anchor"))
    }
}

impl AssayAttribution for FixtureAssay {
    fn per_sensor_bits(&self, anchor: &AnchorId) -> Result<Vec<(LensId, f64)>> {
        Ok(self
            .anchor(anchor)?
            .bits
            .iter()
            .map(|(lens, bits, _modality)| (*lens, *bits))
            .collect())
    }

    fn panel_sufficiency(&self, anchor: &AnchorId) -> Result<f64> {
        Ok(self.anchor(anchor)?.sufficiency)
    }

    fn entropy(&self, anchor: &AnchorId) -> Result<f64> {
        Ok(self.anchor(anchor)?.entropy)
    }

    fn expected_modalities(&self, anchor: &AnchorId) -> Result<Vec<Modality>> {
        Ok(self.anchor(anchor)?.expected_modalities.clone())
    }

    fn lens_modality(&self, lens: &LensId) -> Result<Option<Modality>> {
        Ok(self.modalities.get(lens).copied())
    }
}

struct FixtureAnchor {
    anchor_id: AnchorId,
    entropy: f64,
    sufficiency: f64,
    expected_modalities: Vec<Modality>,
    bits: Vec<(LensId, f64, Modality)>,
}

impl FixtureAnchor {
    fn new(
        anchor_id: AnchorId,
        entropy: f64,
        sufficiency: f64,
        expected_modalities: Vec<Modality>,
        bits: Vec<(LensId, f64, Modality)>,
    ) -> Self {
        Self {
            anchor_id,
            entropy,
            sufficiency,
            expected_modalities,
            bits,
        }
    }
}

struct FailingAssay;

impl AssayAttribution for FailingAssay {
    fn per_sensor_bits(&self, _anchor: &AnchorId) -> Result<Vec<(LensId, f64)>> {
        Err(assay_unavailable("per sensor unavailable"))
    }

    fn panel_sufficiency(&self, _anchor: &AnchorId) -> Result<f64> {
        Err(assay_unavailable("panel sufficiency unavailable"))
    }

    fn entropy(&self, _anchor: &AnchorId) -> Result<f64> {
        Err(assay_unavailable("entropy unavailable"))
    }
}

fn assay_unavailable(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_ASSAY_UNAVAILABLE,
        message: message.into(),
        remediation: "test fixture",
    }
}
