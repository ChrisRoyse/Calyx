use std::collections::BTreeMap;

use calyx_anneal::{
    AlgorithmicKind, AnchorGap, CALYX_ANNEAL_CANDIDATE_INVALID_DEFICIT, CandidateLens,
    CorpusSampleSource, DeficitMap, MAX_SYNTHESIS_CORPUS_SAMPLE, describe, synthesize,
    synthesize_algorithmic, synthesize_from_source,
};
use calyx_core::{
    Anchor, AnchorKind, AnchorValue, CalyxError, Constellation, CxFlags, CxId, InputRef, LedgerRef,
    Modality, Result, VaultId,
};
use proptest::prelude::*;

#[test]
fn temporal_deficit_synthesizes_time_lag_candidate() {
    let deficit = deficit("temporal_latency", 2.0, 0.4, vec![Modality::Structured]);
    let corpus = vec![
        constellation(1, 100, Modality::Structured, &[("time_lag", 1.0)]),
        constellation(2, 160, Modality::Structured, &[("time_lag", 2.0)]),
    ];

    let candidate = synthesize_algorithmic(&deficit, &corpus).unwrap();

    assert!(matches!(
        candidate,
        CandidateLens::Algorithmic {
            kind: AlgorithmicKind::TimeLag,
            ..
        }
    ));
    assert!(describe(&candidate).contains("TimeLag"));
}

#[test]
fn audio_gap_falls_back_to_commission_spec() {
    let deficit = deficit("speaker_identity", 1.8, 0.2, vec![Modality::Audio]);
    let corpus = vec![constellation(3, 100, Modality::Audio, &[])];

    let candidate = synthesize(&deficit, &corpus).unwrap();

    match candidate {
        CandidateLens::Commission { spec } => {
            assert_eq!(spec.target_modality, Modality::Audio);
            assert!(spec.endpoint.is_none());
            assert!(spec.description.contains("audio"));
        }
        other => panic!("expected commission spec, got {other:?}"),
    }
}

#[test]
fn empty_corpus_has_no_algorithmic_candidate_and_commissions() {
    let deficit = deficit("speaker_identity", 1.8, 0.2, vec![Modality::Audio]);

    assert!(synthesize_algorithmic(&deficit, &[]).is_none());
    assert!(matches!(
        synthesize(&deficit, &[]).unwrap(),
        CandidateLens::Commission { .. }
    ));
}

#[test]
fn no_underrepresented_modality_uses_pca_default_candidate() {
    let deficit = deficit("outcome_positive", 2.0, 0.8, Vec::new());
    let corpus = vec![constellation(
        4,
        100,
        Modality::Structured,
        &[("score", 0.4)],
    )];

    let candidate = synthesize(&deficit, &corpus).unwrap();

    assert!(matches!(
        candidate,
        CandidateLens::Algorithmic {
            kind: AlgorithmicKind::Pca,
            ..
        }
    ));
}

#[test]
fn corpus_sample_source_failure_is_fail_closed() {
    let deficit = deficit("temporal_latency", 2.0, 0.4, vec![Modality::Structured]);
    let error = synthesize_from_source(&deficit, &FailingCorpus).unwrap_err();

    assert_eq!(error.code, "CALYX_ASTER_CF_UNAVAILABLE");
    assert!(error.message.contains("corpus sample unavailable"));
}

#[test]
fn corpus_sample_is_capped_at_one_thousand_rows() {
    let deficit = deficit("temporal_latency", 2.0, 0.4, vec![Modality::Structured]);
    let corpus = (0..1005)
        .map(|idx| {
            constellation(
                (idx + 1) as u8,
                idx,
                Modality::Structured,
                &[("time_lag", 1.0)],
            )
        })
        .collect::<Vec<_>>();

    let candidate = synthesize(&deficit, &corpus).unwrap();

    match candidate {
        CandidateLens::Algorithmic { params, .. } => {
            assert_eq!(params.sample_count, MAX_SYNTHESIS_CORPUS_SAMPLE);
        }
        other => panic!("expected algorithmic candidate, got {other:?}"),
    }
}

#[test]
fn invalid_deficit_metrics_fail_closed() {
    let mut deficit = deficit("bad", 1.0, 0.2, vec![Modality::Text]);
    deficit.top_gaps[0].gap = f64::NAN;

    let error = synthesize(&deficit, &[constellation(9, 1, Modality::Text, &[])]).unwrap_err();

    assert_eq!(error.code, CALYX_ANNEAL_CANDIDATE_INVALID_DEFICIT);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(32))]

    #[test]
    fn valid_deficit_always_returns_candidate(
        entropy in 0.1f64..8.0,
        ratio in 0.0f64..1.0,
        use_audio in any::<bool>(),
    ) {
        let sufficiency = entropy * ratio;
        let modalities = if use_audio {
            vec![Modality::Audio]
        } else {
            Vec::new()
        };
        let deficit = deficit("proptest_outcome", entropy, sufficiency, modalities);
        let corpus = vec![constellation(10, 1, Modality::Structured, &[("score", 0.1)])];

        let candidate = synthesize(&deficit, &corpus).unwrap();

        let valid_candidate = matches!(
            candidate,
            CandidateLens::Algorithmic { .. } | CandidateLens::Commission { .. }
        );
        prop_assert!(valid_candidate);
    }
}

fn deficit(
    anchor: &str,
    entropy_h: f64,
    mutual_info_i: f64,
    underrepresented_modalities: Vec<Modality>,
) -> DeficitMap {
    let gap = (entropy_h - mutual_info_i).max(0.0);
    DeficitMap {
        computed_at: 1_845_000_419,
        top_gaps: vec![AnchorGap {
            anchor_class: anchor.to_string(),
            entropy_h,
            mutual_info_i,
            gap,
        }],
        underrepresented_modalities,
        total_bits_deficit: gap,
    }
}

fn constellation(
    id_byte: u8,
    created_at: u64,
    modality: Modality,
    scalars: &[(&str, f64)],
) -> Constellation {
    let mut scalar_map = BTreeMap::new();
    for (key, value) in scalars {
        scalar_map.insert((*key).to_string(), *value);
    }
    let mut metadata = BTreeMap::new();
    metadata.insert("fixture".to_string(), "issue419".to_string());
    Constellation {
        cx_id: CxId::from_bytes([id_byte; 16]),
        vault_id: "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse::<VaultId>().unwrap(),
        panel_version: 1,
        created_at,
        input_ref: InputRef {
            hash: [id_byte; 32],
            pointer: None,
            redacted: false,
        },
        modality,
        slots: BTreeMap::new(),
        scalars: scalar_map,
        metadata,
        anchors: vec![Anchor {
            kind: AnchorKind::Label("fixture".to_string()),
            value: AnchorValue::Enum("ok".to_string()),
            source: "issue419".to_string(),
            observed_at: created_at,
            confidence: 1.0,
        }],
        provenance: LedgerRef {
            seq: u64::from(id_byte),
            hash: [id_byte; 32],
        },
        flags: CxFlags::default(),
    }
}

struct FailingCorpus;

impl CorpusSampleSource for FailingCorpus {
    fn read_corpus_sample(&self, _max_rows: usize) -> Result<Vec<Constellation>> {
        Err(CalyxError {
            code: "CALYX_ASTER_CF_UNAVAILABLE",
            message: "synthetic corpus read failed".to_string(),
            remediation: "test fixture",
        })
    }
}
