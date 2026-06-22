use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use calyx_core::{CxId, Result, SlotId, SlotShape, SlotVector};
use calyx_sextant::{
    IndexSearchHit, IndexStats, Query, QueryAdmissionConfig, SearchEngine, SextantIndex,
    SlotIndexMap,
};
use serde_json::json;

#[test]
fn concurrent_search_deadline_rejects_and_records_metrics() {
    let gate = Arc::new(QueryGate::new());
    let engine = Arc::new(blocking_engine(Arc::clone(&gate), 1, 1, 25));
    let first = spawn_blocking_search(Arc::clone(&engine));
    gate.wait_for_entered(1);

    let before_reject = engine.query_admission_stats();
    let rejected = engine
        .search(&query())
        .expect_err("second query deadline rejects");
    let after_reject = engine.query_admission_stats();
    gate.release_all();
    let first_hits = first.join().unwrap();
    let final_stats = engine.query_admission_stats();

    assert_eq!(first_hits[0].cx_id, hit_id());
    assert_eq!(rejected.code, "CALYX_BACKPRESSURE");
    assert_eq!(before_reject.in_flight, 1);
    assert_eq!(after_reject.deadline_rejected_total, 1);
    assert_eq!(after_reject.rejected_total, 1);
    assert_eq!(after_reject.max_observed_queued, 1);
    assert_eq!(final_stats.in_flight, 0);
    assert!(
        engine
            .query_admission_metrics_text()
            .contains("calyx_query_admission_rejected_total 1")
    );
}

#[test]
fn queue_cap_zero_rejects_immediately_without_queue_growth() {
    let gate = Arc::new(QueryGate::new());
    let engine = Arc::new(blocking_engine(Arc::clone(&gate), 1, 0, 1_000));
    let first = spawn_blocking_search(Arc::clone(&engine));
    gate.wait_for_entered(1);

    let rejected = engine.search(&query()).expect_err("queue cap rejects");
    let stats = engine.query_admission_stats();
    gate.release_all();
    first.join().unwrap();

    assert_eq!(rejected.code, "CALYX_BACKPRESSURE");
    assert_eq!(stats.queue_full_rejected_total, 1);
    assert_eq!(stats.queued_total, 0);
    assert_eq!(stats.max_observed_queued, 0);
}

#[test]
#[ignore = "manual FSV writes PH56 query-admission source-of-truth artifacts"]
fn query_admission_manual_fsv() {
    let root = fsv_root();
    fs::create_dir_all(&root).unwrap();
    let rss_before_kib = rss_kib();
    let happy = fsv_happy_admit_release();
    let deadline = fsv_deadline_reject();
    let queue_full = fsv_queue_full_reject();
    let zero_capacity = fsv_zero_capacity_reject();
    let rss_final_kib = rss_kib();
    let rss_delta_kib = rss_final_kib.saturating_sub(rss_before_kib);
    let metrics_after_reject = deadline["metrics_after_reject"].as_str().unwrap();

    let readback = json!({
        "source_of_truth": {
            "stats_file": root.join("query-admission-readback.json"),
            "metrics_file": root.join("query-admission-metrics.prom"),
        },
        "trigger": "real SearchEngine::search calls through configured query admission limits",
        "cases": [happy, deadline, queue_full, zero_capacity],
        "rss": {
            "rss_before_kib": rss_before_kib,
            "rss_final_kib": rss_final_kib,
            "rss_delta_kib": rss_delta_kib,
            "rss_bounded_for_synthetic_probe": rss_delta_kib < 16 * 1024,
        },
    });

    fs::write(
        root.join("query-admission-readback.json"),
        serde_json::to_vec_pretty(&readback).unwrap(),
    )
    .unwrap();
    fs::write(
        root.join("query-admission-metrics.prom"),
        metrics_after_reject,
    )
    .unwrap();
    println!(
        "QUERY_ADMISSION_FSV_READBACK={}",
        root.join("query-admission-readback.json").display()
    );
    println!("{}", serde_json::to_string_pretty(&readback).unwrap());

    assert!(readback["cases"].as_array().unwrap().iter().all(|case| {
        case["pass"]
            .as_object()
            .unwrap()
            .values()
            .all(|value| value == true)
    }));
    assert!(rss_delta_kib < 16 * 1024);
}

