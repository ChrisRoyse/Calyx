use std::collections::BTreeMap;

use calyx_anneal::{
    AlgorithmicKind, AnchorGap, CALYX_ANNEAL_CANDIDATE_INVALID_DEFICIT, CandidateLens,
    CorpusSampleSource, DeficitMap, MAX_SYNTHESIS_CORPUS_SAMPLE, describe,
    ranked_conversion_targets, synthesize, synthesize_algorithmic, synthesize_from_source,
};
use calyx_core::{
    Anchor, AnchorKind, AnchorValue, CalyxError, Constellation, CxFlags, CxId, InputRef, LedgerRef,
    Modality, Result, VaultId,
};
use proptest::prelude::*;
use serde_json::json;

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
            assert_eq!(spec.axis, "speaker_identity");
            assert_eq!(spec.model_id.as_deref(), Some("Xenova/wav2vec2-base-960h"));
            assert_eq!(spec.suggested_targets[0].hf_id, "Xenova/wav2vec2-base-960h");
            assert!(spec.description.contains("audio"));
        }
        other => panic!("expected commission spec, got {other:?}"),
    }
}

#[test]
fn protein_gap_ranks_lensforge_target_with_axis_and_expected_bits() {
    let deficit = deficit("protein_binding", 2.5, 0.4, vec![Modality::Protein]);

    let first = ranked_conversion_targets(&deficit);
    let second = ranked_conversion_targets(&deficit);

    assert_eq!(first, second);
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].hf_id, "facebook/esm2_t6_8M_UR50D");
    assert_eq!(first[0].modality, Modality::Protein);
    assert_eq!(first[0].axis, "protein_sequence");
    assert_eq!(first[0].formats, vec!["adapter"]);
    assert!((first[0].expected_bits - 2.1).abs() <= 1e-9);
    assert!(first[0].expected_cost.vram_mb > 0.0);
    assert!(first[0].expected_bits_per_ms > 0.0);
}

#[test]
fn text_gap_ranks_small_cpu_target_by_expected_density() {
    let deficit = deficit("semantic_text", 2.5, 0.4, vec![Modality::Text]);

    let targets = ranked_conversion_targets(&deficit);
    let first = targets.first().expect("text targets");
    let bge = targets
        .iter()
        .find(|target| target.hf_id == "Xenova/bge-small-en-v1.5")
        .expect("bge target");

    assert_eq!(first.hf_id, "minishlab/potion-base-8M");
    assert_eq!(first.expected_cost.placement, calyx_core::Placement::Cpu);
    assert!(first.expected_bits < bge.expected_bits);
    assert!(first.expected_bits_per_vram_mb.is_none());
    assert!(first.expected_bits_per_ms > bge.expected_bits_per_ms);
}

