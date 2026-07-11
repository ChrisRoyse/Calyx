use super::*;
use crate::manifest::{ImmutableRef, ManifestStore, VaultManifest};
use calyx_core::{FixedClock, LensId, VaultId};
use proptest::prelude::*;
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;

trait IntoTargetResult<T> {
    fn into_target_result(self) -> Result<T>;
}

impl<'a, C> IntoTargetResult<VaultPanelVersionGcTarget<'a, C>>
    for VaultPanelVersionGcTarget<'a, C>
{
    fn into_target_result(self) -> Result<VaultPanelVersionGcTarget<'a, C>> {
        Ok(self)
    }
}

impl<'a, C> IntoTargetResult<VaultPanelVersionGcTarget<'a, C>>
    for Result<VaultPanelVersionGcTarget<'a, C>>
{
    fn into_target_result(self) -> Result<VaultPanelVersionGcTarget<'a, C>> {
        self
    }
}

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
fn manifest_load_failure_aborts_gc_without_relocating_referenced_assets() {
    let root = std::env::temp_dir().join(format!(
        "calyx-issue1365-manifest-gc-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    let vault_dir = root.join("vault");
    let cold_dir = root.join("cold");
    let panel_path = vault_dir.join("panel/panel-v00000001.bin");
    let codebook_path = vault_dir.join("codebooks/codebook-v00000001.bin");
    let cold_panel_path = cold_dir.join("panel/panel-v00000001.bin");
    let cold_codebook_path = cold_dir.join("codebooks/codebook-v00000001.bin");
    let panel_bytes = b"issue1365-manifest-panel".to_vec();
    let codebook_bytes = b"issue1365-manifest-codebook".to_vec();
    fs::create_dir_all(panel_path.parent().unwrap()).unwrap();
    fs::create_dir_all(codebook_path.parent().unwrap()).unwrap();
    fs::write(&panel_path, &panel_bytes).unwrap();
    fs::write(&codebook_path, &codebook_bytes).unwrap();

    let manifest = VaultManifest::new(
        1,
        0,
        ImmutableRef::from_bytes("panel/panel-v00000001.bin", &panel_bytes).unwrap(),
        vec![
            ImmutableRef::from_bytes("codebooks/codebook-v00000001.bin", &codebook_bytes).unwrap(),
        ],
    )
    .unwrap();
    let manifest_store = ManifestStore::open(&vault_dir);
    manifest_store.write_current(&manifest).unwrap();

    let current_path = vault_dir.join("CURRENT");
    let unavailable_current_path = vault_dir.join("CURRENT.transient-unavailable");
    fs::rename(&current_path, &unavailable_current_path).unwrap();
    let vault_id: VaultId = "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap();
    let vault = AsterVault::with_clock(vault_id, b"issue1365".to_vec(), FixedClock::new(1_365));

    let construction =
        VaultPanelVersionGcTarget::new(&vault, &vault_dir, &cold_dir).into_target_result();
    let constructor_error_code = match construction {
        Err(error) => Some(error.code.to_string()),
        Ok(target) => {
            let policy = RetentionPolicy {
                hot_versions_to_keep: 0,
                cold_tier_first: true,
                max_versions_per_run: 10,
            };
            let panel_gc = PanelVersionGc::new(policy);
            let panel_ids = panel_gc.find_unreferenced(&target).unwrap();
            panel_gc.prune(&target, &panel_ids).unwrap();
            let codebook_gc = CodebookVersionGc::new(policy);
            let codebook_ids = codebook_gc.find_unreferenced(&target).unwrap();
            codebook_gc.prune(&target, &codebook_ids).unwrap();
            None
        }
    };
    fs::rename(&unavailable_current_path, &current_path).unwrap();

    let panel_after = fs::read(&panel_path).ok();
    let codebook_after = fs::read(&codebook_path).ok();
    let manifest_reloads = manifest_store.load_current().is_ok();
    let readback = format!(
        "constructor_error_code={}\npanel_hot_exists={}\npanel_cold_exists={}\npanel_expected_blake3={}\npanel_actual_blake3={}\ncodebook_hot_exists={}\ncodebook_cold_exists={}\ncodebook_expected_blake3={}\ncodebook_actual_blake3={}\nmanifest_reloads={}\n",
        constructor_error_code.as_deref().unwrap_or("NONE"),
        panel_path.exists(),
        cold_panel_path.exists(),
        blake3::hash(&panel_bytes).to_hex(),
        panel_after
            .as_deref()
            .map(blake3::hash)
            .map_or_else(|| "MISSING".to_string(), |hash| hash.to_hex().to_string()),
        codebook_path.exists(),
        cold_codebook_path.exists(),
        blake3::hash(&codebook_bytes).to_hex(),
        codebook_after
            .as_deref()
            .map(blake3::hash)
            .map_or_else(|| "MISSING".to_string(), |hash| hash.to_hex().to_string()),
        manifest_reloads,
    );
    if let Some(root) = std::env::var_os("CALYX_GC_MANIFEST_FAILURE_FSV_ROOT") {
        let root = std::path::PathBuf::from(root);
        fs::create_dir_all(&root).unwrap();
        let path = root.join("manifest-gc-readback.txt");
        fs::write(&path, &readback).unwrap();
        println!("CALYX_GC_MANIFEST_FAILURE_READBACK={}", path.display());
    }
    fs::remove_dir_all(&root).unwrap();

    assert_eq!(
        constructor_error_code.as_deref(),
        Some("CALYX_DISK_PRESSURE"),
        "manifest read failure was swallowed before GC:\n{readback}"
    );
    assert_eq!(panel_after.as_deref(), Some(panel_bytes.as_slice()));
    assert_eq!(codebook_after.as_deref(), Some(codebook_bytes.as_slice()));
    assert!(!cold_panel_path.exists());
    assert!(!cold_codebook_path.exists());
    assert!(manifest_reloads);
}

#[test]
fn validated_manifest_free_target_protects_every_version() {
    let root = std::env::temp_dir().join(format!(
        "calyx-issue1365-manifest-free-gc-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    let vault_dir = root.join("vault");
    let cold_dir = root.join("cold");
    let panel_path = vault_dir.join("panel/panel-v00000001.bin");
    let codebook_path = vault_dir.join("codebooks/codebook-v00000001.bin");
    fs::create_dir_all(panel_path.parent().unwrap()).unwrap();
    fs::create_dir_all(codebook_path.parent().unwrap()).unwrap();
    fs::write(&panel_path, b"fresh-panel").unwrap();
    fs::write(&codebook_path, b"fresh-codebook").unwrap();
    let vault_id: VaultId = "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap();
    let vault = AsterVault::with_clock(vault_id, b"issue1365".to_vec(), FixedClock::new(1_365));
    let target = VaultPanelVersionGcTarget::new(&vault, &vault_dir, &cold_dir)
        .into_target_result()
        .unwrap();
    assert!(
        target
            .panel_versions()
            .unwrap()
            .iter()
            .all(|record| record.ledger_referenced)
    );
    assert!(
        target
            .codebook_versions()
            .unwrap()
            .iter()
            .all(|record| record.ledger_referenced)
    );
    let policy = RetentionPolicy {
        hot_versions_to_keep: 0,
        cold_tier_first: true,
        max_versions_per_run: 10,
    };
    let panel_result = PanelVersionGc::new(policy).prune(&target, &[1]).unwrap();
    let codebook_result = CodebookVersionGc::new(policy).prune(&target, &[1]).unwrap();
    assert_eq!(panel_result.skipped_ledger_referenced, 1);
    assert_eq!(codebook_result.skipped_ledger_referenced, 1);
    assert!(panel_path.exists());
    assert!(codebook_path.exists());
    assert!(!cold_dir.exists());
    fs::remove_dir_all(root).unwrap();
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