fn fsv_happy_admit_release() -> serde_json::Value {
    let gate = Arc::new(QueryGate::new());
    gate.release_all();
    let engine = blocking_engine(gate, 1, 1, 25);
    let stats_before = engine.query_admission_stats();
    let hits = engine.search(&query()).unwrap();
    let stats_after = engine.query_admission_stats();
    json!({
        "name": "happy_admit_release",
        "input": "single query, max_concurrent=1, max_queued=1",
        "expected": {
            "hit": hit_id().to_string(),
            "admitted_total_delta": 1,
            "final_in_flight": 0,
            "rejected_total": 0,
        },
        "before": stats_before,
        "after": stats_after,
        "actual": {
            "hit": hits[0].cx_id.to_string(),
        },
        "pass": {
            "hit_matches": hits[0].cx_id == hit_id(),
            "admitted_once": stats_after.admitted_total == stats_before.admitted_total + 1,
            "in_flight_released": stats_after.in_flight == 0,
            "no_rejects": stats_after.rejected_total == 0,
        },
    })
}

fn fsv_deadline_reject() -> serde_json::Value {
    let gate = Arc::new(QueryGate::new());
    let engine = Arc::new(blocking_engine(Arc::clone(&gate), 1, 1, 25));
    let stats_before = engine.query_admission_stats();
    let first = spawn_blocking_search(Arc::clone(&engine));
    gate.wait_for_entered(1);
    let stats_saturated = engine.query_admission_stats();
    let rejected = engine
        .search(&query())
        .expect_err("deadline rejects saturated query");
    let stats_after_reject = engine.query_admission_stats();
    let metrics_after_reject = engine.query_admission_metrics_text();
    gate.release_all();
    let first_hits = first.join().unwrap();
    let stats_final = engine.query_admission_stats();
    json!({
        "name": "deadline_reject",
        "input": "two concurrent queries, first blocks, second waits past 25ms",
        "expected": {
            "first_hit": hit_id().to_string(),
            "second_error_code": "CALYX_BACKPRESSURE",
            "max_observed_in_flight": 1,
            "max_observed_queued": 1,
            "deadline_rejected_total": 1,
            "queue_after_reject": 0,
        },
        "before": stats_before,
        "saturated": stats_saturated,
        "after_reject": stats_after_reject,
        "after_release": stats_final,
        "actual": {
            "first_hit": first_hits[0].cx_id.to_string(),
            "second_error_code": rejected.code,
        },
        "metrics_after_reject": metrics_after_reject,
        "pass": {
            "first_hit_matches": first_hits[0].cx_id == hit_id(),
            "backpressure_code_matches": rejected.code == "CALYX_BACKPRESSURE",
            "reject_metric_rose": stats_after_reject.rejected_total == 1,
            "deadline_metric_rose": stats_after_reject.deadline_rejected_total == 1,
            "queue_did_not_pile_up": stats_after_reject.queued == 0
                && stats_after_reject.max_observed_queued == 1,
            "in_flight_released": stats_final.in_flight == 0,
        },
    })
}

fn fsv_queue_full_reject() -> serde_json::Value {
    let gate = Arc::new(QueryGate::new());
    let engine = Arc::new(blocking_engine(Arc::clone(&gate), 1, 0, 1_000));
    let stats_before = engine.query_admission_stats();
    let first = spawn_blocking_search(Arc::clone(&engine));
    gate.wait_for_entered(1);
    let stats_saturated = engine.query_admission_stats();
    let rejected = engine
        .search(&query())
        .expect_err("queue cap rejects saturated query");
    let stats_after_reject = engine.query_admission_stats();
    gate.release_all();
    first.join().unwrap();
    let stats_final = engine.query_admission_stats();
    json!({
        "name": "queue_full_reject",
        "input": "max_concurrent=1, max_queued=0, second query while first is blocked",
        "expected": {
            "second_error_code": "CALYX_BACKPRESSURE",
            "queue_full_rejected_total": 1,
            "queued_total": 0,
            "max_observed_queued": 0,
        },
        "before": stats_before,
        "saturated": stats_saturated,
        "after_reject": stats_after_reject,
        "after_release": stats_final,
        "actual": {
            "second_error_code": rejected.code,
        },
        "pass": {
            "backpressure_code_matches": rejected.code == "CALYX_BACKPRESSURE",
            "queue_full_metric_rose": stats_after_reject.queue_full_rejected_total == 1,
            "no_waiter_was_queued": stats_after_reject.queued_total == 0,
            "queue_never_grew": stats_after_reject.max_observed_queued == 0,
            "in_flight_released": stats_final.in_flight == 0,
        },
    })
}

fn fsv_zero_capacity_reject() -> serde_json::Value {
    let gate = Arc::new(QueryGate::new());
    let engine = blocking_engine(gate, 0, 3, 25);
    let stats_before = engine.query_admission_stats();
    let rejected = engine.search(&query()).expect_err("zero capacity rejects");
    let stats_after = engine.query_admission_stats();
    json!({
        "name": "zero_capacity_reject",
        "input": "max_concurrent=0",
        "expected": {
            "error_code": "CALYX_BACKPRESSURE",
            "in_flight": 0,
            "queued": 0,
            "rejected_total": 1,
        },
        "before": stats_before,
        "after": stats_after,
        "actual": {
            "error_code": rejected.code,
        },
        "pass": {
            "backpressure_code_matches": rejected.code == "CALYX_BACKPRESSURE",
            "in_flight_stays_zero": stats_after.in_flight == 0,
            "queue_stays_zero": stats_after.queued == 0,
            "reject_metric_rose": stats_after.rejected_total == 1,
        },
    })
}

