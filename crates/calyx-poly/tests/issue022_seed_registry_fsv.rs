//! Issue #22 - deterministic frozen encoder seed registry FSV.
//!
//! Source of truth: `poly_seed_registry.json`, written and read back from disk.

use std::collections::BTreeMap;
use std::path::Path;

use calyx_core::{SlotId, SlotVector};
use calyx_poly::encode::signed_log;
use calyx_poly::lenses::default_panel;
use calyx_poly::model::MarketSnapshot;
use calyx_poly::seed_registry::{
    ERR_SEED_REGISTRY_COLLISION, ERR_SEED_REGISTRY_INVALID, ERR_SEED_REGISTRY_MISSING,
    REQUIRED_RFF_LENS_KEYS, SEED_REGISTRY_FILE, SeedRegistryArtifact,
    default_seed_registry_artifact, read_seed_registry, run_seed_registry_readback,
    seed_spec_for_lens, validate_seed_registry_artifact,
};
use serde_json::{Value, json};

#[path = "fsv_support.rs"]
mod support;
use support::{collect_files, hex, named_fsv_root, reset_dir, write_blake3sums, write_json};

#[test]
fn issue022_seed_registry_fsv() {
    let (root, _keep) = named_fsv_root("POLY_ISSUE022_FSV_ROOT", "poly-issue022-seeds");
    reset_dir(&root);

    let happy = happy_registry_readback_proves_panel_seeds(&root);
    let duplicate = edge_duplicate_seed_fails_closed(&root);
    let missing = edge_missing_required_lens_fails_closed(&root);
    let malformed = edge_malformed_entry_fails_closed(&root);
    let unknown = edge_unknown_lens_lookup_fails_closed();

    let mut files = Vec::new();
    collect_files(&root, &mut files);
    let readback = json!({
        "issue": 22,
        "proof_claim": "Poly centralizes every seed-bearing production RFF lens spec, writes and reads the registry from disk, rejects collisions/missing/malformed entries, and default_panel emits vectors equal to registry-built encoders.",
        "minimum_sufficient_corpus": {
            "production_seed_bearing_rff_lenses": REQUIRED_RFF_LENS_KEYS.len(),
            "synthetic_bad_registry_artifacts": 3,
            "unknown_lookup_probe": 1,
            "why_this_is_sufficient": "The six production RFF entries are the complete seed-bearing lens set; one duplicate, one missing-required-key, one malformed-entry artifact, and one unknown lookup exercise the #22 fail-closed surface.",
            "why_smaller_is_insufficient": "Dropping any production RFF lens would leave one frozen seed unproven; dropping an edge would leave either collision, required coverage, schema validity, or lookup refusal untested.",
            "why_larger_is_wasteful": "Market-row count is not the claim here. More rows or larger datasets would reuse the same encoder construction and would not add proof beyond the complete registry plus deterministic vector readback."
        },
        "happy_path": happy,
        "edge_cases": {
            "duplicate_seed": duplicate,
            "missing_required_lens": missing,
            "malformed_entry": malformed,
            "unknown_lookup": unknown
        },
        "physical_files": files
    });
    let readback_path = root.join("readback.json");
    write_json(&readback_path, &readback);
    write_blake3sums(&root);
    println!(
        "ISSUE022_SEED_REGISTRY_READBACK={}",
        readback_path.display()
    );
}

fn happy_registry_readback_proves_panel_seeds(root: &Path) -> Value {
    let run = run_seed_registry_readback(&root.join("happy")).expect("registry readback");
    let readback = read_seed_registry(&run.registry_path).expect("read registry");
    assert_eq!(readback, run.registry);
    assert_eq!(run.validation.entry_count, REQUIRED_RFF_LENS_KEYS.len());
    assert_eq!(run.validation.seed_count, REQUIRED_RFF_LENS_KEYS.len());

    let panel = default_panel(1, vec!["us".into()]);
    let slots = panel.measure_all(&sample());
    let mut vectors = Vec::new();
    for (key, slot, input) in rff_inputs() {
        let spec = seed_spec_for_lens(key).expect("seed spec");
        assert_eq!(spec.slot, slot);
        let expected = spec.encoder().encode(input);
        let actual = dense_slot(&slots, slot);
        assert_eq!(actual, expected, "lens {key} must use registry seed");
        assert_eq!(spec.encoder().encode(input), expected, "RFF repeatability");
        vectors.push(json!({
            "lens_key": key,
            "slot": slot,
            "seed": spec.seed,
            "dim": spec.dim,
            "sigma": spec.sigma,
            "input": input,
            "vector_hash": vector_hash(&actual),
            "vector_len": actual.len()
        }));
    }

    json!({
        "registry_path": run.registry_path.display().to_string(),
        "registry_version": run.registry.registry_version.clone(),
        "validation": run.validation.clone(),
        "entry_count": run.registry.entries.len(),
        "deterministic_vectors": vectors
    })
}

