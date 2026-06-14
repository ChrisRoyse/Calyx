//! PH72 T07 integration FSV (#577): stream, reactive, time-travel, and summary.
//!
//! Source of truth: durable Aster CF/WAL/Ledger bytes plus the named
//! `ph72_*.json` artifacts. The assertions below read durable rows back through
//! Aster CF scans or `as_of`, not just returned values.

mod issue577_support;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use calyx_aster::cf::ColumnFamily;
use calyx_aster::stream::{BackpressureGuard, PostIngestHook, StreamIngester, StreamStats};
use calyx_aster::timetravel::{CALYX_TIMETRAVEL_BEFORE_HORIZON, RetentionHorizon, read_all};
use calyx_core::{CalyxError, FixedClock};
use calyx_loom::{ReactiveEngine, ReactiveSignalSet, TriggerCondition};
use proptest::prelude::*;
use serde_json::json;

use issue577_support::*;

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

#[test]
fn ph72_integration_writes_named_fsv_artifacts() {
    let root = fsv_root();
    clean_dir(&root);
    let vault_dir = root.join("test/ph72");
    let (vault, clock) = open_vault(&vault_dir, b"issue577-ph72-main");
    let vault = Arc::new(vault);

    let (stats_first, trigger_id, sub_id, engine) = stream_first_three_with_reactive(&vault);
    let observed = engine.lock().unwrap().observe_delta(sub_id).unwrap();
    assert_eq!(observed.len(), 1, "third recurring event fires once");
    assert_eq!(
        observed[0].cx_id,
        vault.cx_id_for_input(SERIES_RAW, PANEL_VERSION)
    );
    assert_eq!(
        observed[0].ledger_ref.hash,
        ledger_hash(vault.as_ref(), observed[0].ledger_ref.seq)
    );

    let audit = audit_entries(vault.as_ref());
    assert_eq!(
        audit.iter().map(|entry| entry.matched).collect::<Vec<_>>(),
        vec![false, false, true]
    );
    let fired_rows = fired_events(vault.as_ref());
    assert_eq!(fired_rows.len(), 1);

    let stats_rest = stream_range(&vault, 3, 100, 128);
    let stats = combine_stats(&stats_first, &stats_rest);
    assert_eq!(stats.ingested, 100);
    assert_eq!(stats.backpressured, 0);
    assert_eq!(stats.quantized, 200);
    write_json(&root, "ph72_stream_stats.json", stream_stats_json(&stats));

    write_json(
        &root,
        "ph72_trigger_fired.json",
        json!({
            "trigger_id": trigger_id.to_string(),
            "cx_id": observed[0].cx_id.to_string(),
            "condition": "EventRecurs(min_occurrences=3)",
            "ledger_ref": {
                "seq": observed[0].ledger_ref.seq,
                "hash": hex_bytes(&observed[0].ledger_ref.hash),
            },
            "ledger_row_hash": hex_bytes(&ledger_hash(vault.as_ref(), observed[0].ledger_ref.seq)),
            "durable_fired_rows": fired_rows.len(),
        }),
    );
    let matched_sequence = audit.iter().map(|entry| entry.matched).collect::<Vec<_>>();
    let audit_count = audit.len();
    write_json(
        &root,
        "ph72_trigger_audit.json",
        json!({
            "entries": audit,
            "matched_sequence": matched_sequence,
            "audit_count": audit_count,
        }),
    );

    seed_summary_graph(vault.as_ref(), &clock, 100);
    let summary = summarize_latest(vault.as_ref(), 100);
    assert!(summary.kernel_size >= 1);
    assert!(summary.kernel_size <= 100);
    assert!(summary.kernel_only_recall.is_finite());
    assert!(summary.kernel_only_recall > 0.0);
    assert!(
        ledger_payloads(vault.as_ref())
            .iter()
            .any(|payload| payload["marker"] == "SUMMARIZE_INVOKED")
    );
    write_json(&root, "ph72_summarize.json", summary_json(&summary));

    let summary_asof = summarize_asof(vault.as_ref(), 500, 100);
    assert!(summary_asof.kernel_size >= 1);
    assert!(summary_asof.kernel_size <= summary.kernel_size);
    write_json(
        &root,
        "ph72_summarize_asof.json",
        json!({
            "as_of_millis": 500,
            "latest_kernel_size": summary.kernel_size,
            "summary": summary_json(&summary_asof),
        }),
    );
    assert_eq!(
        calyx_lodestar::summarize_vault_as_of(
            vault.as_ref(),
            request(100),
            SUMMARY_HORIZON - 1,
            &mut calyx_lodestar::ScopeCache::new(8),
            &FixedClock::new(7_000),
        )
        .unwrap_err()
        .code,
        CALYX_TIMETRAVEL_BEFORE_HORIZON
    );

    write_backpressure_artifact(&root);
    write_timetravel_artifacts(&root);
    assert_named_artifacts(&root);
    vault.flush().unwrap();
    println!("ISSUE577_FSV_ROOT={}", root.display());
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 6,
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    #[test]
    fn pipeline_property(n_events in 10usize..=200) {
        let root = unique_root("prop");
        let (vault, clock) = open_vault(&root.join("vault"), b"issue577-prop");
        let vault = Arc::new(vault);
        let (_stats_first, _trigger_id, sub_id, engine) = stream_first_three_with_reactive(&vault);
        let rest = stream_range(&vault, 3, n_events, n_events + 8);
        prop_assert_eq!(rest.ingested + 3, n_events);
        prop_assert_eq!(engine.lock().unwrap().observe_delta(sub_id).unwrap().len(), 1);

        seed_summary_graph(vault.as_ref(), &clock, n_events);
        let mid = n_events / 2;
        let snap = vault.as_of((mid as u64) * 10).unwrap();
        let visible = (1..=n_events)
            .filter(|i| snap.read_cf(ColumnFamily::Graph, &graph_node_key(vault.as_ref(), *i)).unwrap().is_some())
            .count();
        prop_assert_eq!(visible, mid);
        let summary = summarize_latest(vault.as_ref(), n_events);
        prop_assert!(summary.kernel_size >= 1);
        prop_assert!(summary.kernel_size <= n_events);
        fs::remove_dir_all(root).ok();
    }
}

