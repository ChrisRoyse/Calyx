//! Refs #1967 — frozen contract test for the MEASURED cross-domain embedder
//! roster `law-crossdomain-v3.1` (`tests/lawroster_v3/roster.json`).
//!
//! v3.1 supersedes v3. v3 ranked every lane on the FIRST 1000 CHARACTERS of each
//! opinion; v3.1 re-measures the identical 1,440-opinion sample, the identical
//! anchors and the identical estimator with NO truncation. Two things follow that
//! the truncated numbers could not have shown, and both are frozen here:
//!
//!   * a lane must declare HOW it represents a long document, because on whole
//!     opinions the representation moves a lane's measured bits by more than the
//!     difference between lanes (medembed: 0.253 pooled vs 0.467 chunk-preserving);
//!   * the search PIN is a budget item, not a footnote. The v3 "free" zero-VRAM
//!     sparse lane costs a measured 3,195 MiB of pin on full text.
//!
//! Invariants:
//!
//!   * dtype is float16;
//!   * NO token-level Multi/ColBERT lane (#1491) — the pin scales with total token
//!     count. Chunk-level Multi is a DIFFERENT regime and is allowed, but only
//!     with a measured pin recorded in-file and inside the panel budget;
//!   * every lane declares a `representation` and a `window_policy` — no lane may
//!     leave how it reads a long document implicit;
//!   * every Multi lane carries its measured pin AND the measured advantage that
//!     bought it, and the panel's summed pin is inside `max_pin_mib`;
//!   * the required cross-domain roles are present;
//!   * total NEW resident VRAM (grouped) <= the hard 10240 MiB cap;
//!   * every shipping lane is commercial-license clean;
//!   * every KEPT lane clears the 0.05-bit floor, with NO exemptions — a lane that
//!     cannot be measured cannot be admitted;
//!   * the panel sits under the 0.6 NMI ceiling;
//!   * content-lens floor (>= 10) is reachable.
//!
//! It is a spec-level golden (no GPU, no live catalog): the content-addressed
//! `panel template save` object id + the resident-attested swap are produced on
//! the GPU host and frozen separately, the same way doc 27's bindings were.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::Value;

