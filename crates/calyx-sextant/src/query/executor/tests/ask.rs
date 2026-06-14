use std::collections::BTreeMap;

use calyx_core::{CxFlags, CxId, InputRef, LedgerRef, Modality, SlotId, SlotVector, VaultStore};

use crate::query::PlanStep;

use super::{execute, plan, vault, vault_id};

#[test]
fn ask_step_appends_grounding_rows_with_provenance() {
    let vault = vault();
    let cx_id = CxId::from_input(b"ask-executor", 1, b"salt");
    vault
        .put(sample_constellation(
            cx_id,
            LedgerRef {
                seq: 42,
                hash: [7; 32],
            },
        ))
        .unwrap();

    let result = execute(
        &vault,
        plan(vec![PlanStep::Ask {
            question: "which orders?".to_string(),
            context_cx_ids: vec![cx_id],
            top_k: 1,
            oracle: false,
        }]),
    )
    .unwrap();

    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.rows[0].key.as_bytes(), cx_id.as_bytes());
    assert_eq!(result.rows[0].ledger_ref.as_ref().unwrap().seq, 42);
}

fn sample_constellation(cx_id: CxId, provenance: LedgerRef) -> calyx_core::Constellation {
    let mut input_hash = [0_u8; 32];
    input_hash[..4].copy_from_slice(b"ask!");
    let mut slots = BTreeMap::new();
    slots.insert(
        SlotId::new(0),
        SlotVector::Dense {
            dim: 2,
            data: vec![0.25, 0.75],
        },
    );
    calyx_core::Constellation {
        cx_id,
        vault_id: vault_id(),
        panel_version: 1,
        created_at: 1,
        input_ref: InputRef {
            hash: input_hash,
            pointer: Some("synthetic://ask-executor".to_string()),
            redacted: false,
        },
        modality: Modality::Text,
        slots,
        scalars: BTreeMap::new(),
        metadata: BTreeMap::new(),
        anchors: Vec::new(),
        provenance,
        flags: CxFlags::default(),
    }
}
