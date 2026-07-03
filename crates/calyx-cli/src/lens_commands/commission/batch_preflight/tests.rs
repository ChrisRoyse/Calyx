use calyx_core::{LensId, Result, SlotShape, SlotVector};
use calyx_registry::NormPolicy;

use super::*;

/// Deterministic stub: dense unit vectors, fails any batch larger than
/// `max_working_batch`, optionally corrupts batched outputs past
/// `unstable_above` to exercise the stability rejection.
struct StubLens {
    max_working_batch: usize,
    unstable_above: Option<usize>,
}

impl Lens for StubLens {
    fn id(&self) -> LensId {
        LensId::from_parts("batch-preflight-stub", &[], &[], &[])
    }

    fn shape(&self) -> SlotShape {
        SlotShape::Dense(4)
    }

    fn modality(&self) -> Modality {
        Modality::Text
    }

    fn measure(&self, input: &Input) -> Result<SlotVector> {
        let seed = input.bytes.iter().map(|b| *b as f32).sum::<f32>() % 7.0 + 1.0;
        let raw = [seed, 2.0, 3.0, 4.0];
        let norm = raw.iter().map(|v| v * v).sum::<f32>().sqrt();
        Ok(SlotVector::Dense {
            dim: 4,
            data: raw.iter().map(|v| v / norm).collect(),
        })
    }

    fn measure_batch(&self, inputs: &[Input]) -> Result<Vec<SlotVector>> {
        if inputs.len() > self.max_working_batch {
            return Err(CalyxError::lens_unreachable(format!(
                "stub OOM at batch {}",
                inputs.len()
            )));
        }
        let mut rows = inputs
            .iter()
            .map(|input| self.measure(input))
            .collect::<Result<Vec<_>>>()?;
        if let Some(limit) = self.unstable_above
            && inputs.len() > limit
            && let Some(SlotVector::Dense { data, .. }) = rows.last_mut()
        {
            data[0] = -data[0];
        }
        Ok(rows)
    }
}

fn probe(stub: &StubLens, cap: usize) -> Vec<LensForgeBatchProbeLevel> {
    let log_path = std::env::temp_dir().join(format!(
        "calyx-batch-preflight-test-{}-{cap}-{}.jsonl",
        std::process::id(),
        stub.max_working_batch.min(9999)
    ));
    let mut log = ConversionLog::create(log_path.clone()).expect("log");
    let levels = probe_levels(stub, SlotShape::Dense(4), NormPolicy::unit(), cap, &mut log)
        .expect("probe levels");
    let _ = std::fs::remove_file(log_path);
    levels
}

#[test]
fn probe_finds_largest_passing_batch_below_cap() {
    let levels = probe(
        &StubLens {
            max_working_batch: 8,
            unstable_above: None,
        },
        32,
    );
    assert_eq!(largest_passing(&levels), 8);
    let last = levels.last().expect("failing level recorded");
    assert_eq!(last.batch, 16);
    assert!(!last.passed);
    assert!(last.failure.as_deref().unwrap().contains("stub OOM"));
}

#[test]
fn probe_stops_at_cap_when_everything_passes() {
    let levels = probe(
        &StubLens {
            max_working_batch: usize::MAX,
            unstable_above: None,
        },
        16,
    );
    assert_eq!(largest_passing(&levels), 16);
    assert!(levels.iter().all(|level| level.passed));
    assert_eq!(
        levels.iter().map(|level| level.batch).collect::<Vec<_>>(),
        vec![1, 2, 4, 8, 16]
    );
}

#[test]
fn probe_rejects_batch_instability_not_just_errors() {
    let levels = probe(
        &StubLens {
            max_working_batch: usize::MAX,
            unstable_above: Some(2),
        },
        8,
    );
    assert_eq!(largest_passing(&levels), 2);
    let failing = levels.iter().find(|level| !level.passed).expect("failure");
    assert_eq!(failing.batch, 4);
    assert!(
        failing
            .failure
            .as_deref()
            .unwrap()
            .contains("diverged from single-input references")
    );
}

fn flags_for(runtime: CommissionRuntime) -> CommissionFlags {
    CommissionFlags::test_flags(runtime)
}

#[test]
fn batch_1_gate_refuses_unjustified_gpu_batch_1() {
    let mut flags = flags_for(CommissionRuntime::OnnxFp32);
    let error = enforce_batch_1_gate(&flags, Some(1), None).expect_err("gate");
    assert!(
        error
            .to_string()
            .contains("CALYX_LENS_COMMISSION_BATCH1_UNJUSTIFIED")
    );

    // Measured batch-1 without an operator request is refused the same way.
    let error = enforce_batch_1_gate(&flags, None, Some(1)).expect_err("gate");
    assert!(
        error
            .to_string()
            .contains("CALYX_LENS_COMMISSION_BATCH1_UNJUSTIFIED")
    );

    flags.allow_batch_1 = Some("cross-attention reranker scores one pair per run".into());
    enforce_batch_1_gate(&flags, Some(1), None).expect("justified batch-1 passes");
}

#[test]
fn batch_1_gate_ignores_remote_tei_and_real_batches() {
    let flags = flags_for(CommissionRuntime::Tei);
    enforce_batch_1_gate(&flags, Some(1), None).expect("tei is not local gpu");

    let flags = flags_for(CommissionRuntime::OnnxFp32);
    enforce_batch_1_gate(&flags, Some(8), Some(8)).expect("real batch passes");
    enforce_batch_1_gate(&flags, None, Some(64)).expect("measured real batch passes");
}

#[test]
fn resolved_max_batch_prefers_operator_then_measurement() {
    let measured = LensForgeBatchPolicy {
        max_batch_source: "preflight-measured".to_string(),
        batch_1_reason: None,
        preflight_skip_reason: None,
        preflight_cap: Some(32),
        preflight_levels: vec![
            LensForgeBatchProbeLevel {
                batch: 1,
                passed: true,
                elapsed_ms: 5,
                ms_per_row: 5.0,
                min_cosine_vs_single: Some(1.0),
                max_abs_delta_vs_single: Some(0.0),
                failure: None,
            },
            LensForgeBatchProbeLevel {
                batch: 2,
                passed: true,
                elapsed_ms: 6,
                ms_per_row: 3.0,
                min_cosine_vs_single: Some(1.0),
                max_abs_delta_vs_single: Some(0.0),
                failure: None,
            },
        ],
    };
    let mut flags = flags_for(CommissionRuntime::OnnxFp32);
    assert_eq!(resolved_max_batch(&flags, &measured).unwrap(), Some(2));
    flags.max_batch = Some(16);
    assert_eq!(resolved_max_batch(&flags, &measured).unwrap(), Some(16));

    flags.max_batch = None;
    let unverified = LensForgeBatchPolicy {
        max_batch_source: "operator-unverified".to_string(),
        batch_1_reason: None,
        preflight_skip_reason: Some("no GPU on the commissioning box".into()),
        preflight_cap: None,
        preflight_levels: Vec::new(),
    };
    assert_eq!(resolved_max_batch(&flags, &unverified).unwrap(), None);
}
