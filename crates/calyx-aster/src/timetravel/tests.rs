use super::*;
use crate::cf::ColumnFamily;
use crate::timetravel::time_index::encode_key;
use calyx_core::{
    Clock, Constellation, CxFlags, CxId, InputRef, LedgerRef, Modality, SlotId, SlotVector, Ts,
    VaultId, VaultStore,
};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// A clock the test advances between commits so each group-commit is stamped
/// with a known wall-clock millisecond.
struct StepClock(AtomicU64);

impl StepClock {
    fn new(start: Ts) -> Self {
        Self(AtomicU64::new(start))
    }
    fn set(&self, t: Ts) {
        self.0.store(t, Ordering::SeqCst);
    }
}

impl Clock for StepClock {
    fn now(&self) -> Ts {
        self.0.load(Ordering::SeqCst)
    }
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}

fn vault_at(t: Ts) -> AsterVault<StepClock> {
    AsterVault::with_clock(vault_id(), b"timetravel", StepClock::new(t))
}

/// Builds a one-slot constellation whose dense vector encodes `tag`, so a
/// time-travel read can be byte-compared against the value at ingest time.
fn constellation(vault: &AsterVault<StepClock>, input: &[u8], tag: f32) -> Constellation {
    let cx_id = vault.cx_id_for_input(input, 1);
    let mut input_hash = [0_u8; 32];
    input_hash[..input.len().min(32)].copy_from_slice(&input[..input.len().min(32)]);
    let mut slots = BTreeMap::new();
    slots.insert(
        SlotId::new(0),
        SlotVector::Dense {
            dim: 2,
            data: vec![tag, tag + 1.0],
        },
    );
    Constellation {
        cx_id,
        vault_id: vault_id(),
        panel_version: 1,
        created_at: 10,
        input_ref: InputRef {
            hash: input_hash,
            pointer: None,
            redacted: false,
        },
        modality: Modality::Text,
        slots,
        scalars: BTreeMap::new(),
        metadata: BTreeMap::new(),
        anchors: Vec::new(),
        provenance: LedgerRef {
            seq: 1,
            hash: [7; 32],
        },
        flags: CxFlags {
            ungrounded: true,
            ..CxFlags::default()
        },
    }
}

fn ingest(vault: &AsterVault<StepClock>, input: &[u8], tag: f32, at: Ts) -> CxId {
    vault.clock_set(at);
    let cx = constellation(vault, input, tag);
    let id = cx.cx_id;
    vault.put(cx).expect("ingest");
    id
}

impl AsterVault<StepClock> {
    fn clock_set(&self, t: Ts) {
        self.clock_ref().set(t);
    }
}

#[test]
fn as_of_excludes_writes_after_the_timestamp() {
    let vault = vault_at(0);
    let c1 = ingest(&vault, b"c1", 1.0, 1000);
    let c2 = ingest(&vault, b"c2", 2.0, 2000);

    // as_of(1500): C1 present, C2 absent (it had not been written yet).
    let snap = vault.as_of(1500).expect("as_of 1500");
    assert!(snap.get_cx(c1).is_ok(), "C1 must exist at t=1500");
    let missing = snap.get_cx(c2).unwrap_err();
    assert!(
        missing.code == "CALYX_STALE_DERIVED" || missing.code == "CALYX_CX_NOT_FOUND",
        "C2 must be reported missing at t=1500, got {}",
        missing.code
    );
}

#[test]
fn as_of_at_later_time_includes_both() {
    let vault = vault_at(0);
    let c1 = ingest(&vault, b"c1", 1.0, 1000);
    let c2 = ingest(&vault, b"c2", 2.0, 2000);
    let snap = vault.as_of(2000).expect("as_of 2000");
    assert!(snap.get_cx(c1).is_ok());
    assert!(snap.get_cx(c2).is_ok());
}

#[test]
fn as_of_returns_pre_mutation_bytes() {
    // Constellations are content-addressed and immutable (re-putting the same
    // CxId with different bytes is correctly rejected as a collision), so
    // time-travel over an in-place mutation is proven on a mutable KV row.
    let vault = vault_at(0);
    vault.clock_set(1000);
    vault
        .write_cf(ColumnFamily::Graph, b"k".to_vec(), b"v1".to_vec())
        .expect("write v1");
    vault.clock_set(2000);
    vault
        .write_cf(ColumnFamily::Graph, b"k".to_vec(), b"v2".to_vec())
        .expect("write v2");

    // as_of(1500) reads the pre-mutation byte value; as_of(2000) reads the new.
    let past = vault.as_of(1500).expect("as_of 1500");
    assert_eq!(
        past.read_cf(ColumnFamily::Graph, b"k").unwrap(),
        Some(b"v1".to_vec()),
        "time-travel must see pre-mutation bytes"
    );
    let now = vault.as_of(2000).expect("as_of 2000");
    assert_eq!(
        now.read_cf(ColumnFamily::Graph, b"k").unwrap(),
        Some(b"v2".to_vec())
    );
}

