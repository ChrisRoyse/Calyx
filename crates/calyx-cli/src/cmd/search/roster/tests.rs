use std::collections::BTreeMap;
use std::str::FromStr;

use calyx_aster::manifest::ImmutableRef;
use calyx_core::{
    Asymmetry, Input, LensId, Modality, Panel, Placement, QuantPolicy, Slot, SlotId, SlotKey,
    SlotResource, SlotShape, SlotState,
};
use calyx_registry::spec::default_recall_delta;
use calyx_registry::{
    AlgorithmicLens, LensRuntime, LensSpec, Registry, VaultPanelState, VaultRegistrySnapshot,
};

use super::*;
use crate::panel_commands::{ResidentMeasuredInput, ResidentSlotMeasure};

fn registered_state(slots: Vec<Slot>) -> (VaultPanelState, LensId) {
    let mut registry = Registry::new();
    let lens = AlgorithmicLens::byte_features("issue1490-byte", Modality::Text);
    let contract = lens.contract().clone();
    let lens_id = contract.lens_id();
    let spec = LensSpec {
        name: "issue1490-byte".to_string(),
        runtime: LensRuntime::Algorithmic {
            kind: "byte-features".to_string(),
        },
        output: contract.shape(),
        modality: contract.modality(),
        weights_sha256: contract.weights_sha256(),
        corpus_hash: contract.corpus_hash(),
        norm_policy: contract.norm_policy(),
        max_batch: None,
        axis: Some("issue1490-byte".to_string()),
        asymmetry: Asymmetry::None,
        quant_default: QuantPolicy::turboquant_default(),
        truncate_dim: None,
        recall_delta: default_recall_delta(),
        retrieval_only: false,
        excluded_from_dedup: false,
    };
    registry
        .register_frozen_with_spec(lens, contract, spec)
        .expect("register lens");
    let lenses = registry.lens_snapshots();
    let state = VaultPanelState {
        panel: Panel {
            version: slots.len() as u32,
            slots,
            created_at: 0,
            kernel_ref: None,
            guard_ref: None,
        },
        registry,
        registry_snapshot: Some(VaultRegistrySnapshot {
            version: 1,
            panel_ref: ImmutableRef::from_bytes("panel.json", b"panel").expect("panel ref"),
            lenses,
        }),
    };
    (state, lens_id)
}

fn slot(
    id: u16,
    lens_id: LensId,
    placement: Placement,
    state: SlotState,
    modality: Modality,
) -> Slot {
    let slot_id = SlotId::new(id);
    Slot {
        slot_id,
        slot_key: SlotKey::new(slot_id, format!("slot_{id:02}")),
        lens_id,
        shape: SlotShape::Dense(4),
        modality,
        asymmetry: Asymmetry::None,
        quant: QuantPolicy::None,
        resource: SlotResource {
            placement,
            ..SlotResource::default()
        },
        axis: None,
        retrieval_only: false,
        excluded_from_dedup: false,
        bits_about: BTreeMap::new(),
        state,
        added_at_panel_version: 1,
    }
}

fn unregistered_lens_id() -> LensId {
    LensId::from_str("22222222222222222222222222222222").unwrap()
}

fn addr() -> std::net::SocketAddr {
    "127.0.0.1:18401".parse().unwrap()
}

#[test]
fn roster_splits_active_slots_by_placement_and_reports_exclusions() {
    let (mut state, lens_id) = registered_state(Vec::new());
    state.panel.slots = vec![
        slot(
            0,
            lens_id,
            Placement::Gpu,
            SlotState::Active,
            Modality::Text,
        ),
        slot(
            1,
            lens_id,
            Placement::Cpu,
            SlotState::Active,
            Modality::Text,
        ),
        slot(
            2,
            lens_id,
            Placement::Cpu,
            SlotState::Parked,
            Modality::Text,
        ),
        slot(
            3,
            lens_id,
            Placement::Gpu,
            SlotState::Retired,
            Modality::Text,
        ),
        slot(
            4,
            unregistered_lens_id(),
            Placement::Gpu,
            SlotState::Active,
            Modality::Text,
        ),
        slot(
            5,
            lens_id,
            Placement::Gpu,
            SlotState::Active,
            Modality::Code,
        ),
    ];

    let roster = SearchTextRoster::derive(&state);

    assert_eq!(ids(&roster.resident_gpu), vec![0]);
    assert_eq!(ids(&roster.local_cpu), vec![1]);
    assert_eq!(ids(&roster.parked_excluded), vec![2]);
    assert_eq!(ids(&roster.retired_excluded), vec![3]);
    assert_eq!(ids(&roster.unregistered_excluded), vec![4]);
}

#[test]
fn roster_out_names_parked_exclusions_for_explain() {
    let (mut state, lens_id) = registered_state(Vec::new());
    state.panel.slots = vec![
        slot(
            0,
            lens_id,
            Placement::Gpu,
            SlotState::Active,
            Modality::Text,
        ),
        slot(
            17,
            lens_id,
            Placement::Cpu,
            SlotState::Parked,
            Modality::Text,
        ),
    ];

    let out = SearchTextRoster::derive(&state).to_out();

    assert_eq!(out.resident_gpu.len(), 1);
    assert_eq!(out.parked_excluded.len(), 1);
    assert_eq!(out.parked_excluded[0].slot, 17);
    assert_eq!(out.parked_excluded[0].key, "slot_17");
    assert_eq!(out.parked_excluded[0].placement, "Cpu");
    let json = serde_json::to_value(&out).expect("serialize roster");
    assert_eq!(json["parked_excluded"][0]["slot"], 17);
}

