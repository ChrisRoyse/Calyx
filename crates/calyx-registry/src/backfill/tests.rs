use super::*;

#[test]
fn durable_scheduler_orders_throttles_and_resumes() {
    let path = test_path("durable_scheduler_orders_throttles_and_resumes");
    let _ = fs::remove_file(&path);
    let mut scheduler = BackfillScheduler::open(
        &path,
        BackfillConfig {
            max_concurrent: 1,
            batch_size: 2,
            throttle_ms: 10,
        },
    )
    .unwrap();
    scheduler
        .enqueue(request(1, BackfillPriority::Normal, 3))
        .unwrap();
    scheduler
        .enqueue(request(2, BackfillPriority::Kernel, 2))
        .unwrap();

    let first = scheduler.claim_next_batch(100).unwrap().unwrap();
    assert_eq!(first.slot_id, SlotId::new(2));
    assert_eq!(first.candidates.len(), 2);
    scheduler
        .complete_batch(first.slot_id, first.lens_id, 100)
        .unwrap();
    assert!(scheduler.claim_next_batch(105).unwrap().is_none());

    let reopened = BackfillScheduler::open(
        &path,
        BackfillConfig {
            max_concurrent: 1,
            batch_size: 2,
            throttle_ms: 10,
        },
    )
    .unwrap();
    let marks = reopened.watermarks();
    let kernel = marks
        .iter()
        .find(|mark| mark.slot_id == SlotId::new(2))
        .unwrap();
    assert!(kernel.complete);
    assert_eq!(kernel.processed, 2);
}

#[test]
fn reenqueue_existing_request_merges_candidates_and_reopens_completed_work() {
    let path = test_path("reenqueue_existing_request_merges_candidates");
    let _ = fs::remove_file(&path);
    let mut scheduler = BackfillScheduler::open(
        &path,
        BackfillConfig {
            max_concurrent: 1,
            batch_size: 8,
            throttle_ms: 0,
        },
    )
    .unwrap();
    scheduler
        .enqueue(request(3, BackfillPriority::Normal, 2))
        .unwrap();
    let first = scheduler.claim_next_batch(0).unwrap().unwrap();
    scheduler
        .complete_batch(first.slot_id, first.lens_id, 0)
        .unwrap();
    scheduler
        .enqueue(BackfillRequest {
            slot_id: SlotId::new(3),
            lens_id: LensId::from_bytes([3; 16]),
            priority: BackfillPriority::Kernel,
            candidates: vec![CxId::from_bytes([1; 16]), CxId::from_bytes([9; 16])],
        })
        .unwrap();

    let marks = scheduler.watermarks();
    let mark = marks
        .iter()
        .find(|mark| mark.slot_id == SlotId::new(3))
        .unwrap();
    assert_eq!(mark.priority, BackfillPriority::Kernel);
    assert_eq!(mark.processed, 2);
    assert_eq!(mark.pending, 1);
    assert!(!mark.complete);
    let next = scheduler.claim_next_batch(0).unwrap().unwrap();
    assert_eq!(next.candidates, vec![CxId::from_bytes([9; 16])]);
}

#[test]
fn claimed_uncompleted_batch_is_retried_after_reopen() {
    let path = test_path("claimed_uncompleted_batch_is_retried_after_reopen");
    let _ = fs::remove_file(&path);
    let mut scheduler = BackfillScheduler::open(&path, BackfillConfig::default()).unwrap();
    scheduler
        .enqueue(request(7, BackfillPriority::Hot, 2))
        .unwrap();
    let first = scheduler.claim_next_batch(0).unwrap().unwrap();
    assert_eq!(first.candidates.len(), 2);

    let mut reopened = BackfillScheduler::open(&path, BackfillConfig::default()).unwrap();
    let retry = reopened.claim_next_batch(0).unwrap().unwrap();
    assert_eq!(retry.candidates, first.candidates);
}

#[test]
fn corrupt_scheduler_state_fails_closed() {
    let path = test_path("corrupt_scheduler_state_fails_closed");
    let _ = fs::remove_file(&path);
    fs::write(&path, b"{").unwrap();

    let error = BackfillScheduler::open(&path, BackfillConfig::default()).unwrap_err();

    assert_eq!(error.code, "CALYX_STALE_DERIVED");
}

#[test]
fn post_rename_persist_failure_rolls_back_file_and_state() {
    let path = test_path("post_rename_persist_failure_rolls_back_file_and_state");
    let _ = fs::remove_file(&path);
    let mut scheduler = BackfillScheduler::open(&path, BackfillConfig::default()).unwrap();
    scheduler
        .enqueue(request(1, BackfillPriority::Normal, 1))
        .unwrap();
    let before_bytes = fs::read(&path).unwrap();
    let before_marks = scheduler.watermarks();
    fs::write(post_rename_failure_marker(&path).unwrap(), b"fail-once").unwrap();

    let error = scheduler
        .enqueue(request(2, BackfillPriority::Kernel, 2))
        .unwrap_err();
    let after_bytes = fs::read(&path).unwrap();
    let reopened = BackfillScheduler::open(&path, BackfillConfig::default()).unwrap();

    assert_eq!(error.code, "CALYX_STALE_DERIVED");
    assert_eq!(scheduler.watermarks(), before_marks);
    assert_eq!(reopened.watermarks(), before_marks);
    assert_eq!(after_bytes, before_bytes);
}

fn request(slot: u16, priority: BackfillPriority, count: u8) -> BackfillRequest {
    BackfillRequest {
        slot_id: SlotId::new(slot),
        lens_id: LensId::from_bytes([slot as u8; 16]),
        priority,
        candidates: (0..count).map(|idx| CxId::from_bytes([idx; 16])).collect(),
    }
}

fn test_path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("calyx-{name}-{}.json", std::process::id()))
}
