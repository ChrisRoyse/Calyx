use std::collections::BTreeMap;
use std::path::PathBuf;

use calyx_core::{
    Asymmetry, CxId, LensId, Modality, Panel, QuantPolicy, Slot, SlotId, SlotKey, SlotShape,
    SlotState, content_address,
};
use calyx_registry::{
    AlgorithmicLens, BackfillCandidate, BackfillConfig, BackfillPriority, BackfillScheduler,
    Registry, SlotSpec, SwapController,
};
use serde_json::json;

#[test]
#[ignore = "manual FSV for PH20 registered hot-swap fail-closed guard"]
fn ph20_unregistered_hot_swap_fails_closed_manual_fsv() {
    let root = fsv_root();
    std::fs::create_dir_all(&root).expect("create fsv root");
    let scheduler_path = root.join("registered-hot-swap-watermark.json");
    if scheduler_path.exists() {
        std::fs::remove_file(&scheduler_path).expect("remove stale scheduler state");
    }
    let mut scheduler = BackfillScheduler::open(
        &scheduler_path,
        BackfillConfig {
            max_concurrent: 1,
            batch_size: 1,
            throttle_ms: 10,
        },
    )
    .expect("open scheduler");
    scheduler.persist().expect("persist empty scheduler state");
    let scheduler_before = std::fs::read(&scheduler_path).expect("read scheduler before");

    let mut controller = SwapController::new(panel());
    let before_version = controller.panel().version;
    let before_slots = controller.panel().slots.len();
    let before_pending = controller.queue().pending_len();
    let registry = Registry::new();
    let error = controller
        .add_lens_durable(
            &registry,
            SlotSpec::dense_text("unregistered-semantic", LensId::from_bytes([9; 16]), 2),
            [BackfillCandidate {
                cx_id: CxId::from_bytes([7; 16]),
                priority: 99,
            }],
            30,
            &mut scheduler,
            BackfillPriority::Kernel,
        )
        .expect_err("unregistered lens must fail before mutation");
    let scheduler_after = std::fs::read(&scheduler_path).expect("read scheduler after");

    let readback = json!({
        "error": error.code,
        "message": error.message,
        "panel_version_before": before_version,
        "panel_version_after": controller.panel().version,
        "slot_count_before": before_slots,
        "slot_count_after": controller.panel().slots.len(),
        "queue_pending_before": before_pending,
        "queue_pending_after": controller.queue().pending_len(),
        "scheduler_before_sha256": digest_hex(&scheduler_before),
        "scheduler_after_sha256": digest_hex(&scheduler_after),
        "scheduler_unchanged": scheduler_before == scheduler_after,
        "watermarks_after": scheduler.watermarks(),
    });
    let path = root.join("hot-swap-registered-readback.json");
    std::fs::write(&path, serde_json::to_vec_pretty(&readback).unwrap()).unwrap();

    println!("PH20_REGISTERED_FSV_ROOT={}", root.display());
    println!("PH20_REGISTERED_READBACK={}", path.display());
    println!("{}", serde_json::to_string_pretty(&readback).unwrap());

    assert_eq!(readback["error"], "CALYX_LENS_FROZEN_VIOLATION");
    assert_eq!(readback["panel_version_after"], before_version);
    assert_eq!(readback["slot_count_after"], before_slots);
    assert_eq!(readback["queue_pending_after"], before_pending);
    assert_eq!(readback["scheduler_unchanged"], true);
}

