use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use calyx_anneal::{anneal_retention_tier, frequency_kernel_bonus, recurrence_schedule_for};
use calyx_aster::cf::{ColumnFamily, base_key};
use calyx_aster::dedup::{Domain, EpochSecs, compression_ratio, domain_compression_stats};
use calyx_aster::recurrence::{
    FREQUENCY_SCALAR, OccurrenceContext, RetentionPolicy, append_occurrence, read_series,
};
use calyx_aster::vault::{AsterVault, VaultOptions, encode};
use calyx_core::{
    CxFlags, CxId, FixedClock, InputRef, LedgerRef, Modality, SlotId, SlotVector, VaultStore,
};
use serde_json::json;

mod fsv_support;
use fsv_support::{ManifestPathStyle, vault_id, write_json, write_tree_manifest};

const MAIN_SEED: u8 = 0x39;

#[test]
#[ignore = "manual FSV trigger for issue 392"]
fn issue392_compression_anneal_fsv_artifacts() {
    let (root, keep_root) = fsv_root();
    let vault_dir = root.join("vault");
    fs::create_dir_all(&root).expect("create fsv root");
    let vault = AsterVault::new_durable(
        &vault_dir,
        vault_id(),
        b"issue392-compression-anneal",
        VaultOptions::default(),
    )
    .expect("open durable vault");
    let clock = FixedClock::new(1_000_000);

    vault.put(row(MAIN_SEED, None)).expect("put main base");
    for idx in 0..50 {
        let t = 1_000_000 + idx * 1_800;
        append_occurrence_at(&vault, MAIN_SEED, i64::from(t), format!("main-{idx}"));
    }
    vault.flush().expect("flush main vault");

    write_main_artifacts(&root, &vault, &clock);
    write_edge_artifact(&root, &vault, &clock);
    write_tree_manifest(&root, ManifestPathStyle::Slash);
    println!("issue392_fsv_root={}", root.display());

    if !keep_root {
        fs::remove_dir_all(root).expect("remove temp fsv root");
    }
}

fn write_main_artifacts(root: &Path, vault: &AsterVault, clock: &FixedClock) {
    let cx_id = cx(MAIN_SEED);
    let compression = compression_ratio(cx_id, vault).expect("compression ratio");
    let schedule = recurrence_schedule_for(cx_id, vault, clock).expect("schedule");
    let tier = anneal_retention_tier(cx_id, vault, clock).expect("retention tier");
    let series = read_series(vault, cx_id).expect("series");
    let base = base_readback(vault, cx_id);
    let domain = Domain::new(vec![cx(MAIN_SEED), cx(MAIN_SEED)]);
    let domain_stats = domain_compression_stats(&domain, vault).expect("domain stats");
    let expected_weight = frequency_kernel_bonus(50);

    let compression_json = json!({
        "surface": "compression-ratio",
        "artifact_kind": "ph42.compression-ratio.v1",
        "schema_version": 1,
        "source_of_truth": "PH42 persisted artifact",
        "vault": root.join("vault").display().to_string(),
        "cx_id": cx_id.to_string(),
        "original_count": compression.original_count,
        "stored_count": compression.stored_count,
        "ratio": compression.ratio,
        "domain_stats": domain_stats,
        "base_cf_readback": base,
    });
    write_json(&root.join("compression-ratio.json"), &compression_json);

    let schedule_json = json!({
        "surface": "anneal-schedule",
        "artifact_kind": "ph42.anneal-schedule.v1",
        "schema_version": 1,
        "source_of_truth": "PH42 persisted artifact",
        "vault": root.join("vault").display().to_string(),
        "cx_id": cx_id.to_string(),
        "frequency": series.frequency,
        "cadence_secs": series.cadence_secs,
        "importance_weight": schedule.importance_weight,
        "expected_importance_weight": expected_weight,
        "next_expected_t": schedule.next_expected_t,
        "refresh_priority": format!("{:?}", schedule.refresh_priority),
        "retention_tier": format!("{tier:?}"),
        "occurrence_count": series.occurrences.len(),
        "first_occurrence_t": series.occurrences.first().map(|occurrence| occurrence.t_k.0),
        "last_occurrence_t": series.occurrences.last().map(|occurrence| occurrence.t_k.0),
        "base_cf_readback": base,
    });
    write_json(&root.join("anneal-schedule.json"), &schedule_json);
}

