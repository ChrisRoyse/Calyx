//! Engine trait boundaries shared by Calyx crates.

use serde::{Deserialize, Serialize};

use crate::{
    Anchor, Constellation, CxId, LensId, Modality, Result, Seq, Signal, SlotShape, SlotVector,
};

/// Raw input presented to a frozen lens.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Input {
    /// Input modality.
    pub modality: Modality,
    /// Raw bytes to measure.
    pub bytes: Vec<u8>,
    /// Optional pointer to retained source bytes.
    pub pointer: Option<String>,
}

impl Input {
    /// Builds an input from modality and bytes.
    pub fn new(modality: Modality, bytes: impl Into<Vec<u8>>) -> Self {
        Self {
            modality,
            bytes: bytes.into(),
            pointer: None,
        }
    }

    /// Attaches a source pointer.
    pub fn with_pointer(mut self, pointer: impl Into<String>) -> Self {
        self.pointer = Some(pointer.into());
        self
    }
}

/// Implemented by Registry lens runtimes as frozen measurement instruments.
pub trait Lens: Send + Sync {
    /// Stable frozen lens id.
    fn id(&self) -> LensId;

    /// Vector shape this lens emits.
    fn shape(&self) -> SlotShape;

    /// Modality this lens accepts.
    fn modality(&self) -> Modality;

    /// Deterministically measures one input.
    fn measure(&self, input: &Input) -> Result<SlotVector>;

    /// Deterministically measures a batch of inputs.
    fn measure_batch(&self, inputs: &[Input]) -> Result<Vec<SlotVector>> {
        inputs.iter().map(|input| self.measure(input)).collect()
    }
}

/// Implemented by per-slot ANN or inverted indexes.
pub trait Index: Send + Sync {
    /// Inserts or replaces a vector for a constellation.
    fn insert(&mut self, cx: CxId, vector: &SlotVector) -> Result<()>;

    /// Searches for nearest constellations.
    fn search(&self, query: &SlotVector, k: usize, ef: Option<usize>) -> Result<Vec<(CxId, f32)>>;

    /// Rebuilds the index from its source store.
    fn rebuild(&mut self) -> Result<()>;
}

/// Implemented by Aster vault storage.
pub trait VaultStore: Send + Sync {
    /// Persists a constellation through the group-commit path.
    fn put(&self, constellation: Constellation) -> Result<CxId>;

    /// Reads a constellation as of a snapshot sequence.
    fn get(&self, id: CxId, snapshot: Seq) -> Result<Constellation>;

    /// Attaches a grounded anchor to an existing constellation.
    fn anchor(&self, id: CxId, anchor: Anchor) -> Result<()>;

    /// Returns the latest readable snapshot sequence.
    fn snapshot(&self) -> Seq;
}

/// Implemented by Assay signal estimators.
pub trait Estimator: Send + Sync {
    /// Estimates information between slot vectors and anchors.
    fn mi(&self, x: &[SlotVector], y: &[Anchor]) -> Result<Signal>;

    /// Estimates redundancy between two slot-vector samples.
    fn redundancy(&self, a: &[SlotVector], b: &[SlotVector]) -> Result<f32>;
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    use super::*;
    use crate::{
        AnchorKind, AnchorValue, CalyxError, ConfidenceInterval, CxFlags, InputRef, LedgerRef,
        VaultId,
    };

    #[test]
    fn engine_traits_are_object_safe() {
        let lens = DummyLens;
        let mut index = DummyIndex::default();
        let store = DummyStore::default();
        let estimator = DummyEstimator;

        let _: &dyn Lens = &lens;
        let _: &mut dyn Index = &mut index;
        let _: &dyn VaultStore = &store;
        let _: &dyn Estimator = &estimator;
    }

    #[test]
    fn default_batch_measurement_is_deterministic() {
        let lens: &dyn Lens = &DummyLens;
        let inputs = [Input::new(Modality::Text, b"abc".to_vec())];
        let first = lens.measure_batch(&inputs).unwrap();
        let second = lens.measure_batch(&inputs).unwrap();

        assert_eq!(first, second);
    }

    struct DummyLens;

    impl Lens for DummyLens {
        fn id(&self) -> LensId {
            LensId::from_bytes([1; 16])
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

    #[derive(Default)]
    struct DummyIndex {
        rows: Vec<(CxId, SlotVector)>,
    }

    impl Index for DummyIndex {
        fn insert(&mut self, cx: CxId, vector: &SlotVector) -> Result<()> {
            self.rows.push((cx, vector.clone()));
            Ok(())
        }

        fn search(
            &self,
            _query: &SlotVector,
            k: usize,
            _ef: Option<usize>,
        ) -> Result<Vec<(CxId, f32)>> {
            Ok(self.rows.iter().take(k).map(|(cx, _)| (*cx, 1.0)).collect())
        }

        fn rebuild(&mut self) -> Result<()> {
            Ok(())
        }
    }

    #[derive(Default)]
    struct DummyStore {
        last: Mutex<Option<Constellation>>,
    }

    impl VaultStore for DummyStore {
        fn put(&self, constellation: Constellation) -> Result<CxId> {
            let id = constellation.cx_id;
            *self.last.lock().unwrap() = Some(constellation);
            Ok(id)
        }

        fn get(&self, id: CxId, _snapshot: Seq) -> Result<Constellation> {
            self.last
                .lock()
                .unwrap()
                .clone()
                .filter(|cx| cx.cx_id == id)
                .ok_or_else(|| CalyxError::stale_derived("dummy store miss"))
        }

        fn anchor(&self, _id: CxId, _anchor: Anchor) -> Result<()> {
            Ok(())
        }

        fn snapshot(&self) -> Seq {
            1
        }
    }

    struct DummyEstimator;

    impl Estimator for DummyEstimator {
        fn mi(&self, _x: &[SlotVector], _y: &[Anchor]) -> Result<Signal> {
            Ok(Signal {
                bits: 0.1,
                ci: ConfidenceInterval {
                    low: 0.05,
                    high: 0.15,
                },
                n: 10,
                estimator: "dummy".to_string(),
                ts: 1,
            })
        }

        fn redundancy(&self, _a: &[SlotVector], _b: &[SlotVector]) -> Result<f32> {
            Ok(0.0)
        }
    }

    fn _sample_constellation() -> Constellation {
        Constellation {
            cx_id: CxId::from_bytes([1; 16]),
            vault_id: "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse::<VaultId>().unwrap(),
            panel_version: 1,
            created_at: 1,
            input_ref: InputRef {
                hash: [1; 32],
                pointer: None,
                redacted: false,
            },
            modality: Modality::Text,
            slots: BTreeMap::new(),
            scalars: BTreeMap::new(),
            metadata: BTreeMap::new(),
            anchors: vec![Anchor {
                kind: AnchorKind::Reward,
                value: AnchorValue::Number(1.0),
                source: "dummy".to_string(),
                observed_at: 1,
                confidence: 1.0,
            }],
            provenance: LedgerRef {
                seq: 1,
                hash: [1; 32],
            },
            flags: CxFlags::default(),
        }
    }
}