#[test]
fn deterministic_prefix_property_each_t_sees_exactly_k() {
    // The proptest property, run deterministically: after k monotonic ingests,
    // as_of(t_k) sees exactly the first k constellations.
    let vault = vault_at(0);
    let mut ids = Vec::new();
    for k in 1..=12u64 {
        let input = format!("cx{k}");
        ids.push(ingest(&vault, input.as_bytes(), k as f32, k * 1000));
    }
    for k in 1..=12u64 {
        let snap = vault.as_of(k * 1000).expect("as_of");
        for (i, id) in ids.iter().enumerate() {
            let present = snap.get_cx(*id).is_ok();
            let expected = (i as u64) < k; // first k present
            assert_eq!(
                present,
                expected,
                "at t={}, cx#{} present={present} expected={expected}",
                k * 1000,
                i + 1
            );
        }
    }
}

#[test]
fn as_of_before_any_write_is_no_data() {
    let vault = vault_at(0);
    ingest(&vault, b"c1", 1.0, 1000);
    let err = vault.as_of(0).unwrap_err();
    assert_eq!(err.code, "CALYX_TIMETRAVEL_NO_DATA");
}

#[test]
fn single_write_boundary() {
    let vault = vault_at(0);
    let c1 = ingest(&vault, b"only", 1.0, 500);
    assert_eq!(
        vault.as_of(499).unwrap_err().code,
        "CALYX_TIMETRAVEL_NO_DATA"
    );
    let snap = vault.as_of(500).expect("as_of 500");
    assert!(snap.get_cx(c1).is_ok());
}

#[test]
fn absolute_horizon_fails_closed_before_inclusive_boundary() {
    let vault = vault_at(0);
    vault
        .set_retention_horizon(RetentionHorizon::absolute(5000))
        .expect("set horizon");
    let c1 = ingest(&vault, b"horizon-boundary", 1.0, 5000);

    let before = vault.as_of(4999).unwrap_err();
    assert_eq!(before.code, CALYX_TIMETRAVEL_BEFORE_HORIZON);
    assert!(before.message.contains("requested_millis=4999"));
    assert!(before.message.contains("horizon_millis=5000"));

    let at_boundary = vault.as_of(5000).expect("inclusive horizon boundary");
    assert!(at_boundary.get_cx(c1).is_ok());
}

#[test]
fn none_horizon_preserves_no_data_error() {
    let vault = vault_at(0);
    ingest(&vault, b"no-data", 1.0, 500);
    vault
        .set_retention_horizon(RetentionHorizon::none())
        .expect("default none is valid");
    let err = vault.as_of(0).unwrap_err();
    assert_eq!(err.code, "CALYX_TIMETRAVEL_NO_DATA");
}

#[test]
fn rolling_zero_horizon_rejects_anything_before_now() {
    let vault = vault_at(10_000);
    vault
        .set_retention_horizon(RetentionHorizon::rolling(Duration::ZERO))
        .expect("set rolling zero");
    let c1 = ingest(&vault, b"rolling-zero", 1.0, 10_000);

    let before = vault.as_of(9_999).unwrap_err();
    assert_eq!(before.code, CALYX_TIMETRAVEL_BEFORE_HORIZON);
    let at_now = vault.as_of(10_000).expect("as_of now");
    assert!(at_now.get_cx(c1).is_ok());
}

#[test]
fn rolling_horizon_uses_current_clock_and_saturating_math() {
    let vault = vault_at(10_000);
    vault
        .set_retention_horizon(RetentionHorizon::rolling(Duration::from_secs(1)))
        .expect("set rolling horizon");
    let c1 = ingest(&vault, b"rolling", 1.0, 9_000);
    vault.clock_set(10_000);

    let before = vault.as_of(8_999).unwrap_err();
    assert_eq!(before.code, CALYX_TIMETRAVEL_BEFORE_HORIZON);
    assert!(before.message.contains("horizon_millis=9000"));
    let at_horizon = vault.as_of(9_000).expect("rolling inclusive horizon");
    assert!(at_horizon.get_cx(c1).is_ok());
}

#[test]
fn dropped_snapshot_releases_its_pin() {
    let vault = vault_at(0);
    ingest(&vault, b"c1", 1.0, 1000);
    let snap = vault.as_of(1000).expect("as_of");
    let lease = snap.lease_id_for_test();
    drop(snap);
    // After drop the lease is already gone: releasing it again is a no-op. If
    // drop had leaked the pin, this would return true.
    assert!(
        !vault.release_reader(lease),
        "drop must have released the lease pin"
    );
}

#[test]
fn corrupt_time_index_key_fails_closed() {
    let vault = vault_at(0);
    ingest(&vault, b"c1", 1.0, 1000);
    // Inject a malformed key directly into the CF. It is longer than a valid
    // key, but sorts inside the as_of(1000) range so resolve must fail closed.
    vault
        .write_cf(
            ColumnFamily::TimeIndex,
            vec![0u8; 17],
            time_index::SENTINEL.to_vec(),
        )
        .expect("inject corrupt key");
    let err = time_index::read_all(&vault).unwrap_err();
    assert_eq!(err.code, "CALYX_ASTER_CORRUPT_SHARD");
    let err = vault.as_of(1000).unwrap_err();
    assert_eq!(err.code, "CALYX_ASTER_CORRUPT_SHARD");
}

#[test]
fn time_index_has_one_entry_per_committed_seq() {
    let vault = vault_at(0);
    ingest(&vault, b"c1", 1.0, 1000);
    ingest(&vault, b"c2", 2.0, 2000);
    let entries = read_all(&vault).expect("read time index");
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].millis, 1000);
    assert_eq!(entries[1].millis, 2000);
    // Each entry's seqno resolves to a real snapshot.
    assert_eq!(encode_key(entries[0].millis, entries[0].seqno).len(), 16);
}
