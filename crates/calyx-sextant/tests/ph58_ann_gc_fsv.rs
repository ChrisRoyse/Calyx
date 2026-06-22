use calyx_aster::gc::{
    AnnGcReclaimer, AnnGcTarget, AnnIndexGraph, AnnTombstoneStats, CALYX_IO_ERROR, SharedAnnIndex,
    ann_io_error,
};
#[path = "sextant_support/mod.rs"]
mod sextant_support;
use calyx_core::{Result, SlotId};
use calyx_sextant::{HnswIndex, SextantIndex};
use serde_json::json;
use sextant_support::{cx_usize_be as cx, dense, raw_blake3_hex, write_json};
use std::fs;
use std::path::PathBuf;
use std::time::Duration;

#[test]
#[ignore = "manual FSV for issue #484 ANN tombstone GC"]
fn ph58_ann_tombstone_gc_fsv() {
    let root = fsv_root().join("ann");
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("create FSV root");

    let mut index = build_index(100, 8);
    for i in 0..30 {
        assert!(index.mark_deleted(cx(i), 1_000 + i as u64).unwrap());
    }
    let deleted_query = dense(unit_vector(0, 8));
    let hits_before = index.search(&deleted_query, 10, Some(64)).unwrap();
    let deleted_vector_before = index.vector(cx(0));
    let stats_before = index.ann_tombstone_stats();
    write_json(
        &root.join("hnsw-before.json"),
        &json!({
            "stats": stats_json(&stats_before),
            "deleted_vector_present": deleted_vector_before.is_some(),
            "deleted_query_hits": hit_ids(&hits_before),
            "deleted_id_absent_from_hits": !hits_before.iter().any(|hit| hit.cx_id == cx(0)),
        }),
    );

    let shared = SharedAnnIndex::new(index);
    let old_reader = shared.current().expect("old reader snapshot");
    let reclaimer = AnnGcReclaimer::with_limits(Duration::ZERO, 0.25, 0.80);
    let happy = reclaimer.run_once_at(&shared, "slot_23", 0.10, 1_000);
    let after = shared.current().expect("after swap");
    let hits_after = after.search(&deleted_query, 10, Some(64)).unwrap();
    fs::write(
        root.join("metrics-happy.prom"),
        happy.to_metrics_text("issue484-ann"),
    )
    .expect("write happy metrics");
    write_json(
        &root.join("hnsw-after.json"),
        &json!({
            "result": result_json(&happy),
            "new_stats": stats_json(&after.ann_tombstone_stats()),
            "old_reader_stats": stats_json(&old_reader.ann_tombstone_stats()),
            "deleted_query_hits": hit_ids(&hits_after),
        }),
    );

    let low_ratio = run_hnsw_case(80, 20, 0.10, 1, 0.25, 0.80);
    let high_load = run_hnsw_case(70, 30, 0.95, 1, 0.25, 0.80);
    let all_tombstoned = run_hnsw_case(0, 10, 0.0, 1, 0.25, 0.80);
    let failing = SharedAnnIndex::new(FailingGraph::new("slot_fail", 70, 30));
    let fail_closed = AnnGcReclaimer::with_limits(Duration::ZERO, 0.25, 0.80).run_once_at(
        &failing,
        "slot_fail",
        0.0,
        1,
    );
    let fail_after = failing.ann_tombstone_stats("slot_fail").unwrap();

    let summary = json!({
        "issue": 484,
        "source_of_truth": {
            "root": root.display().to_string(),
            "before": "hnsw-before.json",
            "after": "hnsw-after.json",
            "metrics": "metrics-happy.prom"
        },
        "trigger": "AnnGcReclaimer::run_once_at(index_id=slot_23, serving_io_load=0.10)",
        "happy": result_json(&happy),
        "happy_before": stats_json(&stats_before),
        "happy_after": stats_json(&after.ann_tombstone_stats()),
        "old_reader_after_swap": stats_json(&old_reader.ann_tombstone_stats()),
        "edge_low_ratio": result_json(&low_ratio),
        "edge_high_load": result_json(&high_load),
        "edge_all_tombstoned": result_json(&all_tombstoned),
        "fail_closed": {
            "result": result_json(&fail_closed),
            "after": stats_json(&fail_after)
        }
    });
    let summary_path = root.join("ann-gc-summary.json");
    write_json(&summary_path, &summary);
    let summary_bytes = fs::read(&summary_path).expect("read summary");
    println!("PH58_ANN_GC_FSV_ROOT={}", root.display());
    println!("PH58_ANN_GC_SUMMARY={}", summary_path.display());
    println!(
        "PH58_ANN_GC_SUMMARY_BLAKE3={}",
        raw_blake3_hex(&summary_bytes)
    );
    println!("{}", serde_json::to_string_pretty(&summary).unwrap());

    assert!(happy.triggered);
    assert_eq!(happy.tombstoned_nodes_before, 30);
    assert_eq!(happy.tombstoned_nodes_after, 0);
    assert_eq!(after.total_nodes(), 70);
    assert_eq!(old_reader.tombstone_count(), 30);
    assert_eq!(
        low_ratio.skipped_reason,
        Some("ann_tombstone_ratio_below_trigger")
    );
    assert_eq!(
        high_load.skipped_reason,
        Some("serving_io_load_above_threshold")
    );
    assert!(all_tombstoned.triggered);
    assert_eq!(all_tombstoned.total_nodes_after, 0);
    assert_eq!(fail_closed.error_code, Some(CALYX_IO_ERROR));
    assert_eq!(fail_after.tombstoned_nodes, 30);
}

