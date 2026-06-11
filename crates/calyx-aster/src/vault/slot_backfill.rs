use super::{AsterVault, encode};
use crate::cf::{ColumnFamily, base_key, slot_key};
use calyx_core::{CalyxError, Clock, CxId, Result, Seq, SlotId, SlotVector};

impl<C> AsterVault<C>
where
    C: Clock,
{
    pub fn put_slot_vector(
        &self,
        cx_id: CxId,
        slot_id: SlotId,
        vector: &SlotVector,
    ) -> Result<Seq> {
        self.ensure_base_exists(cx_id)?;
        let row = encode::WriteRow {
            cf: ColumnFamily::slot(slot_id),
            key: slot_key(cx_id),
            value: encode::encode_slot_vector(vector)?,
        };
        self.commit_rows(&[row])
    }

    pub fn read_slot_vector_at(
        &self,
        snapshot: Seq,
        cx_id: CxId,
        slot_id: SlotId,
    ) -> Result<Option<SlotVector>> {
        self.read_cf_at(snapshot, ColumnFamily::slot(slot_id), &slot_key(cx_id))?
            .map(|bytes| encode::decode_slot_vector(&bytes))
            .transpose()
    }

    fn ensure_base_exists(&self, cx_id: CxId) -> Result<()> {
        if self
            .read_cf_at(self.latest_seq(), ColumnFamily::Base, &base_key(cx_id))?
            .is_some()
        {
            return Ok(());
        }
        Err(CalyxError::stale_derived(format!(
            "constellation {cx_id} missing for slot backfill"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use calyx_core::{
        AbsentReason, CxFlags, FixedClock, InputRef, LedgerRef, Modality, VaultId, VaultStore,
    };
    use std::collections::BTreeMap;

    #[test]
    fn backfill_slot_writes_slot_cf_without_rewriting_base() {
        let vault = AsterVault::with_clock(vault_id(), b"salt".to_vec(), FixedClock::new(123));
        let cx = constellation(&vault);
        let id = cx.cx_id;

        vault.put(cx.clone()).expect("put base constellation");
        let before = vault
            .read_cf_at(vault.snapshot(), ColumnFamily::Base, &base_key(id))
            .expect("read base before")
            .expect("base row before");
        let new_slot = SlotId::new(1);
        let placeholder = SlotVector::Absent {
            reason: AbsentReason::Deferred,
        };

        let placeholder_seq = vault
            .put_slot_vector(id, new_slot, &placeholder)
            .expect("write placeholder");
        assert_eq!(
            vault
                .read_slot_vector_at(placeholder_seq, id, new_slot)
                .expect("read placeholder"),
            Some(placeholder)
        );

        let dense = SlotVector::Dense {
            dim: 2,
            data: vec![0.6, 0.8],
        };
        let dense_seq = vault
            .put_slot_vector(id, new_slot, &dense)
            .expect("write dense slot");
        let after = vault
            .read_cf_at(dense_seq, ColumnFamily::Base, &base_key(id))
            .expect("read base after")
            .expect("base row after");

        assert_eq!(before, after);
        assert_eq!(
            vault
                .read_slot_vector_at(dense_seq, id, new_slot)
                .expect("read dense"),
            Some(dense)
        );
        assert_eq!(vault.get(id, dense_seq).expect("historical get"), cx);
    }

    #[test]
    fn missing_constellation_backfill_fails_closed() {
        let vault = AsterVault::with_clock(vault_id(), b"salt".to_vec(), FixedClock::new(123));
        let error = vault
            .put_slot_vector(
                CxId::from_bytes([9; 16]),
                SlotId::new(1),
                &SlotVector::Absent {
                    reason: AbsentReason::Deferred,
                },
            )
            .expect_err("missing base rejected");

        assert_eq!(error.code, "CALYX_STALE_DERIVED");
    }

    fn vault_id() -> VaultId {
        "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().expect("valid ULID")
    }

    fn constellation(vault: &AsterVault<FixedClock>) -> calyx_core::Constellation {
        let input = b"slot-backfill";
        let cx_id = vault.cx_id_for_input(input, 1);
        let mut input_hash = [0_u8; 32];
        input_hash[..input.len()].copy_from_slice(input);
        let mut slots = BTreeMap::new();
        slots.insert(
            SlotId::new(0),
            SlotVector::Dense {
                dim: 2,
                data: vec![1.0, 0.0],
            },
        );
        calyx_core::Constellation {
            cx_id,
            vault_id: vault_id(),
            panel_version: 1,
            created_at: 123,
            input_ref: InputRef {
                hash: input_hash,
                pointer: Some("synthetic://slot-backfill".to_string()),
                redacted: false,
            },
            modality: Modality::Text,
            slots,
            scalars: BTreeMap::new(),
            metadata: BTreeMap::new(),
            anchors: Vec::new(),
            provenance: LedgerRef {
                seq: 1,
                hash: [7; 32],
            },
            flags: CxFlags {
                ungrounded: true,
                ..CxFlags::default()
            },
        }
    }
}
