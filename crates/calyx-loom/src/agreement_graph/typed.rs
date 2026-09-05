use std::collections::{BTreeMap, BTreeSet};

use calyx_core::{CxId, Result, SlotId, SlotVector, slot_similarity};

use super::{LoomStore, XtermRow};
use crate::cross_term::{CrossTermKey, CrossTermKind, CrossTermValue, SignalProvenanceTag};
use crate::cuda::loom_cuda_strict_requested;
use crate::error::{
    CALYX_LOOM_SLOT_MISSING, CALYX_LOOM_TYPED_CUDA_UNAVAILABLE, CALYX_LOOM_TYPED_XTERM_UNSUPPORTED,
    loom_error,
};
use crate::materialization::{MaterializationAction, MaterializationPlan};

impl LoomStore {
    /// Weave every lens-pair agreement from the original typed slot blocks.
    pub fn weave_typed(&mut self, cx: CxId, slots: &BTreeMap<SlotId, SlotVector>) -> Result<usize> {
        let refs = slots
            .iter()
            .map(|(&slot, vector)| (slot, vector))
            .collect::<BTreeMap<_, _>>();
        self.weave_typed_refs(cx, &refs)
    }

    /// Borrowing counterpart of [`Self::weave_typed`]. Dimension-grouped
    /// callers can select compatible slot blocks without cloning their vector
    /// payloads solely to build an intermediate map (#1999).
    pub fn weave_typed_refs(
        &mut self,
        cx: CxId,
        slots: &BTreeMap<SlotId, &SlotVector>,
    ) -> Result<usize> {
        refuse_typed_cpu_fallback()?;
        self.last_cuda_stats = None;
        for (slot, vector) in slots {
            vector.validate_schema()?;
            self.tag_measured(cx, *slot);
        }
        let ids = slots.keys().copied().collect::<Vec<_>>();
        let mut rows = Vec::new();
        for left in 0..ids.len() {
            for right in left + 1..ids.len() {
                let key = CrossTermKey {
                    cx_id: cx,
                    a: ids[left],
                    b: ids[right],
                    kind: CrossTermKind::Agreement,
                };
                rows.push(XtermRow {
                    key,
                    value: typed_value_refs(key.a, key.b, key.kind, slots)?,
                    tag: SignalProvenanceTag::Derived,
                });
            }
        }
        let inserted = rows.len();
        for row in rows {
            self.xterm_cf.insert(row.key, row);
        }
        Ok(inserted)
    }

    pub fn materialize_plan_typed(
        &mut self,
        cx: CxId,
        slots: &BTreeMap<SlotId, SlotVector>,
        plan: &MaterializationPlan,
    ) -> Result<usize> {
        refuse_typed_cpu_fallback()?;
        self.last_cuda_stats = None;
        for (slot, vector) in slots {
            vector.validate_schema()?;
            self.tag_measured(cx, *slot);
        }
        let mut seen = BTreeSet::new();
        let mut rows = Vec::new();
        for entry in plan
            .entries
            .iter()
            .filter(|entry| entry.action == MaterializationAction::EagerStore)
        {
            let (a, b) = crate::cross_term::canonical_pair(entry.a, entry.b);
            let key = CrossTermKey {
                cx_id: cx,
                a,
                b,
                kind: entry.kind,
            };
            if self.xterm_cf.contains_key(&key) || !seen.insert(key) {
                continue;
            }
            rows.push(XtermRow {
                key,
                value: typed_value(a, b, entry.kind, slots)?,
                tag: SignalProvenanceTag::Derived,
            });
        }
        let inserted = rows.len();
        for row in rows {
            self.xterm_cf.insert(row.key, row);
        }
        Ok(inserted)
    }

    pub fn cross_term_typed(
        &mut self,
        cx: CxId,
        a: SlotId,
        b: SlotId,
        kind: CrossTermKind,
        slots: &BTreeMap<SlotId, SlotVector>,
    ) -> Result<CrossTermValue> {
        refuse_typed_cpu_fallback()?;
        self.last_cuda_stats = None;
        let (a, b) = crate::cross_term::canonical_pair(a, b);
        let key = CrossTermKey {
            cx_id: cx,
            a,
            b,
            kind,
        };
        if let Some(row) = self.xterm_cf.get(&key) {
            return Ok(row.value.clone());
        }
        if let Some(value) = self.cache.get(&key) {
            return Ok(value);
        }
        let value = typed_value(a, b, kind, slots)?;
        self.cache.put(key, value.clone());
        Ok(value)
    }
}

fn typed_value_refs(
    a: SlotId,
    b: SlotId,
    kind: CrossTermKind,
    slots: &BTreeMap<SlotId, &SlotVector>,
) -> Result<CrossTermValue> {
    let left = slots
        .get(&a)
        .ok_or_else(|| loom_error(CALYX_LOOM_SLOT_MISSING, format!("slot {} missing", a.get())))?;
    let right = slots
        .get(&b)
        .ok_or_else(|| loom_error(CALYX_LOOM_SLOT_MISSING, format!("slot {} missing", b.get())))?;
    typed_value_pair(a, b, kind, left, right)
}

fn refuse_typed_cpu_fallback() -> Result<()> {
    if loom_cuda_strict_requested() {
        return Err(loom_error(
            CALYX_LOOM_TYPED_CUDA_UNAVAILABLE,
            "strict CUDA was requested for a typed-block Loom operation",
        ));
    }
    Ok(())
}

fn typed_value(
    a: SlotId,
    b: SlotId,
    kind: CrossTermKind,
    slots: &BTreeMap<SlotId, SlotVector>,
) -> Result<CrossTermValue> {
    let left = slots
        .get(&a)
        .ok_or_else(|| loom_error(CALYX_LOOM_SLOT_MISSING, format!("slot {} missing", a.get())))?;
    let right = slots
        .get(&b)
        .ok_or_else(|| loom_error(CALYX_LOOM_SLOT_MISSING, format!("slot {} missing", b.get())))?;
    typed_value_pair(a, b, kind, left, right)
}

fn typed_value_pair(
    a: SlotId,
    b: SlotId,
    kind: CrossTermKind,
    left: &SlotVector,
    right: &SlotVector,
) -> Result<CrossTermValue> {
    if kind != CrossTermKind::Agreement {
        return Err(loom_error(
            CALYX_LOOM_TYPED_XTERM_UNSUPPORTED,
            format!(
                "typed block operation {kind:?} is not native; slots {} and {} remain unmodified",
                a.get(),
                b.get()
            ),
        ));
    }
    Ok(CrossTermValue::BlockAgreement(Box::new(slot_similarity(
        left, right,
    )?)))
}

pub(super) fn validate_persisted_value(value: &CrossTermValue) -> Result<()> {
    match value {
        CrossTermValue::Scalar(value) if value.is_finite() => Ok(()),
        CrossTermValue::Vector(values) if values.iter().all(|value| value.is_finite()) => Ok(()),
        CrossTermValue::BlockAgreement(evidence) => evidence.validate(),
        CrossTermValue::Scalar(_) | CrossTermValue::Vector(_) => Err(loom_error(
            crate::error::CALYX_LOOM_NON_FINITE_VECTOR,
            "persisted xterm contains NaN or infinity",
        )),
    }
}
