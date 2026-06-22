use std::collections::BTreeMap;

use calyx_assay::{default_outcome_anchor, outcome_occurrence_context};
use calyx_aster::dedup::EpochSecs;
use calyx_aster::recurrence::{RetentionPolicy, append_occurrence};
use calyx_aster::vault::AsterVault;
use calyx_core::{
    AnchorValue, Constellation, CxFlags, CxId, InputRef, LedgerRef, Modality, VaultId,
};

pub(crate) fn append_outcomes(vault: &AsterVault, cx_id: CxId, outcomes: &[&str]) {
    for (index, outcome) in outcomes.iter().enumerate() {
        let context = outcome_occurrence_context(
            default_outcome_anchor(),
            AnchorValue::Text((*outcome).to_string()),
        )
        .expect("context");
        append_occurrence(
            vault,
            cx_id,
            EpochSecs(1_000 + index as i64),
            context,
            EpochSecs(1_000 + index as i64),
            RetentionPolicy::default(),
        )
        .expect("append occurrence");
    }
}

pub(crate) fn base_cx(cx_id: CxId) -> Constellation {
    Constellation {
        cx_id,
        vault_id: vault_id(),
        panel_version: 42,
        created_at: 1_786_406_600,
        input_ref: InputRef {
            hash: [cx_id.to_bytes()[0]; 32],
            pointer: None,
            redacted: false,
        },
        modality: Modality::Text,
        slots: BTreeMap::new(),
        scalars: BTreeMap::new(),
        metadata: BTreeMap::new(),
        anchors: Vec::new(),
        provenance: LedgerRef {
            seq: 0,
            hash: [0; 32],
        },
        flags: CxFlags::default(),
    }
}

pub(crate) fn cx_id(seed: u8) -> CxId {
    CxId::from_bytes([seed; 16])
}

pub(crate) fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV"
        .parse()
        .expect("valid vault id")
}
