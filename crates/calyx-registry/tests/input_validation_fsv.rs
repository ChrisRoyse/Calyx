use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use calyx_aster::cf::{ColumnFamily, base_key, slot_key};
use calyx_aster::vault::{AsterVault, VaultOptions, encode};
use calyx_core::{
    CALYX_RECORD_SCHEMA_VIOLATION, CxFlags, CxId, Input, InputRef, LedgerRef, Lens, LensId,
    Modality, Result, SlotId, SlotShape, SlotVector, VaultId, VaultStore,
};
use calyx_registry::{FrozenLensContract, LensDType, NormPolicy, Registry};
use calyx_sextant::{CALYX_SEXTANT_QUERY_SHAPE, HnswIndex, Query, SearchEngine, SlotIndexMap};
use serde_json::json;

#[test]
#[ignore = "manual FSV for PH60 issue #593 boundary validation"]
fn issue593_input_validation_boundary_fsv() {
    let root = clean_dir(&fsv_root());
    let vault_dir = root.join("vault");
    let readback_path = root.join("issue593-readback.json");
    let vault = AsterVault::new_durable(
        &vault_dir,
        vault_id(),
        b"issue593-input-validation",
        VaultOptions::default(),
    )
    .expect("open durable vault");

    let lens_edges = lens_boundary_edges(&vault);
    let query_edge = query_boundary_edge(&vault);
    let schema_edge = schema_boundary_edge(&vault, &vault_dir);
    let happy = happy_path(&vault, &vault_dir);
    let schema_after_happy = schema_after_happy_edge(&vault, &vault_dir);

    let readback = json!({
        "source_of_truth": "Aster durable CF rows + WAL bytes under vault/, Registry lens measurement errors, and Sextant index stats",
        "fsv_root": root.display().to_string(),
        "vault_dir": vault_dir.display().to_string(),
        "lens_edges": lens_edges,
        "query_edge": query_edge,
        "schema_edge_before_any_write": schema_edge,
        "happy_path": happy,
        "schema_edge_after_valid_write": schema_after_happy,
        "expected_codes": {
            "bad_dim_lens": "CALYX_LENS_DIM_MISMATCH",
            "nan_lens": "CALYX_LENS_NUMERICAL_INVARIANT",
            "non_unit_lens": "CALYX_LENS_NUMERICAL_INVARIANT",
            "malformed_query": CALYX_SEXTANT_QUERY_SHAPE,
            "schema_violation": CALYX_RECORD_SCHEMA_VIOLATION,
        },
    });
    fs::write(
        &readback_path,
        serde_json::to_vec_pretty(&readback).unwrap(),
    )
    .unwrap();

    println!("ISSUE593_FSV_ROOT={}", root.display());
    println!("ISSUE593_READBACK={}", readback_path.display());
    println!("{}", serde_json::to_string_pretty(&readback).unwrap());
}

fn lens_boundary_edges(vault: &AsterVault) -> serde_json::Value {
    let mut registry = Registry::new();
    let input = Input::new(Modality::Text, b"issue593 lens input".to_vec());
    let bad_dim = register_static(
        &mut registry,
        "issue593-bad-dim",
        SlotShape::Dense(2),
        NormPolicy::None,
        SlotVector::Dense {
            dim: 1,
            data: vec![1.0],
        },
    );
    let nan = register_static(
        &mut registry,
        "issue593-nan",
        SlotShape::Dense(2),
        NormPolicy::None,
        SlotVector::Dense {
            dim: 2,
            data: vec![f32::NAN, 1.0],
        },
    );
    let non_unit = register_static(
        &mut registry,
        "issue593-non-unit",
        SlotShape::Dense(2),
        NormPolicy::Unit { tolerance: 1.0e-3 },
        SlotVector::Dense {
            dim: 2,
            data: vec![2.0, 0.0],
        },
    );
    let before_snapshot = vault.snapshot();
    let bad_dim_error = registry.measure(bad_dim, &input).unwrap_err();
    let nan_error = registry.measure(nan, &input).unwrap_err();
    let non_unit_error = registry.measure(non_unit, &input).unwrap_err();
    let after_snapshot = vault.snapshot();

    assert_eq!(bad_dim_error.code, "CALYX_LENS_DIM_MISMATCH");
    assert_eq!(nan_error.code, "CALYX_LENS_NUMERICAL_INVARIANT");
    assert_eq!(non_unit_error.code, "CALYX_LENS_NUMERICAL_INVARIANT");
    assert_eq!(after_snapshot, before_snapshot);
    json!({
        "trigger": "Registry::measure on frozen contracts whose runtime lens emits bad vectors",
        "bad_dim_code": bad_dim_error.code,
        "nan_code": nan_error.code,
        "non_unit_code": non_unit_error.code,
        "aster_snapshot_before": before_snapshot,
        "aster_snapshot_after": after_snapshot,
        "nothing_persisted_to_aster": after_snapshot == before_snapshot,
    })
}

