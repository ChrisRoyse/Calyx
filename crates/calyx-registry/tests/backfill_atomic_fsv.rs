use std::path::PathBuf;

use calyx_core::{
    Asymmetry, CxId, LensId, Modality, Panel, QuantPolicy, Slot, SlotId, SlotKey, SlotShape,
    SlotState, content_address,
};
use calyx_registry::{
    AlgorithmicLens, BackfillCandidate, BackfillConfig, BackfillPriority, BackfillRequest,
    BackfillScheduler, Registry, SlotSpec, SwapController,
};
use serde_json::json;

#[test]
#[ignore = "manual FSV for PH20 atomic backfill scheduler persistence"]
fn ph20_backfill_atomic_persist_manual_fsv() {
    let root = fsv_root();
    std::fs::create_dir_all(&root).expect("create fsv root");
    let good_path = root.join("atomic-backfill-watermark.json");
    let corrupt_path = root.join("corrupt-backfill-watermark.json");
    let rollback_path = root.join("rollback-backfill-watermark.json");
    let _ = std::fs::remove_file(&good_path);
    let _ = std::fs::remove_file(&corrupt_path);
    let _ = std::fs::remove_file(&rollback_path);
    let _ = std::fs::remove_file(post_rename_failure_marker(&rollback_path));

    let mut scheduler =
        BackfillScheduler::open(&good_path, BackfillConfig::default()).expect("open scheduler");
    scheduler
        .enqueue(BackfillRequest {
            slot_id: SlotId::new(3),
            lens_id: LensId::from_bytes([3; 16]),
            priority: BackfillPriority::Kernel,
            candidates: vec![CxId::from_bytes([1; 16]), CxId::from_bytes([2; 16])],
        })
        .expect("enqueue request");
    let good_bytes = std::fs::read(&good_path).expect("read good scheduler");
    let reopened = BackfillScheduler::open(&good_path, BackfillConfig::default())
        .expect("reopen good scheduler");

    std::fs::write(&corrupt_path, b"{").expect("write corrupt scheduler");
    let corrupt_bytes = std::fs::read(&corrupt_path).expect("read corrupt scheduler");
    let corrupt_error = BackfillScheduler::open(&corrupt_path, BackfillConfig::default())
        .expect_err("corrupt scheduler must fail closed");

    let rollback = durable_rollback_readback(&rollback_path);

    let readback = json!({
        "good_path": good_path.display().to_string(),
        "good_sha256": digest_hex(&good_bytes),
        "good_len": good_bytes.len(),
        "good_watermarks": reopened.watermarks(),
        "corrupt_path": corrupt_path.display().to_string(),
        "corrupt_sha256": digest_hex(&corrupt_bytes),
        "corrupt_len": corrupt_bytes.len(),
        "corrupt_error": corrupt_error.code,
        "durable_rollback": rollback,
        "temp_files_after": temp_files(&root),
    });
    let path = root.join("backfill-atomic-readback.json");
    std::fs::write(&path, serde_json::to_vec_pretty(&readback).unwrap()).unwrap();

    println!("PH20_BACKFILL_ATOMIC_FSV_ROOT={}", root.display());
    println!("PH20_BACKFILL_ATOMIC_READBACK={}", path.display());
    println!("{}", serde_json::to_string_pretty(&readback).unwrap());

    assert_eq!(readback["corrupt_error"], "CALYX_STALE_DERIVED");
    assert_eq!(readback["good_watermarks"][0]["pending"], 2);
    assert_eq!(readback["durable_rollback"]["error"], "CALYX_STALE_DERIVED");
    assert_eq!(readback["durable_rollback"]["scheduler_unchanged"], true);
    assert_eq!(readback["durable_rollback"]["panel_unchanged"], true);
    assert_eq!(readback["durable_rollback"]["queue_unchanged"], true);
    assert_eq!(readback["temp_files_after"], json!([]));
}

fn durable_rollback_readback(path: &std::path::Path) -> serde_json::Value {
    let mut scheduler =
        BackfillScheduler::open(path, BackfillConfig::default()).expect("rollback scheduler");
    scheduler.persist().expect("persist rollback baseline");
    let before_bytes = std::fs::read(path).expect("read rollback baseline");
    let before_watermarks = scheduler.watermarks();
    let mut controller = SwapController::new(panel());
    let before_version = controller.panel().version;
    let before_slots = controller.panel().slots.len();
    let before_pending = controller.queue().pending_len();
    let mut registry = Registry::new();
    let lens = AlgorithmicLens::one_hot("rollback-lens", Modality::Text, 2);
    let lens_id = registry
        .register_frozen(lens.clone(), lens.contract().clone())
        .expect("register rollback lens");
    std::fs::write(post_rename_failure_marker(path), b"fail-after-rename")
        .expect("write failure marker");

    let error = controller
        .add_lens_durable(
            &registry,
            SlotSpec::dense_text("rollback-semantic", lens_id, 2),
            [BackfillCandidate {
                cx_id: CxId::from_bytes([9; 16]),
                priority: 7,
            }],
            99,
            &mut scheduler,
            BackfillPriority::Kernel,
        )
        .expect_err("injected post-rename failure must fail closed");
    let after_bytes = std::fs::read(path).expect("read rollback after");
    let reopened = BackfillScheduler::open(path, BackfillConfig::default()).expect("reopen");

    json!({
        "path": path.display().to_string(),
        "error": error.code,
        "message": error.message,
        "scheduler_before_sha256": digest_hex(&before_bytes),
        "scheduler_after_sha256": digest_hex(&after_bytes),
        "scheduler_unchanged": before_bytes == after_bytes,
        "watermarks_before": before_watermarks,
        "watermarks_after_memory": scheduler.watermarks(),
        "watermarks_after_reopen": reopened.watermarks(),
        "panel_version_before": before_version,
        "panel_version_after": controller.panel().version,
        "slot_count_before": before_slots,
        "slot_count_after": controller.panel().slots.len(),
        "queue_pending_before": before_pending,
        "queue_pending_after": controller.queue().pending_len(),
        "panel_unchanged": before_version == controller.panel().version
            && before_slots == controller.panel().slots.len(),
        "queue_unchanged": before_pending == controller.queue().pending_len(),
        "marker_exists_after": post_rename_failure_marker(path).exists(),
    })
}

fn fsv_root() -> PathBuf {
    std::env::var("CALYX_FSV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("calyx-ph20-backfill-atomic-fsv"))
}

fn digest_hex(bytes: &[u8]) -> String {
    content_address([bytes])
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn temp_files(root: &std::path::Path) -> Vec<String> {
    let mut files = std::fs::read_dir(root)
        .expect("read fsv root")
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let name = entry.file_name().into_string().ok()?;
            name.contains(".tmp-").then_some(name)
        })
        .collect::<Vec<_>>();
    files.sort();
    files
}

fn post_rename_failure_marker(path: &std::path::Path) -> PathBuf {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .expect("scheduler filename");
    path.with_file_name(format!(".{name}.fail-after-rename-once"))
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
            bits_about: std::collections::BTreeMap::new(),
            state: SlotState::Active,
            added_at_panel_version: 1,
        }],
        created_at: 1,
        kernel_ref: None,
        guard_ref: None,
    }
}
