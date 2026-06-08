//! In-memory xterm CF and agreement graph readbacks.

use std::collections::BTreeMap;

use calyx_core::{CxId, SlotId};
use serde::{Deserialize, Serialize};

use crate::cross_term::{
    CrossTermKey, CrossTermKind, CrossTermValue, SignalProvenanceTag, agreement_scalar,
    canonical_pair, concat_vec, delta_vec, interaction_vec,
};
use crate::lru_cache::LruCache;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct XtermRow {
    pub key: CrossTermKey,
    pub value: CrossTermValue,
    pub tag: SignalProvenanceTag,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AgreementEdge {
    pub a: SlotId,
    pub b: SlotId,
    pub mean_agreement: f32,
    pub n: usize,
}

#[derive(Clone, Debug)]
pub struct LoomStore {
    xterm_cf: BTreeMap<CrossTermKey, XtermRow>,
    measured_tags: BTreeMap<(CxId, SlotId), SignalProvenanceTag>,
    cache: LruCache<CrossTermKey, CrossTermValue>,
}

impl LoomStore {
    pub fn new(cache_capacity: usize) -> Self {
        Self {
            xterm_cf: BTreeMap::new(),
            measured_tags: BTreeMap::new(),
            cache: LruCache::new(cache_capacity),
        }
    }

    pub fn tag_measured(&mut self, cx: CxId, slot: SlotId) {
        self.measured_tags
            .insert((cx, slot), SignalProvenanceTag::Measured);
    }

    pub fn measured_count(&self) -> usize {
        self.measured_tags.len()
    }

    pub fn xterm_count(&self) -> usize {
        self.xterm_cf.len()
    }

    pub fn cache_count(&self) -> usize {
        self.cache.len()
    }

    pub fn weave(&mut self, cx: CxId, slots: &BTreeMap<SlotId, Vec<f32>>) {
        for slot in slots.keys() {
            self.tag_measured(cx, *slot);
        }
        let ids: Vec<_> = slots.keys().copied().collect();
        for i in 0..ids.len() {
            for j in i + 1..ids.len() {
                let a = ids[i];
                let b = ids[j];
                let value = agreement_scalar(&slots[&a], &slots[&b]);
                let key = CrossTermKey {
                    cx_id: cx,
                    a,
                    b,
                    kind: CrossTermKind::Agreement,
                };
                self.xterm_cf.insert(
                    key,
                    XtermRow {
                        key,
                        value: CrossTermValue::Scalar(value),
                        tag: SignalProvenanceTag::Derived,
                    },
                );
            }
        }
    }

    pub fn cross_term(
        &mut self,
        cx: CxId,
        a: SlotId,
        b: SlotId,
        kind: CrossTermKind,
        slots: &BTreeMap<SlotId, Vec<f32>>,
    ) -> Option<CrossTermValue> {
        let (a, b) = canonical_pair(a, b);
        let key = CrossTermKey {
            cx_id: cx,
            a,
            b,
            kind,
        };
        if let Some(row) = self.xterm_cf.get(&key) {
            return Some(row.value.clone());
        }
        if let Some(value) = self.cache.get(&key) {
            return Some(value);
        }
        let left = slots.get(&a)?;
        let right = slots.get(&b)?;
        let value = match kind {
            CrossTermKind::Agreement => CrossTermValue::Scalar(agreement_scalar(left, right)),
            CrossTermKind::Delta => CrossTermValue::Vector(delta_vec(left, right)),
            CrossTermKind::Interaction => CrossTermValue::Vector(interaction_vec(left, right)),
            CrossTermKind::Concat => CrossTermValue::Vector(concat_vec(left, right)),
        };
        self.cache.put(key, value.clone());
        Some(value)
    }

    pub fn agreement_graph(&self) -> Vec<AgreementEdge> {
        let mut edges = BTreeMap::<(SlotId, SlotId), (f32, usize)>::new();
        for row in self.xterm_cf.values() {
            if let CrossTermValue::Scalar(value) = row.value {
                let entry = edges.entry((row.key.a, row.key.b)).or_default();
                entry.0 += value;
                entry.1 += 1;
            }
        }
        edges
            .into_iter()
            .map(|((a, b), (sum, n))| AgreementEdge {
                a,
                b,
                mean_agreement: sum / n.max(1) as f32,
                n,
            })
            .collect()
    }
}
