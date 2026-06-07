//! Aster `VaultStore` implementation over the PH08 MVCC CF table.

use crate::cf::{ColumnFamily, anchor_key, base_key, ledger_key, slot_key};
use crate::mvcc::{CfRead, Freshness, ReaderLease, Snapshot, VersionedCfStore};
use calyx_core::{
    Anchor, CalyxError, Clock, Constellation, CxId, Result, Seq, SlotId, SystemClock, VaultId,
    VaultStore,
};
use std::collections::BTreeMap;

const DEFAULT_LEASE_MS: u64 = 5_000;

/// Single-vault Aster store with content-addressed ingest semantics.
#[derive(Debug)]
pub struct AsterVault<C = SystemClock> {
    vault_id: VaultId,
    vault_salt: Vec<u8>,
    clock: C,
    rows: VersionedCfStore,
}

impl AsterVault<SystemClock> {
    /// Creates a vault using the system clock.
    pub fn new(vault_id: VaultId, vault_salt: impl Into<Vec<u8>>) -> Self {
        Self::with_clock(vault_id, vault_salt, SystemClock)
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
        let base_bytes = encode(&constellation, "base constellation")?;
        let latest = self.snapshot();
        if let Some(existing) = self.rows.read_at(
            self.snapshot_handle(latest),
            ColumnFamily::Base,
            &base_key,
            &self.clock,
        )? {
            if existing == base_bytes {
                return Ok(id);
            }
            let existing: Constellation = decode(&existing, "base constellation")?;
            if same_ingest_identity(&existing, &constellation) {
                return Ok(id);
            }
            return Err(CalyxError::aster_corrupt_shard(
                "CxId collision or non-idempotent duplicate constellation",
            ));
        }

        let mut rows = Vec::new();
        rows.push((ColumnFamily::Base, base_key, base_bytes));
        for (slot, vector) in &constellation.slots {
            rows.push((
                ColumnFamily::slot(*slot),
                slot_key(id),
                encode(vector, "slot vector")?,
            ));
        }
        for anchor in &constellation.anchors {
            rows.push((
                ColumnFamily::Anchors,
                anchor_key(id, &anchor.kind),
                encode(anchor, "anchor")?,
            ));
        }
        rows.push((
            ColumnFamily::Ledger,
            ledger_key(constellation.provenance.seq),
            encode(&constellation.provenance, "ledger ref")?,
        ));
        self.rows.commit_batch(rows)?;
        Ok(id)
    }

    fn get(&self, id: CxId, snapshot: Seq) -> Result<Constellation> {
        let handle = self.snapshot_handle(snapshot);
        let base = self
            .rows
            .read_at(handle, ColumnFamily::Base, &base_key(id), &self.clock)?
            .ok_or_else(|| CalyxError::stale_derived("constellation missing at snapshot"))?;
        let mut constellation: Constellation = decode(&base, "base constellation")?;
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
            slots.insert(slot, decode(&value, "slot vector")?);
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
                encode(&constellation, "base constellation")?,
            ),
            (
                ColumnFamily::Anchors,
                anchor_key(id, &anchor.kind),
                encode(&anchor, "anchor")?,
            ),
        ];
        self.rows.commit_batch(rows)?;
        Ok(())
    }

    fn snapshot(&self) -> Seq {
        self.latest_seq()
    }
}

fn encode<T>(value: &T, label: &str) -> Result<Vec<u8>>
where
    T: serde::Serialize,
{
    serde_json::to_vec(value)
        .map_err(|error| CalyxError::aster_corrupt_shard(format!("encode {label}: {error}")))
}

fn decode<T>(bytes: &[u8], label: &str) -> Result<T>
where
    T: for<'de> serde::Deserialize<'de>,
{
    serde_json::from_slice(bytes)
        .map_err(|error| CalyxError::aster_corrupt_shard(format!("decode {label}: {error}")))
}

fn same_ingest_identity(left: &Constellation, right: &Constellation) -> bool {
    left.cx_id == right.cx_id
        && left.vault_id == right.vault_id
        && left.panel_version == right.panel_version
        && left.created_at == right.created_at
        && left.input_ref == right.input_ref
        && left.modality == right.modality
        && left.slots == right.slots
        && left.scalars == right.scalars
        && left.provenance == right.provenance
        && left.flags == right.flags
}

#[cfg(test)]
mod tests {
    use super::*;
    use calyx_core::{
        AbsentReason, AnchorKind, AnchorValue, CxFlags, FixedClock, InputRef, LedgerRef, Modality,
        SlotVector,
    };
    use std::collections::BTreeMap;

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
        assert_eq!(decode::<Anchor>(&anchor_bytes, "anchor").unwrap(), anchor);
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
}
