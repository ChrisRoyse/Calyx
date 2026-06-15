use super::*;
use calyx_core::LensId;
use proptest::prelude::*;
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Default)]
struct FakePanelTarget {
    records: RefCell<BTreeMap<PanelVersionId, PanelVersionRecord>>,
    live: BTreeSet<PanelVersionId>,
    cold_moves: RefCell<Vec<PanelVersionId>>,
    purges: RefCell<Vec<PanelVersionId>>,
}

impl PanelVersionGcTarget for FakePanelTarget {
    fn panel_versions(&self) -> Result<Vec<PanelVersionRecord>> {
        Ok(self.records.borrow().values().cloned().collect())
    }

    fn live_panel_versions(&self) -> Result<BTreeSet<PanelVersionId>> {
        Ok(self.live.clone())
    }

    fn move_panel_version_to_cold(&self, id: PanelVersionId) -> Result<u64> {
        self.records.borrow_mut().entry(id).and_modify(|record| {
            record.tier = VersionTier::Cold;
        });
        self.cold_moves.borrow_mut().push(id);
        Ok(0)
    }

    fn purge_cold_panel_version(&self, id: PanelVersionId) -> Result<u64> {
        let bytes = self
            .records
            .borrow_mut()
            .remove(&id)
            .map_or(0, |record| record.bytes);
        self.purges.borrow_mut().push(id);
        Ok(bytes)
    }
}

impl CodebookVersionGcTarget for FakePanelTarget {
    fn codebook_versions(&self) -> Result<Vec<PanelVersionRecord>> {
        Ok(self.records.borrow().values().cloned().collect())
    }

    fn move_codebook_version_to_cold(&self, id: PanelVersionId) -> Result<u64> {
        self.records.borrow_mut().entry(id).and_modify(|record| {
            record.tier = VersionTier::Cold;
        });
        self.cold_moves.borrow_mut().push(id);
        Ok(0)
    }

    fn purge_cold_codebook_version(&self, id: PanelVersionId) -> Result<u64> {
        let bytes = self
            .records
            .borrow_mut()
            .remove(&id)
            .map_or(0, |record| record.bytes);
        self.purges.borrow_mut().push(id);
        Ok(bytes)
    }
}

#[derive(Default)]
struct FakeLensTarget {
    bytes: u64,
    moved: RefCell<Vec<LensId>>,
    purged: RefCell<Vec<LensId>>,
}

impl RetiredLensGcTarget for FakeLensTarget {
    fn retired_lens_bytes(&self, _lens_id: LensId) -> Result<u64> {
        Ok(self.bytes)
    }

    fn move_retired_lens_to_cold(&self, lens_id: LensId) -> Result<u64> {
        self.moved.borrow_mut().push(lens_id);
        Ok(0)
    }

    fn purge_retired_lens(&self, lens_id: LensId) -> Result<u64> {
        self.purged.borrow_mut().push(lens_id);
        Ok(self.bytes)
    }
}

#[test]
fn find_unreferenced_keeps_latest_hot_versions() {
    let target = panel_target(1..=5, [3], []);
    let gc = PanelVersionGc::new(RetentionPolicy {
        hot_versions_to_keep: 2,
        cold_tier_first: true,
        max_versions_per_run: 10,
    });

    let unreferenced = gc.find_unreferenced(&target).unwrap();

    assert_eq!(unreferenced, vec![1, 2]);
}

#[test]
fn prune_moves_hot_first_then_purges_cold_on_second_pass() {
    let target = panel_target(1..=2, [], []);
    let gc = PanelVersionGc::new(RetentionPolicy {
        hot_versions_to_keep: 0,
        cold_tier_first: true,
        max_versions_per_run: 10,
    });
    let first = gc.prune(&target, &[1, 2]).unwrap();
    let second = gc.prune(&target, &[1, 2]).unwrap();

    assert_eq!(first.moved_to_cold, 2);
    assert_eq!(first.pruned, 0);
    assert_eq!(second.pruned, 2);
    assert_eq!(second.panel_versions_pruned_total, 2);
    assert_eq!(*target.cold_moves.borrow(), vec![1, 2]);
    assert_eq!(*target.purges.borrow(), vec![1, 2]);
}

#[test]
fn all_versions_referenced_returns_empty_and_does_not_prune() {
    let target = panel_target(1..=3, [1, 2, 3], []);
    let gc = PanelVersionGc::new(RetentionPolicy {
        hot_versions_to_keep: 0,
        cold_tier_first: false,
        max_versions_per_run: 10,
    });

    let ids = gc.find_unreferenced(&target).unwrap();
    let result = gc.prune(&target, &ids).unwrap();

    assert!(ids.is_empty());
    assert_eq!(result.pruned, 0);
    assert!(target.purges.borrow().is_empty());
}

