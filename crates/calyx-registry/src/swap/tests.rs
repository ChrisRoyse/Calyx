use super::*;
use crate::lens::Registry;
use crate::runtime::algorithmic::AlgorithmicLens;

#[test]
fn add_lens_bumps_panel_allocates_slot_and_queues_priority_backfill() {
    let mut controller = SwapController::new(sample_panel());
    let (registry, spec) = registered_spec("new-semantic", 3);
    let high = CxId::from_bytes([1; 16]);
    let low = CxId::from_bytes([2; 16]);

    let outcome = controller
        .add_lens(
            &registry,
            spec,
            [
                BackfillCandidate {
                    cx_id: low,
                    priority: 10,
                },
                BackfillCandidate {
                    cx_id: high,
                    priority: 99,
                },
            ],
            42,
        )
        .unwrap();

    assert_eq!(outcome.slot.slot_id, SlotId::new(1));
    assert_eq!(controller.panel().version, 2);
    assert_eq!(outcome.index.queued, 2);
    let claimed = controller.queue_mut().claim_batch(1);
    assert_eq!(claimed[0].cx_id, high);
    controller.queue_mut().complete(claimed[0].id).unwrap();
    assert_eq!(controller.queue().pending_len(), 1);
    assert_eq!(controller.queue().completed_len(), 1);
}

#[test]
fn unregistered_lens_fails_without_mutating_panel_or_queue() {
    let mut controller = SwapController::new(sample_panel());
    let registry = Registry::new();
    let before_version = controller.panel().version;
    let before_slots = controller.panel().slots.len();
    let before_pending = controller.queue().pending_len();

    let error = controller
        .add_lens(
            &registry,
            SlotSpec::dense_text("unregistered", LensId::from_bytes([9; 16]), 3),
            [BackfillCandidate {
                cx_id: CxId::from_bytes([7; 16]),
                priority: 1,
            }],
            42,
        )
        .unwrap_err();

    assert_eq!(error.code, "CALYX_LENS_FROZEN_VIOLATION");
    assert!(error.message.contains("not registered"));
    assert_eq!(controller.panel().version, before_version);
    assert_eq!(controller.panel().slots.len(), before_slots);
    assert_eq!(controller.queue().pending_len(), before_pending);
}

#[test]
fn park_unpark_and_retire_preserve_slot_tombstone() {
    let mut controller = SwapController::new(sample_panel());

    let parked = controller.park_lens(SlotId::new(0), 43).unwrap();
    let active = controller.unpark_lens(SlotId::new(0), 44).unwrap();
    let retired = controller.retire_lens(SlotId::new(0), 45).unwrap();

    assert_eq!(parked.state, SlotState::Parked);
    assert_eq!(active.state, SlotState::Active);
    assert_eq!(retired.state, SlotState::Retired);
    assert_eq!(controller.panel().version, 4);
    assert_eq!(controller.panel().slots[0].state, SlotState::Retired);
}

#[test]
fn duplicate_live_lens_fails_closed() {
    let mut controller = SwapController::new(sample_panel());
    let registry = Registry::new();

    let error = controller
        .add_lens(
            &registry,
            SlotSpec::dense_text("dupe", LensId::from_bytes([1; 16]), 3),
            [],
            42,
        )
        .unwrap_err();

    assert_eq!(error.code, "CALYX_LENS_FROZEN_VIOLATION");
}

fn registered_spec(key: &str, buckets: u32) -> (Registry, SlotSpec) {
    let lens = AlgorithmicLens::one_hot(format!("{key}-lens"), Modality::Text, buckets);
    let spec = SlotSpec::dense_text(key, lens.contract().lens_id(), buckets);
    let mut registry = Registry::new();
    registry
        .register_frozen(lens.clone(), lens.contract().clone())
        .unwrap();
    (registry, spec)
}

fn sample_panel() -> Panel {
    Panel {
        version: 1,
        slots: vec![Slot {
            slot_id: SlotId::new(0),
            slot_key: SlotKey::new(SlotId::new(0), "base-semantic"),
            lens_id: LensId::from_bytes([1; 16]),
            shape: SlotShape::Dense(2),
            modality: Modality::Text,
            asymmetry: Asymmetry::None,
            quant: QuantPolicy::None,
            axis: None,
            retrieval_only: false,
            excluded_from_dedup: false,
            bits_about: BTreeMap::new(),
            state: SlotState::Active,
            added_at_panel_version: 1,
        }],
        created_at: 1,
        kernel_ref: None,
        guard_ref: None,
    }
}
