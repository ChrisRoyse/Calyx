//! FSV driver for issues #633 and #1382: windowed multi-primary temporal
//! recall, including pre-window filter and guard drops.
//!
//! Builds the hand-known two-slot fixture (slot 8 = seeds 1..=5, slot 9 =
//! seeds 11..=15; only seed 15 in-window at fused position 10) in a durable
//! Aster vault. Search is rebuilt from a reopened vault, and plan/truth/report
//! evidence is written to Graph CF and verified after another reopen. JSON
//! files in `<out-dir>` are diagnostic copies only.
//!
//! ```text
//! cargo run -p calyx-sextant --example temporal_window_recall_fsv_driver -- <out-dir>
//! ```

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use calyx_core::{DecayFunction, PeriodicOptions};
use calyx_sextant::{
    MetadataPredicate, Query, QueryFilters, QueryGuard, TemporalFixedClock, TemporalPolicy,
    TimeWindow, WindowRecallPolicy, temporal_search, temporal_search_with_recall,
};
use calyx_ward::{GuardPolicy, GuardProfile, NoveltyAction};
use serde_json::json;

mod temporal_window_recall_fsv_support;
use temporal_window_recall_fsv_support::{
    GUARD_SLOT, IN_WINDOW_SEED, QUERY_TIME, SLOT_A, SLOT_B, authoritative_evidence, cx, dense,
    durable_fixture_engine, persist_and_reopen_graph_evidence,
};

fn main() {
    let out_dir = std::env::args()
        .nth(1)
        .expect("usage: temporal_window_recall_fsv_driver <out-dir>");
    let out_dir = Path::new(&out_dir);
    fs::create_dir_all(out_dir).expect("create out dir");

    let vault_dir = out_dir.join("vault");
    let (engine, physical_fixture) = durable_fixture_engine(&vault_dir);
    let clock = TemporalFixedClock::new(QUERY_TIME);
    let window = TimeWindow::last_hours(1, &clock).expect("window");
    let policy = policy();

    let exhaustive = temporal_search(
        &engine,
        &query(2, None, Some(64)),
        Some(window),
        &policy,
        &clock,
        0,
    )
    .expect("exhaustive windowed search");
    write_json(out_dir, "exhaustive.json", &json!(exhaustive));

    let bounded = temporal_search_with_recall(
        &engine,
        &query(1, Some(2), Some(64)),
        Some(window),
        &policy,
        &clock,
        0,
        WindowRecallPolicy::Bounded { max_candidates: 10 },
    )
    .expect("bounded windowed search");
    write_json(out_dir, "bounded-deepen.json", &json!(bounded));

    let filter_deepen = temporal_search_with_recall(
        &engine,
        &filtered_query("/15"),
        Some(window),
        &policy,
        &clock,
        0,
        WindowRecallPolicy::Bounded { max_candidates: 10 },
    )
    .expect("filter drops deepen to seed 15");
    write_json(out_dir, "filter-deepen.json", &json!(&filter_deepen));

    let guard_deepen = temporal_search_with_recall(
        &engine,
        &guarded_query(),
        Some(window),
        &policy,
        &clock,
        0,
        WindowRecallPolicy::Bounded { max_candidates: 10 },
    )
    .expect("guard drops deepen to seed 15");
    write_json(out_dir, "guard-deepen.json", &json!(&guard_deepen));

    let true_exhaustion = temporal_search_with_recall(
        &engine,
        &filtered_query("/not-present"),
        Some(window),
        &policy,
        &clock,
        0,
        WindowRecallPolicy::Bounded { max_candidates: 10 },
    )
    .expect("full fused corpus proves filter exhaustion");
    write_json(
        out_dir,
        "filter-true-exhaustion.json",
        &json!(&true_exhaustion),
    );

    let exhausted = temporal_search_with_recall(
        &engine,
        &query(1, Some(2), Some(64)),
        Some(window),
        &policy,
        &clock,
        0,
        WindowRecallPolicy::Bounded { max_candidates: 4 },
    )
    .expect_err("budget 4 cannot prove completeness");
    write_json(
        out_dir,
        "bounded-exhausted-error.json",
        &json!({
            "code": exhausted.code,
            "message": exhausted.message,
            "remediation": exhausted.remediation,
        }),
    );

    let filter_cap = temporal_search_with_recall(
        &engine,
        &filtered_query("/15"),
        Some(window),
        &policy,
        &clock,
        0,
        WindowRecallPolicy::Bounded { max_candidates: 4 },
    )
    .expect_err("filter cap 4 cannot reach seed 15");
    write_json(
        out_dir,
        "filter-cap-error.json",
        &json!({
            "code": filter_cap.code,
            "message": filter_cap.message.as_str(),
            "remediation": filter_cap.remediation,
        }),
    );

    let ef_raised = temporal_search(
        &engine,
        &query(2, None, Some(2)),
        Some(window),
        &policy,
        &clock,
        0,
    )
    .expect("windowed search with small caller ef");
    write_json(out_dir, "ef-raised.json", &json!(ef_raised));

    let windowless = temporal_search(
        &engine,
        &query(2, Some(3), Some(64)),
        None,
        &policy,
        &clock,
        0,
    )
    .expect("windowless search");
    write_json(out_dir, "windowless-bounded.json", &json!(windowless));

    assert_eq!(filter_deepen.hits[0].cx_id, cx(IN_WINDOW_SEED));
    assert_eq!(guard_deepen.hits[0].cx_id, cx(IN_WINDOW_SEED));
    assert_eq!(filter_deepen.window_recall.rounds, 3);
    assert_eq!(guard_deepen.window_recall.rounds, 3);
    assert_eq!(filter_deepen.window_recall.candidates_fetched, 10);
    assert_eq!(guard_deepen.window_recall.candidates_fetched, 10);
    assert!(true_exhaustion.hits.is_empty());
    assert_eq!(true_exhaustion.window_recall.candidates_fetched, 10);
    assert!(true_exhaustion.window_recall.corpus_exhausted);
    assert_eq!(filter_cap.code, "CALYX_TEMPORAL_WINDOW_BUDGET_EXHAUSTED");
    assert!(filter_cap.message.contains("fetched 4"));

    let evidence = authoritative_evidence(
        physical_fixture,
        &filter_deepen,
        &guard_deepen,
        &true_exhaustion,
        &filter_cap,
    );
    let graph_readback = persist_and_reopen_graph_evidence(&vault_dir, evidence);
    write_json(out_dir, "aster-graph-readback.json", &graph_readback);

    println!(
        "FSV_DRIVER_DONE out_dir={} vault={} graph_rows={} in_window_seed={IN_WINDOW_SEED}",
        out_dir.display(),
        vault_dir.display(),
        graph_readback.as_array().map_or(0, Vec::len)
    );
}

