use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use calyx_aster::cf::{ColumnFamily, slot_key};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{
    CxFlags, CxId, Input, InputRef, LedgerRef, Modality, SlotShape, VaultId, VaultStore,
};
use calyx_registry::{
    AlgorithmicLens, BackfillCandidate, BackfillConfig, BackfillPriority, BackfillScheduler,
    DeterminismProof, Registry, SlotSpec, SwapController,
};
use calyx_sextant::{HnswIndex, ProvenanceSource, Query, SearchEngine, SlotIndexMap};

#[test]
#[ignore = "manual FSV writes Registry->Aster->Sextant source-of-truth readbacks"]
fn registry_add_lens_backfill_populates_sextant_index_fsv() {
    let root = clean_dir(&fsv_root().join("registry-sextant-integration"));
    let vault_dir = root.join("vault");
    let scheduler_path = root.join("backfill-scheduler.json");
    let readback_path = root.join("registry-sextant-readback.json");
    let vault = AsterVault::new_durable(
        &vault_dir,
        vault_id(),
        b"issue339-registry-sextant",
        VaultOptions::default(),
    )
    .unwrap();
    let docs = sample_docs();
    let mut inputs = BTreeMap::new();
    let mut candidates = Vec::new();
    for (idx, (name, text)) in docs.iter().enumerate() {
        let cx_id = cx(idx as u8 + 1);
        inputs.insert(cx_id, Input::new(Modality::Text, text.as_bytes().to_vec()));
        vault
            .put(constellation(cx_id, name, text, idx as u64 + 1))
            .unwrap();
        candidates.push(BackfillCandidate {
            cx_id,
            priority: (docs.len() - idx) as u32,
        });
    }

    let lens = AlgorithmicLens::byte_features("issue339-byte-features", Modality::Text);
    let contract = lens.contract().clone();
    let probe = Input::new(Modality::Text, b"issue339 deterministic probe".to_vec());
    let mut registry = Registry::new();
    let lens_id = registry
        .register_frozen_with_probe(lens, contract.clone(), &probe)
        .unwrap();
    assert_eq!(
        registry.determinism_proof(lens_id),
        Some(DeterminismProof::ProbeVerified)
    );

    let mut controller = SwapController::new(calyx_core::Panel {
        version: 1,
        slots: Vec::new(),
        created_at: 100,
        kernel_ref: None,
        guard_ref: None,
    });
    let mut scheduler = BackfillScheduler::open(
        &scheduler_path,
        BackfillConfig {
            max_concurrent: 1,
            batch_size: docs.len(),
            throttle_ms: 0,
        },
    )
    .unwrap();
    let outcome = controller
        .add_lens_durable(
            &registry,
            SlotSpec::dense_text(
                "issue339-registry-slot",
                lens_id,
                dense_dim(contract.shape()),
            ),
            candidates.clone(),
            200,
            &mut scheduler,
            BackfillPriority::Hot,
        )
        .unwrap();
    assert_eq!(outcome.queued, docs.len());

    let batch = scheduler.claim_next_batch(200).unwrap().unwrap();
    assert_eq!(batch.candidates.len(), docs.len());
    let mut backfilled = Vec::new();
    for cx_id in &batch.candidates {
        let input = inputs.get(cx_id).unwrap();
        let vector = registry.measure(lens_id, input).unwrap();
        vault
            .put_slot_vector(*cx_id, outcome.slot.slot_id, &vector)
            .unwrap();
        backfilled.push((*cx_id, vector));
    }
    scheduler
        .complete_batch(batch.slot_id, batch.lens_id, 200)
        .unwrap();

    let indexes = SlotIndexMap::new();
    indexes
        .register(HnswIndex::new(
            outcome.slot.slot_id,
            dense_dim(contract.shape()),
            42,
        ))
        .unwrap();
    let snapshot = vault.snapshot();
    let mut engine = SearchEngine::new(indexes.clone());
    for (cx_id, vector) in &backfilled {
        let stored = vault
            .read_slot_vector_at(snapshot, *cx_id, outcome.slot.slot_id)
            .unwrap()
            .unwrap();
        assert_eq!(&stored, vector);
        indexes
            .insert(outcome.slot.slot_id, *cx_id, stored, snapshot)
            .unwrap();
        engine.put_constellation(vault.get(*cx_id, snapshot).unwrap());
    }

    let query_input = Input::new(Modality::Text, b"alpha kidney trial".to_vec());
    let query_vector = registry.measure(lens_id, &query_input).unwrap();
    let hits = engine
        .search(
            &Query::new("alpha kidney trial")
                .with_vector(query_vector)
                .with_slots(vec![outcome.slot.slot_id])
                .require_stored_provenance(true)
                .explain(true),
        )
        .unwrap();
    assert!(!hits.is_empty());
    assert!(
        hits.iter()
            .all(|hit| hit.provenance_source == ProvenanceSource::Stored)
    );
    let missing_doc_engine = SearchEngine::new(indexes.clone());
    let missing_error = missing_doc_engine
        .search(
            &Query::new("alpha kidney trial")
                .with_vector(registry.measure(lens_id, &query_input).unwrap())
                .with_slots(vec![outcome.slot.slot_id])
                .require_stored_provenance(true),
        )
        .unwrap_err();
    assert_eq!(missing_error.code, "CALYX_SEXTANT_PROVENANCE_MISSING");

    let slot_cf_rows = batch
        .candidates
        .iter()
        .filter(|cx_id| {
            vault
                .read_cf_at(
                    snapshot,
                    ColumnFamily::slot(outcome.slot.slot_id),
                    &slot_key(**cx_id),
                )
                .unwrap()
                .is_some()
        })
        .count();
    let scheduler_bytes = fs::read(&scheduler_path).unwrap();
    let readback = serde_json::json!({
        "source_of_truth": "AsterVault base+slot CF rows, durable backfill scheduler JSON, Registry determinism proof, and Sextant SearchEngine stored-provenance hits",
        "vault_root": vault_dir.display().to_string(),
        "scheduler_path": scheduler_path.display().to_string(),
        "scheduler_sha256": sha256_hex(&scheduler_bytes),
        "lens_id": lens_id.to_string(),
        "determinism_proof": registry.determinism_proof(lens_id),
        "slot_id": outcome.slot.slot_id.get(),
        "queued": outcome.queued,
        "batch_candidates": batch.candidates.len(),
        "backfilled_vectors": backfilled.len(),
        "slot_cf_rows": slot_cf_rows,
        "index_stats": indexes.stats(),
        "top_hit": hits[0].cx_id.to_string(),
        "hit_count": hits.len(),
        "hit_provenance_sources": hits.iter().map(|hit| hit.provenance_source).collect::<Vec<_>>(),
        "stored_provenance_error": missing_error.code,
    });
    fs::write(
        &readback_path,
        serde_json::to_vec_pretty(&readback).unwrap(),
    )
    .unwrap();
    assert_eq!(
        fs::read(&readback_path).unwrap(),
        serde_json::to_vec_pretty(&readback).unwrap()
    );
    println!("REGISTRY_SEXTANT_READBACK={}", readback_path.display());
}