fn roster() -> Value {
    let path: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("lawroster_v3")
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

fn slot_of(lane: &Value) -> &str {
    lane["slot_key"].as_str().unwrap_or("?")
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
fn no_token_level_multi_lane() {
    // #1491 is about TOKEN-level late interaction, whose pin scales with total
    // token count (~526 GB/lane on this corpus). Chunk-level Multi is ~330x
    // smaller and is a different regime, so the rule is enforced on the
    // representation a lane declares, not on the word "multi".
    let roster = roster();
    for lane in embedder_lanes(&roster) {
        let output = lane["output"].as_str().unwrap_or_default();
        assert_ne!(
            output,
            "colbert",
            "lane {} is a ColBERT lane; forbidden by the pin/RSS budget (#1491)",
            slot_of(lane)
        );
        if output == "multi" {
            let representation = lane["representation"].as_str().unwrap_or_default();
            assert_eq!(
                representation,
                "segments-paragraph",
                "lane {} emits multi vectors but does not declare the chunk-level \
                 paragraph representation; a token-level lane is forbidden (#1491)",
                slot_of(lane)
            );
        }
    }
}

#[test]
fn multi_hash_lane_is_dropped_for_1491() {
    // The v3 top-bits lane (token-law-multi-hash) is a late-interaction-TOKEN
    // lane; it MUST stay in `dropped`, no matter how high its bits.
    let roster = roster();
    let dropped = roster["dropped"].as_array().expect("dropped array");
    let has_multi_hash_drop = dropped.iter().any(|d| {
        d["model_id"]
            .as_str()
            .map(|m| m.contains("multi-hash"))
            .unwrap_or(false)
    });
    assert!(
        has_multi_hash_drop,
        "token-law-multi-hash must be recorded as DROPPED for the #1491 late-interaction \
         pin risk (it topped the bits table but is a token-level MaxSim lane)"
    );
}

#[test]
fn every_lane_declares_a_representation_and_window() {
    // The measurement that motivated v3.1: how a lane reads a long document moves
    // its bits more than the difference between lanes. No lane may leave that
    // implicit, and a declared window must be internally consistent.
    let roster = roster();
    for lane in embedder_lanes(&roster) {
        let slot = slot_of(lane);
        let representation = lane["representation"]
            .as_str()
            .unwrap_or_else(|| panic!("lane {slot} declares no representation"));
        assert!(
            matches!(representation, "pooled-paragraph" | "segments-paragraph"),
            "lane {slot} declares unknown representation {representation:?}"
        );
        let window = &lane["window_policy"];
        let max_input_tokens = window["max_input_tokens"]
            .as_u64()
            .unwrap_or_else(|| panic!("lane {slot} declares no window_policy.max_input_tokens"));
        let special = window["special_tokens"]
            .as_u64()
            .unwrap_or_else(|| panic!("lane {slot} declares no window_policy.special_tokens"));
        let max_ctx = lane["max_ctx"]
            .as_u64()
            .unwrap_or_else(|| panic!("lane {slot} declares no max_ctx"));
        assert_eq!(
            max_input_tokens, max_ctx,
            "lane {slot} packs to {max_input_tokens} tokens but its context is {max_ctx}; \
             per-lane packing means each lane packs to its OWN limit"
        );
        assert!(
            special >= 1 && special < max_input_tokens,
            "lane {slot} declares an impossible special-token count {special}"
        );
        assert_eq!(
            window["overlap_tokens"].as_u64(),
            Some(0),
            "lane {slot} declares segment overlap; paragraph boundaries are already \
             semantic and overlap only inflates the index"
        );
        assert_eq!(
            window["boundary"].as_str(),
            Some("paragraph-cascade-pilcrow-blankline-newline"),
            "lane {slot} does not declare the frozen boundary cascade"
        );
        let expected_pool = if representation == "pooled-paragraph" {
            "mean-token-weighted-l2"
        } else {
            "preserve-segments"
        };
        assert_eq!(
            window["pool"].as_str(),
            Some(expected_pool),
            "lane {slot} declares representation {representation} but pool {:?}",
            window["pool"]
        );
        let expected_output = if representation == "pooled-paragraph" {
            "dense"
        } else {
            "multi"
        };
        assert_eq!(
            lane["output"].as_str(),
            Some(expected_output),
            "lane {slot} declares representation {representation} but output {:?}",
            lane["output"]
        );
    }
}

#[test]
fn every_multi_lane_carries_a_measured_pin_under_budget() {
    // A chunk-Multi slot is bought, not assumed: it must record the measured
    // advantage that justified it and the measured pin it costs, and the panel's
    // summed pin must fit the budget.
    let roster = roster();
    let budget = roster["max_pin_mib"].as_u64().expect("max_pin_mib");
    let mut total = 0u64;
    let mut multi_lanes = 0usize;
    for lane in embedder_lanes(&roster) {
        let slot = slot_of(lane);
        let pin = lane["measured_pin_mib"]
            .as_u64()
            .unwrap_or_else(|| panic!("lane {slot} records no measured_pin_mib"));
        assert!(pin > 0, "lane {slot} records a zero measured pin");
        total += pin;
        if lane["output"].as_str() == Some("multi") {
            multi_lanes += 1;
            let advantage = lane["chunk_advantage_bits"].as_f64().unwrap_or_else(|| {
                panic!("multi lane {slot} records no measured chunk_advantage_bits")
            });
            let pooled = lane["pooled_mean_bits"]
                .as_f64()
                .unwrap_or_else(|| panic!("multi lane {slot} records no pooled_mean_bits"));
            let chunk = lane["measured_mean_bits"]
                .as_f64()
                .unwrap_or_else(|| panic!("multi lane {slot} records no measured_mean_bits"));
            assert!(
                advantage > 0.0,
                "multi lane {slot} was awarded a chunk slot without a positive measured \
                 advantage ({advantage})"
            );
            assert!(
                (chunk - pooled - advantage).abs() < 5e-3,
                "multi lane {slot} advantage {advantage} does not reconcile with \
                 chunk {chunk} - pooled {pooled}"
            );
        }
    }
    let awarded = roster["representation"]["chunk_multi_slots_awarded"]
        .as_u64()
        .expect("chunk_multi_slots_awarded");
    assert_eq!(
        multi_lanes as u64, awarded,
        "roster awards {awarded} chunk-Multi slots but {multi_lanes} lanes emit multi"
    );
    assert!(
        awarded <= 2,
        "full-panel chunk-Multi is inadmissible (15.3 GB measured against an 8.6 GB \
         budget); at most 2 slots may be awarded, got {awarded}"
    );
    assert!(
        total <= budget,
        "summed measured pin {total} MiB exceeds the panel pin budget {budget} MiB"
    );
    let recorded = roster["measured_pin_mib"]
        .as_u64()
        .expect("measured_pin_mib");
    assert_eq!(
        total, recorded,
        "summed lane pin {total} != recorded measured_pin_mib {recorded}"
    );
}

#[test]
fn required_cross_domain_roles_present() {
    let roster = roster();
    let roles: Vec<&str> = embedder_lanes(&roster).iter().map(role_of).collect();
    for required in [
        "general-semantic",
        "legal",
        "medical",
        "finance",
        "scientific",
    ] {
        assert!(
            roles.contains(&required),
            "measured cross-domain roster is missing the {required} lane; have {roles:?}"
        );
    }
    // The lexical-sparse role was carried by v3 and is NOT carried by v3.1: both
    // sparse lanes were dropped on measured grounds. That is a real loss, so the
    // roster must say so rather than let the role quietly disappear.
    assert!(
        !roles.contains(&"lexical-sparse"),
        "roster claims a lexical-sparse lane; v3.1 dropped both sparse lanes"
    );
    let dropped = roster["dropped"].as_array().expect("dropped array");
    assert!(
        dropped
            .iter()
            .any(|d| d["role"].as_str() == Some("structural-lexical")
                && d["measured_pin_mib"].as_u64().is_some()),
        "the dropped sparse-law-keywords lane must record the measured pin that dropped it"
    );
}

#[test]
fn new_resident_vram_within_hard_cap() {
    // Budgets NEW law-owned resident VRAM (reused endpoints are ~0). Grouped by
    // shared weight set, the sum must be <= 10240 MiB.
    let roster = roster();
    let cap = roster["max_new_resident_vram_mib"]
        .as_u64()
        .expect("max_new_resident_vram_mib");
    assert_eq!(cap, 10240, "owner hard cap is 10 GB (10240 MiB)");

    let mut per_group: BTreeMap<String, u64> = BTreeMap::new();
    for lane in embedder_lanes(&roster) {
        let group = lane["vram_group"]
            .as_str()
            .expect("lane vram_group")
            .to_owned();
        let mib = lane["marginal_vram_mib"]
            .as_u64()
            .expect("marginal_vram_mib");
        let slot = per_group.entry(group).or_default();
        *slot = (*slot).max(mib); // shared group counted once, by max (grouped rule)
    }
    let total: u64 = per_group.values().sum();
    assert!(
        total <= cap,
        "total grouped NEW resident VRAM {total} MiB exceeds hard cap {cap} MiB; \
         per-group={per_group:?}"
    );
    let measured = roster["measured_new_resident_vram_mib"]
        .as_u64()
        .expect("measured_new_resident_vram_mib");
    assert_eq!(
        total, measured,
        "summed marginal VRAM {total} != recorded measured_new_resident_vram_mib {measured}"
    );
}

#[test]
fn every_kept_lane_is_commercial_license_clean() {
    let roster = roster();
    for lane in embedder_lanes(&roster) {
        let ok = lane["commercial_ok"].as_bool().unwrap_or(false);
        assert!(
            ok,
            "lane {} ({}) is not commercial-license clean; no CC-BY-NC model may ship",
            slot_of(lane),
            lane["model_id"].as_str().unwrap_or("?")
        );
    }
}

#[test]
fn every_kept_lane_has_a_provable_model_identity() {
    // v3 shipped a lane served from an anonymous local directory whose /info
    // reported no model_sha, so what was physically running could not be proved.
    // Every lane now names the exact upstream revision it serves.
    let roster = roster();
    for lane in embedder_lanes(&roster) {
        let slot = slot_of(lane);
        let sha = lane["model_sha"]
            .as_str()
            .unwrap_or_else(|| panic!("lane {slot} records no model_sha"));
        assert_eq!(
            sha.len(),
            40,
            "lane {slot} model_sha {sha:?} is not a 40-hex revision"
        );
        assert!(
            sha.chars().all(|c| c.is_ascii_hexdigit()),
            "lane {slot} model_sha {sha:?} is not hexadecimal"
        );
    }
}

#[test]
fn every_kept_lane_clears_the_calyx_bit_floor() {
    // Admission follows the Calyx differentiation contract: a lane is kept iff its
    // measured marginal signal >= the bit floor. v3.1 removes v3's exemptions —
    // the two exempted lanes are exactly the two that turned out to be
    // unmeasurable or unaffordable, so an exemption was hiding a real decision.
    let roster = roster();
    let bit_floor = roster["assay"]["bit_floor"].as_f64().expect("bit_floor");
    assert!(
        (bit_floor - 0.05).abs() < 1e-9,
        "Calyx bit floor is ~0.05 bits; got {bit_floor}"
    );
    for lane in embedder_lanes(&roster) {
        let slot = slot_of(lane);
        let marg = lane["measured_marginal_bits"].as_f64();
        assert!(
            marg.map(|m| m >= bit_floor).unwrap_or(false),
            "lane {slot} is kept but its measured_marginal_bits {marg:?} is below the \
             Calyx bit floor {bit_floor}"
        );
    }
}

#[test]
fn panel_redundancy_under_ceiling() {
    let roster = roster();
    let max_nmi = roster["assay"]["max_pairwise_nmi_observed"]
        .as_f64()
        .expect("max_pairwise_nmi_observed");
    let ceiling = roster["assay"]["redundancy_ceiling"]
        .as_f64()
        .expect("redundancy_ceiling");
    assert!(
        max_nmi <= ceiling,
        "max pairwise NMI {max_nmi} exceeds the redundancy ceiling {ceiling}"
    );
}

#[test]
fn scincl_kept_as_measured_positive_lane() {
    // SciNCL is ADMITTED on measurement (it GAINED 0.049 bits on whole opinions),
    // alongside the reused specter2 scientific lane.
    let roster = roster();
    let kept: Vec<&str> = embedder_lanes(&roster)
        .iter()
        .filter_map(|l| l["model_id"].as_str())
        .collect();
    assert!(
        kept.iter().any(|m| m.contains("scincl")),
        "SciNCL must be KEPT (measured-positive)"
    );
    assert!(
        kept.iter().any(|m| m.contains("specter2")),
        "specter2 (reused scientific lane) must also be kept"
    );
}

#[test]
fn full_text_reassay_is_recorded_against_the_truncated_one() {
    // The comparison IS the finding: ranking lanes on opinion openings changed
    // which lanes win. Every lane measured under both must carry both numbers so
    // the distortion stays auditable rather than being replaced silently.
    let roster = roster();
    let finding = &roster["truncation_finding"];
    assert!(
        finding["answer"].as_str().is_some(),
        "the truncated-vs-full-text comparison must be recorded in-file"
    );
    let mut compared = 0usize;
    for lane in embedder_lanes(&roster) {
        if lane["v3_truncated_mean_bits"].as_f64().is_some() {
            assert!(
                lane["measured_mean_bits"].as_f64().is_some(),
                "lane {} records a truncated score but no full-text score",
                slot_of(lane)
            );
            compared += 1;
        }
    }
    assert!(
        compared >= 14,
        "only {compared} lanes carry both the truncated and the full-text measurement"
    );
}

#[test]
fn content_lens_floor_is_reachable() {
    let roster = roster();
    let embedders = embedder_lanes(&roster).len();
    let floor = roster["min_content_lenses"].as_u64().unwrap_or(10) as usize;
    assert!(
        embedders >= floor,
        "kept roster has only {embedders} content lanes; below the >={floor} floor"
    );
}