#[test]
fn zero_event_pipeline_edge() {
    let root = edge_root("zero");
    let (vault, clock) = open_vault(&root.join("vault"), b"issue577-zero");
    let vault = Arc::new(vault);
    let mut engine = ReactiveEngine::new(Arc::new(FixedClock::new(1)));
    let sub = engine
        .subscribe(
            TriggerCondition::EventRecurs {
                series: vault.cx_id_for_input(SERIES_RAW, PANEL_VERSION),
                min_occurrences: 3,
            },
            None,
        )
        .unwrap();

    let stats = stream_ingester(Arc::clone(&vault), 1)
        .drain_and_close()
        .unwrap();
    assert_eq!(stats.ingested, 0);
    let delta = engine.observe_delta(sub).unwrap();
    assert!(delta.is_empty());
    let as_of_error = vault.as_of(1).unwrap_err();
    assert_eq!(as_of_error.code, "CALYX_TIMETRAVEL_NO_DATA");

    seed_summary_graph(vault.as_ref(), &clock, 0);
    let summary = summarize_latest(vault.as_ref(), 1);
    assert_eq!(summary.kernel_size, 0);
    write_json(
        &root,
        "ph72_zero_edge.json",
        json!({
            "edge": "zero_event_pipeline",
            "stream_stats": stream_stats_json(&stats),
            "reactive_delta_count": delta.len(),
            "as_of_error_code": as_of_error.code,
            "summary_kernel_size": summary.kernel_size,
        }),
    );
    cleanup_edge_root(&root);
}

