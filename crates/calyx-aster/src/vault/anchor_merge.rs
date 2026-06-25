use std::collections::BTreeSet;

use super::encode::{self, WriteRow};
use crate::cf::{ColumnFamily, anchor_key, base_key};
use crate::dedup::{AnchorConflictResult, check_anchor_conflict};
use calyx_core::{Anchor, CalyxError, Constellation, CxId, LedgerRef, Result};

pub(super) fn merge_duplicate_anchors(
    existing: &mut Constellation,
    incoming: &Constellation,
) -> Result<Vec<Anchor>> {
    if !same_anchor_merge_identity(existing, incoming)? {
        return Err(CalyxError::aster_corrupt_shard(
            "CxId collision or non-idempotent duplicate constellation",
        ));
    }
    if let AnchorConflictResult::Conflicting {
        anchor_type,
        reason,
    } = check_anchor_conflict(incoming, existing)
    {
        return Err(CalyxError::aster_corrupt_shard(format!(
            "CxId duplicate has conflicting {anchor_type:?} anchor: {reason:?}"
        )));
    }

    let mut existing_kinds = existing
        .anchors
        .iter()
        .map(|anchor| anchor.kind.clone())
        .collect::<BTreeSet<_>>();
    let mut added = Vec::new();
    for anchor in &incoming.anchors {
        if existing_kinds.insert(anchor.kind.clone()) {
            existing.anchors.push(anchor.clone());
            added.push(anchor.clone());
        }
    }
    if !added.is_empty() {
        existing.flags.ungrounded = existing.anchors.is_empty();
        existing.validate_schema()?;
    }
    Ok(added)
}

pub(super) fn stage_anchor_merge_rows(
    id: CxId,
    merged: &Constellation,
    added: &[Anchor],
) -> Result<Vec<WriteRow>> {
    let mut rows = Vec::with_capacity(1 + added.len());
    rows.push(WriteRow {
        cf: ColumnFamily::Base,
        key: base_key(id),
        value: encode::encode_constellation_base(merged)?,
    });
    for anchor in added {
        rows.push(WriteRow {
            cf: ColumnFamily::Anchors,
            key: anchor_key(id, &anchor.kind),
            value: encode::encode_anchor(anchor)?,
        });
    }
    Ok(rows)
}

fn same_anchor_merge_identity(left: &Constellation, right: &Constellation) -> Result<bool> {
    Ok(normalized_anchor_identity(left)? == normalized_anchor_identity(right)?)
}

fn normalized_anchor_identity(cx: &Constellation) -> Result<Vec<u8>> {
    let mut normalized = cx.clone();
    normalized.anchors.clear();
    normalized.created_at = 0;
    normalized.flags.ungrounded = false;
    normalized.provenance = LedgerRef {
        seq: 0,
        hash: [0; 32],
    };
    encode::encode_constellation_base(&normalized)
}
