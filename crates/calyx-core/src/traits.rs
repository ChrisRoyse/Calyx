//! Engine trait boundaries shared by Calyx crates.

use std::borrow::Cow;
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{
    Anchor, Constellation, CxId, LensId, Modality, Result, Seq, Signal, SlotShape, SlotVector,
    StructuredRecord,
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
    /// Canonical explicit fields measured by structured lenses.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structured: Option<StructuredRecord>,
}

impl Input {
    /// Builds an input from modality and bytes.
    pub fn new(modality: Modality, bytes: impl Into<Vec<u8>>) -> Self {
        Self {
            modality,
            bytes: bytes.into(),
            pointer: None,
            structured: None,
        }
    }

    /// Attaches a source pointer.
    pub fn with_pointer(mut self, pointer: impl Into<String>) -> Self {
        self.pointer = Some(pointer.into());
        self
    }

    /// Attaches independently typed explicit fields without flattening them
    /// into the primary byte payload.
    pub fn with_structured(mut self, record: StructuredRecord) -> Self {
        self.structured = Some(record);
        self
    }

    /// Returns the attached typed record, when this is structured or hybrid input.
    pub fn structured(&self) -> Option<&StructuredRecord> {
        self.structured.as_ref()
    }

    /// Returns whether the primary byte payload has the declared modality.
    /// Binding-aware lenses may additionally accept attached structured state
    /// through [`Lens::accepts_input`].
    pub fn supports_modality(&self, modality: Modality) -> bool {
        self.modality == modality
    }

    /// Returns stable bytes used to content-address this exact measurement input.
    ///
    /// Legacy byte-only inputs preserve their historical identity exactly.
    /// Structured and hybrid inputs use a domain-separated, length-delimited
    /// identity containing the primary bytes plus the canonical record.
    pub fn identity_bytes(&self) -> Result<Cow<'_, [u8]>> {
        let Some(record) = &self.structured else {
            return Ok(Cow::Borrowed(&self.bytes));
        };
        let record = record.canonical_bytes()?;
        let mut bytes = Vec::with_capacity(8 + 1 + 8 + self.bytes.len() + 8 + record.len());
        bytes.extend_from_slice(b"CXINPUT1");
        bytes.push(modality_tag(self.modality));
        bytes.extend_from_slice(&(self.bytes.len() as u64).to_be_bytes());
        bytes.extend_from_slice(&self.bytes);
        bytes.extend_from_slice(&(record.len() as u64).to_be_bytes());
        bytes.extend_from_slice(&record);
        Ok(Cow::Owned(bytes))
    }

    /// Persists exact structured source state into constellation scalar and
    /// reserved metadata columns.
    pub fn persist_structured(
        &self,
        scalars: &mut BTreeMap<String, f64>,
        metadata: &mut BTreeMap<String, String>,
    ) -> Result<()> {
        if let Some(record) = &self.structured {
            record.persist_into(scalars, metadata)?;
        }
        Ok(())
    }
}

const fn modality_tag(modality: Modality) -> u8 {
    match modality {
        Modality::Text => 0,
        Modality::Code => 1,
        Modality::Image => 2,
        Modality::Audio => 3,
        Modality::Video => 4,
        Modality::Protein => 5,
        Modality::Dna => 6,
        Modality::Molecule => 7,
        Modality::Structured => 8,
        Modality::Mixed => 9,
    }
}

/// Exact identity of a runtime that can measure multiple frozen lens
/// projections in one forward pass.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MeasurementGroupKey([u8; 32]);

impl MeasurementGroupKey {
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// One independently frozen projection requested from a grouped runtime.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GroupedLensRequest {
    pub lens_id: LensId,
    pub shape: SlotShape,
}

/// Implemented by Registry lens runtimes as frozen measurement instruments.
pub trait Lens: Send + Sync {
    /// Stable frozen lens id.
    fn id(&self) -> LensId;

    /// Vector shape this lens emits.
    fn shape(&self) -> SlotShape;

    /// Modality this lens accepts.
    fn modality(&self) -> Modality;

    /// Returns whether this exact frozen instrument can measure `input`.
    /// The default is strict primary-modality equality.
    fn accepts_input(&self, input: &Input) -> bool {
        input.supports_modality(self.modality())
    }

    /// Returns whether the lens is bound to canonical structured fields.
    fn requires_structured_record(&self) -> bool {
        false
    }

    /// Deterministically measures one input.
    fn measure(&self, input: &Input) -> Result<SlotVector>;

    /// Deterministically measures a batch of inputs.
    fn measure_batch(&self, inputs: &[Input]) -> Result<Vec<SlotVector>> {
        inputs.iter().map(|input| self.measure(input)).collect()
    }

    /// Returns an exact multi-output runtime identity when this lens supports
    /// grouped measurement. The default preserves standalone lens behavior.
    fn measurement_group_key(&self) -> Result<Option<MeasurementGroupKey>> {
        Ok(None)
    }

    /// Measures compatible projections in one runtime call. `None` means this
    /// lens does not implement grouped output; callers must not silently retry
    /// projections independently after a grouped failure.
    fn measure_grouped_batch(
        &self,
        _requests: &[GroupedLensRequest],
        _inputs: &[Input],
    ) -> Result<Option<BTreeMap<LensId, Vec<SlotVector>>>> {
        Ok(None)
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
