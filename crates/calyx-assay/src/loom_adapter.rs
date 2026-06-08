//! Aster-backed Assay adapter for Loom materialization planning.

use std::sync::Mutex;

use calyx_core::{
    Anchor, AnchorKind, AnchorValue, CalyxError, CxId, Result, Seq, SlotId, SlotVector, VaultStore,
};
use calyx_loom::{MaterializationPlan, plan_cross_terms_checked};

use crate::gate::{AssayGate, PairGain};

type PairSamples = (Vec<Vec<f32>>, Vec<Vec<f32>>, Vec<bool>);

pub struct AsterAssayMaterializationGate<'a, S: VaultStore + ?Sized> {
    store: &'a S,
    snapshot: Seq,
    cx_ids: Vec<CxId>,
    anchor_kind: AnchorKind,
    assay: AssayGate,
    last_error: Mutex<Option<CalyxError>>,
}

impl<'a, S> AsterAssayMaterializationGate<'a, S>
where
    S: VaultStore + ?Sized,
{
    pub fn new(store: &'a S, cx_ids: Vec<CxId>, anchor_kind: AnchorKind) -> Self {
        Self::at_snapshot(store, store.snapshot(), cx_ids, anchor_kind)
    }

    pub fn at_snapshot(
        store: &'a S,
        snapshot: Seq,
        cx_ids: Vec<CxId>,
        anchor_kind: AnchorKind,
    ) -> Self {
        Self {
            store,
            snapshot,
            cx_ids,
            anchor_kind,
            assay: AssayGate::default(),
            last_error: Mutex::new(None),
        }
    }

    pub fn with_min_samples(mut self, min_samples: usize) -> Self {
        self.assay.min_samples = min_samples;
        self
    }

    pub fn pair_gain(&self, a: SlotId, b: SlotId) -> Result<PairGain> {
        let (left, right, labels) = self.load_pair_samples(a, b)?;
        self.assay.pair_gain(&left, &right, &labels)
    }

    pub fn materialization_plan(&self, slots: &[SlotId]) -> Result<MaterializationPlan> {
        plan_cross_terms_checked(slots, |a, b| {
            self.pair_gain(a, b).map(|gain| gain.gain_bits)
        })
        .inspect_err(|error| self.record_error(error.clone()))
    }

    pub fn materialization_plan_fail_safe_lazy(&self, slots: &[SlotId]) -> MaterializationPlan {
        plan_cross_terms_checked(slots, |a, b| Ok(self.pair_gain_bits_fail_safe_lazy(a, b)))
            .expect("fail-safe lazy materialization planner is infallible")
    }

    pub fn pair_gain_bits_fail_safe_lazy(&self, a: SlotId, b: SlotId) -> f32 {
        match self.pair_gain(a, b) {
            Ok(gain) => gain.gain_bits,
            Err(error) => {
                self.record_error(error);
                0.0
            }
        }
    }

    pub fn last_error(&self) -> Option<CalyxError> {
        self.last_error
            .lock()
            .expect("materialization gate error mutex poisoned")
            .clone()
    }

    fn record_error(&self, error: CalyxError) {
        *self
            .last_error
            .lock()
            .expect("materialization gate error mutex poisoned") = Some(error);
    }

    fn load_pair_samples(&self, a: SlotId, b: SlotId) -> Result<PairSamples> {
        let mut left = Vec::with_capacity(self.cx_ids.len());
        let mut right = Vec::with_capacity(self.cx_ids.len());
        let mut labels = Vec::with_capacity(self.cx_ids.len());
        for cx_id in &self.cx_ids {
            let cx = self.store.get(*cx_id, self.snapshot)?;
            let left_vector = cx.slots.get(&a).ok_or_else(|| {
                CalyxError::stale_derived(format!("slot {} missing for {cx_id}", a.get()))
            })?;
            let right_vector = cx.slots.get(&b).ok_or_else(|| {
                CalyxError::stale_derived(format!("slot {} missing for {cx_id}", b.get()))
            })?;
            let anchor = cx
                .anchors
                .iter()
                .find(|anchor| anchor.kind == self.anchor_kind)
                .ok_or_else(|| {
                    CalyxError::stale_derived(format!(
                        "anchor {:?} missing for {cx_id}",
                        self.anchor_kind
                    ))
                })?;
            left.push(features(left_vector)?);
            right.push(features(right_vector)?);
            labels.push(anchor_bool(anchor)?);
        }
        Ok((left, right, labels))
    }
}

fn features(vector: &SlotVector) -> Result<Vec<f32>> {
    match vector {
        SlotVector::Dense { data, .. } => Ok(data.clone()),
        SlotVector::Sparse { dim, entries } => {
            let mut dense = vec![0.0; *dim as usize];
            for entry in entries {
                let index = entry.idx as usize;
                if index >= dense.len() {
                    return Err(CalyxError::aster_corrupt_shard(
                        "sparse slot entry exceeds declared dimension",
                    ));
                }
                dense[index] = entry.val;
            }
            Ok(dense)
        }
        SlotVector::Multi { .. } | SlotVector::Absent { .. } => Err(CalyxError::stale_derived(
            "materialization gate requires dense or sparse measured slots",
        )),
    }
}

fn anchor_bool(anchor: &Anchor) -> Result<bool> {
    if anchor.source.trim().is_empty()
        || !anchor.confidence.is_finite()
        || anchor.confidence <= 0.0
        || anchor.confidence > 1.0
    {
        return Err(CalyxError::assay_insufficient_samples(
            "materialization gate requires grounded anchor evidence",
        ));
    }
    match &anchor.value {
        AnchorValue::Bool(value) => Ok(*value),
        AnchorValue::Number(value) if value.is_finite() => Ok(*value > 0.0),
        _ => Err(CalyxError::assay_insufficient_samples(
            "materialization gate requires binary anchor values",
        )),
    }
}
