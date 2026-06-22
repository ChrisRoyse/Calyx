use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{
    AbsentReason, Asymmetry, Clock, Constellation, CxFlags, CxId, InputRef, LedgerRef, LensId,
    Modality, Panel, QuantPolicy, Slot, SlotId, SlotKey, SlotShape, SlotState, SlotVector,
    SystemClock, VaultId, VaultStore, content_address,
};
use calyx_registry::{
    AlgorithmicLens, BackfillCandidate, BackfillConfig, BackfillPriority, BackfillScheduler,
    Registry, SlotSpec, SwapController,
};
use std::collections::BTreeMap;
use std::path::PathBuf;

#[test]
#[ignore = "manual FSV test for PH20 hot-swap lifecycle"]
fn ph20_hot_swap_manual_fsv() {
    let root = fsv_root();
    std::fs::create_dir_all(&root).expect("create fsv root");
    let vault_dir = root.join("vault");
    if vault_dir.exists() {
        std::fs::remove_dir_all(&vault_dir).expect("remove stale generated vault dir");
    }
    std::fs::create_dir_all(&vault_dir).expect("create vault dir");

    let vault = AsterVault::new_durable(
        &vault_dir,
        vault_id(),
        b"ph20-hot-swap-salt".to_vec(),
        VaultOptions::default(),
    )
    .expect("open durable vault");
    let first = constellation(&vault, b"ph20-first", 10, 1, [1.0, 0.0]);
    let second = constellation(&vault, b"ph20-second", 20, 2, [0.0, 1.0]);
    let first_id = first.cx_id;
    let second_id = second.cx_id;

    vault.put(first.clone()).expect("put first");
    vault.put(second.clone()).expect("put second");
    vault.flush().expect("flush initial state");
    let before_seq = vault.snapshot();
    let first_base_before = base_bytes(&vault, first_id, before_seq);
    let second_base_before = base_bytes(&vault, second_id, before_seq);

    println!("PH20_FSV_ROOT={}", root.display());
    println!("PH20_VAULT_DIR={}", vault_dir.display());
    println!("PH20_BEFORE_SEQ={before_seq}");
    println!(
        "PH20_BASE_FIRST_BEFORE_DIGEST={}",
        digest_hex(&first_base_before)
    );
    println!(
        "PH20_BASE_SECOND_BEFORE_DIGEST={}",
        digest_hex(&second_base_before)
    );

    let mut controller = SwapController::new(panel());
    let mut registry = Registry::new();
    let lens = AlgorithmicLens::one_hot("semantic-v2", Modality::Text, 2);
    let lens_id = registry
        .register_frozen(lens.clone(), lens.contract().clone())
        .expect("register frozen hot-swap lens");
    let scheduler_path = root.join("backfill-watermark.json");
    let mut scheduler = BackfillScheduler::open(
        &scheduler_path,
        BackfillConfig {
            max_concurrent: 1,
            batch_size: 1,
            throttle_ms: 10,
        },
    )
    .expect("open durable scheduler");
    let add = controller
        .add_lens_durable(
            &registry,
            SlotSpec::dense_text("semantic-v2", lens_id, 2),
            [
                BackfillCandidate {
                    cx_id: second_id,
                    priority: 99,
                },
                BackfillCandidate {
                    cx_id: first_id,
                    priority: 5,
                },
            ],
            30,
            &mut scheduler,
            BackfillPriority::Kernel,
        )
        .expect("add lens");
    let new_slot = add.slot.slot_id;
    let after_add_seq = vault.snapshot();
    let first_base_after_add = base_bytes(&vault, first_id, after_add_seq);
    let second_base_after_add = base_bytes(&vault, second_id, after_add_seq);

    println!("PH20_ADDED_SLOT={}", new_slot.get());
    println!("PH20_PANEL_VERSION_AFTER_ADD={}", add.panel_version);
    println!("PH20_INDEX_PLACEHOLDER_READY={}", add.index.ready);
    println!("PH20_INDEX_PLACEHOLDER_QUEUED={}", add.index.queued);
    println!(
        "PH20_BASE_UNCHANGED_AFTER_ADD={}",
        first_base_before == first_base_after_add && second_base_before == second_base_after_add
    );
    assert_eq!(add.panel_version, 2);
    assert!(!add.index.ready);
    assert_eq!(add.queued, 2);
    assert_eq!(first_base_before, first_base_after_add);
    assert_eq!(second_base_before, second_base_after_add);
    let scheduler_enqueued = std::fs::read(&scheduler_path).expect("read scheduler enqueue state");
    println!("PH20_SCHEDULER_PATH={}", scheduler_path.display());
    println!(
        "PH20_SCHEDULER_ENQUEUED_DIGEST={}",
        digest_hex(&scheduler_enqueued)
    );

    let duplicate_before_version = controller.panel().version;
    let duplicate_before_pending = controller.queue().pending_len();
    let duplicate_error = controller
        .add_lens(
            &registry,
            SlotSpec::dense_text("semantic-v2-dupe", lens_id, 2),
            [],
            31,
        )
        .expect_err("duplicate live lens rejected");
    println!("PH20_EDGE_DUPLICATE_BEFORE_VERSION={duplicate_before_version}");
    println!(
        "PH20_EDGE_DUPLICATE_AFTER_VERSION={}",
        controller.panel().version
    );
    println!("PH20_EDGE_DUPLICATE_BEFORE_PENDING={duplicate_before_pending}");
    println!(
        "PH20_EDGE_DUPLICATE_AFTER_PENDING={}",
        controller.queue().pending_len()
    );
    println!("PH20_EDGE_DUPLICATE_ERROR={}", duplicate_error.code);
    assert_eq!(duplicate_error.code, "CALYX_LENS_FROZEN_VIOLATION");
    assert_eq!(controller.panel().version, duplicate_before_version);
    assert_eq!(controller.queue().pending_len(), duplicate_before_pending);

    let zero_before_pending = controller.queue().pending_len();
    let zero_claim = controller.queue_mut().claim_batch(0);
    println!("PH20_EDGE_ZERO_CLAIM_BEFORE_PENDING={zero_before_pending}");
    println!("PH20_EDGE_ZERO_CLAIM_COUNT={}", zero_claim.len());
    println!(
        "PH20_EDGE_ZERO_CLAIM_AFTER_PENDING={}",
        controller.queue().pending_len()
    );
    assert!(zero_claim.is_empty());
    assert_eq!(controller.queue().pending_len(), zero_before_pending);

    let missing_before_seq = vault.snapshot();
    let missing_error = vault
        .put_slot_vector(
            CxId::from_bytes([0xee; 16]),
            new_slot,
            &SlotVector::Absent {
                reason: AbsentReason::Deferred,
            },
        )
        .expect_err("missing constellation rejected");
    println!("PH20_EDGE_MISSING_BACKFILL_BEFORE_SEQ={missing_before_seq}");
    println!("PH20_EDGE_MISSING_BACKFILL_AFTER_SEQ={}", vault.snapshot());
    println!("PH20_EDGE_MISSING_BACKFILL_ERROR={}", missing_error.code);
    assert_eq!(missing_error.code, "CALYX_STALE_DERIVED");
    assert_eq!(vault.snapshot(), missing_before_seq);

    let before_park_state = slot_state(controller.panel(), new_slot);
    let parked = controller.park_lens(new_slot, 32).expect("park lens");
    let unparked = controller.unpark_lens(new_slot, 33).expect("unpark lens");
    println!("PH20_PARK_BEFORE_STATE={before_park_state:?}");
    println!("PH20_PARK_AFTER_STATE={:?}", parked.state);
    println!("PH20_UNPARK_AFTER_STATE={:?}", unparked.state);
    println!(
        "PH20_PANEL_VERSION_AFTER_PARK_UNPARK={}",
        controller.panel().version
    );
    assert_eq!(parked.state, SlotState::Parked);
    assert_eq!(unparked.state, SlotState::Active);

    let placeholder = SlotVector::Absent {
        reason: AbsentReason::Deferred,
    };
    let placeholder_seq = vault
        .put_slot_vector(first_id, new_slot, &placeholder)
        .expect("write first placeholder");
    let placeholder_read = vault
        .read_slot_vector_at(placeholder_seq, first_id, new_slot)
        .expect("read first placeholder")
        .expect("first placeholder row");
    println!("PH20_PLACEHOLDER_SEQ={placeholder_seq}");
    println!(
        "PH20_PLACEHOLDER_READ={}",
        slot_vector_summary(&placeholder_read)
    );
    assert_eq!(placeholder_read, placeholder);

    let first_batch = scheduler
        .claim_next_batch(1000)
        .expect("claim first durable batch")
        .expect("first durable batch");
    assert_eq!(first_batch.candidates, vec![second_id]);
    let second_dense = SlotVector::Dense {
        dim: 2,
        data: vec![0.25, 0.75],
    };
    let second_slot_seq = vault
        .put_slot_vector(second_id, new_slot, &second_dense)
        .expect("write second dense");
    scheduler
        .complete_batch(first_batch.slot_id, first_batch.lens_id, 1000)
        .expect("complete first durable batch");
    let scheduler_after_first =
        std::fs::read(&scheduler_path).expect("read scheduler after first complete");
    println!(
        "PH20_BACKFILL_FIRST_TASK_CX={}",
        hex16(first_batch.candidates[0].as_bytes())
    );
    println!("PH20_BACKFILL_FIRST_SLOT_SEQ={second_slot_seq}");
    println!(
        "PH20_BACKFILL_FIRST_READ={}",
        slot_vector_summary(
            &vault
                .read_slot_vector_at(second_slot_seq, second_id, new_slot)
                .expect("read second dense")
                .expect("second dense row")
        )
    );
    println!(
        "PH20_SCHEDULER_AFTER_FIRST_DIGEST={}",
        digest_hex(&scheduler_after_first)
    );
    println!(
        "PH20_SCHEDULER_AFTER_FIRST={}",
        serde_json::to_string(&scheduler.watermarks()).unwrap()
    );

    let mut scheduler = BackfillScheduler::open(
        &scheduler_path,
        BackfillConfig {
            max_concurrent: 1,
            batch_size: 1,
            throttle_ms: 10,
        },
    )
    .expect("reopen durable scheduler");
    let throttled = scheduler
        .claim_next_batch(1005)
        .expect("claim inside throttle")
        .expect("throttle result");
    assert!(throttled.throttled);
    let second_batch = scheduler
        .claim_next_batch(1010)
        .expect("claim resumed durable batch")
        .expect("second durable batch");
    assert_eq!(second_batch.candidates, vec![first_id]);
    let first_dense = SlotVector::Dense {
        dim: 2,
        data: vec![0.6, 0.8],
    };
    let first_slot_seq = vault
        .put_slot_vector(first_id, new_slot, &first_dense)
        .expect("write first dense");
    scheduler
        .complete_batch(second_batch.slot_id, second_batch.lens_id, 1010)
        .expect("complete resumed durable batch");
    let first_dense_read = vault
        .read_slot_vector_at(first_slot_seq, first_id, new_slot)
        .expect("read first dense")
        .expect("first dense row");
    println!(
        "PH20_BACKFILL_SECOND_TASK_CX={}",
        hex16(second_batch.candidates[0].as_bytes())
    );
    println!("PH20_BACKFILL_SECOND_SLOT_SEQ={first_slot_seq}");
    println!(
        "PH20_BACKFILL_SECOND_READ={}",
        slot_vector_summary(&first_dense_read)
    );
    println!(
        "PH20_SCHEDULER_FINAL={}",
        serde_json::to_string(&scheduler.watermarks()).unwrap()
    );
    let scheduler_final = std::fs::read(&scheduler_path).expect("read final scheduler state");
    println!(
        "PH20_SCHEDULER_FINAL_DIGEST={}",
        digest_hex(&scheduler_final)
    );
    assert_eq!(first_dense_read, first_dense);
    assert!(scheduler.watermarks().iter().all(|mark| mark.complete));

    let retired = controller
        .retire_lens(new_slot, 34)
        .expect("retire hot-added slot");
    let final_seq = vault.snapshot();
    let first_base_final = base_bytes(&vault, first_id, final_seq);
    let second_base_final = base_bytes(&vault, second_id, final_seq);
    let historical_first = vault.get(first_id, final_seq).expect("historical first");
    let retired_slot_read = vault
        .read_slot_vector_at(final_seq, first_id, new_slot)
        .expect("read retired slot row")
        .expect("retired slot row still readable");

    println!("PH20_FINAL_SEQ={final_seq}");
    println!(
        "PH20_BASE_FIRST_FINAL_DIGEST={}",
        digest_hex(&first_base_final)
    );
    println!(
        "PH20_BASE_SECOND_FINAL_DIGEST={}",
        digest_hex(&second_base_final)
    );
    println!("PH20_RETIRED_STATE={:?}", retired.state);
    println!(
        "PH20_HISTORICAL_SLOT_COUNT={}",
        historical_first.slots.len()
    );
    println!(
        "PH20_RETIRED_SLOT_STILL_READABLE={}",
        slot_vector_summary(&retired_slot_read)
    );
    println!(
        "PH20_BASE_UNCHANGED_FINAL={}",
        first_base_before == first_base_final && second_base_before == second_base_final
    );

    assert_eq!(retired.state, SlotState::Retired);
    assert_eq!(historical_first, first);
    assert_eq!(retired_slot_read, first_dense);
    assert_eq!(first_base_before, first_base_final);
    assert_eq!(second_base_before, second_base_final);
    vault.flush().expect("flush final state");
    drop(vault);

    let reopened = AsterVault::open(
        &vault_dir,
        vault_id(),
        b"ph20-hot-swap-salt".to_vec(),
        VaultOptions::default(),
    )
    .expect("reopen durable vault");
    let reopened_seq = reopened.snapshot();
    let reopened_first_slot = reopened
        .read_slot_vector_at(reopened_seq, first_id, new_slot)
        .expect("read reopened first slot")
        .expect("reopened first slot row");
    let reopened_second_slot = reopened
        .read_slot_vector_at(reopened_seq, second_id, new_slot)
        .expect("read reopened second slot")
        .expect("reopened second slot row");
    println!("PH20_REOPENED_SEQ={reopened_seq}");
    println!(
        "PH20_REOPENED_FIRST_SLOT_READ={}",
        slot_vector_summary(&reopened_first_slot)
    );
    println!(
        "PH20_REOPENED_SECOND_SLOT_READ={}",
        slot_vector_summary(&reopened_second_slot)
    );
    assert_eq!(reopened_first_slot, first_dense);
    assert_eq!(reopened_second_slot, second_dense);
}

