use calyx_aster::vault::AsterVault;
use calyx_core::{Anchor, AnchorKind, CalyxError, Constellation, CxId, VaultStore};

use crate::error::CliResult;

pub(super) fn verify_base_readback(
    vault: &AsterVault,
    snapshot: u64,
    expected: &Constellation,
    cx_id: CxId,
    required_anchor_kinds: &[AnchorKind],
) -> CliResult {
    let stored = vault.get(cx_id, snapshot)?;
    let mut mismatches = Vec::new();
    if stored.cx_id != expected.cx_id {
        mismatches.push("cx_id");
    }
    if stored.panel_version != expected.panel_version {
        mismatches.push("panel_version");
    }
    if stored.input_ref != expected.input_ref {
        mismatches.push("input_ref");
    }
    if stored.modality != expected.modality {
        mismatches.push("modality");
    }
    if stored.slots != expected.slots {
        mismatches.push("slots");
    }
    if stored.scalars != expected.scalars {
        mismatches.push("scalars");
    }
    if stored.metadata != expected.metadata {
        mismatches.push("metadata");
    }
    if stored.flags != expected.flags {
        mismatches.push("flags");
    }
    if !mismatches.is_empty() {
        return Err(CalyxError::aster_corrupt_shard(format!(
            "durable ingest readback mismatch for cx {cx_id}; mismatched fields: {}",
            mismatches.join(",")
        ))
        .into());
    }
    for anchor in expected
        .anchors
        .iter()
        .filter(|anchor| required_anchor_kinds.contains(&anchor.kind))
    {
        if !contains_anchor(&stored.anchors, anchor) {
            return Err(CalyxError::aster_corrupt_shard(format!(
                "durable ingest readback for cx {cx_id} is missing anchor {:?}",
                anchor.kind
            ))
            .into());
        }
    }
    Ok(())
}

fn contains_anchor(haystack: &[Anchor], needle: &Anchor) -> bool {
    haystack.iter().any(|anchor| anchor == needle)
}
