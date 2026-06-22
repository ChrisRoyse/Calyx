use std::collections::BTreeMap;

use calyx_aster::cf::{ColumnFamily, anchor_key, base_key, slot_key};
use calyx_aster::vault::encode;
use calyx_core::{
    Anchor, Clock, Constellation, CxFlags, InputRef, LedgerRef, Modality, SlotVector, SystemClock,
    VaultStore,
};
use calyx_ledger::{ActorId, EntryKind, PayloadBuilder, RedactionPolicy, SubjectId};
use serde_json::json;

use super::{
    LearnerOriginService, ORIGIN_ACTOR, ORIGIN_PANEL_VERSION, ORIGIN_SLOT_ID, OriginError, hex,
    sha256_array, storage_error,
};

type CfWrite = (ColumnFamily, Vec<u8>, Vec<u8>);

pub(super) struct OriginRow {
    pub(super) cx: Constellation,
}

pub(super) struct StoredRow {
    pub(super) cx_id: String,
    pub(super) ledger_seq: u64,
    pub(super) ledger_hash: String,
}

pub(super) struct OriginCommit {
    pub(super) kind: &'static str,
    pub(super) primary_id: String,
    pub(super) ledger_kind: EntryKind,
    pub(super) metadata: BTreeMap<String, String>,
    pub(super) scalars: BTreeMap<String, f64>,
    pub(super) slot_values: [f32; 4],
    pub(super) anchors: Vec<Anchor>,
}

impl LearnerOriginService {
    pub(super) fn commit_origin_row(&self, commit: OriginCommit) -> Result<StoredRow, OriginError> {
        let payload_hash = commit
            .metadata
            .get("payload_sha256")
            .ok_or_else(|| OriginError::internal("missing payload hash"))?
            .clone();
        let input_bytes = serde_json::to_vec(&json!({
            "kind": commit.kind,
            "primaryId": commit.primary_id,
            "payloadSha256": payload_hash
        }))
        .map_err(|error| OriginError::internal(error.to_string()))?;
        let cx_id = self
            .vault
            .cx_id_for_input(&input_bytes, ORIGIN_PANEL_VERSION);
        let slot = SlotVector::Dense {
            dim: 4,
            data: commit.slot_values.to_vec(),
        };
        let constellation = Constellation {
            cx_id,
            vault_id: self.vault.vault_id(),
            panel_version: ORIGIN_PANEL_VERSION,
            created_at: SystemClock.now(),
            input_ref: InputRef {
                hash: sha256_array(&input_bytes),
                pointer: None,
                redacted: true,
            },
            modality: Modality::Structured,
            slots: BTreeMap::from([(ORIGIN_SLOT_ID, slot)]),
            scalars: commit.scalars,
            metadata: commit.metadata,
            anchors: commit.anchors,
            provenance: LedgerRef {
                seq: 0,
                hash: [0; 32],
            },
            flags: CxFlags {
                ungrounded: true,
                redacted_input: true,
                ..CxFlags::default()
            },
        };
        let rows = encode_origin_rows(&constellation)?;
        let mut payload = PayloadBuilder::default();
        payload
            .insert_str("cx_id", cx_id.to_string())
            .insert_str("input_hash", payload_hash)
            .insert_u64("ts", SystemClock.now());
        let ledger_payload = RedactionPolicy::default().apply_to_payload(&payload);
        self.vault
            .write_cf_batch_with_ledger_entry(
                rows,
                commit.ledger_kind,
                SubjectId::Cx(cx_id),
                ledger_payload,
                ActorId::Service(ORIGIN_ACTOR.to_string()),
            )
            .map_err(storage_error)?;
        self.vault.flush().map_err(storage_error)?;
        let stored = self
            .vault
            .get(cx_id, self.vault.latest_seq())
            .map_err(storage_error)?;
        Ok(StoredRow {
            cx_id: cx_id.to_string(),
            ledger_seq: stored.provenance.seq,
            ledger_hash: hex(&stored.provenance.hash),
        })
    }

    pub(super) fn find_by_idempotency(
        &self,
        kind: &'static str,
        id_key: &str,
        id_value: &str,
        idempotency_key: Option<&str>,
    ) -> Result<Option<Constellation>, OriginError> {
        for row in self.origin_rows()? {
            let cx = row.cx;
            if cx.metadata_value("origin_kind") != Some(kind) {
                continue;
            }
            if cx.metadata_value(id_key) == Some(id_value)
                || idempotency_key
                    .is_some_and(|key| cx.metadata_value("idempotency_key") == Some(key))
            {
                return Ok(Some(cx));
            }
        }
        Ok(None)
    }

    pub(super) fn find_by_metadata(
        &self,
        kind: &'static str,
        key: &str,
        value: &str,
    ) -> Result<Option<Constellation>, OriginError> {
        Ok(self.origin_rows()?.into_iter().find_map(|row| {
            let cx = row.cx;
            (cx.metadata_value("origin_kind") == Some(kind)
                && cx.metadata_value(key) == Some(value))
            .then_some(cx)
        }))
    }

    pub(super) fn origin_rows(&self) -> Result<Vec<OriginRow>, OriginError> {
        let rows = self
            .vault
            .scan_cf_at(self.vault.latest_seq(), ColumnFamily::Base)
            .map_err(storage_error)?;
        rows.into_iter()
            .map(|(_, value)| {
                encode::decode_constellation_base(&value)
                    .map(|cx| OriginRow { cx })
                    .map_err(storage_error)
            })
            .collect()
    }
}

fn encode_origin_rows(constellation: &Constellation) -> Result<Vec<CfWrite>, OriginError> {
    let mut rows = vec![(
        ColumnFamily::Base,
        base_key(constellation.cx_id),
        encode::encode_constellation_base(constellation).map_err(storage_error)?,
    )];
    for (slot, vector) in &constellation.slots {
        rows.push((
            ColumnFamily::slot(*slot),
            slot_key(constellation.cx_id),
            encode::encode_slot_vector(vector).map_err(storage_error)?,
        ));
    }
    for anchor in &constellation.anchors {
        rows.push((
            ColumnFamily::Anchors,
            anchor_key(constellation.cx_id, &anchor.kind),
            encode::encode_anchor(anchor).map_err(storage_error)?,
        ));
    }
    Ok(rows)
}
