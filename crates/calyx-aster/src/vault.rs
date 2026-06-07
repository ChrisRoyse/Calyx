//! Aster `VaultStore` implementation over the PH08 MVCC CF table.

mod anchor_codec;
mod cf_codec;
mod cursor;
mod durable;
pub mod encode;
mod router_bridge;

use crate::cf::{ColumnFamily, anchor_key, base_key, ledger_key, slot_key};
use crate::mvcc::{CfRead, Freshness, ReaderLease, Snapshot, VersionedCfStore};
use crate::vault::durable::DurableVault;
use calyx_core::{
    Anchor, CalyxError, Clock, Constellation, CxId, Result, Seq, SlotId, SystemClock, VaultId,
    VaultStore,
};
use std::collections::BTreeMap;
use std::path::Path;

pub use durable::VaultOptions;

const DEFAULT_LEASE_MS: u64 = 5_000;

/// Single-vault Aster store with content-addressed ingest semantics.
#[derive(Debug)]
pub struct AsterVault<C = SystemClock> {
    vault_id: VaultId,
    vault_salt: Vec<u8>,
    clock: C,
    rows: VersionedCfStore,
    durable: Option<DurableVault>,
}

impl AsterVault<SystemClock> {
    /// Creates a vault using the system clock.
    pub fn new(vault_id: VaultId, vault_salt: impl Into<Vec<u8>>) -> Self {
        Self::with_clock(vault_id, vault_salt, SystemClock)
    }

    pub fn new_durable(
        vault_dir: impl AsRef<Path>,
        vault_id: VaultId,
        vault_salt: impl Into<Vec<u8>>,
        options: VaultOptions,
    ) -> Result<Self> {
        Self::open(vault_dir, vault_id, vault_salt, options)
    }

    pub fn open(
        vault_dir: impl AsRef<Path>,
        vault_id: VaultId,
        vault_salt: impl Into<Vec<u8>>,
        options: VaultOptions,
    ) -> Result<Self> {
        let rows = VersionedCfStore::default();
        for batch in DurableVault::replay_batches(vault_dir.as_ref())? {
            rows.commit_batch(batch.into_iter().map(|row| (row.cf, row.key, row.value)))?;
        }
        let durable = DurableVault::open(vault_dir, &options)?;
        Ok(Self {
            vault_id,
            vault_salt: vault_salt.into(),
            clock: SystemClock,
            rows,
            durable: Some(durable),
        })
    }
}

impl<C> AsterVault<C>
where
    C: Clock,
{
    /// Creates a vault with an injected clock.
    pub fn with_clock(vault_id: VaultId, vault_salt: impl Into<Vec<u8>>, clock: C) -> Self {
        Self {
            vault_id,
            vault_salt: vault_salt.into(),
            clock,
            rows: VersionedCfStore::default(),
            durable: None,
        }
    }

    /// Computes the PRD content-addressed id for raw input bytes.
    pub fn cx_id_for_input(&self, input_bytes: &[u8], panel_version: u32) -> CxId {
        CxId::from_input(input_bytes, panel_version, &self.vault_salt)
    }

    /// Returns the latest committed vault sequence.
    pub fn latest_seq(&self) -> Seq {
        self.rows.current_seq()
    }

    /// Reads one raw CF row at `snapshot`.
    pub fn read_cf_at(
        &self,
        snapshot: Seq,
        cf: ColumnFamily,
        key: &[u8],
    ) -> Result<Option<Vec<u8>>> {
        self.rows
            .read_at(self.snapshot_handle(snapshot), cf, key, &self.clock)
    }

    fn snapshot_handle(&self, seq: Seq) -> Snapshot {
        let lease = ReaderLease::new(0, seq, self.clock.now(), DEFAULT_LEASE_MS);
        Snapshot::new(seq, Freshness::FreshDerived, lease)
    }

    pub fn flush(&self) -> Result<()> {
        if let Some(durable) = &self.durable {
            durable.flush()?;
        }
        Ok(())
    }
}