#[test]
fn corrupt_time_index_is_isolated_from_stream_and_reactive() {
    let root = edge_root("corrupt-time-index");
    let (vault, _clock) = open_vault(&root.join("vault"), b"issue577-corrupt");
    let vault = Arc::new(vault);
    let (_stats_first, _trigger_id, sub_id, engine) = stream_first_three_with_reactive(&vault);
    let rest = stream_range(&vault, 3, 50, 64);
    assert_eq!(rest.ingested + 3, 50);
    assert_eq!(
        engine.lock().unwrap().observe_delta(sub_id).unwrap().len(),
        1
    );

    let before_reactive_rows =
        audit_entries(vault.as_ref()).len() + fired_events(vault.as_ref()).len();
    let before_time_index_rows = read_all(vault.as_ref()).unwrap().len();
    vault
        .write_cf(ColumnFamily::TimeIndex, vec![0; 17], b"bad".to_vec())
        .unwrap();
    let read_all_error = read_all(vault.as_ref()).unwrap_err();
    assert_eq!(read_all_error.code, "CALYX_ASTER_CORRUPT_SHARD");
    let err = vault.as_of(1).unwrap_err();
    assert_eq!(err.code, "CALYX_ASTER_CORRUPT_SHARD");
    let after_reactive_rows =
        audit_entries(vault.as_ref()).len() + fired_events(vault.as_ref()).len();
    assert_eq!(before_reactive_rows, after_reactive_rows);
    let fired_rows = fired_events(vault.as_ref()).len();
    assert_eq!(fired_rows, 1);
    write_json(
        &root,
        "ph72_corrupt_time_index_edge.json",
        json!({
            "edge": "corrupt_time_index_isolated_from_stream_and_reactive",
            "streamed": 50,
            "corrupt_key_len": 17,
            "time_index_rows_before_corruption": before_time_index_rows,
            "read_all_error_code": read_all_error.code,
            "as_of_error_code": err.code,
            "reactive_rows_before": before_reactive_rows,
            "reactive_rows_after": after_reactive_rows,
            "fired_rows_after": fired_rows,
        }),
    );
    cleanup_edge_root(&root);
}

fn stream_first_three_with_reactive(
    vault: &Arc<calyx_aster::vault::AsterVault<StepClock>>,
) -> (
    StreamStats,
    calyx_loom::TriggerId,
    calyx_loom::SubscriptionId,
    Arc<Mutex<ReactiveEngine>>,
) {
    let series = vault.cx_id_for_input(SERIES_RAW, PANEL_VERSION);
    let engine = Arc::new(Mutex::new(ReactiveEngine::new(Arc::new(FixedClock::new(
        1_786_320_000,
    )))));
    let sub_id = engine
        .lock()
        .unwrap()
        .subscribe(
            TriggerCondition::EventRecurs {
                series,
                min_occurrences: 3,
            },
            Some("issue577".to_string()),
        )
        .unwrap();
    let trigger_id = engine
        .lock()
        .unwrap()
        .subscriptions()
        .get(sub_id)
        .unwrap()
        .trigger_id;
    let hook = reactive_hook(Arc::clone(&engine));
    let ingester = StreamIngester::new_with_post_ingest_hook(
        Arc::clone(vault),
        stream_config(),
        BackpressureGuard::new(8, 0),
        Some(hook),
    );
    for index in 0..3 {
        ingester
            .send(stream_input(SERIES_RAW, index), epoch(index))
            .unwrap();
    }
    (
        ingester.drain_and_close().unwrap(),
        trigger_id,
        sub_id,
        engine,
    )
}

fn reactive_hook(engine: Arc<Mutex<ReactiveEngine>>) -> PostIngestHook<StepClock> {
    Arc::new(move |vault, cx_id, ledger_ref| {
        let signals = ReactiveSignalSet::new(vault);
        engine
            .lock()
            .map_err(|_| CalyxError::backpressure("reactive engine lock poisoned"))?
            .evaluate_post_ingest_durable(vault, cx_id, ledger_ref, &signals)
            .map(|_| ())
    })
}

fn stream_range(
    vault: &Arc<calyx_aster::vault::AsterVault<StepClock>>,
    start: usize,
    end: usize,
    capacity: usize,
) -> StreamStats {
    let ingester = stream_ingester(Arc::clone(vault), capacity);
    for index in start..end {
        ingester
            .send(stream_input(SERIES_RAW, index), epoch(index))
            .unwrap();
    }
    ingester.drain_and_close().unwrap()
}