#[test]
fn ledger_referenced_panel_is_skipped_fail_closed() {
    let target = panel_target(1..=1, [], [1]);
    let gc = PanelVersionGc::new(RetentionPolicy {
        hot_versions_to_keep: 0,
        cold_tier_first: false,
        max_versions_per_run: 10,
    });

    let result = gc.prune(&target, &[1]).unwrap();

    assert_eq!(result.skipped_ledger_referenced, 1);
    assert_eq!(result.pruned, 0);
    assert!(target.purges.borrow().is_empty());
}

#[test]
fn codebook_version_gc_moves_then_purges_and_keeps_manifest_reference() {
    let target = panel_target(1..=4, [], [3]);
    let gc = CodebookVersionGc::new(RetentionPolicy {
        hot_versions_to_keep: 1,
        cold_tier_first: true,
        max_versions_per_run: 10,
    });

    let ids = gc.find_unreferenced(&target).unwrap();
    let first = gc.prune(&target, &ids).unwrap();
    let second = gc.prune(&target, &ids).unwrap();

    assert_eq!(ids, vec![1, 2]);
    assert_eq!(first.moved_to_cold, 2);
    assert_eq!(second.pruned, 2);
    assert_eq!(second.codebook_versions_pruned_total, 2);
    assert_eq!(*target.cold_moves.borrow(), vec![1, 2]);
    assert_eq!(*target.purges.borrow(), vec![1, 2]);
    assert!(target.records.borrow().contains_key(&3));
    assert!(target.records.borrow().contains_key(&4));
}

#[test]
fn retired_lens_can_be_purged_after_retention_policy_says_delete() {
    let lens = LensId::from_bytes([9; 16]);
    let target = FakeLensTarget {
        bytes: 128,
        ..FakeLensTarget::default()
    };
    let gc = RetiredLensGc::new(RetentionPolicy {
        hot_versions_to_keep: 0,
        cold_tier_first: false,
        max_versions_per_run: 10,
    });

    let result = gc.prune_retired(&target, lens).unwrap();

    assert_eq!(result.bytes_freed, 128);
    assert_eq!(result.retired_lens_bytes_freed_total, 128);
    assert_eq!(*target.purged.borrow(), vec![lens]);
}

#[test]
fn metrics_text_uses_required_names() {
    let result = PanelVersionGcResult {
        panel_versions_pruned_total: 2,
        codebook_versions_pruned_total: 1,
        retired_lens_bytes_freed_total: 128,
        ..PanelVersionGcResult::default()
    };
    let metrics = result.to_metrics_text("issue485", 3);

    assert!(metrics.contains("calyx_panel_versions_pruned_total{vault=\"issue485\"} 2"));
    assert!(metrics.contains("calyx_panel_versions_live{vault=\"issue485\"} 3"));
    assert!(metrics.contains("calyx_codebook_versions_pruned_total{vault=\"issue485\"} 1"));
    assert!(metrics.contains("calyx_retired_lens_bytes_freed_total{vault=\"issue485\"} 128"));
}

proptest! {
    #[test]
    fn unreferenced_never_contains_live_reference(
        live_bits in prop::collection::vec(any::<bool>(), 1..32),
    ) {
        let versions = 1..=live_bits.len() as u32;
        let live = live_bits
            .iter()
            .enumerate()
            .filter_map(|(idx, is_live)| is_live.then_some(idx as u32 + 1))
            .collect::<Vec<_>>();
        let target = panel_target(versions, live.clone(), []);
        let gc = PanelVersionGc::new(RetentionPolicy {
            hot_versions_to_keep: 0,
            cold_tier_first: true,
            max_versions_per_run: 64,
        });

        let unreferenced = gc.find_unreferenced(&target).unwrap();

        for id in live {
            prop_assert!(!unreferenced.contains(&id));
        }
    }
}

fn panel_target(
    versions: impl IntoIterator<Item = PanelVersionId>,
    live: impl IntoIterator<Item = PanelVersionId>,
    ledger: impl IntoIterator<Item = PanelVersionId>,
) -> FakePanelTarget {
    let ledger = ledger.into_iter().collect::<BTreeSet<_>>();
    let records = versions
        .into_iter()
        .map(|id| {
            (
                id,
                PanelVersionRecord {
                    id,
                    tier: VersionTier::Hot,
                    ledger_referenced: ledger.contains(&id),
                    bytes: u64::from(id) * 10,
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
    FakePanelTarget {
        records: RefCell::new(records),
        live: live.into_iter().collect(),
        ..FakePanelTarget::default()
    }
}
