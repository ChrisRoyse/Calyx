use std::env;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use calyx_core::{CalyxError, Input, Lens, LensId, Modality, Result, SlotShape, SlotVector};
use calyx_registry::frozen::sha256_digest;
use calyx_registry::{
    FrozenLensContract, INGEST_MICROBATCH_INPUT_OVERHEAD_BYTES, IngestMicrobatchConfig,
    IngestMicrobatchController, IngestMicrobatchStats, IngestPanelReadout, LensDType, NormPolicy,
    Registry, estimate_microbatch_bytes,
};
use serde::Serialize;

#[test]
fn ph57_ingest_microbatch_bounds_and_breaker_readback() {
    let root = fsv_root();
    fs::create_dir_all(&root).unwrap();

    let happy = happy_admission_readback();
    let edges = edge_readback();
    let scenario = stalled_lens_scenario();
    let metrics_text = scenario.metrics_text.clone();
    let readback = FsvReadback {
        issue: 590,
        trigger: "bounded ingest microbatch with stalled lens".to_string(),
        intended_outcome:
            "buffer bytes stay within cap, CALYX_BACKPRESSURE fires over cap, breaker trips, good lens acks continue"
                .to_string(),
        happy,
        edges,
        scenario,
    };

    let json_path = root.join("ph57-ingest-microbatch-readback.json");
    let prom_path = root.join("ph57-ingest-microbatch.prom");
    fs::write(&json_path, serde_json::to_vec_pretty(&readback).unwrap()).unwrap();
    fs::write(&prom_path, metrics_text.as_bytes()).unwrap();

    println!("PH57_INGEST_MICROBATCH_FSV_ROOT={}", root.display());
    println!("PH57_INGEST_MICROBATCH_JSON={}", json_path.display());
    println!("PH57_INGEST_MICROBATCH_PROM={}", prom_path.display());
}

fn happy_admission_readback() -> AdmissionReadback {
    let inputs = [
        Input::new(Modality::Text, b"aa".to_vec()).with_pointer("ptr-a"),
        Input::new(Modality::Text, b"bbbb".to_vec()).with_pointer("p"),
    ];
    let expected_bytes = 2 * INGEST_MICROBATCH_INPUT_OVERHEAD_BYTES + 2 + 4 + 5 + 1;
    let controller =
        IngestMicrobatchController::new(IngestMicrobatchConfig::new(160).with_high_water(100));
    let before = controller.stats();
    let computed_bytes = estimate_microbatch_bytes(&inputs);
    assert_eq!(computed_bytes, expected_bytes);

    let permit = controller.admit(&inputs).unwrap();
    let during = controller.stats();
    assert_eq!(permit.bytes(), expected_bytes);
    assert_eq!(during.current_buffer_bytes, expected_bytes);
    drop(permit);
    let after = controller.stats();
    assert_eq!(after.current_buffer_bytes, 0);

    AdmissionReadback {
        hand_expected_bytes: expected_bytes,
        computed_bytes,
        before,
        during,
        after,
    }
}

fn edge_readback() -> Vec<EdgeReadback> {
    vec![empty_edge(), exact_cap_edge(), over_cap_edge()]
}

fn empty_edge() -> EdgeReadback {
    let controller = IngestMicrobatchController::new(IngestMicrobatchConfig::new(0));
    let before = controller.stats();
    let permit = controller.admit(&[]).unwrap();
    let during = controller.stats();
    drop(permit);
    let after = controller.stats();
    EdgeReadback {
        name: "empty_batch_zero_cap".to_string(),
        hand_expected_bytes: 0,
        error_code: None,
        before,
        during,
        after,
    }
}

fn exact_cap_edge() -> EdgeReadback {
    let input = [Input::new(Modality::Text, vec![7; 16])];
    let expected = INGEST_MICROBATCH_INPUT_OVERHEAD_BYTES + 16;
    let controller = IngestMicrobatchController::new(IngestMicrobatchConfig::new(expected));
    let before = controller.stats();
    let permit = controller.admit(&input).unwrap();
    let during = controller.stats();
    assert_eq!(permit.bytes(), expected);
    drop(permit);
    let after = controller.stats();
    EdgeReadback {
        name: "exact_cap_admitted".to_string(),
        hand_expected_bytes: expected,
        error_code: None,
        before,
        during,
        after,
    }
}

