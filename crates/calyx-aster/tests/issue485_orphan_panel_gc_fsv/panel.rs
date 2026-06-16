use super::{
    constellation, cx, dir_bytes, dir_inventory, durable_vault, file_len, hex, result_json,
};
use calyx_aster::cf::{ColumnFamily, base_key};
use calyx_aster::gc::{
    CodebookVersionGc, CodebookVersionGcTarget, PanelVersionGc, PanelVersionGcResult,
    PanelVersionGcTarget, PanelVersionRecord, RetentionPolicy, RetiredLensGc, RetiredLensGcTarget,
    VaultPanelVersionGcTarget,
};
use calyx_aster::manifest::{ImmutableRef, ManifestStore, VaultManifest};
use calyx_aster::vault::encode::encode_constellation_base;
use calyx_core::{CalyxError, FixedClock, LensId, Result};
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};

pub fn panel_codebook_fsv(root: &Path) -> Value {
    let happy = root.join("happy");
    let vault_dir = happy.join("vault");
    let cold_dir = happy.join("cold");
    fs::create_dir_all(vault_dir.join("panel")).unwrap();
    fs::create_dir_all(vault_dir.join("codebooks")).unwrap();
    let vault = durable_vault(&vault_dir);
    write_versioned_files(&vault_dir.join("panel"), "panel", 1..=8, 16);
    write_versioned_files(&vault_dir.join("codebooks"), "codebook", 1..=8, 12);
    write_manifest(&vault_dir, 3);
    for (seed, panel_version) in [(60, 6), (80, 8)] {
        vault
            .write_cf(
                ColumnFamily::Base,
                base_key(cx(seed)),
                encode_constellation_base(&constellation(seed, panel_version, &[])).unwrap(),
            )
            .unwrap();
    }
    vault.flush().unwrap();

    let target = VaultPanelVersionGcTarget::new(&vault, &vault_dir, &cold_dir);
    let policy = RetentionPolicy {
        hot_versions_to_keep: 2,
        cold_tier_first: true,
        max_versions_per_run: 20,
    };
    let panel_gc = PanelVersionGc::new(policy);
    let before = panel_readback(&target, &vault_dir, &cold_dir);
    let panel_ids = panel_gc.find_unreferenced(&target).unwrap();
    let panel_first = panel_gc.prune(&target, &panel_ids).unwrap();
    let after_panel_move = panel_readback(&target, &vault_dir, &cold_dir);
    let panel_second = panel_gc.prune(&target, &panel_ids).unwrap();
    let after_panel_purge = panel_readback(&target, &vault_dir, &cold_dir);

    let codebook_gc = CodebookVersionGc::new(policy);
    let codebook_ids = codebook_gc.find_unreferenced(&target).unwrap();
    let codebook_first = codebook_gc.prune(&target, &codebook_ids).unwrap();
    let after_codebook_move = panel_readback(&target, &vault_dir, &cold_dir);
    let codebook_second = codebook_gc.prune(&target, &codebook_ids).unwrap();
    let after_codebook_purge = panel_readback(&target, &vault_dir, &cold_dir);

    let lens = LensId::from_bytes([9; 16]);
    let lens_target = FileLensTarget::new(&happy.join("retired-lens"));
    lens_target.write_hot(lens, 144);
    let lens_before = lens_target.readback(lens);
    let lens_move_gc = RetiredLensGc::new(policy);
    let lens_move = lens_move_gc.prune_retired(&lens_target, lens).unwrap();
    let lens_after_move = lens_target.readback(lens);
    let lens_purge_gc = RetiredLensGc::new(RetentionPolicy {
        cold_tier_first: false,
        ..policy
    });
    let lens_purge = lens_purge_gc.prune_retired(&lens_target, lens).unwrap();
    let lens_after_purge = lens_target.readback(lens);

    let combined = PanelVersionGcResult {
        panel_versions_pruned_total: panel_second.panel_versions_pruned_total,
        codebook_versions_pruned_total: codebook_second.codebook_versions_pruned_total,
        retired_lens_bytes_freed_total: lens_purge.retired_lens_bytes_freed_total,
        ..PanelVersionGcResult::default()
    };
    let metrics = combined.to_metrics_text(
        "issue485-panel",
        target.live_panel_versions().unwrap().len(),
    );
    fs::write(root.join("panel-metrics.prom"), &metrics).expect("write panel metrics");

    json!({
        "source_of_truth": source_of_truth_json(&vault_dir, &cold_dir),
        "synthetic_input": {
            "panel_versions": [1,2,3,4,5,6,7,8],
            "live_panel_versions": [6,8],
            "manifest_protected_version": 3,
            "hot_versions_to_keep": 2,
            "hand_expected_panel_pruned": [1,2,4,5],
            "hand_expected_codebook_pruned": [1,2,4,5,6]
        },
        "happy": {
            "before": before,
            "panel_unreferenced": panel_ids,
            "panel_first": result_json(&panel_first),
            "after_panel_move": after_panel_move,
            "panel_second": result_json(&panel_second),
            "after_panel_purge": after_panel_purge,
            "codebook_unreferenced": codebook_ids,
            "codebook_first": result_json(&codebook_first),
            "after_codebook_move": after_codebook_move,
            "codebook_second": result_json(&codebook_second),
            "after_codebook_purge": after_codebook_purge,
            "retired_lens_before": lens_before,
            "retired_lens_move": result_json(&lens_move),
            "retired_lens_after_move": lens_after_move,
            "retired_lens_purge": result_json(&lens_purge),
            "retired_lens_after_purge": lens_after_purge
        },
        "edges": {
            "all_referenced": panel_all_referenced_edge(&root.join("edge-all-referenced")),
            "rate_limit": panel_rate_limit_edge(&root.join("edge-rate-limit")),
            "manifest_protected_skip": {
                "protected_panel_still_exists": vault_dir.join("panel/panel-v00000003.bin").exists(),
                "protected_codebook_still_exists": vault_dir.join("codebooks/codebook-v00000003.bin").exists(),
                "panel_skipped": panel_second.skipped_ledger_referenced,
                "codebook_skipped": codebook_second.skipped_ledger_referenced
            }
        },
        "metrics": metrics
    })
}

