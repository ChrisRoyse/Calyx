//! In-memory Assay result CF/cache with provenance.

use std::collections::BTreeMap;

use calyx_aster::cf::{CfRouter, ColumnFamily};
use calyx_core::{AnchorKind, CalyxError, Result, SlotId, VaultId};
use serde::{Deserialize, Serialize};

use crate::estimate::MiEstimate;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct AssayCacheKey {
    #[serde(default)]
    pub vault_id: Option<VaultId>,
    #[serde(default = "default_anchor")]
    pub anchor: AnchorKind,
    pub panel_version: u32,
    pub corpus_shard: String,
}

impl AssayCacheKey {
    /// Compatibility constructor for legacy tests and unscoped probes.
    pub fn new(panel_version: u32, corpus_shard: impl Into<String>) -> Self {
        Self {
            vault_id: None,
            anchor: default_anchor(),
            panel_version,
            corpus_shard: corpus_shard.into(),
        }
    }

    pub fn scoped(
        panel_version: u32,
        corpus_shard: impl Into<String>,
        vault_id: VaultId,
        anchor: AnchorKind,
    ) -> Self {
        Self {
            vault_id: Some(vault_id),
            anchor,
            panel_version,
            corpus_shard: corpus_shard.into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssaySubject {
    Lens { slot: SlotId },
    Pair { a: SlotId, b: SlotId },
    Panel,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AssayRow {
    pub cache_key: AssayCacheKey,
    pub subject: AssaySubject,
    pub estimate: MiEstimate,
    pub provenance: String,
    pub written_at_seq: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AssayStore {
    rows: BTreeMap<(AssayCacheKey, AssaySubject), AssayRow>,
}

impl AssayStore {
    pub fn put(
        &mut self,
        cache_key: AssayCacheKey,
        subject: AssaySubject,
        estimate: MiEstimate,
        provenance: impl Into<String>,
        written_at_seq: u64,
    ) {
        let row = AssayRow {
            cache_key: cache_key.clone(),
            subject: subject.clone(),
            estimate,
            provenance: provenance.into(),
            written_at_seq,
        };
        self.rows.insert((cache_key, subject), row);
    }

    pub fn get(&self, cache_key: &AssayCacheKey, subject: &AssaySubject) -> Option<&AssayRow> {
        self.rows.get(&(cache_key.clone(), subject.clone()))
    }

    pub fn cache_hit(&self, cache_key: &AssayCacheKey, subject: &AssaySubject) -> bool {
        self.get(cache_key, subject).is_some()
    }

    pub fn invalidate_panel(&mut self, panel_version: u32) -> usize {
        let before = self.rows.len();
        self.rows
            .retain(|(key, _), _| key.panel_version != panel_version);
        before - self.rows.len()
    }

    pub fn rows(&self) -> Vec<AssayRow> {
        self.rows.values().cloned().collect()
    }

    pub fn persist_to_aster(&self, router: &mut CfRouter) -> Result<usize> {
        for row in self.rows.values() {
            let key = assay_key(&row.cache_key, &row.subject);
            let value = serde_json::to_vec(row)
                .map_err(|error| CalyxError::disk_pressure(format!("encode assay row: {error}")))?;
            router.put(ColumnFamily::Assay, &key, &value)?;
        }
        router.flush_cf(ColumnFamily::Assay)?;
        Ok(self.rows.len())
    }

    pub fn load_from_aster(router: &CfRouter) -> Result<Self> {
        let mut store = Self::default();
        for entry in router.iter_cf(ColumnFamily::Assay)? {
            let row: AssayRow = serde_json::from_slice(&entry.value).map_err(|error| {
                CalyxError::aster_corrupt_shard(format!("decode assay row: {error}"))
            })?;
            let expected = assay_key(&row.cache_key, &row.subject);
            if entry.key != expected {
                return Err(CalyxError::aster_corrupt_shard(
                    "assay CF key does not match row subject",
                ));
            }
            store
                .rows
                .insert((row.cache_key.clone(), row.subject.clone()), row);
        }
        Ok(store)
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
}

fn assay_key(cache_key: &AssayCacheKey, subject: &AssaySubject) -> Vec<u8> {
    let vault = cache_key
        .vault_id
        .map(|vault_id| vault_id.to_string())
        .unwrap_or_else(|| "vault:unspecified".to_string());
    let shard = cache_key.corpus_shard.as_bytes();
    let anchor = serde_json::to_vec(&cache_key.anchor).expect("anchor kind serializes");
    let mut key = Vec::with_capacity(48 + vault.len() + anchor.len() + shard.len());
    key.extend_from_slice(&cache_key.panel_version.to_be_bytes());
    push_len_prefixed(&mut key, vault.as_bytes());
    push_len_prefixed(&mut key, &anchor);
    push_len_prefixed(&mut key, shard);
    match subject {
        AssaySubject::Lens { slot } => {
            key.push(0);
            key.extend_from_slice(&slot.get().to_be_bytes());
        }
        AssaySubject::Pair { a, b } => {
            key.push(1);
            key.extend_from_slice(&a.get().to_be_bytes());
            key.extend_from_slice(&b.get().to_be_bytes());
        }
        AssaySubject::Panel => key.push(2),
    }
    key
}

fn push_len_prefixed(key: &mut Vec<u8>, value: &[u8]) {
    key.extend_from_slice(&(value.len() as u32).to_be_bytes());
    key.extend_from_slice(value);
}

fn default_anchor() -> AnchorKind {
    AnchorKind::Reward
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::estimate::{EstimatorKind, MiEstimate, TrustTag};
    use calyx_core::{AnchorKind, VaultId};
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn assay_store_roundtrips_through_aster_cf() {
        let dir = test_dir("assay-store");
        let mut router = CfRouter::open(&dir, 1024).unwrap();
        let mut store = AssayStore::default();
        let key = AssayCacheKey::new(7, "stage5-corpus");
        let subject = AssaySubject::Lens {
            slot: SlotId::new(2),
        };
        store.put(
            key.clone(),
            subject.clone(),
            estimate(0.42),
            "stage5 assay persisted",
            99,
        );

        assert_eq!(store.persist_to_aster(&mut router).unwrap(), 1);
        drop(router);
        let reopened = CfRouter::open(&dir, 1024).unwrap();
        let loaded = AssayStore::load_from_aster(&reopened).unwrap();

        assert_eq!(loaded.get(&key, &subject).unwrap().written_at_seq, 99);
        cleanup(dir);
    }

    #[test]
    fn assay_store_keys_are_vault_and_anchor_scoped() {
        let dir = test_dir("assay-scope");
        let mut router = CfRouter::open(&dir, 1024).unwrap();
        let mut store = AssayStore::default();
        let subject = AssaySubject::Lens {
            slot: SlotId::new(2),
        };
        let key_a = AssayCacheKey::scoped(7, "shared", vault_a(), AnchorKind::Reward);
        let key_b = AssayCacheKey::scoped(7, "shared", vault_b(), AnchorKind::Reward);
        let key_c = AssayCacheKey::scoped(
            7,
            "shared",
            vault_a(),
            AnchorKind::Label("gold".to_string()),
        );

        store.put(key_a.clone(), subject.clone(), estimate(0.31), "a", 1);
        store.put(key_b.clone(), subject.clone(), estimate(0.32), "b", 2);
        store.put(key_c.clone(), subject.clone(), estimate(0.33), "c", 3);
        assert_eq!(store.len(), 3);
        assert_eq!(store.persist_to_aster(&mut router).unwrap(), 3);

        let loaded = AssayStore::load_from_aster(&router).unwrap();
        assert_eq!(loaded.len(), 3);
        assert_eq!(loaded.get(&key_a, &subject).unwrap().estimate.bits, 0.31);
        assert_eq!(loaded.get(&key_b, &subject).unwrap().estimate.bits, 0.32);
        assert_eq!(loaded.get(&key_c, &subject).unwrap().estimate.bits, 0.33);
        cleanup(dir);
    }

    #[test]
    fn assay_cf_key_mismatch_fails_closed() {
        let dir = test_dir("assay-key-mismatch");
        let mut router = CfRouter::open(&dir, 1024).unwrap();
        let key = AssayCacheKey::scoped(7, "shared", vault_a(), AnchorKind::Reward);
        let subject = AssaySubject::Lens {
            slot: SlotId::new(2),
        };
        let row = AssayRow {
            cache_key: key,
            subject,
            estimate: estimate(0.42),
            provenance: "bad-key-test".to_string(),
            written_at_seq: 9,
        };
        router
            .put(
                ColumnFamily::Assay,
                b"wrong-assay-key",
                &serde_json::to_vec(&row).unwrap(),
            )
            .unwrap();
        router.flush_cf(ColumnFamily::Assay).unwrap();

        let err = AssayStore::load_from_aster(&router).unwrap_err();
        assert_eq!(err.code, "CALYX_ASTER_CORRUPT_SHARD");
        cleanup(dir);
    }

    fn estimate(bits: f32) -> MiEstimate {
        MiEstimate {
            bits,
            ci_low: bits - 0.01,
            ci_high: bits + 0.01,
            n_samples: 120,
            estimator: EstimatorKind::Ksg,
            trust: TrustTag::Trusted,
        }
    }

    fn test_dir(name: &str) -> PathBuf {
        let id = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("calyx-assay-{name}-{}-{id}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn cleanup(dir: PathBuf) {
        fs::remove_dir_all(dir).unwrap();
    }

    fn vault_a() -> VaultId {
        "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
    }

    fn vault_b() -> VaultId {
        "01BX5ZZKBKACTAV9WEVGEMMVS0".parse().unwrap()
    }
}
