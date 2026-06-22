use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use calyx_anneal::{
    ChangeOutcome, FrozenLensGuard, FrozenLensSource, HeadKind, HeadPromotionGate,
    HeadShadowProposal, HeadStorage, MistakeRef, OnlineHead, OnlineHeadState, ReplayEntry,
};
use calyx_core::{CalyxError, CxId, FixedClock, LensId, Modality, Result};
use calyx_registry::{AlgorithmicLens, FrozenLensSnapshot, Registry};
use serde_json::json;

#[path = "fsv_support/mod.rs"]
mod fsv_support;
use fsv_support::write_json;

const TEST_TS: u64 = 1_785_500_409;

#[test]
#[ignore = "requires CALYX_ISSUE409_FSV_ROOT in a manual verification run"]
fn fsv_frozen_lens_guard_manual() {
    let root = PathBuf::from(
        std::env::var("CALYX_ISSUE409_FSV_ROOT")
            .expect("CALYX_ISSUE409_FSV_ROOT must point at a manual FSV root"),
    );
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();

    let mut registry = Registry::new();
    let byte = AlgorithmicLens::byte_features("issue409-byte", Modality::Text);
    let scalar = AlgorithmicLens::scalar("issue409-scalar", Modality::Text);
    let byte_id = registry
        .register_frozen(byte.clone(), byte.contract().clone())
        .unwrap();
    let scalar_id = registry
        .register_frozen(scalar.clone(), scalar.contract().clone())
        .unwrap();
    let mut guard = FrozenLensGuard::new(Arc::new(registry));
    guard.initialize().unwrap();
    let before_report = guard.check().unwrap();
    write_json(&root.join("before-report.json"), &before_report);

    let mut state = OnlineHeadState::open_with_guard(
        MemoryHeadStorage::default(),
        ScriptedGate::default(),
        Arc::new(FixedClock::new(TEST_TS)),
        [OnlineHead::new(HeadKind::Predictor, vec![0.0]).unwrap()],
        guard.clone(),
    )
    .unwrap();
    let outcome = state.update(&[entry(1.0, 1)], 0.01, 0.0).unwrap();
    let after_report = guard.check().unwrap();
    write_json(&root.join("after-report.json"), &after_report);

    let empty_source = Arc::new(MutableSource::new(Vec::new()));
    let mut empty_guard = FrozenLensGuard::new(empty_source.clone());
    empty_guard.initialize().unwrap();
    let empty_report = empty_guard.check().unwrap();
    empty_source.push(snapshot(9, 10));
    let new_lens_report = empty_guard.check().unwrap();
    write_json(&root.join("edge-new-report.json"), &new_lens_report);

    let mutation_source = Arc::new(MutableSource::new(vec![snapshot(3, 4), snapshot(4, 5)]));
    let mut mutation_guard = FrozenLensGuard::new(mutation_source.clone());
    mutation_guard.initialize().unwrap();
    let before_mutation = mutation_guard.check().unwrap();
    mutation_source.mutate_hash(lens(3), 0x7f);
    let after_mutation = mutation_guard.check().unwrap();
    let violation_error = mutation_guard.assert_no_violation().unwrap_err();
    write_json(&root.join("edge-violation-report.json"), &after_mutation);

    let unavailable_error = FrozenLensGuard::new(Arc::new(UnavailableSource))
        .check()
        .unwrap_err()
        .code
        .to_string();

    let artifact = json!({
        "issue": 409,
        "source_of_truth": "FrozenLensGuard known_hashes plus live registry frozen_lens_snapshots serialized to report JSON files",
        "trigger": "guarded OnlineHeadState::update over two registered frozen AlgorithmicLens entries",
        "expected": {
            "byte_lens_id": byte_id,
            "scalar_lens_id": scalar_id,
            "stable_before_after": true,
            "guarded_update_promoted": true
        },
        "paths": {
            "before_report": root.join("before-report.json").display().to_string(),
            "after_report": root.join("after-report.json").display().to_string(),
            "edge_new_report": root.join("edge-new-report.json").display().to_string(),
            "edge_violation_report": root.join("edge-violation-report.json").display().to_string()
        },
        "before": before_report,
        "after": after_report,
        "guarded_update": {
            "promoted": outcome.promoted,
            "change_id": outcome.change_id
        },
        "edges": {
            "empty_report": empty_report,
            "new_lens_report": new_lens_report,
            "before_mutation": before_mutation,
            "after_mutation": after_mutation,
            "violation_error": violation_error.code,
            "unavailable_error": unavailable_error
        }
    });
    write_json(&root.join("issue409-fsv-artifact.json"), &artifact);

    assert!(outcome.promoted);
    assert_eq!(before_report.rows, after_report.rows);
    assert!(after_report.violations.is_empty());
    assert!(empty_report.rows.is_empty());
    assert_eq!(new_lens_report.new_lenses, vec![lens(9)]);
    assert_eq!(after_mutation.violations, vec![lens(3)]);
    assert_eq!(violation_error.code, "CALYX_LENS_FROZEN_VIOLATION");
    assert_eq!(unavailable_error, "CALYX_REGISTRY_UNAVAILABLE");
}

#[derive(Clone, Default)]
struct MemoryHeadStorage {
    rows: Arc<Mutex<BTreeMap<Vec<u8>, Vec<u8>>>>,
}

impl HeadStorage for MemoryHeadStorage {
    fn load_head(&self, kind: HeadKind) -> Result<Option<Vec<u8>>> {
        Ok(self
            .rows
            .lock()
            .unwrap()
            .get(&calyx_anneal::head_key(kind))
            .cloned())
    }

    fn save_heads(&self, rows: Vec<(HeadKind, Vec<u8>)>) -> Result<()> {
        let mut inner = self.rows.lock().unwrap();
        for (kind, value) in rows {
            inner.insert(calyx_anneal::head_key(kind), value);
        }
        Ok(())
    }

    fn scan_heads(&self) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        Ok(self
            .rows
            .lock()
            .unwrap()
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect())
    }
}

#[derive(Clone, Default)]
struct ScriptedGate {
    next_id: Arc<AtomicU64>,
}

impl HeadPromotionGate for ScriptedGate {
    fn ensure_head_prior(
        &mut self,
        _key: calyx_anneal::ArtifactKey,
        _ptr: calyx_anneal::ArtifactPtr,
    ) -> Result<()> {
        Ok(())
    }

    fn propose_head_change(
        &mut self,
        _key: calyx_anneal::ArtifactKey,
        _candidate_ptr: calyx_anneal::ArtifactPtr,
        _candidate: &HeadShadowProposal,
        _incumbent: &HeadShadowProposal,
        _description: &str,
    ) -> Result<ChangeOutcome> {
        Ok(ChangeOutcome::Promoted(calyx_anneal::ChangeId(
            self.next_id.fetch_add(1, Ordering::SeqCst) + 409_000,
        )))
    }
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
            "registry unavailable in FSV",
        ))
    }
}

fn entry(surprise: f64, seq: u64) -> ReplayEntry {
    ReplayEntry::new(
        CxId::from_bytes([seq as u8; 16]),
        surprise,
        MistakeRef { seq, surprise },
        TEST_TS,
    )
    .unwrap()
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
