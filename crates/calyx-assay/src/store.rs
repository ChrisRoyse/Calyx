//! In-memory Assay result CF/cache with provenance.

use std::collections::BTreeMap;

use calyx_aster::cf::{CfRouter, ColumnFamily};
use calyx_aster::vault::AsterVault;
use calyx_core::{AnchorKind, CalyxError, Clock, Result, SlotId, VaultId, VaultStore};
use serde::{Deserialize, Serialize};

use crate::estimate::MiEstimate;

type AsterAssayRow = (ColumnFamily, Vec<u8>, Vec<u8>);

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
    #[deprecated(note = "Assay CF rows must use AssayCacheKey::scoped before persistence")]
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

    pub fn require_scoped(&self) -> Result<()> {
        if self.vault_id.is_none() {
            return Err(CalyxError::vault_access_denied(
                "assay cache key must include explicit vault scope",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssaySubject {
    Lens { slot: SlotId },
    Pair { a: SlotId, b: SlotId },
    Panel,
    OutcomeEntropy,
    EnsembleCard,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AssayRow {
    pub cache_key: AssayCacheKey,
    pub subject: AssaySubject,
    pub estimate: MiEstimate,
    pub provenance: String,
    pub written_at_seq: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<serde_json::Value>,
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
            payload: None,
        };
        self.rows.insert((cache_key, subject), row);
    }

    pub fn put_with_payload(
        &mut self,
        cache_key: AssayCacheKey,
        subject: AssaySubject,
        estimate: MiEstimate,
        provenance: impl Into<String>,
        written_at_seq: u64,
        payload: serde_json::Value,
    ) {
        let row = AssayRow {
            cache_key: cache_key.clone(),
            subject: subject.clone(),
            estimate,
            provenance: provenance.into(),
            written_at_seq,
            payload: Some(payload),
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
        for (_, key, value) in self.aster_rows()? {
            router.put(ColumnFamily::Assay, &key, &value)?;
        }
        router.flush_cf(ColumnFamily::Assay)?;
        Ok(self.rows.len())
    }

    pub fn persist_to_vault<C>(&self, vault: &AsterVault<C>) -> Result<usize>
    where
        C: Clock,
    {
        vault.write_cf_batch(self.aster_rows()?)?;
        vault.flush()?;
        Ok(self.rows.len())
    }

    pub fn load_from_aster(router: &CfRouter) -> Result<Self> {
        let mut store = Self::default();
        for entry in router.iter_cf(ColumnFamily::Assay)? {
            store.insert_aster_row(entry.key, entry.value)?;
        }
        Ok(store)
    }

    pub fn load_from_vault<C>(vault: &AsterVault<C>) -> Result<Self>
    where
        C: Clock,
    {
        let mut store = Self::default();
        for (key, value) in vault.scan_cf_at(vault.snapshot(), ColumnFamily::Assay)? {
            store.insert_aster_row(key, value)?;
        }
        Ok(store)
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    fn aster_rows(&self) -> Result<Vec<AsterAssayRow>> {
        let mut out = Vec::with_capacity(self.rows.len());
        for row in self.rows.values() {
            row.cache_key.require_scoped()?;
            let key = assay_key(&row.cache_key, &row.subject);
            let value = serde_json::to_vec(row)
                .map_err(|error| CalyxError::disk_pressure(format!("encode assay row: {error}")))?;
            out.push((ColumnFamily::Assay, key, value));
        }
        Ok(out)
    }

    fn insert_aster_row(&mut self, key: Vec<u8>, value: Vec<u8>) -> Result<()> {
        let row: AssayRow = serde_json::from_slice(&value).map_err(|error| {
            CalyxError::aster_corrupt_shard(format!("decode assay row: {error}"))
        })?;
        row.cache_key.require_scoped()?;
        let expected = assay_key(&row.cache_key, &row.subject);
        if key != expected {
            return Err(CalyxError::aster_corrupt_shard(
                "assay CF key does not match row subject",
            ));
        }
        self.rows
            .insert((row.cache_key.clone(), row.subject.clone()), row);
        Ok(())
    }
}

fn assay_key(cache_key: &AssayCacheKey, subject: &AssaySubject) -> Vec<u8> {
    let vault = cache_key
        .vault_id
        .expect("assay cache key scope validated before encoding")
        .to_string();
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
        AssaySubject::OutcomeEntropy => key.push(3),
        AssaySubject::EnsembleCard => key.push(4),
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
    use calyx_core::{AnchorKind, FixedClock, VaultId};
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn assay_store_roundtrips_through_aster_cf() {
        let dir = test_dir("assay-store");
        let mut router = CfRouter::open(&dir, 1024).unwrap();
        let mut store = AssayStore::default();
        let key = AssayCacheKey::scoped(7, "stage5-corpus", vault_a(), AnchorKind::Reward);
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
    fn outcome_entropy_subject_roundtrips_through_vault() {
        let vault = AsterVault::with_clock(vault_a(), b"assay-entropy", FixedClock::new(7));
        let mut store = AssayStore::default();
        let key = AssayCacheKey::scoped(8, "oracle-domain", vault_a(), AnchorKind::Reward);
        store.put(
            key.clone(),
            AssaySubject::OutcomeEntropy,
            MiEstimate::point(1.0, 120, EstimatorKind::OutcomeEntropy, TrustTag::Trusted),
            "oracle entropy fixture",
            7,
        );

        assert_eq!(store.persist_to_vault(&vault).unwrap(), 1);
        let loaded = AssayStore::load_from_vault(&vault).unwrap();

        assert_eq!(
            loaded
                .get(&key, &AssaySubject::OutcomeEntropy)
                .unwrap()
                .estimate
                .bits,
            1.0
        );
    }

    #[test]
    fn assay_store_rejects_unscoped_rows_before_persistence() {
        let dir = test_dir("assay-unscoped");
        let mut router = CfRouter::open(&dir, 1024).unwrap();
        let mut store = AssayStore::default();
        #[allow(deprecated)]
        let key = AssayCacheKey::new(7, "legacy-unscoped");
        store.put(
            key,
            AssaySubject::Panel,
            estimate(0.42),
            "legacy unscoped row",
            9,
        );

        let error = store.persist_to_aster(&mut router).unwrap_err();
        assert_eq!(error.code, "CALYX_VAULT_ACCESS_DENIED");
        assert_eq!(router.iter_cf(ColumnFamily::Assay).unwrap().len(), 0);
        cleanup(dir);
    }

    #[test]
    fn assay_store_rejects_unscoped_rows_on_load() {
        let dir = test_dir("assay-unscoped-load");
        let mut router = CfRouter::open(&dir, 1024).unwrap();
        #[allow(deprecated)]
        let key = AssayCacheKey::new(7, "legacy-unscoped");
        let row = AssayRow {
            cache_key: key,
            subject: AssaySubject::Panel,
            estimate: estimate(0.42),
            provenance: "legacy unscoped row".to_string(),
            written_at_seq: 9,
            payload: None,
        };
        router
            .put(
                ColumnFamily::Assay,
                b"legacy-unscoped-key",
                &serde_json::to_vec(&row).unwrap(),
            )
            .unwrap();
        router.flush_cf(ColumnFamily::Assay).unwrap();

        let error = AssayStore::load_from_aster(&router).unwrap_err();
        assert_eq!(error.code, "CALYX_VAULT_ACCESS_DENIED");
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
            payload: None,
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

    #[test]
    fn ensemble_card_payload_roundtrips_through_assay_cf() {
        let dir = test_dir("assay-ensemble-card");
        let mut router = CfRouter::open(&dir, 1024).unwrap();
        let mut store = AssayStore::default();
        let key = AssayCacheKey::scoped(9, "ensemble", vault_a(), AnchorKind::Label("gold".into()));
        let payload = serde_json::json!({
            "schema_version": 1,
            "panel_lens_count": 10,
            "n_eff": 7.5
        });
        store.put_with_payload(
            key.clone(),
            AssaySubject::EnsembleCard,
            estimate(1.25),
            "ensemble-card-fixture",
            101,
            payload.clone(),
        );

        assert_eq!(store.persist_to_aster(&mut router).unwrap(), 1);
        drop(router);
        let reopened = CfRouter::open(&dir, 1024).unwrap();
        let loaded = AssayStore::load_from_aster(&reopened).unwrap();
        let row = loaded.get(&key, &AssaySubject::EnsembleCard).unwrap();

        assert_eq!(row.payload.as_ref(), Some(&payload));
        assert_eq!(row.estimate.bits, 1.25);
        cleanup(dir);
    }

    fn estimate(bits: f32) -> MiEstimate {
        MiEstimate::new(
            bits,
            bits - 0.01,
            bits + 0.01,
            120,
            EstimatorKind::Ksg,
            TrustTag::Trusted,
        )
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
