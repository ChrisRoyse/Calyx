use calyx_core::{Asymmetry, Modality, QuantPolicy, SlotShape};

use super::{
    DERIVATION_CHANGELOG, DerivationImpact, DerivationVerdict, LENS_DERIVATION_VERSION, classify,
};
use crate::frozen::NormPolicy;
use crate::spec::default_recall_delta;
use crate::{LensRuntime, LensSpec};

// ---------------------------------------------------------------------------
// Changelog invariants
// ---------------------------------------------------------------------------

#[test]
fn changelog_is_contiguous_from_version_one() {
    let mut expected = 1;
    for step in DERIVATION_CHANGELOG {
        assert_eq!(step.from, expected, "changelog gap before {}", step.commit);
        assert_eq!(
            step.to,
            step.from + 1,
            "step {} must advance exactly one version",
            step.commit
        );
        expected = step.to;
    }
    assert_eq!(
        expected, LENS_DERIVATION_VERSION,
        "LENS_DERIVATION_VERSION must equal the last changelog step's `to`"
    );
}

#[test]
fn every_step_names_a_commit_issue_and_affected_runtime() {
    for step in DERIVATION_CHANGELOG {
        assert!(!step.commit.is_empty(), "step v{} has no commit", step.from);
        assert!(!step.issue.is_empty(), "step v{} has no issue", step.from);
        assert!(
            !step.summary.is_empty(),
            "step v{} has no summary",
            step.from
        );
        assert!(
            !step.affects.is_empty(),
            "step v{} affects no runtime; a derivation change that moves nothing is not a step",
            step.from
        );
    }
}

// ---------------------------------------------------------------------------
// Classification
// ---------------------------------------------------------------------------

#[test]
fn current_version_classifies_as_current() {
    assert_eq!(
        classify(Some(LENS_DERIVATION_VERSION)),
        DerivationVerdict::Current
    );
    assert!(classify(Some(LENS_DERIVATION_VERSION)).is_re_attestable());
}

#[test]
fn newer_record_classifies_as_ahead_and_is_not_re_attestable() {
    let verdict = classify(Some(LENS_DERIVATION_VERSION + 1));
    assert_eq!(
        verdict,
        DerivationVerdict::Ahead {
            persisted: LENS_DERIVATION_VERSION + 1
        }
    );
    assert!(
        !verdict.is_re_attestable(),
        "a record from a newer binary must never be re-stamped by an older one"
    );
}

#[test]
fn unstamped_record_spans_the_whole_changelog() {
    let verdict = classify(None);
    assert_eq!(verdict.steps().len(), DERIVATION_CHANGELOG.len());
    assert_eq!(
        verdict.impact(),
        Some(DerivationImpact::MeasurementChanging),
        "the changelog contains a measurement-changing step, so an unstamped record \
         must be classified by the strictest impact"
    );
    assert!(!verdict.is_re_attestable());
}

#[test]
fn stale_record_spans_only_the_steps_after_it() {
    let verdict = classify(Some(3));
    assert_eq!(verdict.steps().len(), 1);
    assert_eq!(verdict.steps()[0].commit, "19dcc189");
    assert_eq!(
        verdict.impact(),
        Some(DerivationImpact::MeasurementChanging)
    );
}

#[test]
fn identity_only_span_is_re_attestable() {
    // v1 -> v3 crosses only the two identity-only steps.
    let verdict = DerivationVerdict::Stale {
        steps: DERIVATION_CHANGELOG
            .iter()
            .filter(|step| step.to <= 3)
            .copied()
            .collect(),
    };
    assert_eq!(verdict.impact(), Some(DerivationImpact::IdentityOnly));
    assert!(
        verdict.is_re_attestable(),
        "a span of purely identity-only steps preserves the measurement"
    );
}

#[test]
fn runtime_filter_excludes_steps_that_cannot_move_this_runtime() {
    // The v3->v4 TEI request-policy step touches TeiHttp only.
    let algorithmic = classify(Some(3)).for_runtime("Algorithmic");
    assert!(
        algorithmic.steps().is_empty(),
        "an algorithmic lens is not re-identified by the TEI request-policy change"
    );

    let tei = classify(Some(3)).for_runtime("TeiHttp");
    assert_eq!(tei.steps().len(), 1);
    assert_eq!(tei.impact(), Some(DerivationImpact::MeasurementChanging));
}

#[test]
fn measurement_changing_span_never_offers_re_attestation() {
    for persisted in [None, Some(1), Some(2), Some(3)] {
        let verdict = classify(persisted);
        assert!(
            !verdict.is_re_attestable(),
            "span from {persisted:?} crosses 19dcc189 and must never be re-stamped"
        );
        let error = super::lens_derivation_stale(&verdict, "fixture");
        assert_eq!(error.code, super::LENS_DERIVATION_STALE);
        assert!(
            error.message.contains("MUST be re-measured"),
            "message must state the honest remedy, got: {}",
            error.message
        );
        assert!(
            error.remediation.contains("must not be re-stamped"),
            "remediation must forbid re-stamping, got: {}",
            error.remediation
        );
    }
}

