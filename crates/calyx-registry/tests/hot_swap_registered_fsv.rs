use std::collections::BTreeMap;
use std::path::PathBuf;

use calyx_core::{
    Asymmetry, CxId, LensId, Modality, Panel, QuantPolicy, Slot, SlotId, SlotKey, SlotShape,
    SlotState, content_address,
};
use calyx_registry::{
    BackfillCandidate, BackfillConfig, BackfillPriority, BackfillScheduler, Registry, SlotSpec,
    SwapController,
};
use serde_json::json;

#[test]
#[ignore = "manual aiwonder FSV for PH20 registered hot-swap fail-closed guard"]
fn ph20_unregistered_hot_swap_fails_closed_aiwonder_fsv() {
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
