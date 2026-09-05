//! Issue #44 - region/geography vocabulary FSV.
//!
//! Source of truth: a persisted `region_vocab.json` report read back from disk.

use std::collections::BTreeMap;
use std::path::Path;

use calyx_core::{SlotId, SlotVector};
use calyx_poly::Domain;
use calyx_poly::lenses::default_panel;
use calyx_poly::model::MarketSnapshot;
use calyx_poly::region_vocab::{
    ERR_REGION_VOCAB_INVALID_REQUEST, RegionTextRecord, RegionVocabRequest,
    build_region_vocab_report, read_region_vocab_report, region_vocab_for_domain,
    run_region_vocab_report,
};
use serde_json::{Value, json};

// calyx-shared-module: path=fsv_support.rs alias=__calyx_shared_fsv_support_rs local=support visibility=private
use crate::__calyx_shared_fsv_support_rs as support;
use support::{collect_files, hex, named_fsv_root, reset_dir, write_blake3sums, write_json};

#[test]
fn issue044_region_vocab_fsv() {
    let (root, _keep) = named_fsv_root("POLY_ISSUE044_FSV_ROOT", "poly-issue044-region-vocab");
    reset_dir(&root);

    let happy = happy_region_vocab_readback_drives_region_lens(&root);
    let empty = edge_empty_request_fails_closed();
    let blank = edge_blank_record_fails_closed();
    let unknown = edge_unknown_geography_is_rejected_not_fabricated();
    let truncation = edge_max_terms_truncates_by_count_then_name();

    let mut files = Vec::new();
    collect_files(&root, &mut files);
    let readback = json!({
        "issue": 44,
        "proof_claim": "Poly builds per-domain region vocabularies from event/tag text, persists and reads back the vocabulary report, and feeds the resulting domain vocab into the existing region_oh one-hot lens.",
        "minimum_sufficient_corpus": {
            "event_tag_records": 6,
            "domains": 4,
            "geography_types": ["state", "city", "country"],
            "edge_cases": 4,
            "why_this_is_sufficient": "Six records are the smallest corpus used here that proves duplicate-count ranking, multiple domains, state/city/country extraction, multi-region text, and a real region_oh lens measurement.",
            "why_smaller_is_insufficient": "Removing any record drops either duplicate ranking, one geography type, one domain grouping, or the lens-driving Florida vocabulary proof.",
            "why_larger_is_wasteful": "More event rows or market datasets would repeat the same alias matching, counting, sorting, persistence, and one-hot paths without adding proof for #44."
        },
        "happy_path": happy,
        "edge_cases": {
            "empty_request": empty,
            "blank_record": blank,
            "unknown_geography": unknown,
            "max_terms_truncation": truncation
        },
        "physical_files": files
    });
    let readback_path = root.join("readback.json");
    write_json(&readback_path, &readback);
    write_blake3sums(&root);
    println!("ISSUE044_REGION_VOCAB_READBACK={}", readback_path.display());
}

fn happy_region_vocab_readback_drives_region_lens(root: &Path) -> Value {
    let run = run_region_vocab_report(&request(happy_records(), 1, 8), &root.join("happy"))
        .expect("region vocab report");
    let report = read_region_vocab_report(&run.report_path).expect("read region vocab report");
    assert_eq!(report, run.report);

    let politics = region_vocab_for_domain(&report, Domain::Politics);
    assert_eq!(politics, vec!["florida".to_string(), "ohio".to_string()]);
    let weather = region_vocab_for_domain(&report, Domain::Weather);
    assert!(weather.contains(&"new_york_city".to_string()));
    let geopolitics = region_vocab_for_domain(&report, Domain::Geopolitics);
    assert_eq!(
        geopolitics,
        vec!["russia".to_string(), "ukraine".to_string()]
    );
    let sports = region_vocab_for_domain(&report, Domain::Sports);
    assert_eq!(sports, vec!["california".to_string(), "texas".to_string()]);

    let lens_state = region_lens_state(&politics, "florida");
    assert_eq!(lens_state["active_region"], json!("florida"));
    assert_eq!(lens_state["active_index"], json!(0));

    json!({
        "report_path": run.report_path.display().to_string(),
        "input_record_count": report.input_record_count,
        "matched_record_count": report.matched_record_count,
        "domain_vocab": report.domain_vocab,
        "entries": report.entries,
        "region_lens": lens_state
    })
}

fn edge_empty_request_fails_closed() -> Value {
    let err =
        build_region_vocab_report(&request(Vec::new(), 1, 8)).expect_err("empty request must fail");
    assert_eq!(err.code(), ERR_REGION_VOCAB_INVALID_REQUEST);
    json!({"code": err.code(), "message": err.message()})
}