fn edge_duplicate_seed_fails_closed(root: &Path) -> Value {
    let mut artifact = default_seed_registry_artifact();
    artifact.entries[1].seed = artifact.entries[0].seed;
    artifact.entries[1].seed_hex = artifact.entries[0].seed_hex.clone();
    persisted_validation_error(
        root,
        "edge-duplicate",
        artifact,
        ERR_SEED_REGISTRY_COLLISION,
    )
}

fn edge_missing_required_lens_fails_closed(root: &Path) -> Value {
    let mut artifact = default_seed_registry_artifact();
    artifact
        .entries
        .retain(|entry| entry.lens_key != "arb_residual");
    persisted_validation_error(root, "edge-missing", artifact, ERR_SEED_REGISTRY_MISSING)
}

fn edge_malformed_entry_fails_closed(root: &Path) -> Value {
    let mut artifact = default_seed_registry_artifact();
    artifact.entries[0].dim = 0;
    persisted_validation_error(root, "edge-malformed", artifact, ERR_SEED_REGISTRY_INVALID)
}

fn edge_unknown_lens_lookup_fails_closed() -> Value {
    let err = seed_spec_for_lens("unregistered_rff").expect_err("unknown lens must fail");
    assert_eq!(err.code(), ERR_SEED_REGISTRY_MISSING);
    json!({"code": err.code(), "message": err.message()})
}

fn persisted_validation_error(
    root: &Path,
    dir_name: &str,
    artifact: SeedRegistryArtifact,
    expected_code: &str,
) -> Value {
    let path = root.join(dir_name).join(SEED_REGISTRY_FILE);
    write_json(
        &path,
        &serde_json::to_value(&artifact).expect("artifact JSON"),
    );
    let readback = read_seed_registry(&path).expect("read bad registry artifact");
    let err = validate_seed_registry_artifact(&readback).expect_err("bad registry must fail");
    assert_eq!(err.code(), expected_code);
    json!({
        "path": path.display().to_string(),
        "code": err.code(),
        "message": err.message()
    })
}

fn rff_inputs() -> [(&'static str, u16, f64); 6] {
    [
        ("price_rff", 0, 0.62),
        ("distance_from_50", 1, 0.12),
        ("spread_rff", 2, signed_log(0.02)),
        ("ofi_vec", 5, 0.20),
        ("momentum_rff", 6, signed_log(-0.03)),
        ("arb_residual", 7, 0.01),
    ]
}

fn dense_slot(slots: &BTreeMap<SlotId, SlotVector>, slot: u16) -> Vec<f32> {
    match slots.get(&SlotId::new(slot)).expect("slot present") {
        SlotVector::Dense { data, .. } => data.clone(),
        other => panic!("slot {slot} should be dense, got {other:?}"),
    }
}

fn vector_hash(data: &[f32]) -> String {
    let mut hasher = blake3::Hasher::new();
    for value in data {
        hasher.update(&value.to_le_bytes());
    }
    hex(hasher.finalize().as_bytes())
}

fn sample() -> MarketSnapshot {
    MarketSnapshot {
        token_id: "issue22-token".into(),
        condition_id: "issue22-condition".into(),
        outcome_index: 0,
        slug: "issue22-market".into(),
        question: Some("Issue 022 seed registry market?".into()),
        event_id: None,
        category: Some("crypto".into()),
        region: Some("us".into()),
        tags: vec![],
        resolution_source: None,
        neg_risk: false,
        snapshot_ts: 1_785_600_022,
        price: Some(0.62),
        mid: Some(0.62),
        best_bid: Some(0.61),
        best_ask: Some(0.63),
        spread: Some(0.02),
        tick_size: Some(0.01),
        volume_24h: Some(125_000.0),
        liquidity: Some(40_000.0),
        one_hour_change: Some(0.01),
        one_day_change: Some(-0.03),
        ofi: Some(0.20),
        yes_no_residual: Some(0.01),
        secs_to_resolution: Some(86_400.0),
        holders: vec![],
        makers: vec![],
        counterparty_volumes: vec![],
        onchain_fills: vec![],
        temporal_reference_ts: None,
        sequence_position: None,
        sequence_total: None,
        oracle_risk: Default::default(),
        book: Default::default(),
    }
}
