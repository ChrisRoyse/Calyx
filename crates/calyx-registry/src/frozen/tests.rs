use super::*;
use crate::AlgorithmicLens;
use crate::Registry;

#[test]
fn same_contract_produces_same_lens_id_across_registries() {
    let contract = unit_contract("unit");
    let mut left = Registry::new();
    let mut right = Registry::new();

    let left_id = left
        .register_frozen(UnitLens::new(contract.clone()), contract.clone())
        .unwrap();
    let right_id = right
        .register_frozen(UnitLens::new(contract.clone()), contract)
        .unwrap();

    println!("FROZEN_SAME_ID_A={left_id} FROZEN_SAME_ID_B={right_id}");
    assert_eq!(left_id, right_id);
}

#[test]
fn mutated_weight_hash_fails_closed() {
    let contract = unit_contract("mutated");
    let lens = UnitLens::new(contract.clone());
    let mut registry = Registry::new();

    let error = registry
        .register_frozen(lens, contract.with_mutated_weight_hash())
        .unwrap_err();

    println!("FROZEN_MUTATED_ERROR={}", error.code);
    assert_eq!(error.code, "CALYX_LENS_FROZEN_VIOLATION");
}

#[test]
fn wrong_dim_vector_fails_closed_at_measure() {
    let contract = unit_contract("wrong-dim");
    let mut registry = Registry::new();
    let id = registry
        .register_frozen(WrongDimLens::new(contract.clone()), contract)
        .unwrap();

    let error = registry
        .measure(id, &Input::new(Modality::Text, b"x".to_vec()))
        .unwrap_err();

    println!("FROZEN_WRONG_DIM_ERROR={}", error.code);
    assert_eq!(error.code, "CALYX_LENS_DIM_MISMATCH");
}

#[test]
fn non_unit_vector_fails_numerical_invariant() {
    let contract = unit_contract("non-unit");
    let mut registry = Registry::new();
    let id = registry
        .register_frozen(NonUnitLens::new(contract.clone()), contract)
        .unwrap();

    let error = registry
        .measure(id, &Input::new(Modality::Text, b"x".to_vec()))
        .unwrap_err();

    println!("FROZEN_NON_UNIT_ERROR={}", error.code);
    assert_eq!(error.code, "CALYX_LENS_NUMERICAL_INVARIANT");
}

#[test]
fn determinism_probe_rejects_changing_output() {
    let contract = unit_contract("nondeterministic");
    let mut registry = Registry::new();

    let error = registry
        .register_frozen_with_probe(
            ChangingLens::new(contract.clone()),
            contract,
            &Input::new(Modality::Text, b"probe".to_vec()),
        )
        .unwrap_err();

    assert_eq!(error.code, "CALYX_LENS_FROZEN_VIOLATION");
}

#[test]
fn algorithmic_runtime_matches_frozen_contract() {
    let contract = FrozenLensContract::algorithmic_byte_features("byte-frozen", Modality::Text);
    let lens = AlgorithmicLens::byte_features("byte-frozen", Modality::Text);
    let mut registry = Registry::new();

    let id = registry
        .register_frozen_with_probe(
            lens,
            contract.clone(),
            &Input::new(Modality::Text, b"abc".to_vec()),
        )
        .unwrap();

    assert_eq!(id, contract.lens_id());
}

fn unit_contract(name: &str) -> FrozenLensContract {
    FrozenLensContract::new(
        name,
        sha256_digest(&[name.as_bytes(), b"weights"]),
        sha256_digest(&[b"corpus"]),
        SlotShape::Dense(2),
        Modality::Text,
        LensDType::F32,
        NormPolicy::unit(),
    )
}

#[derive(Clone)]
struct UnitLens {
    contract: FrozenLensContract,
}

impl UnitLens {
    fn new(contract: FrozenLensContract) -> Self {
        Self { contract }
    }
}

impl Lens for UnitLens {
    fn id(&self) -> LensId {
        self.contract.lens_id()
    }

    fn shape(&self) -> SlotShape {
        self.contract.shape()
    }

    fn modality(&self) -> Modality {
        self.contract.modality()
    }

    fn measure(&self, _input: &Input) -> Result<SlotVector> {
        Ok(SlotVector::Dense {
            dim: 2,
            data: vec![1.0, 0.0],
        })
    }
}

struct WrongDimLens(UnitLens);

impl WrongDimLens {
    fn new(contract: FrozenLensContract) -> Self {
        Self(UnitLens::new(contract))
    }
}

impl Lens for WrongDimLens {
    fn id(&self) -> LensId {
        self.0.id()
    }

    fn shape(&self) -> SlotShape {
        self.0.shape()
    }

    fn modality(&self) -> Modality {
        self.0.modality()
    }

    fn measure(&self, _input: &Input) -> Result<SlotVector> {
        Ok(SlotVector::Dense {
            dim: 1,
            data: vec![1.0],
        })
    }
}

struct NonUnitLens(UnitLens);

impl NonUnitLens {
    fn new(contract: FrozenLensContract) -> Self {
        Self(UnitLens::new(contract))
    }
}

impl Lens for NonUnitLens {
    fn id(&self) -> LensId {
        self.0.id()
    }

    fn shape(&self) -> SlotShape {
        self.0.shape()
    }

    fn modality(&self) -> Modality {
        self.0.modality()
    }

    fn measure(&self, _input: &Input) -> Result<SlotVector> {
        Ok(SlotVector::Dense {
            dim: 2,
            data: vec![2.0, 0.0],
        })
    }
}

struct ChangingLens {
    lens: UnitLens,
    counter: std::sync::atomic::AtomicUsize,
}

impl ChangingLens {
    fn new(contract: FrozenLensContract) -> Self {
        Self {
            lens: UnitLens::new(contract),
            counter: std::sync::atomic::AtomicUsize::new(0),
        }
    }
}

impl Lens for ChangingLens {
    fn id(&self) -> LensId {
        self.lens.id()
    }

    fn shape(&self) -> SlotShape {
        self.lens.shape()
    }

    fn modality(&self) -> Modality {
        self.lens.modality()
    }

    fn measure(&self, _input: &Input) -> Result<SlotVector> {
        let value = self
            .counter
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let data = if value.is_multiple_of(2) {
            vec![1.0, 0.0]
        } else {
            vec![0.0, 1.0]
        };
        Ok(SlotVector::Dense { dim: 2, data })
    }
}
