use std::collections::BTreeMap;

use super::{AsterVault, encode, ledger_hook, ledger_stub};
use crate::cf::{ColumnFamily, base_key, ledger_key};
use crate::dedup::{AnchorConflictResult, check_anchor_conflict};
use calyx_core::{CalyxError, Clock, Constellation, CxId, LedgerRef, Result, VaultStore};
use calyx_ledger::{PayloadBuilder, RedactionPolicy};
use serde_json::json;

const BATCH_ACTOR: &str = "calyx-aster-batch-ingest";

impl<C> AsterVault<C>
where
    C: Clock,
{
    pub fn put_batch<I>(&self, constellations: I) -> Result<Vec<CxId>>
    where
        I: IntoIterator<Item = Constellation>,
    {
        let input = constellations.into_iter().collect::<Vec<_>>();
        if input.is_empty() {
            return Ok(Vec::new());
        }
        self.with_durable_commit_lock(|| self.put_batch_locked(input))
    }

    fn put_batch_locked(&self, input: Vec<Constellation>) -> Result<Vec<CxId>> {
        let latest = self.snapshot();
        let mut accepted_bases = BTreeMap::<Vec<u8>, Vec<u8>>::new();
        let mut accepted = Vec::<Constellation>::new();
        let mut ids = Vec::with_capacity(input.len());
        for constellation in input {
            if constellation.vault_id != self.vault_id {
                return Err(CalyxError::vault_access_denied(
                    "constellation belongs to another vault",
                ));
            }
            constellation.validate_schema()?;
            let id = constellation.cx_id;
            let key = base_key(id);
            let base = encode::encode_constellation_base(&constellation)?;
            if let Some(existing) = self.rows.read_at(
                self.snapshot_handle(latest),
                ColumnFamily::Base,
                &key,
                &self.clock,
            )? {
                accept_duplicate_or_error(&base, &existing)?;
                ids.push(id);
                continue;
            }
            if let Some(existing) = accepted_bases.get(&key) {
                accept_duplicate_or_error(&base, existing)?;
                ids.push(id);
                continue;
            }
            accepted_bases.insert(key, base);
            ids.push(id);
            accepted.push(constellation);
        }
        if accepted.is_empty() {
            return Ok(ids);
        }
        let mut rows = Vec::new();
        let mut hook_guard = match &self.ledger_hook {
            Some(hook) => Some(ledger_hook::lock_hook(hook)?),
            None => None,
        };
        let staged_ledger = if let Some(hook) = hook_guard.as_deref() {
            Some(ledger_hook::stage_ingest_payload(
                hook,
                &mut rows,
                accepted[0].cx_id,
                batch_payload(&accepted),
            )?)
        } else {
            rows.push(encode::WriteRow {
                cf: ColumnFamily::Ledger,
                key: ledger_key(self.latest_seq().saturating_add(1)),
                value: ledger_stub::encode(self.latest_seq().saturating_add(1)),
            });
            None
        };
        let ledger_ref = staged_ledger
            .as_ref()
            .and_then(|staged| staged.first())
            .map(|row| row.ledger_ref())
            .unwrap_or(LedgerRef {
                seq: self.latest_seq().saturating_add(1),
                hash: [0; 32],
            });
        for mut constellation in accepted {
            constellation.provenance = ledger_ref.clone();
            self.stage_constellation_rows(&mut rows, &constellation)?;
        }
        self.commit_rows_locked(&rows)?;
        if let (Some(hook), Some(staged)) = (hook_guard.as_deref_mut(), staged_ledger.as_ref()) {
            ledger_hook::commit_staged(hook, staged)?;
        }
        Ok(ids)
    }
}

fn accept_duplicate_or_error(incoming: &[u8], existing: &[u8]) -> Result<()> {
    if existing == incoming {
        return Ok(());
    }
    if encode::same_constellation_identity(existing, incoming)? {
        let existing_cx = encode::decode_constellation_base(existing)?;
        let incoming_cx = encode::decode_constellation_base(incoming)?;
        if let AnchorConflictResult::Conflicting {
            anchor_type,
            reason,
        } = check_anchor_conflict(&incoming_cx, &existing_cx)
        {
            return Err(CalyxError::aster_corrupt_shard(format!(
                "CxId duplicate has conflicting {anchor_type:?} anchor: {reason:?}"
            )));
        }
        return Ok(());
    }
    Err(CalyxError::aster_corrupt_shard(
        "CxId collision or non-idempotent duplicate constellation",
    ))
}

fn batch_payload(constellations: &[Constellation]) -> Vec<u8> {
    let mut payload = PayloadBuilder::default();
    let hashes = constellations
        .iter()
        .take(32)
        .map(|cx| hex(&cx.input_ref.hash))
        .collect::<Vec<_>>();
    payload
        .insert_str("mode", BATCH_ACTOR)
        .insert_u64("count", constellations.len() as u64)
        .insert_str("first_cx_id", constellations[0].cx_id.to_string())
        .insert_str(
            "last_cx_id",
            constellations
                .last()
                .expect("non-empty batch")
                .cx_id
                .to_string(),
        )
        .insert_value("input_hash_prefix", json!(hashes));
    RedactionPolicy::default().apply_to_payload(&payload)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
