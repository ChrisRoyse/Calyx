use std::sync::{Arc, Mutex};

use calyx_anneal::{
    CALYX_REGISTRY_UNAVAILABLE, FrozenLensGuard, FrozenLensSource, FrozenLensStatus,
};
use calyx_core::{CalyxError, LensId, Modality, Result};
use calyx_registry::{AlgorithmicLens, FrozenLensSnapshot, Registry};
use proptest::prelude::*;

#[test]
fn stable_frozen_lenses_report_ok() {
    let source = Arc::new(MutableSource::new(vec![snapshot(1, 7), snapshot(2, 8)]));
    let mut guard = FrozenLensGuard::new(source);
    guard.initialize().unwrap();

    let report = guard.check().unwrap();

    assert_eq!(report.violations, Vec::<LensId>::new());
    assert_eq!(report.new_lenses, Vec::<LensId>::new());
    assert_eq!(report.ok, vec![lens(1), lens(2)]);
    assert!(report.rows.iter().all(|row| row.stable));
    assert!(
        report
            .rows
            .iter()
            .all(|row| row.status == FrozenLensStatus::Stable)
    );
}

#[test]
fn changed_weight_hash_is_a_frozen_violation() {
    let source = Arc::new(MutableSource::new(vec![snapshot(1, 7), snapshot(2, 8)]));
    let mut guard = FrozenLensGuard::new(source.clone());
    guard.initialize().unwrap();
    source.mutate_hash(lens(1), 0xff);

    let report = guard.check().unwrap();
    let error = guard.assert_no_violation().unwrap_err();

    assert_eq!(report.violations, vec![lens(1)]);
    assert_eq!(report.ok, vec![lens(2)]);
    assert_eq!(error.code, "CALYX_LENS_FROZEN_VIOLATION");
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(24))]

    #[test]
    fn unchanged_hashes_never_violate(values in prop::collection::vec(any::<[u8; 32]>(), 0..16)) {
        let rows = values
            .iter()
            .enumerate()
            .map(|(index, hash)| FrozenLensSnapshot {
                lens_id: lens(index as u8),
                weights_sha256: *hash,
            })
            .collect();
        let source = Arc::new(MutableSource::new(rows));
        let mut guard = FrozenLensGuard::new(source);
        guard.initialize().unwrap();

        prop_assert!(guard.check().unwrap().violations.is_empty());
    }
}

#[test]
fn zero_new_and_unavailable_edges_are_fail_closed() {
    let source = Arc::new(MutableSource::new(Vec::new()));
    let mut guard = FrozenLensGuard::new(source.clone());
    guard.initialize().unwrap();
    assert!(guard.check().unwrap().rows.is_empty());

    source.push(snapshot(3, 9));
    let report = guard.check().unwrap();
    assert_eq!(report.new_lenses, vec![lens(3)]);
    assert!(report.violations.is_empty());
    assert_eq!(report.rows[0].status, FrozenLensStatus::New);

    let unavailable = FrozenLensGuard::new(Arc::new(UnavailableSource));
    let error = unavailable.check().unwrap_err();
    assert_eq!(error.code, CALYX_REGISTRY_UNAVAILABLE);
}

#[test]
fn real_registry_snapshots_are_guarded() {
    let mut registry = Registry::new();
    let byte = AlgorithmicLens::byte_features("guard-byte", Modality::Text);
    let scalar = AlgorithmicLens::scalar("guard-scalar", Modality::Text);
    registry
        .register_frozen(byte.clone(), byte.contract().clone())
        .unwrap();
    registry
        .register_frozen(scalar.clone(), scalar.contract().clone())
        .unwrap();
    let mut guard = FrozenLensGuard::new(Arc::new(registry));

    guard.initialize().unwrap();
    let report = guard.check().unwrap();

    assert_eq!(report.ok.len(), 2);
    assert!(report.violations.is_empty());
    assert_eq!(guard.report().unwrap().len(), 2);
}

#[derive(Default)]
struct MutableSource {
    rows: Mutex<Vec<FrozenLensSnapshot>>,
}

impl MutableSource {
    fn new(rows: Vec<FrozenLensSnapshot>) -> Self {
        Self {
            rows: Mutex::new(rows),
        }
    }

    fn push(&self, snapshot: FrozenLensSnapshot) {
        self.rows.lock().unwrap().push(snapshot);
    }

    fn mutate_hash(&self, lens_id: LensId, mask: u8) {
        for row in self.rows.lock().unwrap().iter_mut() {
            if row.lens_id == lens_id {
                row.weights_sha256[0] ^= mask;
            }
        }
    }
}

impl FrozenLensSource for MutableSource {
    fn frozen_lens_snapshots(&self) -> Result<Vec<FrozenLensSnapshot>> {
        Ok(self.rows.lock().unwrap().clone())
    }
}

struct UnavailableSource;

impl FrozenLensSource for UnavailableSource {
    fn frozen_lens_snapshots(&self) -> Result<Vec<FrozenLensSnapshot>> {
        Err(CalyxError::registry_unavailable(
            "registry test double unavailable",
        ))
    }
}

fn snapshot(id: u8, hash: u8) -> FrozenLensSnapshot {
    FrozenLensSnapshot {
        lens_id: lens(id),
        weights_sha256: [hash; 32],
    }
}

fn lens(seed: u8) -> LensId {
    LensId::from_bytes([seed; 16])
}
