use calyx_aster::gc::PanelVersionGcResult;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{
    CxFlags, CxId, FixedClock, InputRef, LedgerRef, Modality, SlotId, SlotVector, VaultId,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

mod orphan;
mod panel;

pub fn run_fsv(root: &Path) -> Value {
    let _ = fs::remove_dir_all(root);
    fs::create_dir_all(root).expect("create FSV root");

    let orphan = orphan::orphan_fsv(&root.join("orphan"));
    let panel = panel::panel_codebook_fsv(&root.join("panel-codebook"));
    let metrics = [
        orphan["metrics"].as_str().unwrap_or_default(),
        panel["metrics"].as_str().unwrap_or_default(),
    ]
    .join("");
    fs::write(root.join("metrics.prom"), metrics.as_bytes()).expect("write metrics");

    json!({
        "issue": 485,
        "source_of_truth": {
            "root": root.display().to_string(),
            "orphan_vault": "orphan/happy/vault",
            "panel_vault": "panel-codebook/happy/vault",
            "metrics": "metrics.prom"
        },
        "trigger": [
            "OrphanReconciler::repair over Aster Base/Slot CF rows",
            "PanelVersionGc::prune and CodebookVersionGc::prune over hot/cold files",
            "RetiredLensGc::prune_retired over retired lens files"
        ],
        "orphan": orphan,
        "panel_codebook_retired_lens": panel
    })
}

pub fn write_and_assert(root: &Path, summary: &Value) {
    let summary_path = root.join("issue485-summary.json");
    write_json(&summary_path, summary);
    let summary_bytes = fs::read(&summary_path).expect("read summary");

    println!("ISSUE485_FSV_ROOT={}", root.display());
    println!("ISSUE485_SUMMARY={}", summary_path.display());
    println!("ISSUE485_SUMMARY_BLAKE3={}", digest_hex(&summary_bytes));
    println!("{}", serde_json::to_string_pretty(summary).unwrap());

    assert_eq!(
        summary["orphan"]["happy"]["after"]["slot_rows"].as_u64(),
        Some(3)
    );
    assert_eq!(
        summary["orphan"]["happy"]["after"]["anneal_replay_rows"].as_u64(),
        Some(2)
    );
    assert_eq!(
        summary["orphan"]["edges"]["rate_limit"]["after"]["slot_rows"].as_u64(),
        Some(7)
    );
    assert_eq!(
        summary["orphan"]["edges"]["fail_closed_missing_base"]["error_code"].as_str(),
        Some("CALYX_ORPHAN_RECONCILER_ERROR")
    );
    let panel = &summary["panel_codebook_retired_lens"]["happy"];
    assert_eq!(panel["panel_second"]["pruned"].as_u64(), Some(4));
    assert_eq!(panel["codebook_second"]["pruned"].as_u64(), Some(5));
    assert_eq!(
        panel["retired_lens_purge"]["bytes_freed"].as_u64(),
        Some(144)
    );
}

pub fn fsv_root() -> PathBuf {
    std::env::var_os("CALYX_FSV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let pid = std::process::id();
            PathBuf::from(format!("/var/lib/calyx/data/fsv-issue485-{pid}"))
        })
}

pub(crate) fn durable_vault(vault_dir: &Path) -> AsterVault<FixedClock> {
    AsterVault::new_durable_with_clock(
        vault_dir,
        vault_id(),
        b"issue485-fsv",
        VaultOptions::default(),
        FixedClock::new(485_000),
    )
    .unwrap()
}

pub(crate) fn constellation(
    seed: u8,
    panel_version: u32,
    slots: &[SlotId],
) -> calyx_core::Constellation {
    let slot_rows = slots
        .iter()
        .copied()
        .map(|slot| {
            (
                slot,
                SlotVector::Dense {
                    dim: 1,
                    data: vec![f32::from(seed)],
                },
            )
        })
        .collect();
    calyx_core::Constellation {
        cx_id: cx(seed),
        vault_id: vault_id(),
        panel_version,
        created_at: u64::from(seed),
        input_ref: InputRef {
            hash: [seed; 32],
            pointer: Some(format!("synthetic://issue485/{seed}")),
            redacted: false,
        },
        modality: Modality::Text,
        slots: slot_rows,
        scalars: BTreeMap::new(),
        metadata: BTreeMap::new(),
        anchors: Vec::new(),
        provenance: LedgerRef {
            seq: u64::from(seed),
            hash: [seed; 32],
        },
        flags: CxFlags::default(),
    }
}

pub(crate) fn cx(seed: u8) -> CxId {
    CxId::from_bytes([seed; 16])
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}

pub(crate) fn dir_inventory(dir: &Path) -> Vec<Value> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut rows = entries
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            let meta = path.metadata().ok()?;
            if meta.is_dir() {
                return Some(json!({
                    "name": path.file_name()?.to_string_lossy(),
                    "dir": true,
                    "children": dir_inventory(&path)
                }));
            }
            let bytes = fs::read(&path).ok()?;
            Some(json!({
                "name": path.file_name()?.to_string_lossy(),
                "bytes": bytes.len(),
                "blake3": digest_hex(&bytes),
                "prefix_hex": hex_prefix(&bytes, 48)
            }))
        })
        .collect::<Vec<_>>();
    rows.sort_by_key(|row| row["name"].as_str().unwrap_or_default().to_string());
    rows
}

pub(crate) fn dir_bytes(dir: &Path) -> u64 {
    let Ok(entries) = fs::read_dir(dir) else {
        return 0;
    };
    entries
        .filter_map(|entry| entry.ok())
        .map(|entry| {
            let path = entry.path();
            if path.is_dir() {
                dir_bytes(&path)
            } else {
                file_len(&path)
            }
        })
        .sum()
}

pub(crate) fn file_len(path: &Path) -> u64 {
    path.metadata().map(|meta| meta.len()).unwrap_or(0)
}

pub(crate) fn result_json(result: &PanelVersionGcResult) -> Value {
    json!({
        "moved_to_cold": result.moved_to_cold,
        "pruned": result.pruned,
        "skipped_ledger_referenced": result.skipped_ledger_referenced,
        "bytes_freed": result.bytes_freed,
        "rate_limited": result.rate_limited,
        "panel_versions_pruned_total": result.panel_versions_pruned_total,
        "codebook_versions_pruned_total": result.codebook_versions_pruned_total,
        "retired_lens_bytes_freed_total": result.retired_lens_bytes_freed_total
    })
}

pub(crate) fn write_json(path: &Path, value: &Value) {
    fs::write(path, serde_json::to_vec_pretty(value).unwrap()).expect("write json");
}

pub(crate) fn digest_hex(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub(crate) fn hex_prefix(bytes: &[u8], max: usize) -> String {
    hex(&bytes[..bytes.len().min(max)])
}