fn panel_all_referenced_edge(root: &Path) -> Value {
    let vault_dir = root.join("vault");
    let cold_dir = root.join("cold");
    fs::create_dir_all(vault_dir.join("panel")).unwrap();
    write_versioned_files(&vault_dir.join("panel"), "panel", 1..=3, 8);
    let vault = durable_vault(&vault_dir);
    for seed in 1..=3 {
        vault
            .write_cf(
                ColumnFamily::Base,
                base_key(cx(seed)),
                encode_constellation_base(&constellation(seed, seed as u32, &[])).unwrap(),
            )
            .unwrap();
    }
    let target = VaultPanelVersionGcTarget::new(&vault, &vault_dir, &cold_dir);
    let gc = PanelVersionGc::new(RetentionPolicy {
        hot_versions_to_keep: 0,
        cold_tier_first: true,
        max_versions_per_run: 10,
    });
    let before = panel_readback(&target, &vault_dir, &cold_dir);
    let ids = gc.find_unreferenced(&target).unwrap();
    let result = gc.prune(&target, &ids).unwrap();
    let after = panel_readback(&target, &vault_dir, &cold_dir);
    json!({ "before": before, "unreferenced": ids, "result": result_json(&result), "after": after })
}

fn panel_rate_limit_edge(root: &Path) -> Value {
    let vault_dir = root.join("vault");
    let cold_dir = root.join("cold");
    fs::create_dir_all(vault_dir.join("panel")).unwrap();
    write_versioned_files(&vault_dir.join("panel"), "panel", 1..=5, 8);
    let vault = durable_vault(&vault_dir);
    let target = VaultPanelVersionGcTarget::new(&vault, &vault_dir, &cold_dir);
    let gc = PanelVersionGc::new(RetentionPolicy {
        hot_versions_to_keep: 0,
        cold_tier_first: true,
        max_versions_per_run: 2,
    });
    let before = panel_readback(&target, &vault_dir, &cold_dir);
    let ids = gc.find_unreferenced(&target).unwrap();
    let result = gc.prune(&target, &ids).unwrap();
    let after = panel_readback(&target, &vault_dir, &cold_dir);
    json!({
        "before": before,
        "unreferenced": ids,
        "result": result_json(&result),
        "after": after,
        "hand_expected_moved_to_cold": 2
    })
}

struct FileLensTarget {
    hot: PathBuf,
    cold: PathBuf,
}

impl FileLensTarget {
    fn new(root: &Path) -> Self {
        let target = Self {
            hot: root.join("hot"),
            cold: root.join("cold"),
        };
        fs::create_dir_all(&target.hot).unwrap();
        fs::create_dir_all(&target.cold).unwrap();
        target
    }

    fn write_hot(&self, lens_id: LensId, bytes: usize) {
        fs::write(self.hot.join(lens_file(lens_id)), vec![0x5a; bytes]).unwrap();
    }