fn write_edge_artifact(root: &Path, vault: &AsterVault, clock: &FixedClock) {
    let zero = cx(0);
    let missing = cx(1);
    let cold = cx(2);
    vault.put(row(0, Some(0.0))).expect("put zero");
    vault.put(row(1, None)).expect("put missing");
    vault.put(row(2, None)).expect("put cold");
    append_occurrence_at(vault, 2, 10_000, "cold-a");
    append_occurrence_at(vault, 2, 100_000, "cold-b");
    vault.flush().expect("flush edges");

    let zero_before = base_readback(vault, zero);
    let zero_compression = compression_ratio(zero, vault).expect("zero compression");
    let zero_schedule = recurrence_schedule_for(zero, vault, clock).expect("zero schedule");
    let missing_before = base_readback(vault, missing);
    let missing_error = compression_ratio(missing, vault).expect_err("missing frequency");
    let schedule_missing_error =
        recurrence_schedule_for(missing, vault, clock).expect_err("missing schedule frequency");
    let cold_before = base_readback(vault, cold);
    let cold_schedule = recurrence_schedule_for(cold, vault, clock).expect("cold schedule");
    let cold_tier = anneal_retention_tier(cold, vault, clock).expect("cold tier");

    let edge_json = json!({
        "zero_frequency": {
            "before": zero_before,
            "after": base_readback(vault, zero),
            "ratio": zero_compression.ratio,
            "importance_weight": zero_schedule.importance_weight,
            "refresh_priority": format!("{:?}", zero_schedule.refresh_priority),
        },
        "missing_frequency": {
            "before": missing_before,
            "after": base_readback(vault, missing),
            "compression_error_code": missing_error.code,
            "schedule_error_code": schedule_missing_error.code,
        },
        "cold_cadence": {
            "before": cold_before,
            "after": base_readback(vault, cold),
            "cadence_secs": read_series(vault, cold).expect("cold series").cadence_secs,
            "refresh_priority": format!("{:?}", cold_schedule.refresh_priority),
            "retention_tier": format!("{cold_tier:?}"),
        }
    });
    write_json(&root.join("edge-readbacks.json"), &edge_json);
}

fn append_occurrence_at(vault: &AsterVault, seed: u8, time: i64, context: impl Into<String>) {
    append_occurrence(
        vault,
        cx(seed),
        EpochSecs(time),
        OccurrenceContext::new(context.into()).expect("context"),
        EpochSecs(time),
        RetentionPolicy::default(),
    )
    .expect("append occurrence");
}

fn base_readback(vault: &AsterVault, cx_id: CxId) -> serde_json::Value {
    let key = base_key(cx_id);
    let bytes = vault
        .read_cf_at(vault.latest_seq(), ColumnFamily::Base, &key)
        .expect("read base cf")
        .expect("base exists");
    let cx = encode::decode_constellation_base(&bytes).expect("decode base");
    json!({
        "key_hex": hex(&key),
        "raw_blake3": blake3::hash(&bytes).to_hex().to_string(),
        "raw_len": bytes.len(),
        "frequency_scalar": cx.scalars.get(FREQUENCY_SCALAR),
    })
}

fn row(seed: u8, frequency: Option<f64>) -> calyx_core::Constellation {
    let mut scalars = BTreeMap::new();
    if let Some(frequency) = frequency {
        scalars.insert(FREQUENCY_SCALAR.to_string(), frequency);
    }
    calyx_core::Constellation {
        cx_id: cx(seed),
        vault_id: vault_id(),
        panel_version: 1,
        created_at: 1_000_000,
        input_ref: InputRef {
            hash: [seed; 32],
            pointer: Some(format!("zfs://calyx/issue392/{seed}")),
            redacted: true,
        },
        modality: Modality::Text,
        slots: BTreeMap::<SlotId, SlotVector>::new(),
        scalars,
        metadata: BTreeMap::new(),
        anchors: Vec::new(),
        provenance: LedgerRef {
            seq: seed as u64,
            hash: [seed; 32],
        },
        flags: CxFlags::default(),
    }
}

fn fsv_root() -> (PathBuf, bool) {
    if let Ok(root) = std::env::var("CALYX_ISSUE392_FSV_ROOT") {
        return (PathBuf::from(root), true);
    }
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time")
        .as_nanos();
    (
        std::env::temp_dir().join(format!("issue392-fsv-{}-{nonce}", std::process::id())),
        false,
    )
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn cx(seed: u8) -> CxId {
    CxId::from_bytes([seed; 16])
}
