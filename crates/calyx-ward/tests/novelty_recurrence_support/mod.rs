use std::collections::BTreeMap;

use calyx_aster::dedup::EpochSecs;
use calyx_aster::recurrence::{
    FREQUENCY_SCALAR, OccurrenceContext, RetentionPolicy, append_occurrence,
};
use calyx_aster::vault::AsterVault;
use calyx_core::{
    Constellation, CxFlags, CxId, InputRef, LedgerRef, Modality, VaultId, VaultStore,
};

pub(crate) fn append_times(vault: &AsterVault, cx_id: CxId, times: &[i64]) {
    for time in times {
        append_occurrence(
            vault,
            cx_id,
            EpochSecs(*time),
            OccurrenceContext::new(format!("t={time}").into_bytes()).unwrap(),
            EpochSecs(*time),
            RetentionPolicy::default(),
        )
        .unwrap();
    }
}

pub(crate) fn put_base(vault: &AsterVault, cx_id: CxId, frequency: Option<f64>) {
    let mut cx = base_cx(cx_id);
    if let Some(frequency) = frequency {
        cx.scalars.insert(FREQUENCY_SCALAR.to_string(), frequency);
    }
    vault.put(cx).unwrap();
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

pub(crate) fn cx(seed: u8) -> CxId {
    CxId::from_bytes([seed; 16])
}

pub(crate) fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV"
        .parse()
        .expect("valid vault id")
}
