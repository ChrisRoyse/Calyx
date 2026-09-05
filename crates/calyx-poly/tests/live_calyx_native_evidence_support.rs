use calyx_anneal::{
    GoodhartReport, HeldOutSet, JTerms, JValue, JWeights, RegressionReport, RegressionResult,
};
use calyx_assay::{EnsembleConfig, EnsembleLensInput};
use calyx_aster::vault::AsterVault;
use calyx_core::{AnchorKind, Clock, CxId, SlotId};
use calyx_lodestar::{KernelGraphParams, KernelParams, RecallQuery, RecallTestParams};
use calyx_mincut::{AgreementEdge, FrequencyEntry};
use calyx_poly::Domain;
use calyx_poly::calibration_refit::{
    CalibrationRefitObservation, CalibrationRefitReport, CalibrationRefitRequest,
    compute_calibration_refit_report,
};
use calyx_poly::kernel_recall_admission::{
    ComputedKernelRecall, ComputedKernelRecallRequest, measure_computed_kernel_recall,
};
use calyx_poly::live_calyx_native_evidence::{
    LiveCalyxNativeEvidenceRequest, StoredLiveCalyxNativeEvidence,
    record_live_calyx_native_evidence,
};
use calyx_poly::panel_sufficiency::{
    PolyPanelSufficiencyReport, PolyPanelSufficiencyRequest, compute_panel_sufficiency_report,
};

pub struct EvidenceParts {
    pub panel: PolyPanelSufficiencyReport,
    pub kernel_recall: ComputedKernelRecall,
    pub calibration: CalibrationRefitReport,
    pub goodhart: GoodhartReport,
    pub held_out: HeldOutSet,
    pub mistakes: RegressionReport,
}

pub fn record_strong_evidence<C: Clock>(
    vault: &AsterVault<C>,
    domain: &str,
    horizon_bucket: &str,
    panel_version: u32,
    as_of_millis: u64,
) -> StoredLiveCalyxNativeEvidence {
    let parts = strong_evidence_parts(domain, horizon_bucket, panel_version, as_of_millis);
    record_evidence_parts(vault, &parts)
}

pub fn strong_evidence_parts(
    domain: &str,
    horizon_bucket: &str,
    panel_version: u32,
    as_of_millis: u64,
) -> EvidenceParts {
    EvidenceParts {
        panel: strong_panel(domain, panel_version),
        kernel_recall: strong_kernel_recall(),
        calibration: strong_calibration(domain, horizon_bucket, as_of_millis),
        goodhart: GoodhartReport {
            passed: true,
            violations: Vec::new(),
            p_goodhart_increment: 0.0,
            j_train_delta: 0.05,
            j_heldout_delta: Some(0.04),
            in_region_frac: Some(0.96),
            warnings: Vec::new(),
        },
        held_out: HeldOutSet::sealed("issue1292-held-out", 80, j_value(1.0), j_value(1.04)),
        mistakes: RegressionReport::new(vec![RegressionResult {
            cx_id: cx(90),
            old_prediction: 0.60,
            observed: 1.0,
            old_surprise: 0.40,
            new_prediction: 0.92,
            new_surprise: 0.08,
            recurred: false,
            anchor: AnchorKind::Reward,
            prediction_error: None,
        }]),
    }
}

pub fn record_evidence_parts<C: Clock>(
    vault: &AsterVault<C>,
    parts: &EvidenceParts,
) -> StoredLiveCalyxNativeEvidence {
    record_live_calyx_native_evidence(
        vault,
        LiveCalyxNativeEvidenceRequest {
            panel: &parts.panel,
            kernel_recall: &parts.kernel_recall,
            calibration: &parts.calibration,
            goodhart: &parts.goodhart,
            goodhart_held_out: &parts.held_out,
            mistake_replay: &parts.mistakes,
        },
    )
    .expect("record typed live CalyxNative evidence")
}

pub fn strong_panel(domain: &str, panel_version: u32) -> PolyPanelSufficiencyReport {
    let labels = (0..80).map(|index| index % 2 == 0).collect::<Vec<_>>();
    let lenses = (0..10)
        .map(|lens_index| {
            let samples = labels
                .iter()
                .enumerate()
                .map(|(sample_index, label)| {
                    let signal = if *label { 1.0 } else { -1.0 };
                    let jitter = ((sample_index + lens_index) % 7) as f32 * 0.002;
                    vec![signal * (1.0 - lens_index as f32 * 0.025) + jitter]
                })
                .collect();
            EnsembleLensInput::new(
                format!("strong_{lens_index}"),
                SlotId::new(lens_index as u16 + 1),
                samples,
            )
        })
        .collect();
    compute_panel_sufficiency_report(&PolyPanelSufficiencyRequest {
        domain: domain.to_string(),
        panel_id: "issue1292-panel".to_string(),
        panel_version,
        lenses,
        labels,
        groups: None,
        config: EnsembleConfig {
            source: "issue1292-measured-panel".to_string(),
            min_gate_lenses: 10,
            min_marginal_bits: 0.05,
            max_redundancy: 0.95,
            nmi_bins: 8,
        },
    })
    .expect("compute strong panel-sufficiency report")
}