fn edge_blank_record_fails_closed() -> Value {
    let err = build_region_vocab_report(&request(
        vec![RegionTextRecord {
            domain: Domain::Politics,
            event_text: "   ".to_string(),
            tags: vec![" ".to_string()],
        }],
        1,
        8,
    ))
    .expect_err("blank record must fail");
    assert_eq!(err.code(), ERR_REGION_VOCAB_INVALID_REQUEST);
    json!({"code": err.code(), "message": err.message()})
}

fn edge_unknown_geography_is_rejected_not_fabricated() -> Value {
    let report = build_region_vocab_report(&request(
        vec![record(
            Domain::Crypto,
            "Will this token close above its weekly moving average?",
            &["crypto", "technical"],
        )],
        1,
        8,
    ))
    .expect("unknown geography report");
    assert!(report.domain_vocab.is_empty());
    assert_eq!(report.rejected_records.len(), 1);
    json!({
        "domain_vocab": report.domain_vocab,
        "rejected_records": report.rejected_records
    })
}

fn edge_max_terms_truncates_by_count_then_name() -> Value {
    let report =
        build_region_vocab_report(&request(happy_records(), 1, 1)).expect("truncated vocab report");
    let politics = region_vocab_for_domain(&report, Domain::Politics);
    assert_eq!(politics, vec!["florida".to_string()]);
    json!({"politics_vocab": politics, "max_terms_per_domain": report.max_terms_per_domain})
}

fn region_lens_state(vocab: &[String], region: &str) -> Value {
    let panel = default_panel(44, vocab.to_vec());
    let mut snapshot = sample_snapshot();
    snapshot.region = Some(region.to_string());
    let slots: BTreeMap<_, _> = panel.measure_all(&snapshot);
    match slots.get(&SlotId::new(9)).expect("region slot present") {
        SlotVector::Dense { dim, data } => {
            let active_index = data
                .iter()
                .position(|value| (*value - 1.0).abs() <= f32::EPSILON)
                .expect("one active region");
            assert_eq!(data.iter().filter(|value| **value == 1.0).count(), 1);
            json!({
                "slot": 9,
                "dim": dim,
                "vocab": vocab,
                "active_region": vocab[active_index],
                "active_index": active_index,
                "vector_hash": vector_hash(data),
                "vector": data
            })
        }
        other => panic!("region slot should be dense, got {other:?}"),
    }
}

fn vector_hash(data: &[f32]) -> String {
    let mut hasher = blake3::Hasher::new();
    for value in data {
        hasher.update(&value.to_le_bytes());
    }
    hex(hasher.finalize().as_bytes())
}

fn happy_records() -> Vec<RegionTextRecord> {
    vec![
        record(
            Domain::Politics,
            "Will Florida vote Republican in the 2028 presidential election?",
            &["election", "FL"],
        ),
        record(
            Domain::Politics,
            "Florida Senate race margin above five points?",
            &["florida"],
        ),
        record(
            Domain::Politics,
            "Will Ohio certify its Senate result by Friday?",
            &["OH"],
        ),
        record(
            Domain::Weather,
            "Will New York City record measurable snow this week?",
            &["NYC", "weather"],
        ),
        record(
            Domain::Geopolitics,
            "Will Ukraine and Russia agree to a ceasefire before September?",
            &["Ukraine", "Russia"],
        ),
        record(
            Domain::Sports,
            "California vs Texas championship market",
            &["CA", "TX"],
        ),
    ]
}

fn request(
    records: Vec<RegionTextRecord>,
    min_count: usize,
    max_terms_per_domain: usize,
) -> RegionVocabRequest {
    RegionVocabRequest {
        records,
        min_count,
        max_terms_per_domain,
    }
}

fn record(domain: Domain, event_text: &str, tags: &[&str]) -> RegionTextRecord {
    RegionTextRecord {
        domain,
        event_text: event_text.to_string(),
        tags: tags.iter().map(|tag| (*tag).to_string()).collect(),
    }
}

fn sample_snapshot() -> MarketSnapshot {
    MarketSnapshot {
        token_id: "issue44-token".into(),
        condition_id: "issue44-condition".into(),
        outcome_index: 0,
        slug: "issue44-region-market".into(),
        question: Some("Issue 044 region vocabulary market?".into()),
        event_id: Some("issue44-event".into()),
        category: Some("politics".into()),
        region: None,
        tags: vec!["issue44".into()],
        resolution_source: None,
        neg_risk: false,
        snapshot_ts: 1_785_600_044,
        price: Some(0.55),
        mid: Some(0.55),
        best_bid: Some(0.54),
        best_ask: Some(0.56),
        spread: Some(0.02),
        tick_size: Some(0.01),
        volume_24h: Some(1_000.0),
        liquidity: Some(500.0),
        one_hour_change: Some(0.01),
        one_day_change: Some(0.02),
        ofi: Some(0.1),
        yes_no_residual: Some(0.0),
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