impl<C> VaultStore for AsterVault<C>
where
    C: Clock,
{
    fn put(&self, constellation: Constellation) -> Result<CxId> {
        if constellation.vault_id != self.vault_id {
            return Err(CalyxError::vault_access_denied(
                "constellation belongs to another vault",
            ));
        }

        let id = constellation.cx_id;
        let base_key = base_key(id);
        let base_bytes = encode::encode_constellation_base(&constellation)?;
        let latest = self.snapshot();
        if let Some(existing) = self.rows.read_at(
            self.snapshot_handle(latest),
            ColumnFamily::Base,
            &base_key,
            &self.clock,
        )? {
            if existing == base_bytes
                || encode::same_constellation_identity(&existing, &base_bytes)?
            {
                return Ok(id);
            }
            return Err(CalyxError::aster_corrupt_shard(
                "CxId collision or non-idempotent duplicate constellation",
            ));
        }

        let mut rows = Vec::new();
        rows.push(encode::WriteRow {
            cf: ColumnFamily::Base,
            key: base_key,
            value: base_bytes,
        });
        for (slot, vector) in &constellation.slots {
            rows.push(encode::WriteRow {
                cf: ColumnFamily::slot(*slot),
                key: slot_key(id),
                value: encode::encode_slot_vector(vector)?,
            });
        }
        for anchor in &constellation.anchors {
            rows.push(encode::WriteRow {
                cf: ColumnFamily::Anchors,
                key: anchor_key(id, &anchor.kind),
                value: encode::encode_anchor(anchor)?,
            });
        }
        rows.push(encode::WriteRow {
            cf: ColumnFamily::Ledger,
            key: ledger_key(constellation.provenance.seq),
            value: encode::encode_ledger_stub(constellation.provenance.seq),
        });
        if let Some(durable) = &self.durable {
            durable.write_batch(&rows)?;
        }
        self.rows.commit_batch(
            rows.iter()
                .map(|row| (row.cf, row.key.clone(), row.value.clone())),
        )?;
        Ok(id)
    }

    fn get(&self, id: CxId, snapshot: Seq) -> Result<Constellation> {
        let handle = self.snapshot_handle(snapshot);
        let base = self
            .rows
            .read_at(handle, ColumnFamily::Base, &base_key(id), &self.clock)?
            .ok_or_else(|| CalyxError::stale_derived("constellation missing at snapshot"))?;
        let mut constellation = encode::decode_constellation_base(&base)?;
        let slot_ids: Vec<SlotId> = constellation.slots.keys().copied().collect();
        let reads: Vec<_> = slot_ids
            .iter()
            .map(|slot| CfRead::new(ColumnFamily::slot(*slot), slot_key(id)))
            .collect();
        let values = self.rows.read_batch(handle, &reads, &self.clock)?;
        let mut slots = BTreeMap::new();
        for (slot, value) in slot_ids.into_iter().zip(values) {
            let value =
                value.ok_or_else(|| CalyxError::aster_corrupt_shard("slot CF row missing"))?;
            slots.insert(slot, encode::decode_slot_vector(&value)?);
        }
        constellation.slots = slots;
        Ok(constellation)
    }

    fn anchor(&self, id: CxId, anchor: Anchor) -> Result<()> {
        let latest = self.snapshot();
        let mut constellation = self.get(id, latest)?;
        constellation.anchors.push(anchor.clone());
        let rows = [
            (
                ColumnFamily::Base,
                base_key(id),
                encode::encode_constellation_base(&constellation)?,
            ),
            (
                ColumnFamily::Anchors,
                anchor_key(id, &anchor.kind),
                encode::encode_anchor(&anchor)?,
            ),
        ];
        let rows = rows
            .into_iter()
            .map(|(cf, key, value)| encode::WriteRow { cf, key, value })
            .collect::<Vec<_>>();
        if let Some(durable) = &self.durable {
            durable.write_batch(&rows)?;
        }
        self.rows.commit_batch(
            rows.iter()
                .map(|row| (row.cf, row.key.clone(), row.value.clone())),
        )?;
        Ok(())
    }

    fn snapshot(&self) -> Seq {
        self.latest_seq()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use calyx_core::{
        AbsentReason, AnchorKind, AnchorValue, CxFlags, FixedClock, InputRef, LedgerRef, Modality,
        SlotVector,
    };
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

    fn vault_id() -> VaultId {
        "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().expect("valid ULID")
    }

    fn sample_constellation(vault: &AsterVault<FixedClock>) -> Constellation {
        let input = b"same-input";
        let cx_id = vault.cx_id_for_input(input, 7);
        let mut input_hash = [0_u8; 32];
        input_hash[..input.len()].copy_from_slice(input);
        let mut slots = BTreeMap::new();
        slots.insert(
            SlotId::new(0),
            SlotVector::Dense {
                dim: 2,
                data: vec![0.25, 0.75],
            },
        );
        slots.insert(
            SlotId::new(1),
            SlotVector::Absent {
                reason: AbsentReason::LensUnavailable,
            },
        );
        Constellation {
            cx_id,
            vault_id: vault_id(),
            panel_version: 7,
            created_at: 123,
            input_ref: InputRef {
                hash: input_hash,
                pointer: Some("synthetic://same-input".to_string()),
                redacted: false,
            },
            modality: Modality::Text,
            slots,
            scalars: BTreeMap::new(),
            anchors: Vec::new(),
            provenance: LedgerRef {
                seq: 1,
                hash: [9; 32],
            },
            flags: CxFlags {
                ungrounded: true,
                ..CxFlags::default()
            },
        }
    }

    #[test]
    fn put_get_roundtrips_base_and_slot_cfs() {
        let vault = AsterVault::with_clock(vault_id(), b"salt".to_vec(), FixedClock::new(123));
        let cx = sample_constellation(&vault);
        let id = cx.cx_id;

        vault.put(cx.clone()).expect("put");
        let got = vault.get(id, vault.snapshot()).expect("get");

        assert_eq!(got, cx);
        assert!(matches!(
            got.slots.get(&SlotId::new(1)),
            Some(SlotVector::Absent {
                reason: AbsentReason::LensUnavailable
            })
        ));
    }

    #[test]
    fn duplicate_put_is_idempotent_noop() {
        let vault = AsterVault::with_clock(vault_id(), b"salt".to_vec(), FixedClock::new(123));
        let cx = sample_constellation(&vault);

        vault.put(cx.clone()).expect("first put");
        let seq_after_first = vault.snapshot();
        vault.put(cx).expect("duplicate put");

        assert_eq!(vault.snapshot(), seq_after_first);
    }

    #[test]
    fn same_cxid_with_different_bytes_fails_closed() {
        let vault = AsterVault::with_clock(vault_id(), b"salt".to_vec(), FixedClock::new(123));
        let cx = sample_constellation(&vault);
        let mut changed = cx.clone();
        changed.created_at += 1;

        vault.put(cx).expect("first put");
        let error = vault.put(changed).expect_err("collision rejected");

        assert_eq!(error.code, "CALYX_ASTER_CORRUPT_SHARD");
    }

    #[test]
    fn anchor_writes_anchor_cf_and_updates_get() {
        let vault = AsterVault::with_clock(vault_id(), b"salt".to_vec(), FixedClock::new(123));
        let cx = sample_constellation(&vault);
        let id = cx.cx_id;
        let anchor = Anchor {
            kind: AnchorKind::Reward,
            value: AnchorValue::Number(1.0),
            source: "unit-test".to_string(),
            observed_at: 124,
            confidence: 1.0,
        };

        vault.put(cx).expect("put");
        vault.anchor(id, anchor.clone()).expect("anchor");
        let got = vault.get(id, vault.snapshot()).expect("get anchored");
        let anchor_bytes = vault
            .read_cf_at(
                vault.snapshot(),
                ColumnFamily::Anchors,
                &anchor_key(id, &AnchorKind::Reward),
            )
            .expect("read anchor cf")
            .expect("anchor row");

        assert_eq!(got.anchors.as_slice(), std::slice::from_ref(&anchor));
        assert_eq!(encode::decode_anchor(&anchor_bytes).unwrap(), anchor);
    }

    #[test]
    fn duplicate_put_after_anchor_preserves_anchor_noop() {
        let vault = AsterVault::with_clock(vault_id(), b"salt".to_vec(), FixedClock::new(123));
        let cx = sample_constellation(&vault);
        let id = cx.cx_id;
        let anchor = Anchor {
            kind: AnchorKind::Reward,
            value: AnchorValue::Number(1.0),
            source: "unit-test".to_string(),
            observed_at: 124,
            confidence: 1.0,
        };

        vault.put(cx.clone()).expect("put");
        vault.anchor(id, anchor.clone()).expect("anchor");
        let seq_after_anchor = vault.snapshot();
        vault.put(cx).expect("duplicate put after anchor");
        let got = vault.get(id, vault.snapshot()).expect("get anchored");

        assert_eq!(vault.snapshot(), seq_after_anchor);
        assert_eq!(got.anchors.as_slice(), std::slice::from_ref(&anchor));
    }

    #[test]
    fn binary_codecs_roundtrip_known_offsets_and_fail_closed() {
        let vault = AsterVault::with_clock(vault_id(), b"salt".to_vec(), FixedClock::new(123));
        let cx = sample_constellation(&vault);
        let header = encode::encode_header(&cx);

        assert_eq!(&header[0..16], cx.cx_id.as_bytes());
        assert_eq!(&header[32..36], &7_u32.to_be_bytes());
        assert_eq!(header.len(), encode::HEADER_LEN);
        assert_eq!(encode::decode_header(&header).unwrap().cx_id, cx.cx_id);

        let base = encode::encode_constellation_base(&cx).expect("encode base");
        let decoded = encode::decode_constellation_base(&base).expect("decode base");
        assert_eq!(decoded.cx_id, cx.cx_id);
        assert_eq!(decoded.input_ref, cx.input_ref);
        assert!(encode::decode_header(&header[..encode::HEADER_LEN - 1]).is_err());

        for vector in cx.slots.values() {
            let bytes = encode::encode_slot_vector(vector).expect("encode slot");
            assert_eq!(encode::decode_slot_vector(&bytes).unwrap(), *vector);
        }
        let anchor = Anchor {
            kind: AnchorKind::Label("axis".to_string()),
            value: AnchorValue::Text("grounded".to_string()),
            source: "unit-test".to_string(),
            observed_at: 125,
            confidence: 0.5,
        };
        let bytes = encode::encode_anchor(&anchor).expect("encode anchor");
        assert_eq!(encode::decode_anchor(&bytes).unwrap(), anchor);
        assert!(encode::decode_anchor(&bytes[..bytes.len() - 1]).is_err());
    }

    #[test]
    fn durable_vault_writes_wal_sst_manifest_and_cold_opens() {
        let dir = test_dir("durable");
        let vault =
            AsterVault::new_durable(&dir, vault_id(), b"salt".to_vec(), VaultOptions::default())
                .expect("open durable");
        let cx = sample_constellation(&AsterVault::with_clock(
            vault_id(),
            b"salt".to_vec(),
            FixedClock::new(123),
        ));
        let id = cx.cx_id;

        vault.put(cx.clone()).expect("durable put");
        vault.flush().expect("flush durable");

        let wal = dir.join("wal/00000000000000000000.wal");
        let wal_bytes = fs::read(&wal).expect("read wal");
        assert_eq!(&wal_bytes[0..4], b"CXW1");
        assert!(dir.join("CURRENT").exists());
        assert_eq!(sst_count(dir.join("cf/base")), 1);

        let reopened =
            AsterVault::open(&dir, vault_id(), b"salt".to_vec(), VaultOptions::default())
                .expect("cold open");
        assert_eq!(reopened.snapshot(), 1);
        assert_eq!(reopened.get(id, reopened.snapshot()).unwrap(), cx);
        cleanup(dir);
    }

    #[test]
    fn durable_open_empty_dir_starts_at_zero() {
        let dir = test_dir("durable-empty");
        let vault = AsterVault::open(&dir, vault_id(), b"salt".to_vec(), VaultOptions::default())
            .expect("open empty durable");

        assert_eq!(vault.snapshot(), 0);
        cleanup(dir);
    }

    fn test_dir(name: &str) -> PathBuf {
        let id = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "calyx-aster-vault-{name}-{}-{id}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create test dir");
        dir
    }

    fn sst_count(dir: PathBuf) -> usize {
        fs::read_dir(dir)
            .unwrap()
            .filter(|entry| entry.as_ref().unwrap().path().extension().unwrap() == "sst")
            .count()
    }

    fn cleanup(dir: PathBuf) {
        fs::remove_dir_all(dir).expect("cleanup test dir");
    }
}
