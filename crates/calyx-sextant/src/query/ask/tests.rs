use std::collections::BTreeMap;

use calyx_aster::vault::AsterVault;
use calyx_core::{
    AbsentReason, CxFlags, CxId, InputRef, LedgerRef, Modality, SlotId, SlotVector, VaultId,
    VaultStore,
};

use crate::error::{CALYX_ANSWER_UNGROUNDED, CALYX_INVALID_ARGUMENT, CALYX_LENS_NOT_FOUND};

use super::{AskSpec, ask};

fn vault() -> AsterVault {
    AsterVault::new(vault_id(), b"ask-test-salt".to_vec())
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}

fn spec(question: &str, context_cx_ids: Vec<CxId>, top_k: usize, oracle: bool) -> AskSpec {
    AskSpec {
        question: question.to_string(),
        context_cx_ids,
        top_k,
        oracle,
    }
}

#[test]
fn stub_path_returns_kernel_stub_and_grounding() {
    let vault = vault();
    let cx_id = put_dense(&vault, b"stub-grounding", 11, [0.1, 0.9]);

    let result = ask(
        &vault,
        &spec("what should be grounded?", vec![cx_id], 10, false),
        vault.latest_seq(),
    )
    .unwrap();

    assert_eq!(result.answer, "[kernel stub]");
    assert_eq!(result.grounding.len(), 1);
    assert_eq!(result.grounding[0].key.as_bytes(), cx_id.as_bytes());
    assert_eq!(result.grounding[0].ledger_ref.as_ref().unwrap().seq, 11);
    assert!(result.gaps.is_empty());
    assert_eq!(result.oracle_conf, None);
}

#[test]
fn provenance_tag_uses_constellation_ledger_ref_at_snapshot() {
    let vault = vault();
    let cx_id = put_dense(&vault, b"provenance", 77, [0.4, 0.6]);
    let snapshot = vault.latest_seq();

    let result = ask(
        &vault,
        &spec("show provenance", vec![cx_id], 1, false),
        snapshot,
    )
    .unwrap();

    assert_eq!(result.grounding.len(), 1);
    assert_eq!(
        result.grounding[0].ledger_ref,
        Some(LedgerRef {
            seq: 77,
            hash: [77; 32],
        })
    );
}

#[test]
fn empty_context_searches_full_vault() {
    let vault = vault();
    put_dense(&vault, b"full-a", 21, [0.8, 0.2]);
    put_dense(&vault, b"full-b", 22, [0.2, 0.8]);

    let result = ask(
        &vault,
        &spec("rank the full vault", Vec::new(), 2, false),
        vault.latest_seq(),
    )
    .unwrap();

    assert_eq!(result.grounding.len(), 2);
}

#[test]
fn top_k_one_limits_grounding_and_oracle_false_has_no_confidence() {
    let vault = vault();
    put_dense(&vault, b"top-a", 31, [0.8, 0.2]);
    put_dense(&vault, b"top-b", 32, [0.2, 0.8]);

    let result = ask(
        &vault,
        &spec("one only", Vec::new(), 1, false),
        vault.latest_seq(),
    )
    .unwrap();

    assert_eq!(result.grounding.len(), 1);
    assert_eq!(result.oracle_conf, None);
}

#[test]
fn empty_question_fails_invalid_argument() {
    let vault = vault();
    let error = ask(
        &vault,
        &spec("   ", Vec::new(), 1, false),
        vault.latest_seq(),
    )
    .unwrap_err();

    assert_eq!(error.code, CALYX_INVALID_ARGUMENT);
}

#[test]
fn empty_kernel_grounding_fails_closed() {
    let vault = vault();
    let error = ask(
        &vault,
        &spec("nothing to ground", Vec::new(), 1, false),
        vault.latest_seq(),
    )
    .unwrap_err();

    assert_eq!(error.code, CALYX_ANSWER_UNGROUNDED);
}

#[test]
fn unavailable_lens_fails_closed() {
    let vault = vault();
    let cx_id = put_absent(&vault, b"missing-lens", 41);

    let error = ask(
        &vault,
        &spec("requires a lens", vec![cx_id], 1, false),
        vault.latest_seq(),
    )
    .unwrap_err();

    assert_eq!(error.code, CALYX_LENS_NOT_FOUND);
}

fn put_dense(vault: &AsterVault, input: &[u8], seq: u64, data: [f32; 2]) -> CxId {
    let cx_id = CxId::from_input(input, 1, b"ask-test-salt");
    vault
        .put(constellation(
            cx_id,
            LedgerRef {
                seq,
                hash: [seq as u8; 32],
            },
            SlotVector::Dense {
                dim: 2,
                data: data.to_vec(),
            },
        ))
        .unwrap();
    cx_id
}

fn put_absent(vault: &AsterVault, input: &[u8], seq: u64) -> CxId {
    let cx_id = CxId::from_input(input, 1, b"ask-test-salt");
    vault
        .put(constellation(
            cx_id,
            LedgerRef {
                seq,
                hash: [seq as u8; 32],
            },
            SlotVector::Absent {
                reason: AbsentReason::LensUnavailable,
            },
        ))
        .unwrap();
    cx_id
}

fn constellation(
    cx_id: CxId,
    provenance: LedgerRef,
    vector: SlotVector,
) -> calyx_core::Constellation {
    let mut input_hash = [0_u8; 32];
    input_hash[..16].copy_from_slice(cx_id.as_bytes());
    let mut slots = BTreeMap::new();
    slots.insert(SlotId::new(0), vector);
    calyx_core::Constellation {
        cx_id,
        vault_id: vault_id(),
        panel_version: 1,
        created_at: 1,
        input_ref: InputRef {
            hash: input_hash,
            pointer: Some(format!("synthetic://ask/{cx_id}")),
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