#[test]
fn stale_error_names_the_specific_policy_delta_not_an_opaque_hash() {
    let error = super::lens_derivation_stale(&classify(Some(3)), "lens=semantic-gte-base-tei");
    assert!(error.message.contains("v3 -> v4"), "{}", error.message);
    assert!(error.message.contains("truncate=true"), "{}", error.message);
    assert!(error.message.contains("19dcc189"), "{}", error.message);
    assert!(error.message.contains("#1902/#1952"), "{}", error.message);
    assert!(
        error.message.contains("semantic-gte-base-tei"),
        "{}",
        error.message
    );
}

#[test]
fn template_stale_error_uses_the_template_specific_code() {
    let error = super::panel_template_derivation_stale(&classify(Some(3)), "template=legal-v1");
    assert_eq!(error.code, "CALYX_PANEL_TEMPLATE_DERIVATION_STALE");
    assert!(error.message.contains("template=legal-v1"));
}

// ---------------------------------------------------------------------------
// Golden identity regression
// ---------------------------------------------------------------------------
//
// These pins are the tripwire required by #1943. Each entry fixes the
// `lens_id` of a representative frozen spec under the CURRENT derivation.
// Any change to an input of `LensSpec::lens_id()` moves one or more of these
// and fails this test.
//
// If this test fails, DO NOT edit the expected hash to match. Instead:
//   1. bump `LENS_DERIVATION_VERSION`,
//   2. append a `DerivationStep` recording the commit, the affected runtimes,
//      and — honestly — whether the change is IdentityOnly or
//      MeasurementChanging,
//   3. then update the pin below in the same commit.
// Editing the pin alone silently orphans every persisted corpus.

fn fixture_spec(name: &str, runtime: LensRuntime, output: SlotShape) -> LensSpec {
    LensSpec {
        name: name.to_string(),
        runtime,
        output,
        modality: Modality::Text,
        weights_sha256: [0x11; 32],
        corpus_hash: [0x22; 32],
        norm_policy: NormPolicy::unit(),
        max_batch: Some(4),
        axis: None,
        asymmetry: Asymmetry::None,
        quant_default: QuantPolicy::turboquant_default(),
        truncate_dim: None,
        recall_delta: default_recall_delta(),
        retrieval_only: false,
        excluded_from_dedup: false,
    }
}

fn golden_fixtures() -> Vec<(&'static str, LensSpec)> {
    vec![
        (
            "tei-http",
            fixture_spec(
                "golden-tei-http",
                LensRuntime::TeiHttp {
                    endpoint: "http://127.0.0.1:8088/embed".to_string(),
                },
                SlotShape::Dense(768),
            ),
        ),
        (
            "algorithmic",
            fixture_spec(
                "golden-algorithmic",
                LensRuntime::Algorithmic {
                    kind: "byte-features".to_string(),
                },
                SlotShape::Dense(16),
            ),
        ),
        (
            "onnx",
            fixture_spec(
                "golden-onnx",
                LensRuntime::Onnx {
                    model_id: "golden/onnx-model".to_string(),
                    files: vec!["onnx/model_fp16.onnx".into()],
                },
                SlotShape::Dense(384),
            ),
        ),
        (
            "onnx-colbert",
            fixture_spec(
                "golden-onnx-colbert",
                LensRuntime::OnnxColbert {
                    model_id: "golden/colbert".to_string(),
                    files: vec!["onnx/model_fp16.onnx".into()],
                },
                SlotShape::Multi { token_dim: 128 },
            ),
        ),
        (
            "static-lookup",
            fixture_spec(
                "golden-static-lookup",
                LensRuntime::StaticLookup {
                    embeddings_file: "static/embeddings.bin".into(),
                    tokenizer: "static/tokenizer.json".into(),
                    dim: 256,
                },
                SlotShape::Dense(256),
            ),
        ),
    ]
}

/// Pinned `lens_id` per fixture under derivation v4.
const GOLDEN_LENS_IDS: &[(&str, &str)] = &[
    ("tei-http", "3672f02d09dd1a8ddbd8cfb1680c2fab"),
    ("algorithmic", "935d04c38fad844ce8cab265bf127b35"),
    ("onnx", "6f8f44413c5a9b1a9fa6cc9be8f627ad"),
    ("onnx-colbert", "4259a63cf942f6402fddfc7c6525b137"),
    ("static-lookup", "2424dc122d1ef3691ee8f8c81c938d9d"),
];

#[test]
fn golden_identity_pins_hold_under_the_current_derivation() {
    let mut observed = Vec::new();
    for (key, spec) in golden_fixtures() {
        observed.push((key, spec.lens_id().to_string()));
    }
    let expected: Vec<(&str, String)> = GOLDEN_LENS_IDS
        .iter()
        .map(|(key, id)| (*key, (*id).to_string()))
        .collect();
    assert_eq!(
        observed, expected,
        "\nLENS IDENTITY DERIVATION CHANGED.\n\
         Do not edit the pins to match. Bump LENS_DERIVATION_VERSION (currently \
         {LENS_DERIVATION_VERSION}), append a DerivationStep classifying the change as \
         IdentityOnly or MeasurementChanging, and update the pins in the same commit.\n\
         Observed: {observed:#?}\n"
    );
}
