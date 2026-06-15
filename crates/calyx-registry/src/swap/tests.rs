use super::*;
use crate::BackfillConfig;
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
    let parked_again = controller.park_lens(SlotId::new(0), 44).unwrap();
    let active = controller.unpark_lens(SlotId::new(0), 44).unwrap();
    let active_again = controller.unpark_lens(SlotId::new(0), 45).unwrap();
    let retired = controller.retire_lens(SlotId::new(0), 45).unwrap();
    let retired_again = controller.retire_lens(SlotId::new(0), 46).unwrap();

    assert_eq!(parked.state, SlotState::Parked);
    assert_eq!(parked_again.panel_version, parked.panel_version);
    assert_eq!(active.state, SlotState::Active);
    assert_eq!(active_again.panel_version, active.panel_version);
    assert_eq!(retired.state, SlotState::Retired);
    assert_eq!(retired_again.panel_version, retired.panel_version);
    assert_eq!(controller.panel().version, 4);
    assert_eq!(controller.panel().slots[0].state, SlotState::Retired);

    let error = controller.unpark_lens(SlotId::new(0), 47).unwrap_err();
    assert_eq!(error.code, "CALYX_LENS_FROZEN_VIOLATION");
}

#[test]
fn identical_live_lens_add_is_idempotent() {
    let mut controller = SwapController::new(sample_panel());
    let (registry, spec) = registered_spec("new-semantic", 3);
    let first = controller
        .add_lens(
            &registry,
            spec.clone(),
            [BackfillCandidate {
                cx_id: CxId::from_bytes([3; 16]),
                priority: 1,
            }],
            42,
        )
        .unwrap();
    let pending_after_first = controller.queue().pending_len();

    let second = controller
        .add_lens(
            &registry,
            spec,
            [BackfillCandidate {
                cx_id: CxId::from_bytes([4; 16]),
                priority: 99,
            }],
            43,
        )
        .unwrap();

    assert_eq!(first.slot, second.slot);
    assert_eq!(second.panel_version, first.panel_version);
    assert_eq!(second.queued, 0);
    assert!(second.index.ready);
    assert_eq!(controller.queue().pending_len(), pending_after_first);
}

#[test]
fn identical_durable_lens_add_does_not_enqueue_scheduler() {
    let mut controller = SwapController::new(sample_panel());
    let (registry, spec) = registered_spec("durable-semantic", 3);
    let path = std::env::temp_dir().join(format!(
        "calyx-swap-durable-idempotent-{}.json",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    let mut scheduler = BackfillScheduler::open(
        &path,
        BackfillConfig {
            max_concurrent: 1,
            batch_size: 1,
            throttle_ms: 0,
        },
    )
    .unwrap();

    let first = controller
        .add_lens_durable(
            &registry,
            spec.clone(),
            [BackfillCandidate {
                cx_id: CxId::from_bytes([3; 16]),
                priority: 1,
            }],
            42,
            &mut scheduler,
            BackfillPriority::Kernel,
        )
        .unwrap();
    let scheduler_after_first = std::fs::read(&path).unwrap();

    let second = controller
        .add_lens_durable(
            &registry,
            spec,
            [BackfillCandidate {
                cx_id: CxId::from_bytes([4; 16]),
                priority: 99,
            }],
            43,
            &mut scheduler,
            BackfillPriority::Kernel,
        )
        .unwrap();
    let scheduler_after_second = std::fs::read(&path).unwrap();

    assert_eq!(first.slot, second.slot);
    assert_eq!(second.queued, 0);
    assert!(second.index.ready);
    assert_eq!(controller.queue().pending_len(), 1);
    assert_eq!(scheduler.watermarks().len(), 1);
    assert_eq!(scheduler.watermarks()[0].pending, 1);
    assert_eq!(scheduler_after_second, scheduler_after_first);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn duplicate_live_lens_with_different_key_fails_closed() {
    let mut controller = SwapController::new(sample_panel());
    let (registry, spec) = registered_spec("new-semantic", 3);
    controller
        .add_lens(&registry, spec, [], 42)
        .expect("first add");
    let lens_id = controller.panel().slots[1].lens_id;

    let error = controller
        .add_lens(&registry, SlotSpec::dense_text("dupe", lens_id, 3), [], 43)
        .unwrap_err();

    assert_eq!(error.code, "CALYX_LENS_FROZEN_VIOLATION");
}

#[test]
fn parking_or_retiring_cancels_pending_backfill_for_slot() {
    let mut controller = SwapController::new(sample_panel());
    let (registry, spec) = registered_spec("new-semantic", 3);
    let add = controller
        .add_lens(
            &registry,
            spec,
            [
                BackfillCandidate {
                    cx_id: CxId::from_bytes([3; 16]),
                    priority: 1,
                },
                BackfillCandidate {
                    cx_id: CxId::from_bytes([4; 16]),
                    priority: 2,
                },
            ],
            42,
        )
        .unwrap();
    assert_eq!(controller.queue().pending_len(), 2);

    controller.park_lens(add.slot.slot_id, 43).unwrap();
    assert_eq!(controller.queue().pending_len(), 0);

    controller.unpark_lens(add.slot.slot_id, 44).unwrap();
    controller.queue_mut().enqueue(
        add.slot.slot_id,
        add.slot.lens_id,
        BackfillCandidate {
            cx_id: CxId::from_bytes([5; 16]),
            priority: 3,
        },
    );
    assert_eq!(controller.queue().pending_len(), 1);

    controller.retire_lens(add.slot.slot_id, 45).unwrap();
    assert_eq!(controller.queue().pending_len(), 0);
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
            resource: Default::default(),
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