fn query(k: usize, recall_k: Option<usize>, ef: Option<usize>) -> Query {
    Query {
        k,
        recall_k,
        ef,
        ..Query::new("window recall fsv")
            .with_vector(dense(vec![1.0, 0.0]))
            .with_slots(vec![SLOT_A, SLOT_B])
    }
}

fn filtered_query(pointer_fragment: &str) -> Query {
    query(1, Some(2), Some(64)).with_filters(QueryFilters {
        metadata: vec![MetadataPredicate::InputPointerContains(
            pointer_fragment.to_string(),
        )],
        ..QueryFilters::default()
    })
}

fn guarded_query() -> Query {
    query(1, Some(2), Some(64)).with_guard(QueryGuard::InRegionOnly(guard_profile()))
}

fn policy() -> TemporalPolicy {
    TemporalPolicy::new(
        true,
        DecayFunction::Step,
        PeriodicOptions::new(None, None).expect("periodic"),
        Default::default(),
        Default::default(),
        Default::default(),
        true,
    )
    .expect("policy")
}

fn guard_profile() -> GuardProfile {
    GuardProfile {
        guard_id: "018f48a4-9a79-74d2-8a5c-9ad7f6b8c101"
            .parse()
            .expect("guard id"),
        panel_version: 1,
        domain: "issue-1382-window-recall".to_string(),
        tau: BTreeMap::from([(GUARD_SLOT, 0.70)]),
        required_slots: vec![GUARD_SLOT],
        policy: GuardPolicy::AllRequired,
        calibration: None,
        novelty_action: NoveltyAction::Quarantine,
    }
}

fn write_json(out_dir: &Path, name: &str, value: &serde_json::Value) {
    let path = out_dir.join(name);
    fs::write(&path, serde_json::to_vec_pretty(value).expect("json")).expect("write readback");
    println!("FSV_READBACK={}", path.display());
}
