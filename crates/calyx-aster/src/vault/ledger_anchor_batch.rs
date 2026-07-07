use std::collections::BTreeSet;

use super::{AsterVault, encode, ledger_hook};
use crate::cf::{ColumnFamily, anchor_key, base_key, ledger_key};
use crate::ledger_view::parse_aster_ledger_seq;
use calyx_core::{Anchor, CalyxError, Clock, CxId, LedgerRef, Result, SystemClock, VaultStore};
use calyx_ledger::{
    ActorId, EntryKind, LedgerAppender, LedgerCfStore, LedgerHeadAnchor, LedgerRow, SubjectId,
    decode as decode_ledger,
};

struct AnchorBatchLedgerInput {
    kind: EntryKind,
    subject: SubjectId,
    payload: Vec<u8>,
    actor: ActorId,
}

impl<C> AsterVault<C>
where
    C: Clock,
{
    /// Adds multiple anchors and stamps the stored base row with one shared ledger ref.
    ///
    /// This is for one semantic grounding event that has more than one anchor axis. The batch is
    /// idempotent only when every requested anchor already exists and the Base provenance points to
    /// the same requested ledger entry. A partial pre-existing batch fails closed so a legacy
    /// unstamped anchor is never silently upgraded.
    pub fn anchors_with_ledger_entry(
        &self,
        id: CxId,
        anchors: Vec<Anchor>,
        kind: EntryKind,
        subject: SubjectId,
        payload: Vec<u8>,
        actor: ActorId,
    ) -> Result<LedgerRef> {
        validate_anchor_batch(&anchors)?;
        let entry = AnchorBatchLedgerInput {
            kind,
            subject,
            payload,
            actor,
        };
        self.with_durable_commit_lock(|| {
            let latest = self.snapshot();
            let mut constellation = self.get(id, latest)?;
            let mut missing = Vec::new();
            let mut existing_count = 0usize;
            for anchor in &anchors {
                match classify_anchor_state(self, latest, id, &constellation, anchor)? {
                    AnchorState::Existing => existing_count += 1,
                    AnchorState::Missing => missing.push(anchor.clone()),
                }
            }
            if existing_count == anchors.len() {
                validate_existing_batch_ledger(
                    self,
                    latest,
                    id,
                    &constellation.provenance,
                    &entry,
                )?;
                return Ok(constellation.provenance.clone());
            }
            if existing_count != 0 {
                return Err(CalyxError::aster_corrupt_shard(format!(
                    "partial anchor batch for {id}: {existing_count} existing anchors and {} \
                     missing anchors",
                    missing.len()
                )));
            }

            let Some(hook) = &self.ledger_hook else {
                let store = AnchorBatchRawLedgerStore { vault: self };
                let appender = LedgerAppender::open(store, SystemClock)?;
                let prepared =
                    appender.prepare(entry.kind, entry.subject, entry.payload, entry.actor)?;
                let ledger_ref = prepared.ledger_ref();
                let mut rows = anchor_batch_rows_with_ledger_ref(
                    id,
                    &mut constellation,
                    &missing,
                    &ledger_ref,
                )?;
                rows.push(encode::WriteRow {
                    cf: ColumnFamily::Ledger,
                    key: ledger_key(prepared.seq()),
                    value: prepared.bytes().to_vec(),
                });
                self.commit_rows_locked(&rows)?;
                return Ok(ledger_ref);
            };

            let mut guard = ledger_hook::lock_hook(hook)?;
            let staged = guard.stage_with_checkpoints(
                entry.kind,
                entry.subject,
                entry.payload,
                entry.actor,
            )?;
            let ledger_ref = staged
                .first()
                .ok_or_else(|| CalyxError::ledger_group_commit_failed("no staged ledger rows"))?
                .ledger_ref();
            let mut rows =
                anchor_batch_rows_with_ledger_ref(id, &mut constellation, &missing, &ledger_ref)?;
            rows.extend(staged.iter().map(|row| encode::WriteRow {
                cf: ColumnFamily::Ledger,
                key: row.key().to_vec(),
                value: row.value().to_vec(),
            }));
            self.commit_rows_locked(&rows)?;
            for row in &staged {
                guard.commit_staged(row)?;
            }
            Ok(ledger_ref)
        })
    }
}

enum AnchorState {
    Existing,
    Missing,
}

