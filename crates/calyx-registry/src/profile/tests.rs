use super::*;
use calyx_core::{Lens, Modality, SlotShape};

use crate::runtime::algorithmic::AlgorithmicLens;

#[test]
fn profiles_algorithmic_lens_with_real_numbers() {
    let mut registry = Registry::new();
    let id = registry
        .register(AlgorithmicLens::byte_features(
            "profile-test",
            Modality::Text,
        ))
        .unwrap();
    let probes = profile_probes();

    let card = profile_lens(&registry, id, &probes).unwrap();

    println!("{}", serde_json::to_string_pretty(&card).unwrap());
    assert_eq!(card.coverage.requested, probes.len());
    assert_eq!(card.coverage.failed, 0);
    assert!(card.spread.participation_ratio > 0.0);
    assert!(card.spread.normalized_participation_ratio > 0.0);
    assert_eq!(card.signal, None);
    assert_eq!(card.signal_source, MetricSource::AssayPending);
    assert!(card.proxy_signal.is_finite());
    assert_eq!(card.differentiation, None);
    assert_eq!(card.differentiation_source, MetricSource::AssayPending);
    assert!(card.proxy_differentiation.is_finite());
    assert!(card.cost.ms_per_input >= 0.0);
}

#[test]
fn assay_owned_metrics_serialize_as_null_until_attached() {
    let mut registry = Registry::new();
    let id = registry
        .register(AlgorithmicLens::byte_features(
            "profile-null-assay-fields",
            Modality::Text,
        ))
        .unwrap();

    let card = profile_lens(&registry, id, &profile_probes()).unwrap();
    let json = serde_json::to_value(&card).unwrap();

    assert!(json["signal"].is_null());
    assert_eq!(json["signal_source"], "assay_pending");
    assert!(json["proxy_signal"].as_f64().unwrap().is_finite());
    assert!(json["differentiation"].is_null());
    assert_eq!(json["differentiation_source"], "assay_pending");
    assert!(json["proxy_differentiation"].as_f64().unwrap().is_finite());
}

#[test]
fn collapsed_lens_is_flagged_low_spread() {
    let mut registry = Registry::new();
    let id = registry.register(CollapsedLens).unwrap();

    let card = profile_lens(&registry, id, &profile_probes()).unwrap();

    assert!(card.low_spread);
    assert_eq!(card.spread.participation_ratio, 0.0);
    assert_eq!(card.spread.mean_pairwise_distance, 0.0);
}

#[test]
fn wrong_modality_counts_as_failed_coverage() {
    let mut registry = Registry::new();
    let id = registry
        .register(AlgorithmicLens::byte_features(
            "profile-coverage",
            Modality::Text,
        ))
        .unwrap();
    let probes = vec![
        ProfileProbe::new(Input::new(Modality::Text, b"ok".to_vec())),
        ProfileProbe::new(Input::new(Modality::Image, vec![1, 2, 3])),
    ];

    let card = profile_lens(&registry, id, &probes).unwrap();

    assert_eq!(card.coverage.measured, 1);
    assert_eq!(card.coverage.failed, 1);
    assert_eq!(card.coverage.rate, 0.5);
}

#[test]
fn empty_probe_set_fails_closed() {
    let registry = Registry::new();
    let error = profile_lens(&registry, LensId::from_bytes([7; 16]), &[]).unwrap_err();

    assert_eq!(error.code, "CALYX_ASSAY_INSUFFICIENT_SAMPLES");
}

fn profile_probes() -> Vec<ProfileProbe> {
    vec![
        ProfileProbe::labeled(Input::new(Modality::Text, b"alpha words".to_vec()), "words"),
        ProfileProbe::labeled(Input::new(Modality::Text, b"beta phrase".to_vec()), "words"),
        ProfileProbe::labeled(
            Input::new(Modality::Text, b"12345 67890".to_vec()),
            "digits",
        ),
        ProfileProbe::labeled(
            Input::new(Modality::Text, b"98765 43210".to_vec()),
            "digits",
        ),
    ]
}

struct CollapsedLens;

impl Lens for CollapsedLens {
    fn id(&self) -> LensId {
        LensId::from_bytes([8; 16])
    }

    fn shape(&self) -> SlotShape {
        SlotShape::Dense(4)
    }

    fn modality(&self) -> Modality {
        Modality::Text
    }

    fn measure(&self, _input: &Input) -> Result<SlotVector> {
        Ok(SlotVector::Dense {
            dim: 4,
            data: vec![1.0, 0.0, 0.0, 0.0],
        })
    }
}
