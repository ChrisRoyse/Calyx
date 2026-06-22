use std::collections::BTreeMap;
use std::fs;

#[path = "sextant_support/mod.rs"]
mod sextant_support;
use calyx_core::{
    Anchor, AnchorKind, AnchorValue, CxFlags, InputRef, LedgerRef, Modality, SlotId, SlotVector,
    VaultId,
};
use calyx_sextant::{
    CALYX_SEXTANT_GPU_PARITY_UNAVAILABLE, HnswIndex, MaxSimIndex, QuantConfig, Query, SearchEngine,
    SlotIndexMap,
};
use serde_json::json;
use sextant_support::{cx_u8_fill as cx, hex};

#[test]
fn gpu_parity_shims_fail_loud_and_search_fanout_is_explicit_cpu() {
    let maxsim_error =
        MaxSimIndex::cpu_gpu_delta(&[vec![1.0, 0.0], vec![0.0, 1.0]], &[vec![1.0, 0.0]])
            .unwrap_err();
    assert_eq!(maxsim_error.code, CALYX_SEXTANT_GPU_PARITY_UNAVAILABLE);

    let quant_error = QuantConfig::scalar8(0.01)
        .cpu_gpu_delta(&[0.1, -0.2, 0.3])
        .unwrap_err();
    assert_eq!(quant_error.code, CALYX_SEXTANT_GPU_PARITY_UNAVAILABLE);

    let (engine, _) = fanout_engine();
    let hits = engine.search(&fanout_query()).unwrap();
    assert_eq!(hits[0].rank, 1);
    assert!(hits.iter().all(|hit| hit.provenance.hash[0] != 0));
    assert!(
        hits.iter()
            .any(|hit| hit.per_lens.iter().any(|lens| lens.slot == SlotId::new(8)))
    );
    assert!(
        hits.iter()
            .any(|hit| hit.per_lens.iter().any(|lens| lens.slot == SlotId::new(9)))
    );
}

#[test]
#[ignore = "manual FSV writes Sextant GPU parity/fan-out source-of-truth artifacts"]
fn gpu_parity_and_fanout_manual_fsv() {
    let root = std::env::var("CALYX_FSV_ROOT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("calyx-gpu-parity-fsv"));
    fs::create_dir_all(&root).unwrap();

    let maxsim_error =
        MaxSimIndex::cpu_gpu_delta(&[vec![1.0, 0.0], vec![0.0, 1.0]], &[vec![1.0, 0.0]])
            .unwrap_err();
    let quant_error = QuantConfig::scalar8(0.01)
        .cpu_gpu_delta(&[0.1, -0.2, 0.3])
        .unwrap_err();
    let (engine, rows) = fanout_engine();
    let hits = engine.search(&fanout_query()).unwrap();

    let readback = json!({
        "maxsim_cpu_gpu_delta": {
            "available": false,
            "code": maxsim_error.code,
            "message": maxsim_error.message,
        },
        "quant_cpu_gpu_delta": {
            "available": false,
            "code": quant_error.code,
            "message": quant_error.message,
        },
        "search_fanout_backend": "per_slot_cpu_index_calls",
        "forge_grouped_fanout_wired": false,
        "result_ids": ids(&hits),
        "result_scores": hits.iter().map(|hit| hit.score).collect::<Vec<_>>(),
        "per_lens_slots": hits
            .iter()
            .map(|hit| hit.per_lens.iter().map(|lens| lens.slot.to_string()).collect::<Vec<_>>())
            .collect::<Vec<_>>(),
        "provenance_hashes": hits
            .iter()
            .map(|hit| hex(&hit.provenance.hash))
            .collect::<Vec<_>>(),
    });

    fs::write(
        root.join("fanout-source-rows.json"),
        serde_json::to_vec_pretty(&rows).unwrap(),
    )
    .unwrap();
    fs::write(
        root.join("gpu-parity-fanout-readback.json"),
        serde_json::to_vec_pretty(&readback).unwrap(),
    )
    .unwrap();

    assert_eq!(maxsim_error.code, CALYX_SEXTANT_GPU_PARITY_UNAVAILABLE);
    assert_eq!(quant_error.code, CALYX_SEXTANT_GPU_PARITY_UNAVAILABLE);
    assert_eq!(readback["forge_grouped_fanout_wired"], false);
    assert!(!hits.is_empty());
}

fn fanout_engine() -> (SearchEngine, Vec<calyx_core::Constellation>) {
    let map = SlotIndexMap::new();
    map.register(HnswIndex::new(SlotId::new(8), 3, 42)).unwrap();
    map.register(HnswIndex::new(SlotId::new(9), 3, 43)).unwrap();
    let mut engine = SearchEngine::new(map);
    let rows = vec![
        row(1, basis_vec(0), basis_vec(2)),
        row(2, basis_vec(1), basis_vec(1)),
    ];
    for (idx, row) in rows.iter().enumerate() {
        let seq = idx as u64 + 1;
        engine
            .indexes
            .insert(
                SlotId::new(8),
                row.cx_id,
                row.slots[&SlotId::new(8)].clone(),
                seq,
            )
            .unwrap();
        engine
            .indexes
            .insert(
                SlotId::new(9),
                row.cx_id,
                row.slots[&SlotId::new(9)].clone(),
                seq,
            )
            .unwrap();
        engine.put_constellation(row.clone());
    }
    (engine, rows)
}

fn row(value: u8, slot8: SlotVector, slot9: SlotVector) -> calyx_core::Constellation {
    let mut slots = BTreeMap::new();
    slots.insert(SlotId::new(8), slot8);
    slots.insert(SlotId::new(9), slot9);
    calyx_core::Constellation {
        cx_id: cx(value),
        vault_id: vault(),
        panel_version: 1,
        created_at: u64::from(value),
        input_ref: InputRef {
            hash: [value; 32],
            pointer: Some(format!("zfs://calyx/gpu-parity/{value}")),
            redacted: false,
        },
        modality: Modality::Text,
        slots,
        scalars: BTreeMap::new(),
        metadata: BTreeMap::new(),
        anchors: vec![Anchor {
            kind: AnchorKind::Label("gpu-parity".to_string()),
            value: AnchorValue::Text("explicit-cpu-fanout".to_string()),
            source: "issue299-fsv".to_string(),
            observed_at: u64::from(value),
            confidence: 1.0,
        }],
        provenance: LedgerRef {
            seq: u64::from(value),
            hash: [value; 32],
        },
        flags: CxFlags::default(),
    }
}

fn fanout_query() -> Query {
    Query::new("fanout")
        .with_vector(basis_vec(0))
        .with_slots(vec![SlotId::new(8), SlotId::new(9)])
        .explain(true)
}

fn ids(hits: &[calyx_sextant::Hit]) -> Vec<String> {
    hits.iter().map(|hit| hit.cx_id.to_string()).collect()
}

fn basis_vec(index: usize) -> SlotVector {
    let mut data = vec![0.0; 3];
    data[index % 3] = 1.0;
    SlotVector::Dense { dim: 3, data }
}

fn vault() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}