pub fn strong_calibration(
    domain: &str,
    horizon_bucket: &str,
    as_of_millis: u64,
) -> CalibrationRefitReport {
    let mut observations = Vec::new();
    for index in 0..15 {
        observations.push(calibration_observation(
            0.60,
            index % 5 != 0,
            as_of_millis,
            index,
        ));
    }
    for index in 0..15 {
        observations.push(calibration_observation(
            0.40,
            index % 5 == 0,
            as_of_millis,
            15 + index,
        ));
    }
    compute_calibration_refit_report(&CalibrationRefitRequest {
        out_dir: std::path::Path::new("unused"),
        domain,
        horizon_bucket,
        previous_version: None,
        as_of_millis,
        observations,
    })
    .expect("compute improving calibration refit")
}

fn calibration_observation(
    p_raw: f64,
    outcome_yes: bool,
    as_of_millis: u64,
    offset: u64,
) -> CalibrationRefitObservation {
    CalibrationRefitObservation {
        p_raw,
        outcome_yes,
        resolved_at_millis: as_of_millis - 30_000 + offset,
    }
}

pub fn strong_kernel_recall() -> ComputedKernelRecall {
    let agreements = vec![
        edge(1, 2),
        edge(2, 3),
        edge(3, 1),
        edge(4, 5),
        edge(5, 6),
        edge(6, 4),
        edge(7, 8),
        edge(8, 9),
        edge(9, 7),
        edge(10, 11),
        edge(11, 12),
        edge(12, 10),
    ];
    let frequencies = (1..=12)
        .map(|id| FrequencyEntry {
            cx_id: cx(id),
            frequency: 1.0,
        })
        .collect::<Vec<_>>();
    let anchors = vec![cx(1), cx(4), cx(7), cx(10)];
    let members = [cx(1), cx(4), cx(7), cx(10)];
    let mut corpus = members
        .iter()
        .enumerate()
        .map(|(index, member)| RecallQuery {
            cx_id: *member,
            vector: one_hot(index, 12),
        })
        .collect::<Vec<_>>();
    for offset in 0..8u8 {
        corpus.push(RecallQuery {
            cx_id: cx(20 + offset),
            vector: one_hot(offset as usize % members.len(), 12),
        });
    }
    let kernel_params = KernelParams {
        kernel_graph: KernelGraphParams {
            target_fraction: 1.0,
            max_groundedness_distance: 64,
            ..KernelGraphParams::default()
        },
        ..KernelParams::default()
    };
    let recall_params = RecallTestParams {
        held_out_fraction: 1.0,
        top_k: 1,
        rng_seed: 1_292,
        min_recall_ratio: 0.95,
    };
    measure_computed_kernel_recall(&ComputedKernelRecallRequest {
        domain: Domain::Crypto,
        corpus: &corpus,
        agreements: &agreements,
        frequencies: &frequencies,
        anchors: &anchors,
        kernel_params: &kernel_params,
        recall_params: &recall_params,
    })
    .expect("measure computed FVS-kernel recall")
}

fn j_value(j: f64) -> JValue {
    JValue {
        j,
        terms: JTerms {
            w1_info: 0.2,
            w2_n_eff: 0.2,
            w3_sufficiency: 0.2,
            w4_kernel_recall: 0.2,
            w5_oracle_accuracy: 0.2,
            w6_mistake_rate: 0.0,
            w7_compression: 0.0,
            w8_coverage: 0.0,
            p_redundant: 0.0,
            p_ungrounded: 0.0,
            p_goodhart: 0.0,
        },
        dpi_ceiling: 1.0,
        dpi_headroom: 0.0,
        provisional_excluded: 0,
        weights: JWeights::default(),
    }
}

fn cx(id: u8) -> CxId {
    let mut bytes = [0; 16];
    bytes[15] = id;
    CxId::from_bytes(bytes)
}

fn edge(src: u8, dst: u8) -> AgreementEdge {
    AgreementEdge {
        src: cx(src),
        dst: cx(dst),
        agreement: 0.9,
        directional_confidence: 0.9,
    }
}

fn one_hot(index: usize, dimensions: usize) -> Vec<f32> {
    let mut vector = vec![0.0; dimensions];
    vector[index % dimensions] = 1.0;
    vector
}
