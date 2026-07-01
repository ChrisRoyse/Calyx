use std::collections::BTreeMap;

use calyx_aster::cf::{ColumnFamily, anchor_key};
use calyx_aster::mvcc::{Freshness, Snapshot};
use calyx_aster::vault::AsterVault;
use calyx_aster::vault::encode::{decode_anchor, decode_constellation_base, encode_anchor};
use calyx_core::{CalyxError, CxId, SlotId};
use serde::{Deserialize, Serialize};

use crate::error::CliResult;

pub(crate) const GROUNDING_SOURCE_OF_TRUTH: &str =
    "Aster Base CF rows and byte-matching anchors CF rows at one pinned snapshot";
pub(crate) const NO_GROUNDED_CANDIDATES_CODE: &str = "CALYX_FSV_NO_GROUNDED_CANDIDATES";
pub(crate) const ANCHOR_CF_DRIFT_CODE: &str = "CALYX_FSV_ANCHOR_CF_DRIFT";
pub(crate) const PROBE_NO_GROUNDED_CANDIDATES_CODE: &str = "CALYX_PROBE_NO_GROUNDED_CANDIDATES";
pub(crate) const PROBE_ANCHOR_CF_DRIFT_CODE: &str = "CALYX_PROBE_ANCHOR_CF_DRIFT";
pub(crate) const GROUNDING_FLAG_DRIFT_CODE: &str = "CALYX_GROUNDING_FLAG_DRIFT";
pub(crate) const GROUNDING_REMEDIATION: &str = "ingest or replay real anchored content, rebuild derived indexes, then rerun the grounding audit before FSV";