fn sample_docs() -> Vec<(&'static str, &'static str)> {
    vec![
        ("alpha", "alpha kidney trial cohort"),
        ("beta", "beta neural retrieval graph"),
        ("gamma", "gamma civic policy ballot"),
        ("delta", "delta image caption audio"),
    ]
}

fn constellation(cx_id: CxId, name: &str, text: &str, seq: u64) -> calyx_core::Constellation {
    let hash = sha256_bytes(text.as_bytes());
    calyx_core::Constellation {
        cx_id,
        vault_id: vault_id(),
        panel_version: 1,
        created_at: seq,
        input_ref: InputRef {
            hash,
            pointer: Some(format!("issue339://{name}")),
            redacted: false,
        },
        modality: Modality::Text,
        slots: BTreeMap::new(),
        scalars: BTreeMap::new(),
        metadata: BTreeMap::new(),
        anchors: Vec::new(),
        provenance: LedgerRef { seq, hash },
        flags: CxFlags::default(),
    }
}

fn clean_dir(path: &Path) -> PathBuf {
    let _ = fs::remove_dir_all(path);
    fs::create_dir_all(path).unwrap();
    path.to_path_buf()
}

fn fsv_root() -> PathBuf {
    std::env::var("CALYX_FSV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("calyx-issue339-registry-sextant-fsv"))
}

fn sha256_hex(bytes: &[u8]) -> String {
    sha256_bytes(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn sha256_bytes(bytes: &[u8]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher.finalize().into()
}

fn dense_dim(shape: SlotShape) -> u32 {
    match shape {
        SlotShape::Dense(dim) => dim,
        _ => panic!("issue339 FSV expects a dense registry lens"),
    }
}

fn cx(value: u8) -> CxId {
    CxId::from_bytes([value; 16])
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}
