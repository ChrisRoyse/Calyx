use super::*;
use calyx_core::{Asymmetry, Modality, QuantPolicy, SlotShape};

use crate::frozen::{FrozenLensContract, LensDType, NormPolicy, sha256_digest};
use crate::spec::LensRuntime;

#[test]
fn plain_register_fails_closed_without_frozen_contract() {
    let mut registry = Registry::new();
    let lens = OneDimLens::new("plain-register");
    let id = lens.id();

    let error = registry.register(lens).unwrap_err();

    assert_eq!(error.code, "CALYX_LENS_FROZEN_VIOLATION");
    assert!(!registry.contains(id));
}

#[test]
fn registry_measures_registered_lens() {
    let mut registry = Registry::new();
    let lens = OneDimLens::new("one-dim");
    let id = registry
        .register_frozen(lens.clone(), lens.contract.clone())
        .unwrap();
    let input = Input::new(Modality::Text, b"abc".to_vec());

    let vector = registry.measure(id, &input).unwrap();

    assert_eq!(
        vector,
        SlotVector::Dense {
            dim: 1,
            data: vec![3.0]
        }
    );
}

#[test]
fn registry_records_determinism_proof_or_exemption() {
    let mut registry = Registry::new();
    let exempt = OneDimLens::new("contract-only");
    let exempt_id = registry
        .register_frozen(exempt.clone(), exempt.contract.clone())
        .unwrap();
    let probed = OneDimLens::new("probe-verified");
    let probe = Input::new(Modality::Text, b"deterministic-probe".to_vec());
    let probed_id = registry
        .register_frozen_with_probe(probed.clone(), probed.contract.clone(), &probe)
        .unwrap();

    assert_eq!(
        registry.determinism_proof(exempt_id),
        Some(DeterminismProof::ContractOnlyExemption)
    );
    assert_eq!(
        registry.determinism_proof(probed_id),
        Some(DeterminismProof::ProbeVerified)
    );
}

#[test]
fn frozen_lens_snapshots_return_weight_hashes_in_id_order() {
    let mut registry = Registry::new();
    let left = OneDimLens::new("snapshot-left");
    let right = OneDimLens::new("snapshot-right");
    let left_id = registry
        .register_frozen(left.clone(), left.contract.clone())
        .unwrap();
    let right_id = registry
        .register_frozen(right.clone(), right.contract.clone())
        .unwrap();

    let snapshots = registry.frozen_lens_snapshots();

    assert_eq!(snapshots.len(), 2);
    assert!(
        snapshots
            .windows(2)
            .all(|pair| pair[0].lens_id < pair[1].lens_id)
    );
    assert_eq!(
        registry.frozen_contract(left_id).unwrap().weights_sha256(),
        snapshots
            .iter()
            .find(|snapshot| snapshot.lens_id == left_id)
            .unwrap()
            .weights_sha256
    );
    assert_eq!(
        registry.frozen_contract(right_id).unwrap().weights_sha256(),
        snapshots
            .iter()
            .find(|snapshot| snapshot.lens_id == right_id)
            .unwrap()
            .weights_sha256
    );
}

#[test]
fn registry_finds_runtime_lens_by_spec_id() {
    let mut registry = Registry::new();
    let lens = OneDimLens::new("spec-id-runtime");
    let spec = lens_spec_for(&lens);
    let spec_id = spec.lens_id();
    let runtime_id = registry
        .register_frozen_with_spec(lens.clone(), lens.contract.clone(), spec.clone())
        .unwrap();

    assert_eq!(registry.find_lens_by_spec_id(spec_id), Some(runtime_id));
    assert_eq!(registry.lens_spec(runtime_id), Some(&spec));
}

#[test]
fn registry_rejects_wrong_modality() {
    let mut registry = Registry::new();
    let lens = OneDimLens::new("wrong-modality");
    let id = registry
        .register_frozen(lens.clone(), lens.contract.clone())
        .unwrap();
    let input = Input::new(Modality::Image, vec![1, 2, 3]);

    let error = registry.measure(id, &input).unwrap_err();

    assert_eq!(error.code, "CALYX_LENS_DIM_MISMATCH");
}

#[test]
fn registry_rejects_mismatched_batch_count() {
    let mut registry = Registry::new();
    let lens = ShortBatchLens::new();
    let id = registry
        .register_frozen(lens.clone(), lens.contract.clone())
        .unwrap();
    let inputs = [
        Input::new(Modality::Text, b"a".to_vec()),
        Input::new(Modality::Text, b"b".to_vec()),
    ];

    let error = registry.measure_batch(id, &inputs).unwrap_err();

    println!("MISMATCHED_BATCH_ERROR={}", error.code);
    assert_eq!(error.code, "CALYX_LENS_DIM_MISMATCH");
}

#[test]
fn registry_rejects_non_finite_dense_values() {
    let mut registry = Registry::new();
    let lens = NanLens::new();
    let id = registry
        .register_frozen(lens.clone(), lens.contract.clone())
        .unwrap();
    let input = Input::new(Modality::Text, b"x".to_vec());

    let error = registry.measure(id, &input).unwrap_err();

    assert_eq!(error.code, "CALYX_LENS_NUMERICAL_INVARIANT");
}

#[derive(Clone)]
struct OneDimLens {
    contract: FrozenLensContract,
}

impl OneDimLens {
    fn new(name: &str) -> Self {
        Self {
            contract: contract(name),
        }
    }
}

impl Lens for OneDimLens {
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
}

#[derive(Clone)]
struct ShortBatchLens {
    contract: FrozenLensContract,
}

impl ShortBatchLens {
    fn new() -> Self {
        Self {
            contract: contract("short-batch"),
        }
    }
}

impl Lens for ShortBatchLens {
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
        Ok(SlotVector::Dense {
            dim: 1,
            data: vec![1.0],
        })
    }

    fn measure_batch(&self, _inputs: &[Input]) -> Result<Vec<SlotVector>> {
        Ok(vec![SlotVector::Dense {
            dim: 1,
            data: vec![1.0],
        }])
    }
}

#[derive(Clone)]
struct NanLens {
    contract: FrozenLensContract,
}

impl NanLens {
    fn new() -> Self {
        Self {
            contract: contract("nan-lens"),
        }
    }
}

impl Lens for NanLens {
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
        Ok(SlotVector::Dense {
            dim: 1,
            data: vec![f32::NAN],
        })
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

fn lens_spec_for(lens: &OneDimLens) -> LensSpec {
    LensSpec {
        name: lens.contract.name().to_string(),
        runtime: LensRuntime::Algorithmic {
            kind: "scalar".to_string(),
        },
        output: SlotShape::Dense(1),
        modality: Modality::Text,
        weights_sha256: lens.contract.weights_sha256(),
        corpus_hash: lens.contract.corpus_hash(),
        norm_policy: NormPolicy::None,
        max_batch: None,
        axis: None,
        asymmetry: Asymmetry::None,
        quant_default: QuantPolicy::turboquant_default(),
        truncate_dim: None,
        recall_delta: crate::spec::default_recall_delta(),
        retrieval_only: false,
        excluded_from_dedup: false,
    }
}