#[test]
fn scientific_text_gap_includes_scincl_fp32_domain_target() {
    let deficit = deficit("scientific_domain", 2.5, 0.4, vec![Modality::Text]);

    let targets = ranked_conversion_targets(&deficit);
    let scincl = targets
        .iter()
        .find(|target| target.hf_id == "malteos/scincl")
        .expect("SciNCL domain target");

    assert_eq!(scincl.axis, "scientific_text");
    assert_eq!(scincl.formats, vec!["onnx-fp32"]);
    assert_eq!(scincl.expected_cost.placement, calyx_core::Placement::Gpu);
    assert!(scincl.expected_cost.ms_per_input > 1.0);
    assert!(scincl.expected_bits > 0.0);
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
fn value_density_prior_beats_control_flow_when_both_are_plausible() {
    let deficit = deficit(
        "runtime_value_control_flow",
        2.0,
        0.4,
        vec![Modality::Structured],
    );
    let corpus = vec![constellation(
        0x51,
        100,
        Modality::Structured,
        &[("return_value", 4.0), ("branch", 1.0)],
    )];

    let candidate = synthesize_algorithmic(&deficit, &corpus).unwrap();

    let (kind, features) = algorithmic(&candidate);
    assert_eq!(kind, AlgorithmicKind::ValueDivergence);
    assert_eq!(features["channel_prior"], "runtime_value");
    assert_eq!(
        features["independence_contract"],
        "max_pairwise_corr<=0.600000"
    );
}

#[test]
fn control_flow_channel_is_selected_when_no_value_channel_exists() {
    let deficit = deficit("control_flow_branch_path", 2.0, 0.4, vec![Modality::Code]);
    let corpus = vec![constellation(
        0x52,
        100,
        Modality::Code,
        &[("branch", 1.0), ("cfg", 1.0)],
    )];

    let candidate = synthesize_algorithmic(&deficit, &corpus).unwrap();

    let (kind, features) = algorithmic(&candidate);
    assert_eq!(kind, AlgorithmicKind::ControlFlow);
    assert_eq!(features["channel_prior"], "control_flow");
}

#[test]
fn exception_value_channel_records_value_complementarity() {
    let deficit = deficit("exception_error_path", 2.0, 0.4, vec![Modality::Structured]);
    let corpus = vec![constellation(
        0x53,
        100,
        Modality::Structured,
        &[("exception", 1.0), ("error", 1.0)],
    )];

    let candidate = synthesize_algorithmic(&deficit, &corpus).unwrap();

    let (kind, features) = algorithmic(&candidate);
    assert_eq!(kind, AlgorithmicKind::ExceptionValue);
    assert_eq!(features["channel_prior"], "exception_value");
    assert_eq!(features["complementary_channel"], "value_divergence");
}

#[test]
#[ignore = "manual FSV writes source-of-truth artifacts"]
fn issue774_channel_prior_manual_fsv() {
    let root =
        std::env::var("CALYX_ISSUE774_FSV_ROOT").expect("CALYX_ISSUE774_FSV_ROOT is required");
    std::fs::create_dir_all(&root).expect("create issue774 fsv root");
    let value_control = synthesize_algorithmic(
        &deficit(
            "runtime_value_control_flow",
            2.0,
            0.4,
            vec![Modality::Structured],
        ),
        &[constellation(
            0x61,
            100,
            Modality::Structured,
            &[("return_value", 4.0), ("branch", 1.0)],
        )],
    )
    .unwrap();
    let control_only = synthesize_algorithmic(
        &deficit("control_flow_branch_path", 2.0, 0.4, vec![Modality::Code]),
        &[constellation(
            0x62,
            100,
            Modality::Code,
            &[("branch", 1.0), ("cfg", 1.0)],
        )],
    )
    .unwrap();
    let exception_only = synthesize_algorithmic(
        &deficit("exception_error_path", 2.0, 0.4, vec![Modality::Structured]),
        &[constellation(
            0x63,
            100,
            Modality::Structured,
            &[("exception", 1.0), ("error", 1.0)],
        )],
    )
    .unwrap();
    let mut invalid = deficit("runtime_value", 2.0, 0.4, vec![Modality::Structured]);
    invalid.top_gaps[0].mutual_info_i = f64::NAN;
    let invalid_code = synthesize(
        &invalid,
        &[constellation(
            0x64,
            100,
            Modality::Structured,
            &[("return_value", 4.0)],
        )],
    )
    .unwrap_err()
    .code;
    let readback = json!({
        "source_of_truth": "issue774 channel-prior JSON bytes written by calyx-anneal FSV test in a manual verification run",
        "happy_path": {
            "before": {"anchor": "runtime_value_control_flow", "signals": ["return_value", "branch"]},
            "expected_after": {"kind": "ValueDivergence", "channel_prior": "runtime_value"},
            "after": algorithmic_summary(&value_control),
        },
        "edge_cases": [
            {
                "name": "control_flow_without_value_stays_control_flow",
                "before": {"anchor": "control_flow_branch_path", "signals": ["branch", "cfg"]},
                "expected_after": {"kind": "ControlFlow", "channel_prior": "control_flow"},
                "after": algorithmic_summary(&control_only),
            },
            {
                "name": "exception_value_records_complementarity",
                "before": {"anchor": "exception_error_path", "signals": ["exception", "error"]},
                "expected_after": {"kind": "ExceptionValue", "channel_prior": "exception_value", "complementary_channel": "value_divergence"},
                "after": algorithmic_summary(&exception_only),
            },
            {
                "name": "invalid_metric_fails_closed",
                "before": {"mutual_info_i": "NaN"},
                "expected_after": {"error_code": CALYX_ANNEAL_CANDIDATE_INVALID_DEFICIT},
                "after": {"error_code": invalid_code},
            }
        ]
    });
    assert_eq!(
        readback["happy_path"]["after"],
        readback["happy_path"]["expected_after"]
    );
    assert_eq!(
        readback["edge_cases"][0]["after"],
        readback["edge_cases"][0]["expected_after"]
    );
    assert_eq!(
        readback["edge_cases"][1]["after"],
        readback["edge_cases"][1]["expected_after"]
    );
    assert_eq!(
        readback["edge_cases"][2]["after"],
        readback["edge_cases"][2]["expected_after"]
    );
    let path = std::path::Path::new(&root).join("issue774-channel-prior-readback.json");
    std::fs::write(&path, serde_json::to_vec_pretty(&readback).unwrap()).unwrap();
    println!("ISSUE774_CHANNEL_PRIOR_READBACK={}", path.display());
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

fn algorithmic(candidate: &CandidateLens) -> (AlgorithmicKind, &BTreeMap<String, String>) {
    match candidate {
        CandidateLens::Algorithmic { kind, params } => (*kind, &params.features),
        other => panic!("expected algorithmic candidate, got {other:?}"),
    }
}

fn algorithmic_summary(candidate: &CandidateLens) -> serde_json::Value {
    let (kind, features) = algorithmic(candidate);
    let mut value = json!({
        "kind": format!("{kind:?}"),
        "channel_prior": features["channel_prior"],
    });
    if let Some(channel) = features.get("complementary_channel") {
        value["complementary_channel"] = json!(channel);
    }
    value
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
