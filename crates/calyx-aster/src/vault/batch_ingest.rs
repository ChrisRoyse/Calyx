use std::collections::BTreeMap;

use super::{AsterVault, anchor_merge, encode, ledger_hook};
use crate::cf::{ColumnFamily, base_key};
use crate::media_artifact::{
    DerivedMediaArtifactDraft, DerivedMediaArtifactRecord, derived_media_artifact_write_rows,
    ensure_no_artifact_collision,
};
use calyx_core::{CalyxError, Clock, Constellation, CxId, Result, VaultStore};
use calyx_ledger::{ActorId, EntryKind, PayloadBuilder, RedactionPolicy, SubjectId};
use serde_json::json;

const BATCH_ACTOR: &str = "calyx-aster-batch-ingest";

#[derive(Clone, Debug, PartialEq)]
pub struct MediaArtifactIngestCommit {
    pub ids: Vec<CxId>,
    pub artifact: DerivedMediaArtifactRecord,
}

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

    pub fn put_batch_with_ingest_ledger<I>(
        &self,
        constellations: I,
        subject: SubjectId,
        payload: Vec<u8>,
        actor: ActorId,
    ) -> Result<Vec<CxId>>
    where
        I: IntoIterator<Item = Constellation>,
    {
        RedactionPolicy::check_payload(&payload)?;
        let input = constellations.into_iter().collect::<Vec<_>>();
        if input.is_empty() {
            return Ok(Vec::new());
        }
        self.with_durable_commit_lock(|| {
            self.put_batch_locked_with_ledger(
                input,
                Some(BatchLedgerEntry {
                    subject,
                    payload,
                    actor,
                }),
            )
        })
    }

    pub fn put_batch_with_ingest_ledger_and_media_artifact<I>(
        &self,
        constellations: I,
        subject: SubjectId,
        payload: Vec<u8>,
        actor: ActorId,
        artifact: DerivedMediaArtifactDraft,
    ) -> Result<MediaArtifactIngestCommit>
    where
        I: IntoIterator<Item = Constellation>,
    {
        RedactionPolicy::check_payload(&payload)?;
        let input = constellations.into_iter().collect::<Vec<_>>();
        self.with_durable_commit_lock(|| {
            let commit = self.put_batch_locked_with_options(
                input,
                Some(BatchLedgerEntry {
                    subject,
                    payload,
                    actor,
                }),
                Some(artifact),
            )?;
            let artifact = commit.artifact.ok_or_else(|| {
                CalyxError::aster_corrupt_shard(
                    "media artifact ingest committed without returning artifact record",
                )
            })?;
            Ok(MediaArtifactIngestCommit {
                ids: commit.ids,
                artifact,
            })
        })
    }

    fn put_batch_locked(&self, input: Vec<Constellation>) -> Result<Vec<CxId>> {
        self.put_batch_locked_with_ledger(input, None)
    }

    fn put_batch_locked_with_ledger(
        &self,
        input: Vec<Constellation>,
        ledger_entry: Option<BatchLedgerEntry>,
    ) -> Result<Vec<CxId>> {
        self.put_batch_locked_with_options(input, ledger_entry, None)
            .map(|commit| commit.ids)
    }

    fn put_batch_locked_with_options(
        &self,
        input: Vec<Constellation>,
        ledger_entry: Option<BatchLedgerEntry>,
        artifact: Option<DerivedMediaArtifactDraft>,
    ) -> Result<BatchIngestCommit> {
        let latest = self.snapshot();
        let snapshot = self.snapshot_handle(latest);
        let mut accepted_indexes = BTreeMap::<Vec<u8>, usize>::new();
        let mut existing_merges = BTreeMap::<Vec<u8>, Constellation>::new();
        let mut anchor_merge_rows = Vec::new();
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
            if let Some(existing) =
                self.rows
                    .read_at(snapshot.snapshot(), ColumnFamily::Base, &key, &self.clock)?
            {
                if existing == base {
                    ids.push(id);
                    continue;
                }
                let merged = if let Some(merged) = existing_merges.get_mut(&key) {
                    merged
                } else {
                    existing_merges
                        .insert(key.clone(), self.get_at_snapshot(id, snapshot.snapshot())?);
                    existing_merges
                        .get_mut(&key)
                        .expect("inserted existing merge")
                };
                let added = anchor_merge::merge_duplicate_anchors(merged, &constellation)?;
                if !added.is_empty() {
                    anchor_merge_rows
                        .extend(anchor_merge::stage_anchor_merge_rows(id, merged, &added)?);
                }
                ids.push(id);
                continue;
            }
            if let Some(index) = accepted_indexes.get(&key).copied() {
                anchor_merge::merge_duplicate_anchors(&mut accepted[index], &constellation)?;
                ids.push(id);
                continue;
            }
            accepted_indexes.insert(key, accepted.len());
            ids.push(id);
            accepted.push(constellation);
        }
        if accepted.is_empty() && artifact.is_none() {
            if !anchor_merge_rows.is_empty() {
                self.commit_rows_locked(&anchor_merge_rows)?;
            }
            return Ok(BatchIngestCommit {
                ids,
                artifact: None,
            });
        }
        let mut rows = anchor_merge_rows;
        let mut hook_guard = match &self.ledger_hook {
            Some(hook) => Some(ledger_hook::lock_hook(hook)?),
            None => None,
        };
        let (staged_ledger, ledger_ref) = if let Some(hook) = hook_guard.as_deref() {
            let staged = match ledger_entry {
                Some(entry) => ledger_hook::stage_entry_payload(
                    hook,
                    &mut rows,
                    EntryKind::Ingest,
                    entry.subject,
                    entry.payload,
                    entry.actor,
                )?,
                None => ledger_hook::stage_ingest_payload(
                    hook,
                    &mut rows,
                    accepted
                        .first()
                        .ok_or_else(|| {
                            CalyxError::ledger_group_commit_failed(
                                "batch ingest without accepted rows requires explicit ledger entry",
                            )
                        })?
                        .cx_id,
                    batch_payload(&accepted),
                )?,
            };
            let ledger_ref = staged
                .first()
                .ok_or_else(|| CalyxError::ledger_group_commit_failed("no staged ledger rows"))?
                .ledger_ref();
            (Some(staged), ledger_ref)
        } else {
            let ledger_ref = match ledger_entry {
                Some(entry) => self.stage_raw_ledger_entry_locked(
                    &mut rows,
                    EntryKind::Ingest,
                    entry.subject,
                    entry.payload,
                    entry.actor,
                )?,
                None => self.stage_raw_ingest_ledger_locked(
                    &mut rows,
                    accepted
                        .first()
                        .ok_or_else(|| {
                            CalyxError::ledger_group_commit_failed(
                                "batch ingest without accepted rows requires explicit ledger entry",
                            )
                        })?
                        .cx_id,
                    batch_payload(&accepted),
                )?,
            };
            (None, ledger_ref)
        };
        let artifact_record = if let Some(artifact) = artifact {
            let record = artifact.into_record(ledger_ref.clone())?;
            ensure_no_artifact_collision(self, latest, &record)?;
            rows.extend(derived_media_artifact_write_rows(&record)?);
            Some(record)
        } else {
            None
        };
        for mut constellation in accepted {
            constellation.provenance = ledger_ref.clone();
            self.stage_constellation_rows(&mut rows, &constellation)?;
        }
        self.commit_rows_locked(&rows)?;
        if let (Some(hook), Some(staged)) = (hook_guard.as_deref_mut(), staged_ledger.as_ref()) {
            ledger_hook::commit_staged(hook, staged)?;
        }
        Ok(BatchIngestCommit {
            ids,
            artifact: artifact_record,
        })
    }
}

struct BatchIngestCommit {
    ids: Vec<CxId>,
    artifact: Option<DerivedMediaArtifactRecord>,
}

struct BatchLedgerEntry {
    subject: SubjectId,
    payload: Vec<u8>,
    actor: ActorId,
}

fn batch_payload(constellations: &[Constellation]) -> Vec<u8> {
    let mut payload = PayloadBuilder::default();
    let cx_ids = constellations
        .iter()
        .map(|cx| cx.cx_id.to_string())
        .collect::<Vec<_>>();
    let hashes = constellations
        .iter()
        .map(|cx| hex(&cx.input_ref.hash))
        .collect::<Vec<_>>();
    payload
        .insert_str("mode", BATCH_ACTOR)
        .insert_u64("count", constellations.len() as u64)
        .insert_value("cx_id", json!(cx_ids))
        .insert_str("first_cx_id", constellations[0].cx_id.to_string())
        .insert_str(
            "last_cx_id",
            constellations
                .last()
                .expect("non-empty batch")
                .cx_id
                .to_string(),
        )
        .insert_value("input_hash", json!(hashes));
    RedactionPolicy::default().apply_to_payload(&payload)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