fn query_boundary_edge(vault: &AsterVault) -> serde_json::Value {
    let map = SlotIndexMap::new();
    map.register(HnswIndex::new(slot(), 2, 42)).unwrap();
    let engine = SearchEngine::new(map);
    engine
        .indexes
        .insert(slot(), cx(0x21), dense(vec![1.0, 0.0]), 1)
        .unwrap();
    let before_stats = engine.indexes.stats();
    let before_snapshot = vault.snapshot();
    let error = engine
        .search(
            &Query::new("issue593 malformed")
                .with_slots(vec![slot()])
                .with_vector(SlotVector::Dense {
                    dim: 2,
                    data: vec![f32::NAN, 0.0],
                }),
        )
        .unwrap_err();
    let after_stats = engine.indexes.stats();
    let after_snapshot = vault.snapshot();

    assert_eq!(error.code, CALYX_SEXTANT_QUERY_SHAPE);
    assert_eq!(after_stats, before_stats);
    assert_eq!(after_snapshot, before_snapshot);
    json!({
        "trigger": "SearchEngine::search with NaN query vector",
        "code": error.code,
        "index_stats_before": before_stats,
        "index_stats_after": after_stats,
        "index_stats_unchanged": after_stats == before_stats,
        "aster_snapshot_before": before_snapshot,
        "aster_snapshot_after": after_snapshot,
    })
}

fn schema_boundary_edge(vault: &AsterVault, vault_dir: &Path) -> serde_json::Value {
    let bad_id = cx(0x31);
    let before = storage_state(vault, bad_id);
    let wal_before = wal_summary(vault_dir);
    let mut bad = constellation(bad_id, dense(vec![1.0, 0.0]));
    bad.scalars.insert("quality".to_string(), f64::NAN);

    let error = vault.put(bad).unwrap_err();
    let after = storage_state(vault, bad_id);
    let wal_after = wal_summary(vault_dir);

    assert_eq!(error.code, CALYX_RECORD_SCHEMA_VIOLATION);
    assert_eq!(after, before);
    assert_eq!(wal_after, wal_before);
    json!({
        "trigger": "AsterVault::put with scalar NaN",
        "code": error.code,
        "before": before,
        "after": after,
        "wal_before": wal_before,
        "wal_after": wal_after,
        "no_cf_or_wal_delta": after == before && wal_after == wal_before,
    })
}

fn happy_path(vault: &AsterVault, vault_dir: &Path) -> serde_json::Value {
    let valid_id = cx(0x41);
    let valid = constellation(valid_id, dense(vec![1.0, 0.0]));
    let before = storage_state(vault, valid_id);
    vault.put(valid).expect("valid record persists");
    vault.flush().expect("flush valid record");
    let after = storage_state(vault, valid_id);
    let wal = wal_summary(vault_dir);

    assert!(!before.base_present);
    assert!(after.base_present);
    assert!(after.slot_present);
    assert!(after.ledger_rows >= 1);
    json!({
        "trigger": "AsterVault::put with valid finite dense record",
        "before": before,
        "after": after,
        "wal": wal,
        "base_and_slot_present": after.base_present && after.slot_present,
    })
}

fn schema_after_happy_edge(vault: &AsterVault, vault_dir: &Path) -> serde_json::Value {
    let bad_id = cx(0x51);
    let before = storage_state(vault, bad_id);
    let wal_before = wal_summary(vault_dir);
    let bad = constellation(
        bad_id,
        SlotVector::Dense {
            dim: 2,
            data: vec![1.0],
        },
    );

    let error = vault.put(bad).unwrap_err();
    let after = storage_state(vault, bad_id);
    let wal_after = wal_summary(vault_dir);

    assert_eq!(error.code, CALYX_RECORD_SCHEMA_VIOLATION);
    assert_eq!(after, before);
    assert_eq!(wal_after, wal_before);
    json!({
        "trigger": "AsterVault::put with dense dim/data mismatch after a valid write",
        "code": error.code,
        "before": before,
        "after": after,
        "wal_before": wal_before,
        "wal_after": wal_after,
        "valid_data_preserved_and_no_invalid_delta": after == before && wal_after == wal_before,
    })
}