fn over_cap_edge() -> EdgeReadback {
    let input = [Input::new(Modality::Text, vec![8; 17])];
    let expected = INGEST_MICROBATCH_INPUT_OVERHEAD_BYTES + 17;
    let controller = IngestMicrobatchController::new(IngestMicrobatchConfig::new(expected - 1));
    let before = controller.stats();
    let error = controller.admit(&input).unwrap_err();
    let after = controller.stats();
    assert_eq!(error.code, "CALYX_BACKPRESSURE");
    assert_eq!(after.current_buffer_bytes, 0);
    EdgeReadback {
        name: "over_cap_rejected".to_string(),
        hand_expected_bytes: expected,
        error_code: Some(error.code.to_string()),
        before: before.clone(),
        during: before,
        after,
    }
}

fn stalled_lens_scenario() -> ScenarioReadback {
    let mut registry = Registry::new();
    let stalled = StalledLens::new("ph57-stalled");
    let good = LengthLens::new("ph57-good");
    let stalled_calls = Arc::clone(&stalled.calls);
    let good_calls = Arc::clone(&good.calls);
    let stalled_id = registry
        .register_frozen(stalled.clone(), stalled.contract.clone())
        .unwrap();
    let good_id = registry
        .register_frozen(good.clone(), good.contract.clone())
        .unwrap();
    let inputs = [
        Input::new(Modality::Text, b"aa".to_vec()),
        Input::new(Modality::Text, b"bbbb".to_vec()),
    ];
    let expected_batch_bytes = 2 * INGEST_MICROBATCH_INPUT_OVERHEAD_BYTES + 2 + 4;
    let controller =
        IngestMicrobatchController::new(IngestMicrobatchConfig::new(512).with_breaker(1, 10_000));
    let before = controller.stats();
    let oversize = [Input::new(Modality::Text, vec![9; 600])];
    let overcap_error = controller.admit(&oversize).unwrap_err();
    assert_eq!(overcap_error.code, "CALYX_BACKPRESSURE");
    let after_overcap = controller.stats();
    assert_eq!(after_overcap.current_buffer_bytes, 0);
    assert_eq!(after_overcap.backpressure_total, 1);

    let first = registry
        .measure_ingest_microbatch(&[stalled_id, good_id], &inputs, &controller, 100)
        .unwrap();
    assert_eq!(first.acknowledged_inputs, 2);

    let mut acknowledged_total = first.acknowledged_inputs;
    let mut last = first.clone();
    for now_ms in 101..105 {
        last = registry
            .measure_ingest_microbatch(&[stalled_id, good_id], &inputs, &controller, now_ms)
            .unwrap();
        acknowledged_total += last.acknowledged_inputs;
    }

    let after = controller.stats();
    let dense_lengths = measured_lengths(&last, good_id);
    assert_eq!(dense_lengths, vec![2.0, 4.0]);
    assert_eq!(stalled_calls.load(Ordering::SeqCst), 1);
    assert_eq!(good_calls.load(Ordering::SeqCst), 5);
    assert_eq!(after.current_buffer_bytes, 0);
    assert!(after.buffer_high_water_bytes <= after.cap_bytes);
    assert_eq!(after.breaker_trips_total, 1);
    assert_eq!(after.lens_timeouts_total, 1);

    ScenarioReadback {
        lens_ids: vec![stalled_id, good_id],
        hand_expected_batch_bytes: expected_batch_bytes,
        before,
        first_panel: first,
        final_panel: last,
        after,
        stalled_lens_calls: stalled_calls.load(Ordering::SeqCst),
        good_lens_calls: good_calls.load(Ordering::SeqCst),
        acknowledged_inputs_total: acknowledged_total,
        good_dense_lengths: dense_lengths,
        overcap_error_code: overcap_error.code.to_string(),
        after_overcap,
        metrics_text: controller.metrics_text(),
    }
}

