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
    if stored.cx_id != expected.cx_id
        || stored.panel_version != expected.panel_version
        || stored.input_ref != expected.input_ref
        || stored.modality != expected.modality
        || stored.slots != expected.slots
        || stored.scalars != expected.scalars
        || stored.metadata != expected.metadata
        || stored.flags != expected.flags
    {
        return Err(CalyxError::aster_corrupt_shard(format!(
            "durable ingest readback mismatch for cx {cx_id}"
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
