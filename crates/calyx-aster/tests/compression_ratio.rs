use std::collections::BTreeMap;

use calyx_aster::dedup::{
    CALYX_DEDUP_MISSING_FREQUENCY, Domain, compression_ratio, domain_compression_stats,
};
use calyx_aster::recurrence::FREQUENCY_SCALAR;
use calyx_aster::vault::AsterVault;
use calyx_core::{
    CxFlags, CxId, InputRef, LedgerRef, Modality, SlotId, SlotVector, VaultId, VaultStore,
};

#[test]
fn compression_ratio_frequency_edges_match_contract() {
    let vault = vault();
    for (seed, frequency, original_count, expected_ratio) in
        [(1, 1.0, 1, 1.0), (50, 50.0, 50, 50.0), (0, 0.0, 0, 1.0)]
    {
        vault.put(row(seed, Some(frequency))).expect("put base");

        let ratio = compression_ratio(cx(seed), &vault).expect("ratio");

        assert_eq!(ratio.original_count, original_count);
        assert_eq!(ratio.stored_count, 1);
        assert_eq!(ratio.ratio, expected_ratio);
    }
}

#[test]
fn domain_compression_stats_aggregate_original_and_stored_counts() {
    let vault = vault();
    for (seed, frequency) in [(1, 1.0), (10, 10.0), (50, 50.0)] {
        vault.put(row(seed, Some(frequency))).expect("put base");
    }

    let stats = domain_compression_stats(&Domain::new(vec![cx(1), cx(10), cx(50)]), &vault)
        .expect("domain stats");

    assert_eq!(stats.total_original, 61);
    assert_eq!(stats.total_stored, 3);
    assert!((stats.mean_ratio - (61.0 / 3.0)).abs() < 1.0e-6);
    assert_eq!(stats.max_ratio, 50.0);
}

#[test]
fn missing_frequency_fails_closed() {
    let vault = vault();
    vault.put(row(7, None)).expect("put base");

    let error = compression_ratio(cx(7), &vault).expect_err("missing frequency");

    assert_eq!(error.code, CALYX_DEDUP_MISSING_FREQUENCY);
}

fn row(seed: u8, frequency: Option<f64>) -> calyx_core::Constellation {
    let mut scalars = BTreeMap::new();
    if let Some(frequency) = frequency {
        scalars.insert(FREQUENCY_SCALAR.to_string(), frequency);
    }
    calyx_core::Constellation {
        cx_id: cx(seed),
        vault_id: vault_id(),
        panel_version: 1,
        created_at: 1_000_000,
        input_ref: InputRef {
            hash: [seed; 32],
            pointer: None,
            redacted: true,
        },
        modality: Modality::Text,
        slots: BTreeMap::<SlotId, SlotVector>::new(),
        scalars,
        metadata: BTreeMap::new(),
        anchors: Vec::new(),
        provenance: LedgerRef {
            seq: seed as u64,
            hash: [seed; 32],
        },
        flags: CxFlags::default(),
    }
}

fn vault() -> AsterVault {
    AsterVault::new(vault_id(), b"compression-ratio")
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().expect("vault id")
}

fn cx(seed: u8) -> CxId {
    CxId::from_bytes([seed; 16])
}