fn blocking_engine(
    gate: Arc<QueryGate>,
    max_concurrent: usize,
    max_queued: usize,
    timeout_ms: u64,
) -> SearchEngine {
    let map = SlotIndexMap::new();
    map.register(BlockingIndex { slot: slot(), gate }).unwrap();
    let mut engine = SearchEngine::new(map);
    engine.set_query_admission_config(QueryAdmissionConfig {
        max_concurrent,
        max_queued,
        queue_timeout: Duration::from_millis(timeout_ms),
    });
    engine
}

fn spawn_blocking_search(engine: Arc<SearchEngine>) -> thread::JoinHandle<Vec<IndexSearchHit>> {
    thread::spawn(move || {
        engine
            .search(&query())
            .unwrap()
            .into_iter()
            .map(|hit| IndexSearchHit {
                cx_id: hit.cx_id,
                score: hit.score,
                rank: hit.rank,
            })
            .collect()
    })
}

#[derive(Clone)]
struct BlockingIndex {
    slot: SlotId,
    gate: Arc<QueryGate>,
}

impl SextantIndex for BlockingIndex {
    fn slot(&self) -> SlotId {
        self.slot
    }

    fn shape(&self) -> SlotShape {
        SlotShape::Dense(2)
    }

    fn insert(&mut self, _cx_id: CxId, _vector: SlotVector, _seq: u64) -> Result<()> {
        Ok(())
    }

    fn search(
        &self,
        _query: &SlotVector,
        _k: usize,
        _ef: Option<usize>,
    ) -> Result<Vec<IndexSearchHit>> {
        self.gate.block_until_released();
        Ok(vec![IndexSearchHit {
            cx_id: hit_id(),
            score: 1.0,
            rank: 1,
        }])
    }

    fn rebuild(&mut self) -> Result<()> {
        Ok(())
    }

    fn vector(&self, _cx_id: CxId) -> Option<SlotVector> {
        None
    }

    fn set_base_seq(&mut self, _seq: u64) {}

    fn stats(&self) -> IndexStats {
        IndexStats {
            slot: self.slot,
            shape: SlotShape::Dense(2),
            len: 1,
            built_at_seq: 1,
            base_seq: 1,
            kind: "blocking",
        }
    }
}

struct QueryGate {
    entered: Mutex<usize>,
    entered_cv: Condvar,
    released: Mutex<bool>,
    released_cv: Condvar,
}

impl QueryGate {
    fn new() -> Self {
        Self {
            entered: Mutex::new(0),
            entered_cv: Condvar::new(),
            released: Mutex::new(false),
            released_cv: Condvar::new(),
        }
    }

    fn block_until_released(&self) {
        let mut entered = self.entered.lock().unwrap();
        *entered += 1;
        self.entered_cv.notify_all();
        drop(entered);

        let mut released = self.released.lock().unwrap();
        while !*released {
            released = self.released_cv.wait(released).unwrap();
        }
    }

    fn wait_for_entered(&self, count: usize) {
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut entered = self.entered.lock().unwrap();
        while *entered < count {
            let now = Instant::now();
            assert!(now < deadline, "blocking search did not enter index");
            let remaining = deadline.saturating_duration_since(now);
            let (next, _) = self.entered_cv.wait_timeout(entered, remaining).unwrap();
            entered = next;
        }
    }

    fn release_all(&self) {
        let mut released = self.released.lock().unwrap();
        *released = true;
        self.released_cv.notify_all();
    }
}

fn query() -> Query {
    Query::new("admission")
        .with_slots(vec![slot()])
        .with_vector(SlotVector::Dense {
            dim: 2,
            data: vec![1.0, 0.0],
        })
}

fn rss_kib() -> u64 {
    fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|status| {
            status.lines().find_map(|line| {
                line.strip_prefix("VmRSS:").and_then(|rest| {
                    rest.split_whitespace()
                        .next()
                        .and_then(|value| value.parse::<u64>().ok())
                })
            })
        })
        .unwrap_or(0)
}

fn fsv_root() -> PathBuf {
    std::env::var("CALYX_FSV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("calyx-query-admission-fsv"))
}

const fn slot() -> SlotId {
    SlotId::new(56)
}

fn hit_id() -> CxId {
    CxId::from_bytes([0x59; 16])
}