fn classify_anchor_state<C: Clock>(
    vault: &AsterVault<C>,
    snapshot: u64,
    id: CxId,
    constellation: &calyx_core::Constellation,
    anchor: &Anchor,
) -> Result<AnchorState> {
    let key = anchor_key(id, &anchor.kind);
    let anchor_bytes = encode::encode_anchor(anchor)?;
    let matching_base_anchor_count = constellation
        .anchors
        .iter()
        .filter(|existing| existing.kind == anchor.kind)
        .count();
    let exact_base_anchor_count = constellation
        .anchors
        .iter()
        .filter(|existing| *existing == anchor)
        .count();
    match vault.read_cf_at(snapshot, ColumnFamily::Anchors, &key)? {
        Some(existing_bytes) => {
            let stored_anchor = encode::decode_anchor(&existing_bytes)?;
            if existing_bytes != anchor_bytes || stored_anchor != *anchor {
                return Err(CalyxError::aster_corrupt_shard(format!(
                    "conflicting Anchors CF row for {id} {:?}; existing persisted row does not \
                     match requested anchor",
                    anchor.kind
                )));
            }
            if matching_base_anchor_count != 1 || exact_base_anchor_count != 1 {
                return Err(CalyxError::aster_corrupt_shard(format!(
                    "Anchors CF row for {id} {:?} matches request but Base row has {} \
                     matching-kind anchors",
                    anchor.kind, matching_base_anchor_count
                )));
            }
            Ok(AnchorState::Existing)
        }
        None => {
            if matching_base_anchor_count != 0 {
                return Err(CalyxError::aster_corrupt_shard(format!(
                    "Base row for {id} already has {} {:?} anchors but Anchors CF row is missing",
                    matching_base_anchor_count, anchor.kind
                )));
            }
            Ok(AnchorState::Missing)
        }
    }
}

fn validate_existing_batch_ledger<C: Clock>(
    vault: &AsterVault<C>,
    snapshot: u64,
    id: CxId,
    ledger_ref: &LedgerRef,
    entry: &AnchorBatchLedgerInput,
) -> Result<()> {
    let row = vault
        .read_cf_at(snapshot, ColumnFamily::Ledger, &ledger_key(ledger_ref.seq))?
        .ok_or_else(|| {
            CalyxError::aster_corrupt_shard(format!(
                "matching anchor batch for {id} has provenance seq {} but Ledger CF row is missing",
                ledger_ref.seq
            ))
        })?;
    let stored = decode_ledger(&row)?;
    if stored.seq != ledger_ref.seq || stored.entry_hash != ledger_ref.hash {
        return Err(CalyxError::aster_corrupt_shard(format!(
            "matching anchor batch for {id} has provenance ref that does not match Ledger CF row"
        )));
    }
    if stored.kind != entry.kind
        || stored.subject != entry.subject
        || stored.payload != entry.payload
        || stored.actor != entry.actor
    {
        return Err(CalyxError::aster_corrupt_shard(format!(
            "matching anchor batch for {id} is not backed by the requested ledger entry"
        )));
    }
    Ok(())
}

fn anchor_batch_rows_with_ledger_ref(
    id: CxId,
    constellation: &mut calyx_core::Constellation,
    anchors: &[Anchor],
    ledger_ref: &LedgerRef,
) -> Result<Vec<encode::WriteRow>> {
    constellation.provenance = ledger_ref.clone();
    constellation.anchors.extend_from_slice(anchors);
    constellation.flags.ungrounded = false;
    constellation.validate_schema()?;
    let mut rows = Vec::with_capacity(1 + anchors.len());
    rows.push(encode::WriteRow {
        cf: ColumnFamily::Base,
        key: base_key(id),
        value: encode::encode_constellation_base(constellation)?,
    });
    for anchor in anchors {
        rows.push(encode::WriteRow {
            cf: ColumnFamily::Anchors,
            key: anchor_key(id, &anchor.kind),
            value: encode::encode_anchor(anchor)?,
        });
    }
    Ok(rows)
}

fn validate_anchor_batch(anchors: &[Anchor]) -> Result<()> {
    if anchors.is_empty() {
        return Err(CalyxError::aster_corrupt_shard(
            "ledger-stamped anchor batch must contain at least one anchor",
        ));
    }
    let mut kinds = BTreeSet::new();
    for anchor in anchors {
        anchor.validate_schema()?;
        if !kinds.insert(anchor.kind.clone()) {
            return Err(CalyxError::aster_corrupt_shard(format!(
                "ledger-stamped anchor batch contains duplicate anchor kind {:?}",
                anchor.kind
            )));
        }
    }
    Ok(())
}

struct AnchorBatchRawLedgerStore<'a, C> {
    vault: &'a AsterVault<C>,
}

