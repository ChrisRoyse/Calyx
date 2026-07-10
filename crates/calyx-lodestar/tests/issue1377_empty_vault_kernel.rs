use std::collections::BTreeMap;

use calyx_aster::vault::AsterVault;
use calyx_core::{
    Anchor, AnchorKind, AnchorValue, Constellation, CxFlags, CxId, InputRef, LedgerRef, Modality,
    SlotId, SlotVector, VaultId, VaultStore,
};
use calyx_lodestar::{
    KernelParams, LodestarError, RecallTestParams, measured_kernel_from_vault,
    measured_kernel_with_contributions_from_vault_allow_partial,
};

const CONTENT_SLOT: SlotId = SlotId(1);

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}

fn seeded_vault(with_anchor: bool) -> AsterVault {
    let vault = AsterVault::new(vault_id(), b"issue1377-empty-kernel");
    for seed in 1..=3 {
        let anchored = with_anchor && seed == 1;
        let mut slots = BTreeMap::new();
        slots.insert(
            CONTENT_SLOT,
            SlotVector::Dense {
                dim: 2,
                data: vec![f32::from(seed), 1.0],
            },
        );
        let anchors = anchored
            .then(|| Anchor {
                kind: AnchorKind::Label("issue1377".to_string()),
                value: AnchorValue::Text("grounded".to_string()),
                source: "issue1377-regression".to_string(),
                observed_at: u64::from(seed),
                confidence: 1.0,
            })
            .into_iter()
            .collect();
        vault
            .put(Constellation {
                cx_id: CxId::from_bytes([seed; 16]),
                vault_id: vault.vault_id(),
                panel_version: 1,
                created_at: u64::from(seed),
                input_ref: InputRef {
                    hash: [seed; 32],
                    pointer: None,
                    redacted: true,
                },
                modality: Modality::Text,
                slots,
                scalars: BTreeMap::new(),
                metadata: BTreeMap::new(),
                anchors,
                provenance: LedgerRef {
                    seq: u64::from(seed),
                    hash: [seed; 32],
                },
                flags: CxFlags {
                    ungrounded: !anchored,
                    redacted_input: true,
                    ..CxFlags::default()
                },
            })
            .unwrap();
    }
    vault
}

fn recall_params() -> RecallTestParams {
    RecallTestParams {
        held_out_fraction: 1.0,
        top_k: 2,
        rng_seed: 1_377,
        min_recall_ratio: 0.0,
    }
}

#[test]
fn strict_vault_kernel_rejects_empty_selection_instead_of_using_the_corpus() {
    let vault = seeded_vault(true);
    let result = measured_kernel_from_vault(
        &vault,
        CONTENT_SLOT,
        &KernelParams::default(),
        &recall_params(),
        0,
        0.5,
    );

    let error = match result {
        Ok(measured) => panic!(
            "empty selection became a {}-member measured kernel",
            measured.kernel.members.len()
        ),
        Err(error) => error,
    };
    assert_eq!(error, LodestarError::KernelEmptyResult);
}

#[test]
fn partial_vault_kernel_rejects_empty_selection_instead_of_using_the_corpus() {
    let vault = seeded_vault(false);
    let result = measured_kernel_with_contributions_from_vault_allow_partial(
        &vault,
        CONTENT_SLOT,
        &KernelParams::default(),
        &recall_params(),
        0,
        0.5,
    );

    let error = match result {
        Ok((measured, _)) => panic!(
            "empty selection became a {}-member measured kernel",
            measured.kernel.members.len()
        ),
        Err(error) => error,
    };
    assert_eq!(error, LodestarError::KernelEmptyResult);
}