const GROUNDING_READER_LEASE_MS: u64 = 300_000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct GroundingAudit {
    pub(crate) source_of_truth: String,
    pub(crate) pinned_seq: u64,
    pub(crate) base_row_count: usize,
    pub(crate) anchors_cf_row_count: usize,
    pub(crate) base_anchor_count: usize,
    pub(crate) matched_anchor_cf_row_count: usize,
    pub(crate) missing_anchor_cf_row_count: usize,
    pub(crate) mismatched_anchor_cf_row_count: usize,
    pub(crate) anchored_base_row_count: usize,
    pub(crate) anchor_cf_drift_row_count: usize,
    pub(crate) ungrounded_flag_row_count: usize,
    pub(crate) degraded_flag_row_count: usize,
    pub(crate) grounding_flag_drift_count: usize,
    pub(crate) accepted_eligible_base_row_count: usize,
    pub(crate) accepted_eligible_active_slot_row_count: usize,
    pub(crate) first_anchor_cf_drift_cx_id: Option<String>,
    pub(crate) first_ineligible_cx_id: Option<String>,
    pub(crate) active_slots: Vec<SlotGroundingAudit>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SlotGroundingAudit {
    pub(crate) slot: SlotId,
    pub(crate) declared_base_row_count: usize,
    pub(crate) anchored_base_row_count: usize,
    pub(crate) anchor_cf_drift_row_count: usize,
    pub(crate) ungrounded_flag_row_count: usize,
    pub(crate) degraded_flag_row_count: usize,
    pub(crate) accepted_eligible_row_count: usize,
    pub(crate) first_ineligible_cx_id: Option<String>,
}

pub(crate) fn audit_grounding(
    vault: &AsterVault,
    active_slots: &[SlotId],
) -> CliResult<GroundingAudit> {
    let read = GroundingRead::pin(vault);
    let mut slots = active_slots.to_vec();
    slots.sort();
    slots.dedup();
    let mut slot_rows = slots
        .iter()
        .copied()
        .map(|slot| {
            (
                slot,
                SlotGroundingAudit {
                    slot,
                    declared_base_row_count: 0,
                    anchored_base_row_count: 0,
                    anchor_cf_drift_row_count: 0,
                    ungrounded_flag_row_count: 0,
                    degraded_flag_row_count: 0,
                    accepted_eligible_row_count: 0,
                    first_ineligible_cx_id: None,
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
    let base_rows = vault.scan_cf_snapshot(read.snapshot(), ColumnFamily::Base)?;
    let anchor_rows = vault.scan_cf_snapshot(read.snapshot(), ColumnFamily::Anchors)?;
    let anchors_cf_row_count = anchor_rows.len();
    let anchors_by_key = anchors_by_key(anchor_rows)?;

    let base_row_count = base_rows.len();
    let mut base_anchor_count = 0usize;
    let mut matched_anchor_cf_row_count = 0usize;
    let mut missing_anchor_cf_row_count = 0usize;
    let mut mismatched_anchor_cf_row_count = 0usize;
    let mut anchored_base_row_count = 0usize;
    let mut anchor_cf_drift_row_count = 0usize;
    let mut ungrounded_flag_row_count = 0usize;
    let mut degraded_flag_row_count = 0usize;
    let mut grounding_flag_drift_count = 0usize;
    let mut accepted_eligible_base_row_count = 0usize;
    let mut accepted_eligible_active_slot_row_count = 0usize;
    let mut first_anchor_cf_drift_cx_id = None;
    let mut first_ineligible_cx_id = None;

    for (key, bytes) in base_rows {
        let key_cx_id = cx_id_from_base_key(&key)?;
        let cx = decode_constellation_base(&bytes)?;
        if cx.cx_id != key_cx_id {
            return Err(CalyxError::aster_corrupt_shard(format!(
                "base CF key {key_cx_id} contains constellation {}",
                cx.cx_id
            ))
            .into());
        }
        let has_anchors = !cx.anchors.is_empty();
        if has_anchors {
            anchored_base_row_count += 1;
        }
        let mut anchor_cf_drift = false;
        for anchor in &cx.anchors {
            base_anchor_count += 1;
            let expected_key = anchor_key(cx.cx_id, &anchor.kind);
            let expected_bytes = encode_anchor(anchor)?;
            match anchors_by_key.get(&expected_key) {
                Some(bytes) if bytes == &expected_bytes => {
                    matched_anchor_cf_row_count += 1;
                }
                Some(_) => {
                    mismatched_anchor_cf_row_count += 1;
                    anchor_cf_drift = true;
                }
                None => {
                    missing_anchor_cf_row_count += 1;
                    anchor_cf_drift = true;
                }
            }
        }
        if anchor_cf_drift {
            anchor_cf_drift_row_count += 1;
            if first_anchor_cf_drift_cx_id.is_none() {
                first_anchor_cf_drift_cx_id = Some(cx.cx_id.to_string());
            }
        }
        if cx.flags.ungrounded {
            ungrounded_flag_row_count += 1;
        }
        if cx.flags.degraded {
            degraded_flag_row_count += 1;
        }
        if cx.flags.ungrounded == has_anchors {
            grounding_flag_drift_count += 1;
        }
        let eligible = has_anchors && !anchor_cf_drift && !cx.flags.degraded;
        if eligible {
            accepted_eligible_base_row_count += 1;
        } else if first_ineligible_cx_id.is_none() {
            first_ineligible_cx_id = Some(cx.cx_id.to_string());
        }
        for slot in &slots {
            if !cx.slots.contains_key(slot) {
                continue;
            }
            let row = slot_rows
                .get_mut(slot)
                .expect("slot audit row initialized for active slot");
            row.declared_base_row_count += 1;
            if has_anchors {
                row.anchored_base_row_count += 1;
            }
            if anchor_cf_drift {
                row.anchor_cf_drift_row_count += 1;
            }
            if cx.flags.ungrounded {
                row.ungrounded_flag_row_count += 1;
            }
            if cx.flags.degraded {
                row.degraded_flag_row_count += 1;
            }
            if eligible {
                row.accepted_eligible_row_count += 1;
                accepted_eligible_active_slot_row_count += 1;
            } else if row.first_ineligible_cx_id.is_none() {
                row.first_ineligible_cx_id = Some(cx.cx_id.to_string());
            }
        }
    }

    if slots.is_empty() {
        accepted_eligible_active_slot_row_count = accepted_eligible_base_row_count;
    }

    Ok(GroundingAudit {
        source_of_truth: GROUNDING_SOURCE_OF_TRUTH.to_string(),
        pinned_seq: read.seq(),
        base_row_count,
        anchors_cf_row_count,
        base_anchor_count,
        matched_anchor_cf_row_count,
        missing_anchor_cf_row_count,
        mismatched_anchor_cf_row_count,
        anchored_base_row_count,
        anchor_cf_drift_row_count,
        ungrounded_flag_row_count,
        degraded_flag_row_count,
        grounding_flag_drift_count,
        accepted_eligible_base_row_count,
        accepted_eligible_active_slot_row_count,
        first_anchor_cf_drift_cx_id,
        first_ineligible_cx_id,
        active_slots: slot_rows.into_values().collect(),
    })
}

pub(crate) fn grounding_failure_for_probe(audit: &GroundingAudit) -> Option<CalyxError> {
    if audit.missing_anchor_cf_row_count > 0 || audit.mismatched_anchor_cf_row_count > 0 {
        return Some(CalyxError {
            code: PROBE_ANCHOR_CF_DRIFT_CODE,
            message: format!(
                "probe-matrix grounding source of truth drifted at pinned_seq={} base_anchors={} matched_anchor_rows={} missing_anchor_rows={} mismatched_anchor_rows={} first_anchor_cf_drift_cx_id={}",
                audit.pinned_seq,
                audit.base_anchor_count,
                audit.matched_anchor_cf_row_count,
                audit.missing_anchor_cf_row_count,
                audit.mismatched_anchor_cf_row_count,
                audit
                    .first_anchor_cf_drift_cx_id
                    .as_deref()
                    .unwrap_or("<none>")
            ),
            remediation: GROUNDING_REMEDIATION,
        });
    }
    if audit.accepted_eligible_active_slot_row_count == 0 {
        return Some(CalyxError {
            code: PROBE_NO_GROUNDED_CANDIDATES_CODE,
            message: format!(
                "probe-matrix active slots have zero persisted anchor-eligible Base rows at pinned_seq={} base_rows={} base_anchors={} matched_anchor_rows={} anchored_base_rows={} ungrounded_rows={} degraded_rows={} anchors_cf_rows={} first_ineligible_cx_id={}",
                audit.pinned_seq,
                audit.base_row_count,
                audit.base_anchor_count,
                audit.matched_anchor_cf_row_count,
                audit.anchored_base_row_count,
                audit.ungrounded_flag_row_count,
                audit.degraded_flag_row_count,
                audit.anchors_cf_row_count,
                audit.first_ineligible_cx_id.as_deref().unwrap_or("<none>")
            ),
            remediation: GROUNDING_REMEDIATION,
        });
    }
    None
}

fn anchors_by_key(anchor_rows: Vec<(Vec<u8>, Vec<u8>)>) -> CliResult<BTreeMap<Vec<u8>, Vec<u8>>> {
    let mut rows = BTreeMap::new();
    for (key, bytes) in anchor_rows {
        decode_anchor(&bytes)?;
        rows.insert(key, bytes);
    }
    Ok(rows)
}

fn cx_id_from_base_key(key: &[u8]) -> calyx_core::Result<CxId> {
    let bytes: [u8; 16] = key.try_into().map_err(|_| {
        CalyxError::vault_access_denied(format!("base CF key has {} bytes", key.len()))
    })?;
    Ok(CxId::from_bytes(bytes))
}

struct GroundingRead<'a> {
    vault: &'a AsterVault,
    snapshot: Snapshot,
}

impl<'a> GroundingRead<'a> {
    fn pin(vault: &'a AsterVault) -> Self {
        Self {
            vault,
            snapshot: vault.pin_reader(Freshness::FreshDerived, GROUNDING_READER_LEASE_MS),
        }
    }

    fn snapshot(&self) -> Snapshot {
        self.snapshot
    }

    fn seq(&self) -> u64 {
        self.snapshot.seq()
    }
}

impl Drop for GroundingRead<'_> {
    fn drop(&mut self) {
        let _ = self.vault.release_reader(self.snapshot.lease().id());
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    use calyx_aster::cf::{ColumnFamily, base_key};
    use calyx_aster::vault::encode::encode_constellation_base;
    use calyx_aster::vault::{AsterVault, VaultOptions};
    use calyx_core::{
        Anchor, AnchorKind, AnchorValue, Constellation, CxFlags, InputRef, LedgerRef, Modality,
        SlotId, SlotVector, VaultId,
    };

    use super::{PROBE_ANCHOR_CF_DRIFT_CODE, audit_grounding, grounding_failure_for_probe};

    #[test]
    fn audit_requires_matching_anchor_cf_rows_for_base_anchors() {
        let root = temp_root("anchor-cf-drift");
        let vault_id: VaultId = "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().expect("vault id");
        let vault = AsterVault::new_durable(
            &root,
            vault_id,
            b"issue1081-grounding-test".to_vec(),
            VaultOptions::default(),
        )
        .expect("new durable vault");
        let input = b"grounded-base-with-missing-anchor-cf";
        let cx_id = vault.cx_id_for_input(input, 1);
        let slot = SlotId::new(14);
        let input_hash = *blake3::hash(input).as_bytes();
        let mut slots = BTreeMap::new();
        slots.insert(
            slot,
            SlotVector::Dense {
                dim: 2,
                data: vec![1.0, 0.0],
            },
        );
        let cx = Constellation {
            cx_id,
            vault_id,
            panel_version: 1,
            created_at: 1,
            input_ref: InputRef {
                hash: input_hash,
                pointer: Some("synthetic://issue1081/missing-anchor-cf".to_string()),
                redacted: false,
            },
            modality: Modality::Text,
            slots,
            scalars: BTreeMap::new(),
            metadata: BTreeMap::new(),
            anchors: vec![Anchor {
                kind: AnchorKind::Label("answer".to_string()),
                value: AnchorValue::Text("grounded".to_string()),
                source: "issue1081-regression".to_string(),
                observed_at: 1,
                confidence: 1.0,
            }],
            provenance: LedgerRef {
                seq: 1,
                hash: [7; 32],
            },
            flags: CxFlags {
                ungrounded: false,
                degraded: false,
                novel_region: false,
                redacted_input: false,
            },
        };
        vault
            .write_cf_batch(vec![(
                ColumnFamily::Base,
                base_key(cx_id),
                encode_constellation_base(&cx).expect("encode base"),
            )])
            .expect("write base without anchors cf");
        vault.flush().expect("flush vault");

        let audit = audit_grounding(&vault, &[slot]).expect("audit grounding source of truth");
        assert_eq!(audit.base_row_count, 1);
        assert_eq!(audit.anchors_cf_row_count, 0);
        assert_eq!(audit.base_anchor_count, 1);
        assert_eq!(audit.missing_anchor_cf_row_count, 1);
        assert_eq!(audit.anchor_cf_drift_row_count, 1);
        assert_eq!(audit.accepted_eligible_active_slot_row_count, 0);
        assert_eq!(audit.active_slots[0].anchor_cf_drift_row_count, 1);
        let error = grounding_failure_for_probe(&audit).expect("drift fails closed");
        assert_eq!(error.code, PROBE_ANCHOR_CF_DRIFT_CODE);

        fs::remove_dir_all(root).expect("cleanup temp vault");
    }

    fn temp_root(label: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "calyx-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock after epoch")
                .as_nanos()
        ))
    }
}