fn measured_lengths(readout: &IngestPanelReadout, good_id: LensId) -> Vec<f32> {
    readout
        .outcomes
        .iter()
        .find(|outcome| outcome.lens_id == good_id)
        .unwrap()
        .vectors
        .iter()
        .map(|vector| match vector {
            SlotVector::Dense { data, .. } => data[0],
            other => panic!("unexpected vector {other:?}"),
        })
        .collect()
}

fn fsv_root() -> PathBuf {
    env::var_os("CALYX_FSV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/fsv-issue590-ingest-microbatch"))
}

#[derive(Serialize)]
struct FsvReadback {
    issue: u32,
    trigger: String,
    intended_outcome: String,
    happy: AdmissionReadback,
    edges: Vec<EdgeReadback>,
    scenario: ScenarioReadback,
}

#[derive(Serialize)]
struct AdmissionReadback {
    hand_expected_bytes: usize,
    computed_bytes: usize,
    before: IngestMicrobatchStats,
    during: IngestMicrobatchStats,
    after: IngestMicrobatchStats,
}

#[derive(Serialize)]
struct EdgeReadback {
    name: String,
    hand_expected_bytes: usize,
    error_code: Option<String>,
    before: IngestMicrobatchStats,
    during: IngestMicrobatchStats,
    after: IngestMicrobatchStats,
}

#[derive(Serialize)]
struct ScenarioReadback {
    lens_ids: Vec<LensId>,
    hand_expected_batch_bytes: usize,
    before: IngestMicrobatchStats,
    first_panel: IngestPanelReadout,
    final_panel: IngestPanelReadout,
    after: IngestMicrobatchStats,
    stalled_lens_calls: usize,
    good_lens_calls: usize,
    acknowledged_inputs_total: usize,
    good_dense_lengths: Vec<f32>,
    overcap_error_code: String,
    after_overcap: IngestMicrobatchStats,
    metrics_text: String,
}

#[derive(Clone)]
struct LengthLens {
    contract: FrozenLensContract,
    calls: Arc<AtomicUsize>,
}

impl LengthLens {
    fn new(name: &str) -> Self {
        Self {
            contract: contract(name),
            calls: Arc::new(AtomicUsize::new(0)),
        }
    }
}

impl Lens for LengthLens {
    fn id(&self) -> LensId {
        self.contract.lens_id()
    }

    fn shape(&self) -> SlotShape {
        SlotShape::Dense(1)
    }

    fn modality(&self) -> Modality {
        Modality::Text
    }

    fn measure(&self, input: &Input) -> Result<SlotVector> {
        Ok(SlotVector::Dense {
            dim: 1,
            data: vec![input.bytes.len() as f32],
        })
    }

    fn measure_batch(&self, inputs: &[Input]) -> Result<Vec<SlotVector>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        inputs.iter().map(|input| self.measure(input)).collect()
    }
}

#[derive(Clone)]
struct StalledLens {
    contract: FrozenLensContract,
    calls: Arc<AtomicUsize>,
}

impl StalledLens {
    fn new(name: &str) -> Self {
        Self {
            contract: contract(name),
            calls: Arc::new(AtomicUsize::new(0)),
        }
    }
}

impl Lens for StalledLens {
    fn id(&self) -> LensId {
        self.contract.lens_id()
    }

    fn shape(&self) -> SlotShape {
        SlotShape::Dense(1)
    }

    fn modality(&self) -> Modality {
        Modality::Text
    }

    fn measure(&self, _input: &Input) -> Result<SlotVector> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(CalyxError::lens_unreachable("synthetic stalled lens"))
    }

    fn measure_batch(&self, _inputs: &[Input]) -> Result<Vec<SlotVector>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(CalyxError::lens_unreachable("synthetic stalled lens"))
    }
}

fn contract(name: &str) -> FrozenLensContract {
    FrozenLensContract::new(
        name,
        sha256_digest(&[name.as_bytes(), b"weights"]),
        sha256_digest(&[name.as_bytes(), b"corpus"]),
        SlotShape::Dense(1),
        Modality::Text,
        LensDType::F32,
        NormPolicy::None,
    )
}