#[derive(Clone)]
struct StaticLens {
    contract: FrozenLensContract,
    output: SlotVector,
}

impl Lens for StaticLens {
    fn id(&self) -> LensId {
        self.contract.lens_id()
    }

    fn shape(&self) -> SlotShape {
        self.contract.shape()
    }

    fn modality(&self) -> Modality {
        Modality::Text
    }

    fn measure(&self, _input: &Input) -> Result<SlotVector> {
        Ok(self.output.clone())
    }
}

fn register_static(
    registry: &mut Registry,
    name: &str,
    shape: SlotShape,
    norm: NormPolicy,
    output: SlotVector,
) -> LensId {
    let contract = FrozenLensContract::new(
        name,
        hash(name, b"weights"),
        hash(name, b"corpus"),
        shape,
        Modality::Text,
        LensDType::F32,
        norm,
    );
    registry
        .register_frozen(StaticLens { contract, output }, frozen(name, shape, norm))
        .unwrap()
}

fn frozen(name: &str, shape: SlotShape, norm: NormPolicy) -> FrozenLensContract {
    FrozenLensContract::new(
        name,
        hash(name, b"weights"),
        hash(name, b"corpus"),
        shape,
        Modality::Text,
        LensDType::F32,
        norm,
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
struct StorageState {
    snapshot: u64,
    base_present: bool,
    slot_present: bool,
    ledger_rows: usize,
}

fn storage_state(vault: &AsterVault, cx_id: CxId) -> StorageState {
    let snapshot = vault.snapshot();
    StorageState {
        snapshot,
        base_present: vault
            .read_cf_at(snapshot, ColumnFamily::Base, &base_key(cx_id))
            .unwrap()
            .is_some(),
        slot_present: vault
            .read_cf_at(snapshot, ColumnFamily::slot(slot()), &slot_key(cx_id))
            .unwrap()
            .is_some(),
        ledger_rows: vault
            .scan_cf_at(snapshot, ColumnFamily::Ledger)
            .unwrap()
            .len(),
    }
}

fn wal_summary(vault_dir: &Path) -> serde_json::Value {
    let wal_dir = vault_dir.join("wal");
    let files = fs::read_dir(&wal_dir)
        .ok()
        .into_iter()
        .flat_map(|entries| entries.filter_map(std::result::Result::ok))
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "wal"))
        .collect::<Vec<_>>();
    if files.is_empty() {
        return json!({"files": 0, "records": 0, "record_rows": []});
    }
    let replay = calyx_aster::wal::replay_dir(&wal_dir).unwrap();
    let record_rows = replay
        .records
        .iter()
        .map(|record| {
            let rows = encode::decode_write_batch(&record.payload).unwrap();
            json!({
                "seq": record.seq,
                "payload_bytes": record.payload.len(),
                "row_count": rows.len(),
                "cfs": rows.iter().map(|row| format!("{:?}", row.cf)).collect::<Vec<_>>(),
            })
        })
        .collect::<Vec<_>>();
    json!({"files": files.len(), "records": replay.records.len(), "record_rows": record_rows})
}

fn constellation(cx_id: CxId, vector: SlotVector) -> calyx_core::Constellation {
    let mut slots = BTreeMap::new();
    slots.insert(slot(), vector);
    calyx_core::Constellation {
        cx_id,
        vault_id: vault_id(),
        panel_version: 1,
        created_at: 123,
        input_ref: InputRef {
            hash: [1; 32],
            pointer: Some(format!("synthetic://issue593/{cx_id}")),
            redacted: false,
        },
        modality: Modality::Text,
        slots,
        scalars: BTreeMap::new(),
        metadata: BTreeMap::new(),
        anchors: Vec::new(),
        provenance: LedgerRef {
            seq: 1,
            hash: [2; 32],
        },
        flags: CxFlags::default(),
    }
}

fn dense(data: Vec<f32>) -> SlotVector {
    SlotVector::Dense {
        dim: data.len() as u32,
        data,
    }
}

fn hash(name: &str, suffix: &[u8]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(name.as_bytes());
    hasher.update(suffix);
    hasher.finalize().into()
}

fn clean_dir(path: &Path) -> PathBuf {
    let _ = fs::remove_dir_all(path);
    fs::create_dir_all(path).unwrap();
    path.to_path_buf()
}

fn fsv_root() -> PathBuf {
    std::env::var("CALYX_ISSUE593_FSV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("calyx-issue593-input-validation-fsv"))
}

const fn slot() -> SlotId {
    SlotId::new(8)
}

fn cx(value: u8) -> CxId {
    CxId::from_bytes([value; 16])
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}
