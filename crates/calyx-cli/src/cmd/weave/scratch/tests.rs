//! Scratch-store tests (#1991). The writer routes every file through the
//! `private_fs` owner-only primitives, which have no implementation off Unix and
//! fail closed there, so the on-disk roundtrip tests are Unix-only (they run in
//! the WSL weave suite). The byte-identity guarantee they prove is the #1960
//! contract: a vector read back out-of-core is byte-for-byte the vector the scan
//! would have held in-heap.
#![cfg(unix)]

use super::*;
use calyx_aster::vault::encode::encode_slot_vector;
use calyx_core::{CxId, SlotId, SlotVector};

fn run_dir() -> std::path::PathBuf {
    let base = std::env::temp_dir().join(format!(
        "calyx-weave-scratch-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&base).unwrap();
    base
}

fn dense(i: u8) -> SlotVector {
    SlotVector::Dense {
        dim: 4,
        data: vec![i as f32, i as f32 + 0.5, -(i as f32), 3.25],
    }
}

fn multi(i: u8) -> SlotVector {
    SlotVector::Multi {
        token_dim: 2,
        tokens: vec![
            vec![i as f32, 1.0 - i as f32],
            vec![2.0 * i as f32, 0.125],
            vec![-1.0, i as f32],
        ],
    }
}

fn cx(i: u8) -> CxId {
    CxId::from_bytes([i; 16])
}

#[test]
fn roundtrip_pooled_and_multi_vectors_byte_identical() {
    // The canonical roster shape: one pooled dense lane + one chunk-Multi lane,
    // over three constellations, with a deliberate absent gap in the dense lane.
    let dense_slot = SlotId::new(9);
    let multi_slot = SlotId::new(10);
    let slots = [dense_slot, multi_slot];
    let dir = run_dir();

    let dense_vectors = [Some(dense(0)), None, Some(dense(2))];
    let multi_vectors = [Some(multi(0)), Some(multi(1)), Some(multi(2))];

    let mut writer = WeaveVectorScratchWriter::create(&dir, &slots).unwrap();
    for (ordinal, vector) in dense_vectors.iter().enumerate() {
        let encoded = vector.as_ref().map(|v| encode_slot_vector(v).unwrap());
        writer
            .push(dense_slot, ordinal as u64, encoded.as_deref())
            .unwrap();
    }
    for (ordinal, vector) in multi_vectors.iter().enumerate() {
        let encoded = vector.as_ref().map(|v| encode_slot_vector(v).unwrap());
        writer
            .push(multi_slot, ordinal as u64, encoded.as_deref())
            .unwrap();
    }
    let scratch = writer.finish(3).unwrap();

    assert_eq!(scratch.node_count(), 3);
    // Every stored vector reads back byte-identically.
    for (ordinal, vector) in dense_vectors.iter().enumerate() {
        assert_eq!(
            scratch.read_vector(dense_slot, ordinal as u64).unwrap(),
            *vector,
            "dense ordinal {ordinal} roundtrip"
        );
    }
    for (ordinal, vector) in multi_vectors.iter().enumerate() {
        assert_eq!(
            scratch.read_vector(multi_slot, ordinal as u64).unwrap(),
            *vector,
            "multi ordinal {ordinal} roundtrip"
        );
    }
    // Presence tracks the absent gap exactly.
    assert!(scratch.is_present(dense_slot, 0).unwrap());
    assert!(!scratch.is_present(dense_slot, 1).unwrap());
    assert!(scratch.is_present(dense_slot, 2).unwrap());
    assert!(scratch.is_present(multi_slot, 1).unwrap());

    // read_panel fails closed on the ordinal whose dense lane is absent.
    assert!(scratch.read_panel(0, &slots).is_ok());
    let err = scratch.read_panel(1, &slots).unwrap_err();
    assert!(
        err.message()
            .contains("no measured vector in contracted slot")
    );

    scratch.cleanup().unwrap();
}

#[test]
fn out_of_order_push_fails_closed() {
    let slot = SlotId::new(5);
    let dir = run_dir();
    let mut writer = WeaveVectorScratchWriter::create(&dir, &[slot]).unwrap();
    let encoded = encode_slot_vector(&dense(1)).unwrap();
    let err = writer.push(slot, 1, Some(&encoded)).unwrap_err();
    assert!(err.message().contains("expected ordinal 0"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn finish_rejects_ragged_lane_counts() {
    let a = SlotId::new(1);
    let b = SlotId::new(2);
    let dir = run_dir();
    let mut writer = WeaveVectorScratchWriter::create(&dir, &[a, b]).unwrap();
    let encoded = encode_slot_vector(&dense(1)).unwrap();
    writer.push(a, 0, Some(&encoded)).unwrap();
    writer.push(b, 0, Some(&encoded)).unwrap();
    writer.push(a, 1, None).unwrap();
    // Slot `b` has one fewer record than the declared node_count.
    let err = writer.finish(2).unwrap_err();
    assert!(err.message().contains("expected 2"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn corrupt_offsets_sidecar_fails_closed_on_open() {
    let slot = SlotId::new(7);
    let dir = run_dir();
    let mut writer = WeaveVectorScratchWriter::create(&dir, &[slot]).unwrap();
    writer
        .push(slot, 0, Some(&encode_slot_vector(&dense(1)).unwrap()))
        .unwrap();
    let scratch = writer.finish(1).unwrap();
    drop(scratch);
    // Corrupt the magic of the published sidecar, then prove open fails closed.
    let scratch_dir = dir.join(super::SCRATCH_DIR_NAME);
    let off_path = scratch_dir.join(super::slot_off_name(slot));
    let mut bytes = std::fs::read(&off_path).unwrap();
    bytes[0] = b'X';
    std::fs::write(&off_path, &bytes).unwrap();
    let err = PackedSlot::open(
        slot,
        &scratch_dir.join(super::slot_rec_name(slot)),
        &off_path,
    )
    .unwrap_err();
    assert!(err.message().contains("bad magic"));
    let _ = std::fs::remove_dir_all(&dir);
}