#[test]
#[ignore = "manual FSV for PH20 lifecycle idempotency and backfill cancellation"]
fn ph20_lifecycle_idempotency_manual_fsv() {
    let root = fsv_root();
    std::fs::create_dir_all(&root).expect("create fsv root");

    let mut controller = SwapController::new(panel());
    let mut registry = Registry::new();
    let lens = AlgorithmicLens::one_hot("ph20-lifecycle-fsv", Modality::Text, 3);
    let contract = lens.contract().clone();
    let lens_id = registry
        .register_frozen(lens, contract)
        .expect("registered lens");
    let spec = SlotSpec::dense_text("ph20-lifecycle", lens_id, 3);

    let initial = json!({
        "panel_version": controller.panel().version,
        "slot_count": controller.panel().slots.len(),
        "queue_pending": controller.queue().pending_len(),
    });
    let add = controller
        .add_lens(
            &registry,
            spec.clone(),
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
            40,
        )
        .expect("add lens");
    let after_add = json!({
        "slot_id": add.slot.slot_id,
        "panel_version": add.panel_version,
        "slot_count": controller.panel().slots.len(),
        "queue_pending": controller.queue().pending_len(),
    });
    let duplicate = controller
        .add_lens(
            &registry,
            spec,
            [BackfillCandidate {
                cx_id: CxId::from_bytes([5; 16]),
                priority: 99,
            }],
            41,
        )
        .expect("duplicate exact add is idempotent");
    let after_duplicate = json!({
        "same_slot": duplicate.slot == add.slot,
        "panel_version": duplicate.panel_version,
        "slot_count": controller.panel().slots.len(),
        "queue_pending": controller.queue().pending_len(),
        "duplicate_queued": duplicate.queued,
    });

    let parked = controller
        .park_lens(add.slot.slot_id, 42)
        .expect("park slot");
    let pending_after_park = controller.queue().pending_len();
    let parked_again = controller
        .park_lens(add.slot.slot_id, 43)
        .expect("repeat park is no-op");
    let unparked = controller
        .unpark_lens(add.slot.slot_id, 44)
        .expect("unpark slot");
    let unparked_again = controller
        .unpark_lens(add.slot.slot_id, 45)
        .expect("repeat unpark is no-op");
    controller.queue_mut().enqueue(
        add.slot.slot_id,
        add.slot.lens_id,
        BackfillCandidate {
            cx_id: CxId::from_bytes([6; 16]),
            priority: 7,
        },
    );
    let pending_before_retire = controller.queue().pending_len();
    let retired = controller
        .retire_lens(add.slot.slot_id, 46)
        .expect("retire slot");
    let retired_again = controller
        .retire_lens(add.slot.slot_id, 47)
        .expect("repeat retire is no-op");
    let unpark_retired = controller
        .unpark_lens(add.slot.slot_id, 48)
        .expect_err("retired slot cannot become active");

    let durable_path = root.join("ph20-durable-duplicate-scheduler.json");
    if durable_path.exists() {
        std::fs::remove_file(&durable_path).expect("remove stale durable scheduler state");
    }
    let mut durable_controller = SwapController::new(panel());
    let mut durable_registry = Registry::new();
    let durable_lens = AlgorithmicLens::one_hot("ph20-durable-duplicate-fsv", Modality::Text, 3);
    let durable_contract = durable_lens.contract().clone();
    let durable_lens_id = durable_registry
        .register_frozen(durable_lens, durable_contract)
        .expect("registered durable lens");
    let durable_spec = SlotSpec::dense_text("ph20-durable-duplicate", durable_lens_id, 3);
    let mut durable_scheduler = BackfillScheduler::open(
        &durable_path,
        BackfillConfig {
            max_concurrent: 1,
            batch_size: 1,
            throttle_ms: 0,
        },
    )
    .expect("open durable scheduler");
    let durable_first = durable_controller
        .add_lens_durable(
            &durable_registry,
            durable_spec.clone(),
            [BackfillCandidate {
                cx_id: CxId::from_bytes([7; 16]),
                priority: 1,
            }],
            50,
            &mut durable_scheduler,
            BackfillPriority::Kernel,
        )
        .expect("durable first add");
    let durable_scheduler_after_first =
        std::fs::read(&durable_path).expect("read durable scheduler after first");
    let durable_duplicate = durable_controller
        .add_lens_durable(
            &durable_registry,
            durable_spec,
            [BackfillCandidate {
                cx_id: CxId::from_bytes([8; 16]),
                priority: 99,
            }],
            51,
            &mut durable_scheduler,
            BackfillPriority::Kernel,
        )
        .expect("durable duplicate add");
    let durable_scheduler_after_duplicate =
        std::fs::read(&durable_path).expect("read durable scheduler after duplicate");

    let readback = json!({
        "initial": initial,
        "after_add": after_add,
        "after_duplicate": after_duplicate,
        "park": {
            "state": format!("{:?}", parked.state),
            "panel_version": parked.panel_version,
            "repeat_version": parked_again.panel_version,
            "queue_pending": pending_after_park,
        },
        "unpark": {
            "state": format!("{:?}", unparked.state),
            "panel_version": unparked.panel_version,
            "repeat_version": unparked_again.panel_version,
        },
        "retire": {
            "pending_before_retire": pending_before_retire,
            "state": format!("{:?}", retired.state),
            "panel_version": retired.panel_version,
            "repeat_version": retired_again.panel_version,
            "queue_pending": controller.queue().pending_len(),
            "unpark_retired_error": unpark_retired.code,
        },
        "durable_duplicate": {
            "same_slot": durable_first.slot == durable_duplicate.slot,
            "duplicate_queued": durable_duplicate.queued,
            "queue_pending": durable_controller.queue().pending_len(),
            "watermark_count": durable_scheduler.watermarks().len(),
            "watermark_pending": durable_scheduler.watermarks()[0].pending,
            "scheduler_first_sha256": digest_hex(&durable_scheduler_after_first),
            "scheduler_duplicate_sha256": digest_hex(&durable_scheduler_after_duplicate),
            "scheduler_unchanged": durable_scheduler_after_first == durable_scheduler_after_duplicate,
        },
        "final_panel_version": controller.panel().version,
        "final_slot_state": format!("{:?}", controller.panel().slots[1].state),
    });
    let path = root.join("ph20-lifecycle-readback.json");
    std::fs::write(&path, serde_json::to_vec_pretty(&readback).unwrap()).unwrap();

    println!("PH20_LIFECYCLE_FSV_ROOT={}", root.display());
    println!("PH20_LIFECYCLE_READBACK={}", path.display());
    println!("{}", serde_json::to_string_pretty(&readback).unwrap());

    assert_eq!(readback["after_duplicate"]["same_slot"], true);
    assert_eq!(readback["after_duplicate"]["duplicate_queued"], 0);
    assert_eq!(
        readback["after_duplicate"]["panel_version"],
        readback["after_add"]["panel_version"]
    );
    assert_eq!(
        readback["park"]["panel_version"],
        readback["park"]["repeat_version"]
    );
    assert_eq!(readback["park"]["queue_pending"], 0);
    assert_eq!(
        readback["unpark"]["panel_version"],
        readback["unpark"]["repeat_version"]
    );
    assert_eq!(
        readback["retire"]["panel_version"],
        readback["retire"]["repeat_version"]
    );
    assert_eq!(readback["retire"]["queue_pending"], 0);
    assert_eq!(
        readback["retire"]["unpark_retired_error"],
        "CALYX_LENS_FROZEN_VIOLATION"
    );
    assert_eq!(readback["durable_duplicate"]["same_slot"], true);
    assert_eq!(readback["durable_duplicate"]["duplicate_queued"], 0);
    assert_eq!(readback["durable_duplicate"]["scheduler_unchanged"], true);
}

fn fsv_root() -> PathBuf {
    std::env::var("CALYX_FSV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("calyx-ph20-registered-hot-swap-fsv"))
}

fn panel() -> Panel {
    Panel {
        version: 1,
        slots: vec![Slot {
            slot_id: SlotId::new(0),
            slot_key: SlotKey::new(SlotId::new(0), "semantic-v1"),
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

fn digest_hex(bytes: &[u8]) -> String {
    content_address([bytes])
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