fn write_backpressure_artifact(root: &std::path::Path) {
    let dir = root.join("test/ph72-backpressure");
    let (vault, _clock) = open_vault(&dir, b"issue577-backpressure");
    let vault = Arc::new(vault);
    let ingester = stream_ingester(Arc::clone(&vault), 10);
    for index in 0..10 {
        ingester
            .send(stream_input(b"issue577-backpressure", index), epoch(index))
            .unwrap();
    }
    let err = ingester
        .send(stream_input(b"issue577-backpressure", 10), epoch(10))
        .unwrap_err();
    assert_eq!(err.code, "CALYX_STREAM_BACKPRESSURE");
    let stats = ingester.drain_and_close().unwrap();
    write_json(
        root,
        "ph72_backpressure.json",
        json!({
            "error_code": err.code,
            "event_index": 11,
            "accepted_before_error": stats.ingested,
            "backpressured": stats.backpressured,
        }),
    );
}

fn write_timetravel_artifacts(root: &std::path::Path) {
    let dir = root.join("test/ph72-time");
    let (vault, clock) = open_vault(&dir, b"issue577-time");
    let c1 = put_time_cx(&vault, &clock, b"ph72-c1", 500);
    let c2 = put_time_cx(&vault, &clock, b"ph72-c2", 1000);
    let at_700 = vault.as_of(700).unwrap();
    assert!(at_700.get_cx(c1).is_ok());
    assert!(at_700.get_cx(c2).is_err());
    let at_1000 = vault.as_of(1000).unwrap();
    assert!(at_1000.get_cx(c1).is_ok());
    assert!(at_1000.get_cx(c2).is_ok());

    write_json(
        root,
        "ph72_asof_500.json",
        json!({
            "as_of_millis": 700,
            "contains": [c1.to_string()],
            "excludes": [c2.to_string()],
            "resolved_seqno": at_700.seqno(),
            "time_index": read_all(&vault).unwrap().iter().map(|entry| {
                json!({"millis": entry.millis, "seqno": entry.seqno})
            }).collect::<Vec<_>>(),
        }),
    );
    write_json(
        root,
        "ph72_asof_1000.json",
        json!({
            "as_of_millis": 1000,
            "contains": [c1.to_string(), c2.to_string()],
            "resolved_seqno": at_1000.seqno(),
        }),
    );

    vault
        .set_retention_horizon(RetentionHorizon::absolute(300))
        .unwrap();
    let err = vault.as_of(200).unwrap_err();
    assert_eq!(err.code, CALYX_TIMETRAVEL_BEFORE_HORIZON);
    write_json(
        root,
        "ph72_horizon_error.json",
        json!({
            "error_code": err.code,
            "requested_millis": 200,
            "horizon_millis": 300,
            "message": err.message,
        }),
    );
    vault.flush().unwrap();
}

fn combine_stats(left: &StreamStats, right: &StreamStats) -> StreamStats {
    StreamStats {
        ingested: left.ingested + right.ingested,
        quantized: left.quantized + right.quantized,
        backpressured: left.backpressured + right.backpressured,
        batches: left.batches + right.batches,
    }
}

fn graph_node_key(vault: &calyx_aster::vault::AsterVault<StepClock>, index: usize) -> Vec<u8> {
    calyx_aster::plain_graph::PlainGraph::new(vault, COLLECTION)
        .unwrap()
        .node_key(summary_cx(index))
}

fn unique_root(name: &str) -> std::path::PathBuf {
    let id = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
    let root =
        std::env::temp_dir().join(format!("calyx-issue577-{name}-{}-{id}", std::process::id()));
    clean_dir(&root);
    root
}

fn edge_root(name: &str) -> PathBuf {
    if let Some(base) = std::env::var_os("CALYX_ISSUE577_FSV_ROOT") {
        let root = PathBuf::from(base).join("edge").join(name);
        clean_dir(&root);
        root
    } else {
        unique_root(name)
    }
}

fn cleanup_edge_root(root: &Path) {
    if std::env::var_os("CALYX_ISSUE577_FSV_ROOT").is_none() {
        fs::remove_dir_all(root).ok();
    }
}