fn fsv_root() -> PathBuf {
    if let Ok(root) = std::env::var("CALYX_FSV_ROOT") {
        return PathBuf::from(root);
    }
    let home = std::env::var("CALYX_HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home)
        .join("data")
        .join(format!("fsv-issue106-test-{}", std::process::id()))
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().expect("valid ULID")
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

fn constellation(
    vault: &AsterVault<SystemClock>,
    input: &[u8],
    created_at: u64,
    seq: u64,
    data: [f32; 2],
) -> Constellation {
    let cx_id = vault.cx_id_for_input(input, 1);
    let mut input_hash = [0_u8; 32];
    input_hash[..input.len()].copy_from_slice(input);
    let mut slots = BTreeMap::new();
    slots.insert(
        SlotId::new(0),
        SlotVector::Dense {
            dim: 2,
            data: data.to_vec(),
        },
    );
    Constellation {
        cx_id,
        vault_id: vault_id(),
        panel_version: 1,
        created_at,
        input_ref: InputRef {
            hash: input_hash,
            pointer: Some(format!("synthetic://{}", String::from_utf8_lossy(input))),
            redacted: false,
        },
        modality: Modality::Text,
        slots,
        scalars: BTreeMap::new(),
        metadata: BTreeMap::new(),
        anchors: Vec::new(),
        provenance: LedgerRef {
            seq,
            hash: [seq as u8; 32],
        },
        flags: CxFlags {
            ungrounded: true,
            ..CxFlags::default()
        },
    }
}

fn base_bytes<C: Clock>(vault: &AsterVault<C>, cx_id: CxId, snapshot: u64) -> Vec<u8> {
    vault
        .read_cf_at(
            snapshot,
            calyx_aster::cf::ColumnFamily::Base,
            cx_id.as_bytes(),
        )
        .expect("read base")
        .expect("base row")
}

fn slot_state(panel: &Panel, slot_id: SlotId) -> SlotState {
    panel
        .slots
        .iter()
        .find(|slot| slot.slot_id == slot_id)
        .expect("slot exists")
        .state
}

fn digest_hex(bytes: &[u8]) -> String {
    let digest = content_address([bytes]);
    hex16(&digest)
}

fn slot_vector_summary(vector: &SlotVector) -> String {
    match vector {
        SlotVector::Dense { dim, data } => format!(
            "dense:dim={dim}:len={}:first={:.3}:last={:.3}",
            data.len(),
            data.first().copied().unwrap_or_default(),
            data.last().copied().unwrap_or_default()
        ),
        SlotVector::Absent { reason } => format!("absent:{reason:?}"),
        SlotVector::Sparse { dim, entries } => format!("sparse:dim={dim}:nnz={}", entries.len()),
        SlotVector::Multi { token_dim, tokens } => {
            format!("multi:token_dim={token_dim}:tokens={}", tokens.len())
        }
    }
}

fn hex16(bytes: &[u8; 16]) -> String {
    let mut out = String::with_capacity(32);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}
