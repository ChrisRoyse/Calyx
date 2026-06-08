use std::path::PathBuf;

use calyx_core::{CxId, LensId, SlotId, content_address};
use calyx_registry::{BackfillConfig, BackfillPriority, BackfillRequest, BackfillScheduler};
use serde_json::json;

#[test]
#[ignore = "manual aiwonder FSV for PH20 atomic backfill scheduler persistence"]
fn ph20_backfill_atomic_persist_aiwonder_fsv() {
    let root = fsv_root();
    std::fs::create_dir_all(&root).expect("create fsv root");
    let good_path = root.join("atomic-backfill-watermark.json");
    let corrupt_path = root.join("corrupt-backfill-watermark.json");
    let _ = std::fs::remove_file(&good_path);
    let _ = std::fs::remove_file(&corrupt_path);

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

    let readback = json!({
        "good_path": good_path.display().to_string(),
        "good_sha256": digest_hex(&good_bytes),
        "good_len": good_bytes.len(),
        "good_watermarks": reopened.watermarks(),
        "corrupt_path": corrupt_path.display().to_string(),
        "corrupt_sha256": digest_hex(&corrupt_bytes),
        "corrupt_len": corrupt_bytes.len(),
        "corrupt_error": corrupt_error.code,
        "temp_files_after": temp_files(&root),
    });
    let path = root.join("backfill-atomic-readback.json");
    std::fs::write(&path, serde_json::to_vec_pretty(&readback).unwrap()).unwrap();

    println!("PH20_BACKFILL_ATOMIC_FSV_ROOT={}", root.display());
    println!("PH20_BACKFILL_ATOMIC_READBACK={}", path.display());
    println!("{}", serde_json::to_string_pretty(&readback).unwrap());

    assert_eq!(readback["corrupt_error"], "CALYX_STALE_DERIVED");
    assert_eq!(readback["good_watermarks"][0]["pending"], 2);
    assert_eq!(readback["temp_files_after"], json!([]));
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
