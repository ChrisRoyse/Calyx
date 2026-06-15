use super::*;
use std::time::Duration;

#[derive(Clone, Debug)]
struct FakeGraph {
    id: String,
    nodes: Vec<FakeNode>,
    fail_rebuild: bool,
}

#[derive(Clone, Debug)]
struct FakeNode {
    tombstoned: bool,
}

impl FakeGraph {
    fn with_counts(id: &str, live: usize, tombstoned: usize) -> Self {
        let mut nodes = Vec::with_capacity(live + tombstoned);
        nodes.extend((0..live).map(|_| FakeNode { tombstoned: false }));
        nodes.extend((0..tombstoned).map(|_| FakeNode { tombstoned: true }));
        Self {
            id: id.to_string(),
            nodes,
            fail_rebuild: false,
        }
    }

    fn failing(mut self) -> Self {
        self.fail_rebuild = true;
        self
    }
}

impl AnnIndexGraph for FakeGraph {
    fn ann_index_id(&self) -> String {
        self.id.clone()
    }

    fn ann_tombstone_stats(&self) -> AnnTombstoneStats {
        let tombstoned_nodes = self.nodes.iter().filter(|node| node.tombstoned).count();
        AnnTombstoneStats {
            index_id: self.id.clone(),
            total_nodes: self.nodes.len(),
            tombstoned_nodes,
            live_nodes: self.nodes.len() - tombstoned_nodes,
        }
    }

    fn rebuild_without_tombstones(&self) -> Result<Self> {
        if self.fail_rebuild {
            return Err(ann_io_error("synthetic ANN rebuild I/O failure"));
        }
        Ok(Self {
            id: self.id.clone(),
            nodes: self
                .nodes
                .iter()
                .filter(|node| !node.tombstoned)
                .cloned()
                .collect(),
            fail_rebuild: false,
        })
    }
}

#[test]
fn rebuild_triggers_above_ratio_and_removes_tombstones() {
    let target = SharedAnnIndex::new(FakeGraph::with_counts("slot_23", 70, 30));
    let reclaimer = AnnGcReclaimer::with_limits(Duration::ZERO, 0.25, 0.80);

    let result = reclaimer.run_once_at(&target, "slot_23", 0.20, 1_000);

    assert!(result.triggered);
    assert_eq!(result.total_nodes_before, 100);
    assert_eq!(result.tombstoned_nodes_before, 30);
    assert_eq!(result.total_nodes_after, 70);
    assert_eq!(result.tombstoned_nodes_after, 0);
    assert_eq!(result.live_nodes_after, 70);
    assert_eq!(result.rebuild_total, 1);
    assert_eq!(
        target
            .ann_tombstone_stats("slot_23")
            .unwrap()
            .tombstone_ratio(),
        0.0
    );
}

#[test]
fn reader_holding_old_arc_survives_rebuild_swap() {
    let target = SharedAnnIndex::new(FakeGraph::with_counts("slot_23", 70, 30));
    let old_reader = target.current().expect("old reader snapshot");
    let reclaimer = AnnGcReclaimer::with_limits(Duration::ZERO, 0.25, 0.80);

    let result = reclaimer.run_once_at(&target, "slot_23", 0.10, 1);

    assert!(result.triggered);
    assert_eq!(old_reader.ann_tombstone_stats().tombstoned_nodes, 30);
    assert_eq!(
        target
            .current()
            .unwrap()
            .ann_tombstone_stats()
            .tombstoned_nodes,
        0
    );
}

#[test]
fn low_ratio_and_high_load_are_noops() {
    let target = SharedAnnIndex::new(FakeGraph::with_counts("slot_23", 80, 20));
    let reclaimer = AnnGcReclaimer::with_limits(Duration::ZERO, 0.25, 0.80);

    let low_ratio = reclaimer.run_once_at(&target, "slot_23", 0.10, 1);
    assert_eq!(low_ratio.skipped_reason, Some(SKIP_LOW_RATIO));
    assert_eq!(low_ratio.rebuild_total, 0);

    let high_load_target = SharedAnnIndex::new(FakeGraph::with_counts("slot_24", 70, 30));
    let high_load = reclaimer.run_once_at(&high_load_target, "slot_24", 0.90, 2);
    assert_eq!(high_load.skipped_reason, Some(SKIP_HIGH_LOAD));
    assert!(high_load.rate_limited);
    assert_eq!(
        high_load_target
            .ann_tombstone_stats("slot_24")
            .unwrap()
            .tombstoned_nodes,
        30
    );
}

#[test]
fn all_tombstoned_rebuilds_to_empty_graph() {
    let target = SharedAnnIndex::new(FakeGraph::with_counts("slot_23", 0, 10));
    let reclaimer = AnnGcReclaimer::with_limits(Duration::ZERO, 0.25, 0.80);

    let result = reclaimer.run_once_at(&target, "slot_23", 0.0, 1);

    assert!(result.triggered);
    assert_eq!(result.total_nodes_after, 0);
    assert_eq!(result.tombstone_ratio_after, 0.0);
}

#[test]
fn rebuild_error_retains_old_graph_and_code() {
    let target = SharedAnnIndex::new(FakeGraph::with_counts("slot_23", 70, 30).failing());
    let reclaimer = AnnGcReclaimer::with_limits(Duration::ZERO, 0.25, 0.80);

    let result = reclaimer.run_once_at(&target, "slot_23", 0.0, 1);

    assert_eq!(result.error_code, Some(CALYX_IO_ERROR));
    assert_eq!(result.tombstoned_nodes_after, 30);
    assert_eq!(
        target
            .ann_tombstone_stats("slot_23")
            .unwrap()
            .tombstoned_nodes,
        30
    );
}

#[test]
fn interval_skip_prevents_repeated_rebuilds() {
    let target = SharedAnnIndex::new(FakeGraph::with_counts("slot_23", 70, 30));
    let reclaimer = AnnGcReclaimer::with_limits(Duration::from_millis(1_000), 0.25, 0.80);

    let first = reclaimer.run_once_at(&target, "slot_23", 0.0, 1_000);
    assert!(first.triggered);
    let second = reclaimer.run_once_at(&target, "slot_23", 0.0, 1_500);

    assert_eq!(second.skipped_reason, Some(SKIP_INTERVAL));
    assert!(second.rate_limited);
}

#[test]
fn metrics_text_uses_required_metric_names() {
    let result = AnnGcResult {
        triggered: true,
        rate_limited: false,
        skipped_reason: None,
        error_code: None,
        error_message: None,
        index_id: "slot_23".to_string(),
        tombstone_ratio_before: 0.3,
        tombstone_ratio_after: 0.0,
        total_nodes_before: 100,
        total_nodes_after: 70,
        tombstoned_nodes_before: 30,
        tombstoned_nodes_after: 0,
        live_nodes_after: 70,
        rebuild_total: 1,
    };

    let metrics = result.to_metrics_text("issue484");

    assert!(
        metrics
            .contains("calyx_ann_tombstone_ratio{vault=\"issue484\",index=\"slot_23\"} 0.000000")
    );
    assert!(metrics.contains("calyx_ann_gc_rebuild_total{vault=\"issue484\",index=\"slot_23\"} 1"));
}
