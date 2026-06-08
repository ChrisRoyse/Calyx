//! In-memory xterm CF and agreement graph readbacks.

use std::collections::BTreeMap;

use calyx_aster::cf::{CfRouter, ColumnFamily};
use calyx_core::{CalyxError, CxId, Result, SlotId};
use serde::{Deserialize, Serialize};

use crate::cross_term::{
    CrossTermKey, CrossTermKind, CrossTermValue, SignalProvenanceTag, agreement_scalar,
    agreement_weight, canonical_pair, concat_vec, delta_vec, interaction_vec,
};
use crate::error::{CALYX_LOOM_SLOT_MISSING, loom_error};
use crate::lru_cache::LruCache;
use crate::materialization::{MaterializationAction, MaterializationPlan};

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
    pub raw_mean_agreement: f32,
    pub mean_agreement: f32,
    pub agreement_weight: f32,
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

    pub fn weave(&mut self, cx: CxId, slots: &BTreeMap<SlotId, Vec<f32>>) -> Result<usize> {
        let mut inserted = 0;
        for slot in slots.keys() {
            self.tag_measured(cx, *slot);
        }
        let ids: Vec<_> = slots.keys().copied().collect();
        for i in 0..ids.len() {
            for j in i + 1..ids.len() {
                let a = ids[i];
                let b = ids[j];
                let value = agreement_scalar(&slots[&a], &slots[&b])?;
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
                inserted += 1;
            }
        }
        Ok(inserted)
    }

    pub fn materialize_plan(
        &mut self,
        cx: CxId,
        slots: &BTreeMap<SlotId, Vec<f32>>,
        plan: &MaterializationPlan,
    ) -> Result<usize> {
        let mut inserted = 0;
        for slot in slots.keys() {
            self.tag_measured(cx, *slot);
        }
        for entry in plan
            .entries
            .iter()
            .filter(|entry| entry.action == MaterializationAction::EagerStore)
        {
            let (a, b) = canonical_pair(entry.a, entry.b);
            let key = CrossTermKey {
                cx_id: cx,
                a,
                b,
                kind: entry.kind,
            };
            if self.xterm_cf.contains_key(&key) {
                continue;
            }
            let value = compute_cross_term(a, b, entry.kind, slots)?;
            self.xterm_cf.insert(
                key,
                XtermRow {
                    key,
                    value,
                    tag: SignalProvenanceTag::Derived,
                },
            );
            inserted += 1;
        }
        Ok(inserted)
    }

    pub fn cross_term(
        &mut self,
        cx: CxId,
        a: SlotId,
        b: SlotId,
        kind: CrossTermKind,
        slots: &BTreeMap<SlotId, Vec<f32>>,
    ) -> Result<CrossTermValue> {
        let (a, b) = canonical_pair(a, b);
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
        let value = compute_cross_term(a, b, kind, slots)?;
        self.cache.put(key, value.clone());
        Ok(value)
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
        let mut out = Vec::new();
        for ((a, b), (sum, n)) in edges {
            let raw = sum / n.max(1) as f32;
            out.push(AgreementEdge {
                a,
                b,
                raw_mean_agreement: raw,
                mean_agreement: raw,
                agreement_weight: agreement_weight(raw).unwrap_or(0.0),
                n,
            });
        }
        out
    }

    pub fn xterm_rows(&self) -> Vec<XtermRow> {
        self.xterm_cf.values().cloned().collect()
    }

    pub fn persist_xterms_to_aster(&self, router: &mut CfRouter) -> Result<usize> {
        for row in self.xterm_cf.values() {
            let key = xterm_key(&row.key);
            let value = serde_json::to_vec(row)
                .map_err(|error| CalyxError::disk_pressure(format!("encode xterm row: {error}")))?;
            router.put(ColumnFamily::XTerm, &key, &value)?;
        }
        router.flush_cf(ColumnFamily::XTerm)?;
        Ok(self.xterm_cf.len())
    }

    pub fn load_xterms_from_aster(router: &CfRouter, cache_capacity: usize) -> Result<Self> {
        let mut store = Self::new(cache_capacity);
        for entry in router.iter_cf(ColumnFamily::XTerm)? {
            let row: XtermRow = serde_json::from_slice(&entry.value).map_err(|error| {
                CalyxError::aster_corrupt_shard(format!("decode xterm row: {error}"))
            })?;
            if entry.key != xterm_key(&row.key) {
                return Err(CalyxError::aster_corrupt_shard(
                    "xterm CF key does not match row key",
                ));
            }
            store.xterm_cf.insert(row.key, row);
        }
        Ok(store)
    }
}

fn xterm_key(key: &CrossTermKey) -> Vec<u8> {
    let mut out = Vec::with_capacity(21);
    out.extend_from_slice(key.cx_id.as_bytes());
    out.extend_from_slice(&key.a.get().to_be_bytes());
    out.extend_from_slice(&key.b.get().to_be_bytes());
    out.push(match key.kind {
        CrossTermKind::Concat => 0,
        CrossTermKind::Interaction => 1,
        CrossTermKind::Agreement => 2,
        CrossTermKind::Delta => 3,
    });
    out
}

fn compute_cross_term(
    a: SlotId,
    b: SlotId,
    kind: CrossTermKind,
    slots: &BTreeMap<SlotId, Vec<f32>>,
) -> Result<CrossTermValue> {
    let left = slots
        .get(&a)
        .ok_or_else(|| loom_error(CALYX_LOOM_SLOT_MISSING, format!("slot {} missing", a.get())))?;
    let right = slots
        .get(&b)
        .ok_or_else(|| loom_error(CALYX_LOOM_SLOT_MISSING, format!("slot {} missing", b.get())))?;
    match kind {
        CrossTermKind::Agreement => Ok(CrossTermValue::Scalar(agreement_scalar(left, right)?)),
        CrossTermKind::Delta => Ok(CrossTermValue::Vector(delta_vec(left, right)?)),
        CrossTermKind::Interaction => Ok(CrossTermValue::Vector(interaction_vec(left, right)?)),
        CrossTermKind::Concat => Ok(CrossTermValue::Vector(concat_vec(left, right)?)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn xterms_roundtrip_through_aster_cf() {
        let dir = test_dir("xterm");
        let mut router = CfRouter::open(&dir, 1024).unwrap();
        let mut store = LoomStore::new(8);
        let slots = BTreeMap::from([
            (SlotId::new(1), vec![1.0, 0.0]),
            (SlotId::new(2), vec![0.0, 1.0]),
        ]);
        store.weave(CxId::from_bytes([1; 16]), &slots).unwrap();

        assert_eq!(store.persist_xterms_to_aster(&mut router).unwrap(), 1);
        drop(router);
        let reopened = CfRouter::open(&dir, 1024).unwrap();
        let loaded = LoomStore::load_xterms_from_aster(&reopened, 8).unwrap();

        assert_eq!(loaded.xterm_count(), 1);
        assert_eq!(loaded.agreement_graph()[0].n, 1);
        cleanup(dir);
    }

    fn test_dir(name: &str) -> PathBuf {
        let id = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("calyx-loom-{name}-{}-{id}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn cleanup(dir: PathBuf) {
        fs::remove_dir_all(dir).unwrap();
    }
}