#[test]
fn parked_slot_is_never_demanded_from_resident_row() {
    let (mut state, lens_id) = registered_state(Vec::new());
    state.panel.slots = vec![
        slot(
            0,
            lens_id,
            Placement::Gpu,
            SlotState::Active,
            Modality::Text,
        ),
        slot(
            17,
            lens_id,
            Placement::Cpu,
            SlotState::Parked,
            Modality::Text,
        ),
    ];
    let roster = SearchTextRoster::derive(&state);
    let vector = state
        .registry
        .measure(lens_id, &Input::new(Modality::Text, b"query".to_vec()))
        .expect("measure");
    // The resident row carries ONLY the GPU slot — exactly what the resident
    // serves. Before #1490 the parked/CPU slots deadlocked this path.
    let row = row(vec![measured_slot(0, lens_id, Placement::Gpu, vector)]);

    let vectors =
        query_vectors_from_resident_row(&state, &roster.resident_gpu, &row, addr()).unwrap();

    assert_eq!(vectors.len(), 1);
    assert_eq!(vectors[0].0, SlotId::new(0));
}

#[test]
fn missing_demanded_gpu_slot_in_resident_row_is_refused() {
    let (mut state, lens_id) = registered_state(Vec::new());
    state.panel.slots = vec![slot(
        0,
        lens_id,
        Placement::Gpu,
        SlotState::Active,
        Modality::Text,
    )];
    let roster = SearchTextRoster::derive(&state);
    let row = row(Vec::new());

    let error =
        query_vectors_from_resident_row(&state, &roster.resident_gpu, &row, addr()).unwrap_err();

    assert!(
        error
            .message()
            .contains("did not return active GPU text slot 0"),
        "unexpected message: {}",
        error.message()
    );
}

#[test]
fn resident_row_placement_mismatch_is_refused() {
    let (mut state, lens_id) = registered_state(Vec::new());
    state.panel.slots = vec![slot(
        0,
        lens_id,
        Placement::Gpu,
        SlotState::Active,
        Modality::Text,
    )];
    let roster = SearchTextRoster::derive(&state);
    let vector = state
        .registry
        .measure(lens_id, &Input::new(Modality::Text, b"query".to_vec()))
        .expect("measure");
    let row = row(vec![measured_slot(0, lens_id, Placement::Cpu, vector)]);

    let error =
        query_vectors_from_resident_row(&state, &roster.resident_gpu, &row, addr()).unwrap_err();

    assert!(
        error.message().contains("placement"),
        "unexpected message: {}",
        error.message()
    );
}

#[test]
fn local_cpu_slots_measure_in_process_like_ingest() {
    let (mut state, lens_id) = registered_state(Vec::new());
    state.panel.slots = vec![
        slot(
            0,
            lens_id,
            Placement::Gpu,
            SlotState::Active,
            Modality::Text,
        ),
        slot(
            1,
            lens_id,
            Placement::Cpu,
            SlotState::Active,
            Modality::Text,
        ),
    ];
    let roster = SearchTextRoster::derive(&state);

    let vectors = measure_local_cpu_query_vectors(&state, &roster.local_cpu, "query").unwrap();

    assert_eq!(vectors.len(), 1);
    assert_eq!(vectors[0].0, SlotId::new(1));
    assert!(vectors[0].1.is_indexable());
}

#[test]
fn local_cpu_measurement_requires_persisted_snapshot_contract() {
    let (mut state, lens_id) = registered_state(Vec::new());
    state.panel.slots = vec![slot(
        1,
        lens_id,
        Placement::Cpu,
        SlotState::Active,
        Modality::Text,
    )];
    state.registry_snapshot = None;
    let roster = SearchTextRoster::derive(&state);

    let error = measure_local_cpu_query_vectors(&state, &roster.local_cpu, "query").unwrap_err();

    assert!(
        error
            .message()
            .contains("persisted registry snapshot contract"),
        "unexpected message: {}",
        error.message()
    );
}

#[test]
fn resident_template_binding_mismatch_is_refused() {
    let error = require_resident_template_binding(
        "vault:/zfs/hot/calyx/OTHER-VAULT",
        "vault:/zfs/hot/calyx/THIS-VAULT",
        addr(),
    )
    .unwrap_err();

    assert_eq!(error.code(), "CALYX_SEARCH_RESIDENT_MISMATCH");
    assert!(error.message().contains("vault:/zfs/hot/calyx/OTHER-VAULT"));
    assert!(error.message().contains("vault:/zfs/hot/calyx/THIS-VAULT"));

    require_resident_template_binding(
        "vault:/zfs/hot/calyx/THIS-VAULT",
        "vault:/zfs/hot/calyx/THIS-VAULT",
        addr(),
    )
    .expect("matching binding accepted");
}

fn ids(slots: &[&Slot]) -> Vec<u16> {
    slots.iter().map(|slot| slot.slot_id.get()).collect()
}

fn row(slots: Vec<ResidentSlotMeasure>) -> ResidentMeasuredInput {
    ResidentMeasuredInput {
        input_index: 0,
        input_len: 5,
        measured_slot_count: slots.iter().filter(|slot| slot.measured).count(),
        absent_slot_count: 0,
        slots,
    }
}

fn measured_slot(
    id: u16,
    lens_id: LensId,
    placement: Placement,
    vector: SlotVector,
) -> ResidentSlotMeasure {
    ResidentSlotMeasure {
        slot: id,
        key: format!("slot_{id:02}"),
        lens_id: lens_id.to_string(),
        modality: Modality::Text,
        placement,
        measured: true,
        vector: Some(vector),
        absent_reason: None,
    }
}