impl<C> LedgerCfStore for AnchorBatchRawLedgerStore<'_, C>
where
    C: Clock,
{
    fn scan(&self) -> Result<Vec<LedgerRow>> {
        let mut rows = Vec::new();
        for (key, bytes) in self
            .vault
            .scan_cf_at(self.vault.snapshot(), ColumnFamily::Ledger)?
        {
            rows.push(LedgerRow {
                seq: parse_aster_ledger_seq(&key)?,
                bytes,
            });
        }
        rows.sort_by_key(|row| row.seq);
        Ok(rows)
    }

    fn put_new(&mut self, seq: u64, bytes: &[u8]) -> Result<()> {
        let key = ledger_key(seq);
        if self
            .vault
            .read_cf_at(self.vault.snapshot(), ColumnFamily::Ledger, &key)?
            .is_some()
        {
            return Err(CalyxError::ledger_append_only_violation(format!(
                "ledger seq {seq} already exists"
            )));
        }
        self.vault
            .write_cf(ColumnFamily::Ledger, key, bytes.to_vec())
            .map(|_| ())
    }

    fn head_anchor(&self) -> Result<Option<LedgerHeadAnchor>> {
        match &self.vault.durable {
            Some(durable) => crate::ledger_head::read_head_anchor(durable.root()),
            None => Ok(None),
        }
    }

    fn put_head_anchor(&mut self, anchor: &LedgerHeadAnchor) -> Result<()> {
        if let Some(durable) = &self.vault.durable {
            crate::ledger_head::write_head_anchor(durable.root(), anchor)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use calyx_core::{
        AnchorKind, AnchorValue, CxFlags, FixedClock, InputRef, Modality, SlotVector, VaultId,
    };

    #[test]
    fn batch_anchor_write_is_atomic_and_idempotent() {
        let dir =
            std::env::temp_dir().join(format!("calyx-aster-batch-anchor-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let vault = AsterVault::new_durable_with_clock(
            &dir,
            vault_id(),
            b"batch-anchor".to_vec(),
            crate::vault::VaultOptions::default(),
            FixedClock::new(1),
        )
        .expect("durable vault");
        let base = sample_constellation(&vault);
        let id = base.cx_id;
        vault.put(base).expect("put base");

        let anchors = vec![test_pass_anchor(true), label_anchor("YES")];
        let payload = br#"{"event":"test.batch_grounding"}"#.to_vec();
        let first = vault
            .anchors_with_ledger_entry(
                id,
                anchors.clone(),
                EntryKind::Grounding,
                SubjectId::Cx(id),
                payload.clone(),
                ActorId::Service("test".to_string()),
            )
            .expect("batch anchor write");
        let second = vault
            .anchors_with_ledger_entry(
                id,
                anchors,
                EntryKind::Grounding,
                SubjectId::Cx(id),
                payload,
                ActorId::Service("test".to_string()),
            )
            .expect("duplicate batch anchor write");
        let snapshot = vault.snapshot();
        let stored = vault.get(id, snapshot).expect("stored constellation");
        let test_pass = vault
            .read_cf_at(
                snapshot,
                ColumnFamily::Anchors,
                &anchor_key(id, &AnchorKind::TestPass),
            )
            .expect("read test_pass")
            .expect("test_pass anchor");
        let label = vault
            .read_cf_at(
                snapshot,
                ColumnFamily::Anchors,
                &anchor_key(id, &AnchorKind::Label("outcome".to_string())),
            )
            .expect("read label")
            .expect("label anchor");

        assert_eq!(first, second);
        assert_eq!(stored.anchors.len(), 2);
        assert_eq!(
            encode::decode_anchor(&test_pass).unwrap().value,
            AnchorValue::Bool(true)
        );
        assert_eq!(
            encode::decode_anchor(&label).unwrap().value,
            AnchorValue::Enum("YES".to_string())
        );

        drop(vault);
        std::fs::remove_dir_all(dir).expect("remove batch anchor vault");
    }

    fn sample_constellation(vault: &AsterVault<FixedClock>) -> calyx_core::Constellation {
        let input = b"batch-anchor-input";
        let cx_id = vault.cx_id_for_input(input, 7);
        let mut input_hash = [0_u8; 32];
        input_hash[..input.len()].copy_from_slice(input);
        calyx_core::Constellation {
            cx_id,
            vault_id: vault_id(),
            panel_version: 7,
            created_at: 1,
            input_ref: InputRef {
                hash: input_hash,
                pointer: None,
                redacted: false,
            },
            modality: Modality::Structured,
            slots: [(
                calyx_core::SlotId::new(0),
                SlotVector::Dense {
                    dim: 1,
                    data: vec![1.0],
                },
            )]
            .into(),
            scalars: [("x".to_string(), 1.0)].into(),
            metadata: [("record_type".to_string(), "batch-anchor-test".to_string())].into(),
            anchors: Vec::new(),
            provenance: LedgerRef {
                seq: 0,
                hash: [0; 32],
            },
            flags: CxFlags {
                ungrounded: true,
                ..CxFlags::default()
            },
        }
    }

    fn test_pass_anchor(value: bool) -> Anchor {
        Anchor {
            kind: AnchorKind::TestPass,
            value: AnchorValue::Bool(value),
            source: "uma:test:YES".to_string(),
            observed_at: 1,
            confidence: 1.0,
        }
    }

    fn label_anchor(value: &str) -> Anchor {
        Anchor {
            kind: AnchorKind::Label("outcome".to_string()),
            value: AnchorValue::Enum(value.to_string()),
            source: "uma:test".to_string(),
            observed_at: 1,
            confidence: 1.0,
        }
    }

    fn vault_id() -> VaultId {
        "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().expect("vault id")
    }
}