#[derive(Clone, Debug)]
struct FailingGraph {
    id: String,
    live: usize,
    tombstones: usize,
}

impl FailingGraph {
    fn new(id: &str, live: usize, tombstones: usize) -> Self {
        Self {
            id: id.to_string(),
            live,
            tombstones,
        }
    }
}

impl AnnIndexGraph for FailingGraph {
    fn ann_index_id(&self) -> String {
        self.id.clone()
    }

    fn ann_tombstone_stats(&self) -> AnnTombstoneStats {
        AnnTombstoneStats {
            index_id: self.id.clone(),
            total_nodes: self.live + self.tombstones,
            tombstoned_nodes: self.tombstones,
            live_nodes: self.live,
        }
    }

    fn rebuild_without_tombstones(&self) -> Result<Self> {
        Err(ann_io_error("synthetic ANN rebuild I/O failure"))
    }
}

fn run_hnsw_case(
    live: usize,
    tombstoned: usize,
    serving_io_load: f64,
    now_ms: u64,
    max_ratio: f64,
    max_load: f64,
) -> calyx_aster::gc::AnnGcResult {
    let mut index = build_index(live + tombstoned, 8);
    for i in live..(live + tombstoned) {
        assert!(index.mark_deleted(cx(i), 2_000 + i as u64).unwrap());
    }
    let shared = SharedAnnIndex::new(index);
    AnnGcReclaimer::with_limits(Duration::ZERO, max_ratio, max_load).run_once_at(
        &shared,
        "slot_23",
        serving_io_load,
        now_ms,
    )
}

fn build_index(n: usize, dim: usize) -> HnswIndex {
    let mut index = HnswIndex::new(SlotId::new(23), dim as u32, 7);
    for i in 0..n {
        index
            .insert(cx(i), dense(unit_vector(i, dim)), i as u64 + 1)
            .unwrap();
    }
    index
}

fn unit_vector(i: usize, dim: usize) -> Vec<f32> {
    let t = i as f32 * 0.013;
    let mut data = vec![0.0_f32; dim];
    data[0] = t.cos();
    data[1] = t.sin();
    for (axis, value) in data.iter_mut().enumerate().skip(2) {
        *value = (((i + axis * 17) % 31) as f32 - 15.0) * 0.002;
    }
    normalize(data)
}

fn normalize(mut data: Vec<f32>) -> Vec<f32> {
    let norm = data.iter().map(|value| value * value).sum::<f32>().sqrt();
    for value in &mut data {
        *value /= norm;
    }
    data
}

fn hit_ids(hits: &[calyx_sextant::IndexSearchHit]) -> Vec<String> {
    hits.iter().map(|hit| hit.cx_id.to_string()).collect()
}

fn stats_json(stats: &AnnTombstoneStats) -> serde_json::Value {
    json!({
        "index_id": stats.index_id.clone(),
        "total_nodes": stats.total_nodes,
        "tombstoned_nodes": stats.tombstoned_nodes,
        "live_nodes": stats.live_nodes,
        "tombstone_ratio": stats.tombstone_ratio(),
    })
}

fn result_json(result: &calyx_aster::gc::AnnGcResult) -> serde_json::Value {
    json!({
        "triggered": result.triggered,
        "rate_limited": result.rate_limited,
        "skipped_reason": result.skipped_reason,
        "error_code": result.error_code,
        "index_id": result.index_id.clone(),
        "tombstone_ratio_before": result.tombstone_ratio_before,
        "tombstone_ratio_after": result.tombstone_ratio_after,
        "total_nodes_before": result.total_nodes_before,
        "total_nodes_after": result.total_nodes_after,
        "tombstoned_nodes_before": result.tombstoned_nodes_before,
        "tombstoned_nodes_after": result.tombstoned_nodes_after,
        "live_nodes_after": result.live_nodes_after,
        "rebuild_total": result.rebuild_total,
    })
}

fn fsv_root() -> PathBuf {
    std::env::var_os("CALYX_PH58_WAL_ANN_GC_FSV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("calyx-ph58-wal-ann-gc-fsv"))
}