    fn readback(&self, lens_id: LensId) -> Value {
        let name = lens_file(lens_id);
        let hot_path = self.hot.join(&name);
        let cold_path = self.cold.join(&name);
        json!({
            "hot_exists": hot_path.exists(),
            "cold_exists": cold_path.exists(),
            "hot_bytes": file_len(&hot_path),
            "cold_bytes": file_len(&cold_path)
        })
    }
}

impl RetiredLensGcTarget for FileLensTarget {
    fn retired_lens_bytes(&self, lens_id: LensId) -> Result<u64> {
        let name = lens_file(lens_id);
        Ok(file_len(&self.hot.join(&name)) + file_len(&self.cold.join(&name)))
    }

    fn move_retired_lens_to_cold(&self, lens_id: LensId) -> Result<u64> {
        let name = lens_file(lens_id);
        let source = self.hot.join(&name);
        if !source.exists() {
            return Ok(0);
        }
        fs::rename(&source, self.cold.join(name))
            .map_err(|error| CalyxError::disk_pressure(error.to_string()))?;
        Ok(0)
    }

    fn purge_retired_lens(&self, lens_id: LensId) -> Result<u64> {
        let path = self.cold.join(lens_file(lens_id));
        let bytes = file_len(&path);
        if path.exists() {
            fs::remove_file(path).map_err(|error| CalyxError::disk_pressure(error.to_string()))?;
        }
        Ok(bytes)
    }
}

fn panel_readback(
    target: &VaultPanelVersionGcTarget<'_, FixedClock>,
    vault_dir: &Path,
    cold_dir: &Path,
) -> Value {
    json!({
        "live_panel_versions": target.live_panel_versions().unwrap(),
        "panel_records": target.panel_versions().unwrap().into_iter().map(record_json).collect::<Vec<_>>(),
        "codebook_records": target.codebook_versions().unwrap().into_iter().map(record_json).collect::<Vec<_>>(),
        "hot_panel": dir_inventory(&vault_dir.join("panel")),
        "cold_panel": dir_inventory(&cold_dir.join("panel")),
        "hot_codebook": dir_inventory(&vault_dir.join("codebooks")),
        "cold_codebook": dir_inventory(&cold_dir.join("codebooks")),
        "hot_panel_bytes": dir_bytes(&vault_dir.join("panel")),
        "cold_panel_bytes": dir_bytes(&cold_dir.join("panel")),
        "hot_codebook_bytes": dir_bytes(&vault_dir.join("codebooks")),
        "cold_codebook_bytes": dir_bytes(&cold_dir.join("codebooks"))
    })
}

fn source_of_truth_json(vault_dir: &Path, cold_dir: &Path) -> Value {
    json!({
        "vault": vault_dir.display().to_string(),
        "hot_panel_dir": vault_dir.join("panel").display().to_string(),
        "cold_panel_dir": cold_dir.join("panel").display().to_string(),
        "hot_codebook_dir": vault_dir.join("codebooks").display().to_string(),
        "cold_codebook_dir": cold_dir.join("codebooks").display().to_string(),
        "manifest": vault_dir.join("CURRENT").display().to_string()
    })
}

fn write_versioned_files(
    dir: &Path,
    kind: &str,
    range: impl IntoIterator<Item = u32>,
    width: usize,
) {
    fs::create_dir_all(dir).unwrap();
    for id in range {
        let name = format!("{kind}-v{id:08}.bin");
        fs::write(dir.join(name), vec![id as u8; id as usize * width]).unwrap();
    }
}

fn write_manifest(vault_dir: &Path, protected: u32) {
    let panel_name = format!("panel/panel-v{protected:08}.bin");
    let codebook_name = format!("codebooks/codebook-v{protected:08}.bin");
    let panel_bytes = fs::read(vault_dir.join(&panel_name)).unwrap();
    let codebook_bytes = fs::read(vault_dir.join(&codebook_name)).unwrap();
    let manifest = VaultManifest::new(
        1,
        1,
        ImmutableRef::from_bytes(panel_name, &panel_bytes).unwrap(),
        vec![ImmutableRef::from_bytes(codebook_name, &codebook_bytes).unwrap()],
    )
    .unwrap();
    ManifestStore::open(vault_dir)
        .write_current(&manifest)
        .unwrap();
}

fn record_json(record: PanelVersionRecord) -> Value {
    json!({
        "id": record.id,
        "tier": format!("{:?}", record.tier),
        "ledger_referenced": record.ledger_referenced,
        "bytes": record.bytes
    })
}

fn lens_file(lens_id: LensId) -> String {
    format!("lens-{}.bin", hex(lens_id.as_bytes()))
}
