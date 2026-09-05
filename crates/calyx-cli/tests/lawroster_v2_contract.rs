//! Refs #1967 — frozen contract test for the cross-domain embedder roster
//! `law-crossdomain-v2` (`tests/lawroster_v2/roster.json`).
//!
//! This encodes the owner directive and the hard budgets as enforceable
//! invariants over the committed roster spec, so the roster cannot silently
//! regress before the one-shot 109k-corpus build (#1965 Phase B):
//!
//!   * exactly ONE general-semantic lane (redundant semantics collapsed);
//!   * one each legal / medical / finance domain lane present (+ optional
//!     scientific);
//!   * NO token-level Multi/ColBERT lane (search-pin + build-RSS budget, #1491);
//!   * total resident embedder VRAM (grouped by shared weight set) <= the hard
//!     10240 MiB cap;
//!   * every lane commercial-license clean (no CC-BY-NC shipping model);
//!   * dtype is float16.
//!
//! It is a spec-level golden (no GPU, no live catalog): the content-addressed
//! `panel template save` object id is produced on the GPU host in a GPU window and
//! frozen separately, the same way doc 27's bindings were.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::Value;

fn roster() -> Value {
    let path: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("lawroster_v2")
        .join("roster.json");
    let bytes =
        std::fs::read(&path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    serde_json::from_slice(&bytes)
        .unwrap_or_else(|error| panic!("roster.json is not valid JSON: {error}"))
}

fn embedder_lanes(roster: &Value) -> &Vec<Value> {
    roster["embedder_lanes"]
        .as_array()
        .expect("embedder_lanes array")
}

fn role_of(lane: &Value) -> &str {
    lane["role"].as_str().expect("lane role")
}

#[test]
fn dtype_is_fp16() {
    let roster = roster();
    assert_eq!(
        roster["dtype"].as_str(),
        Some("float16"),
        "roster must serve fp16 (bf16 unsupported until TEI v1.10; fp8 N/A for BERT; \
         int8 dynamic-quant falls back to CPU on the CUDA EP)"
    );
}

#[test]
fn exactly_one_general_semantic_lane() {
    let roster = roster();
    let count = embedder_lanes(&roster)
        .iter()
        .filter(|lane| role_of(lane) == "general-semantic")
        .count();
    assert_eq!(
        count, 1,
        "owner directive: collapse general-semantic embedders to exactly ONE (got {count})"
    );
}

#[test]
fn required_domain_lanes_present() {
    let roster = roster();
    let roles: Vec<&str> = embedder_lanes(&roster).iter().map(role_of).collect();
    for required in ["legal", "medical", "finance"] {
        assert!(
            roles.contains(&required),
            "cross-domain roster is missing the {required} lane; have {roles:?}"
        );
    }
}

#[test]
fn no_token_level_multi_lane() {
    // #1491: the bge-m3 ColBERT (multi_maxsim_segments) lane OOM-killed the index
    // build at scale (59.1 GiB RSS) and blows the 8.6 GB search-pin budget.
    let roster = roster();
    for lane in embedder_lanes(&roster) {
        let output = lane["output"].as_str().unwrap_or_default();
        assert_ne!(
            output,
            "multi",
            "lane {} is a token-level Multi lane; forbidden by the pin/RSS budget (#1491)",
            lane["slot_key"].as_str().unwrap_or("?")
        );
        assert_ne!(
            output,
            "colbert",
            "lane {} is a ColBERT lane; forbidden by the pin/RSS budget (#1491)",
            lane["slot_key"].as_str().unwrap_or("?")
        );
    }
}

#[test]
fn resident_vram_within_hard_cap() {
    // Grouped by shared weight set (bge-m3 dense+sparse count once) exactly as
    // crates/calyx-cli/src/panel_commands/warm/resident_vram.rs sums it.
    let roster = roster();
    let cap = roster["max_resident_vram_mib"]
        .as_u64()
        .expect("max_resident_vram_mib");
    assert_eq!(cap, 10240, "owner hard cap is 10 GB (10240 MiB)");

    let mut per_group: BTreeMap<String, u64> = BTreeMap::new();
    for lane in embedder_lanes(&roster) {
        let group = lane["vram_group"]
            .as_str()
            .expect("lane vram_group")
            .to_owned();
        let mib = lane["measured_fp16_vram_mib"]
            .as_u64()
            .expect("measured_fp16_vram_mib");
        let slot = per_group.entry(group).or_default();
        *slot = (*slot).max(mib); // shared group counted once, by max (grouped rule)
    }
    let total: u64 = per_group.values().sum();
    assert!(
        total <= cap,
        "total grouped resident embedder VRAM {total} MiB exceeds hard cap {cap} MiB; \
         per-group={per_group:?}"
    );
}

#[test]
fn every_lane_is_commercial_license_clean() {
    let roster = roster();
    for lane in embedder_lanes(&roster) {
        let ok = lane["commercial_ok"].as_bool().unwrap_or(false);
        assert!(
            ok,
            "lane {} ({}) is not commercial-license clean; no CC-BY-NC model may ship",
            lane["slot_key"].as_str().unwrap_or("?"),
            lane["model_id"].as_str().unwrap_or("?")
        );
    }
}

#[test]
fn content_lens_floor_is_reachable() {
    // MIN_CONTENT_LENSES = 10. Embedder lanes + zero-VRAM structural lanes must
    // clear the floor with only a handful of VRAM-bearing model groups.
    let roster = roster();
    let embedders = embedder_lanes(&roster).len();
    let structural = roster["zero_vram_lanes"]
        .as_array()
        .map(Vec::len)
        .unwrap_or(0);
    assert!(
        embedders + structural >= 9,
        "roster + structural lanes ({}) leaves no margin to reach the >=10 content-lens \
         floor once courtlistener structured bindings are added",
        embedders + structural
    );
}
