use std::fs;
use std::path::{Path, PathBuf};

use calyx_anneal::{
    AnnealLedgerAction, bandit_key, decode_anneal_ledger_payload, decode_config_bandit,
    index_slot_label, loom_plan_shape_key, shape_key_hash,
};
use calyx_aster::cf::ColumnFamily;
use calyx_aster::sst::SstReader;
use calyx_core::SlotId;
use calyx_ledger::{EntryKind, LedgerCfStore, decode};
use serde_json::{Value, json};

use crate::cf_read::{hex_bytes as hex, list_sst_files};
use crate::ledger_store::AsterLedgerCfStore;

pub(crate) fn run(args: &[String]) -> crate::error::CliResult {
    let request = ReportRequest::parse(args)?;
    request.validate()?;
    let cache = read_cache(&request.cache, &request)?;
    let promotions = read_promotions(&request.vault, &request)?;
    let recent_ab = read_recent_ab(&request.vault, &request)?;
    let bandit = request
        .shape_key()
        .map(|shape_key| read_bandit_status(&request.vault, &shape_key))
        .transpose()?;
    let loom_plan = loom_plan_summary(&cache.entries, &request);
    let storage_plans = storage_plans_summary(&cache.entries, &request, &request.vault)?;
    let report = json!({
        "scope": request.scope,
        "slot": request.slot,
        "shape_key": request.shape_key(),
        "cache": request.cache.display().to_string(),
        "vault": request.vault.display().to_string(),
        "last": request.last,
        "cache_bytes": cache.bytes,
        "cache_entries": cache.entries,
        "loom_plan": loom_plan,
        "storage_plans": storage_plans,
        "bandit": bandit,
        "recent_ab": recent_ab,
        "recent_promotions": promotions,
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&report).map_err(|error| error.to_string())?
    );
    Ok(())
}

struct ReportRequest {
    scope: String,
    cache: PathBuf,
    vault: PathBuf,
    last: usize,
    slot: Option<u16>,
}

impl ReportRequest {
    fn parse(args: &[String]) -> Result<Self, String> {
        let mut scope = None;
        let mut cache = None;
        let mut vault = None;
        let mut last = None;
        let mut slot = None;
        let mut idx = 0;
        while idx < args.len() {
            match args[idx].as_str() {
                "--scope" => {
                    scope = args.get(idx + 1).cloned();
                    idx += 2;
                }
                "--cache" => {
                    cache = args.get(idx + 1).map(PathBuf::from);
                    idx += 2;
                }
                "--vault" => {
                    vault = args.get(idx + 1).map(PathBuf::from);
                    idx += 2;
                }
                "--last" => {
                    last = Some(
                        args.get(idx + 1)
                            .ok_or_else(|| "--last requires a value".to_string())?
                            .parse::<usize>()
                            .map_err(|error| format!("invalid --last: {error}"))?,
                    );
                    idx += 2;
                }
                "--slot" => {
                    slot = Some(
                        args.get(idx + 1)
                            .ok_or_else(|| "--slot requires a value".to_string())?
                            .parse::<u16>()
                            .map_err(|error| format!("invalid --slot: {error}"))?,
                    );
                    idx += 2;
                }
                other => return Err(format!("unknown autotune-report arg: {other}")),
            }
        }
        let last = last.unwrap_or(5);
        if last == 0 {
            return Err("--last must be positive".to_string());
        }
        Ok(Self {
            scope: scope.ok_or_else(|| "autotune-report requires --scope".to_string())?,
            cache: cache.ok_or_else(|| "autotune-report requires --cache".to_string())?,
            vault: vault.ok_or_else(|| "autotune-report requires --vault".to_string())?,
            last,
            slot,
        })
    }

    fn validate(&self) -> Result<(), String> {
        match self.scope.as_str() {
            "forge" => Ok(()),
            "storage" if self.slot.is_none() => Ok(()),
            "storage" => Err("autotune-report --scope storage does not accept --slot".to_string()),
            "loom" if self.slot.is_none() => Ok(()),
            "loom" => Err("autotune-report --scope loom does not accept --slot".to_string()),
            "index" if self.slot.is_some() => Ok(()),
            "index" => Err("autotune-report --scope index requires --slot".to_string()),
            other => Err(format!(
                "autotune-report currently supports --scope forge, --scope index, --scope loom, or --scope storage, got {other}"
            )),
        }
    }

    fn shape_key(&self) -> Option<String> {
        match self.scope.as_str() {
            "forge" => None,
            "index" => self.slot.map(|slot| index_slot_label(SlotId::new(slot))),
            "loom" => Some(loom_plan_shape_key().to_string()),
            "storage" => None,
            _ => None,
        }
    }
}

struct CacheReport {
    bytes: usize,
    entries: Value,
}

fn read_cache(path: &Path, request: &ReportRequest) -> Result<CacheReport, String> {
    let bytes =
        fs::read(path).map_err(|error| format!("read cache {}: {error}", path.display()))?;
    let json: Value = serde_json::from_slice(&bytes)
        .map_err(|error| format!("parse cache {}: {error}", path.display()))?;
    let entries = json.get("entries").cloned().unwrap_or_else(|| json!([]));
    Ok(CacheReport {
        bytes: bytes.len(),
        entries: filter_cache_entries(entries, request),
    })
}

fn filter_cache_entries(entries: Value, request: &ReportRequest) -> Value {
    let Some(list) = entries.as_array() else {
        return json!([]);
    };
    Value::Array(
        list.iter()
            .filter(|entry| cache_entry_matches(entry, request))
            .cloned()
            .collect(),
    )
}

fn cache_entry_matches(entry: &Value, request: &ReportRequest) -> bool {
    match request.scope.as_str() {
        "forge" => entry["key"]["op"].as_str() != Some("index"),
        "loom" => entry["key"]["op"].as_str() == Some("loom"),
        "storage" => entry["key"]["op"].as_str() == Some("storage"),
        "index" => {
            let Some(slot) = request.slot else {
                return false;
            };
            entry["key"]["op"].as_str() == Some("index")
                && entry["key"]["shape"]
                    .as_array()
                    .and_then(|shape| shape.first())
                    .and_then(Value::as_u64)
                    == Some(u64::from(slot))
        }
        _ => false,
    }
}

fn read_promotions(vault: &Path, request: &ReportRequest) -> Result<Vec<Value>, String> {
    read_anneal_entries(vault, request, true)
}

fn read_recent_ab(vault: &Path, request: &ReportRequest) -> Result<Vec<Value>, String> {
    read_anneal_entries(vault, request, false)
}

fn read_anneal_entries(
    vault: &Path,
    request: &ReportRequest,
    promotions_only: bool,
) -> Result<Vec<Value>, String> {
    let store = AsterLedgerCfStore::open(vault).map_err(|error| error.to_string())?;
    let mut rows = Vec::new();
    for row in store.scan().map_err(|error| error.to_string())? {
        let entry = decode(&row.bytes).map_err(|error| error.to_string())?;
        if entry.kind != EntryKind::Anneal {
            continue;
        }
        let anneal =
            decode_anneal_ledger_payload(&entry.payload).map_err(|error| error.to_string())?;
        let action_matches = if promotions_only {
            anneal.action == AnnealLedgerAction::AutotunePromote
        } else {
            matches!(
                anneal.action,
                AnnealLedgerAction::AutotuneAB
                    | AnnealLedgerAction::AutotuneAbandoned
                    | AnnealLedgerAction::AutotunePromote
            )
        };
        if action_matches && promotion_matches(&anneal.artifact_id, request) {
            rows.push(json!({
                "seq": row.seq,
                "entry_hash": hex(&entry.entry_hash),
                "payload_hex": hex(&entry.payload),
                "payload_json": anneal,
            }));
        }
    }
    if request.last < rows.len() {
        rows.drain(0..rows.len() - request.last);
    }
    Ok(rows)
}

fn promotion_matches(artifact_id: &str, request: &ReportRequest) -> bool {
    match request.scope.as_str() {
        "forge" => artifact_id.starts_with("forge:"),
        "loom" => artifact_id == loom_plan_shape_key(),
        "storage" => artifact_id.starts_with("storage:"),
        "index" => request
            .slot
            .map(|slot| artifact_id == index_slot_label(SlotId::new(slot)))
            .unwrap_or(false),
        _ => false,
    }
}

fn storage_plans_summary(
    entries: &Value,
    request: &ReportRequest,
    vault: &Path,
) -> Result<Value, String> {
    if request.scope != "storage" {
        return Ok(Value::Null);
    }
    let Some(list) = entries.as_array() else {
        return Ok(json!({
            "found": false,
            "plans": []
        }));
    };
    let mut plans = Vec::new();
    for entry in list {
        let extra = &entry["config"]["extra"];
        let shape_key = extra_value(extra, "shape_key");
        let bandit = shape_key
            .as_str()
            .map(|value| read_bandit_status(vault, value))
            .transpose()?;
        plans.push(json!({
            "key": entry["key"].clone(),
            "shape_key": shape_key,
            "shape_key_hash": extra_value(extra, "shape_key_hash"),
            "vault_id": extra_value(extra, "vault_id"),
            "workload_id": extra_value(extra, "workload_id"),
            "config": storage_config_summary(extra),
            "bandit": bandit,
        }));
    }
    Ok(json!({
        "found": !plans.is_empty(),
        "plans": plans,
    }))
}

fn storage_config_summary(extra: &Value) -> Value {
    json!({
        "compaction_interval_ms": extra_value(extra, "compaction_interval_ms"),
        "debt_trigger_score_milli": extra_value(extra, "debt_trigger_score_milli"),
        "max_write_amp_milli": extra_value(extra, "max_write_amp_milli"),
        "hot_tier_min_hits": extra_value(extra, "hot_tier_min_hits"),
        "cold_tier_idle_secs": extra_value(extra, "cold_tier_idle_secs"),
        "codebook_refresh_secs": extra_value(extra, "codebook_refresh_secs"),
        "prefetch_bytes": extra_value(extra, "prefetch_bytes"),
    })
}

fn loom_plan_summary(entries: &Value, request: &ReportRequest) -> Value {
    if request.scope != "loom" {
        return Value::Null;
    }
    let Some(entry) = entries.as_array().and_then(|values| values.first()) else {
        return json!({
            "found": false,
            "eager_pairs_count": 0,
            "indexed_concat_keys_count": 0,
            "bits_sum": 0.0,
            "avg_latency_ns": 0
        });
    };
    let extra = &entry["config"]["extra"];
    json!({
        "found": true,
        "eager_pairs_count": extra_value(extra, "eager_pairs_count"),
        "indexed_concat_keys_count": extra_value(extra, "indexed_concat_keys_count"),
        "bits_sum": extra_value(extra, "bits_sum"),
        "avg_latency_ns": extra_value(extra, "avg_latency_ns"),
        "eager_pairs": extra_value(extra, "eager_pairs"),
        "indexed_concat_keys": extra_value(extra, "indexed_concat_keys"),
        "plan_hash": extra_value(extra, "plan_hash"),
    })
}

fn extra_value(extra: &Value, key: &str) -> Value {
    extra.get(key).cloned().unwrap_or(Value::Null)
}

fn read_bandit_status(vault: &Path, shape_key: &str) -> Result<Value, String> {
    let cf = ColumnFamily::AnnealBandit;
    let shape_hash = shape_key_hash(shape_key);
    let wanted_key = bandit_key(shape_hash);
    let mut physical_rows = Vec::new();
    let mut latest = None;
    for file in list_sst_files(&vault.join("cf").join(cf.name()))? {
        let reader = SstReader::open(&file).map_err(|error| error.to_string())?;
        for row in reader.iter().map_err(|error| error.to_string())? {
            let bandit = decode_config_bandit(&row.value).map_err(|error| error.to_string())?;
            let status = bandit
                .status(shape_hash)
                .map_err(|error| error.to_string())?;
            let readback = json!({
                "file": file.display().to_string(),
                "key_hex": hex(&row.key),
                "value_hex": hex(&row.value),
                "value_len": row.value.len(),
                "status": status,
            });
            if row.key == wanted_key {
                latest = Some(readback.clone());
            }
            physical_rows.push(readback);
        }
    }
    let status = latest.as_ref().and_then(|row| row.get("status")).cloned();
    Ok(json!({
        "cf": cf.name(),
        "shape_key": shape_key,
        "shape_key_hash": hex(&shape_hash),
        "key_hex": hex(&wanted_key),
        "found": latest.is_some(),
        "incumbent": status.as_ref().and_then(|value| value.get("incumbent")).cloned(),
        "arm_count": status.as_ref().and_then(|value| value.get("arm_count")).cloned(),
        "arms": status.as_ref().and_then(|value| value.get("arms")).cloned(),
        "row": latest,
        "physical_row_count": physical_rows.len(),
        "physical_rows": physical_rows,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn storage_scope_accepts_no_slot_and_has_multi_shape_bandit() {
        let request = request("storage", None);

        assert!(request.validate().is_ok());
        assert_eq!(request.shape_key(), None);
    }

    #[test]
    fn storage_scope_rejects_slot() {
        let error = request("storage", Some(0)).validate().unwrap_err();

        assert!(error.contains("--scope storage does not accept --slot"));
    }

    #[test]
    fn storage_cache_filter_keeps_only_storage_entries() {
        let request = request("storage", None);
        let entries = json!([
            {"key": {"op": "storage"}, "config": {"extra": {"shape_key": "storage:v:w:1"}}},
            {"key": {"op": "index", "shape": [0]}, "config": {"extra": {}}},
            {"key": {"op": "loom"}, "config": {"extra": {}}}
        ]);

        let filtered = filter_cache_entries(entries, &request);

        assert_eq!(filtered.as_array().unwrap().len(), 1);
        assert_eq!(filtered[0]["key"]["op"], "storage");
    }

    #[test]
    fn storage_config_summary_returns_all_storage_knobs() {
        let extra = json!({
            "compaction_interval_ms": "5000",
            "debt_trigger_score_milli": "750",
            "max_write_amp_milli": "1500",
            "hot_tier_min_hits": "4",
            "cold_tier_idle_secs": "3600",
            "codebook_refresh_secs": "900",
            "prefetch_bytes": "131072"
        });

        let summary = storage_config_summary(&extra);

        assert_eq!(summary["compaction_interval_ms"], "5000");
        assert_eq!(summary["hot_tier_min_hits"], "4");
        assert_eq!(summary["codebook_refresh_secs"], "900");
        assert_eq!(summary["prefetch_bytes"], "131072");
    }

    fn request(scope: &str, slot: Option<u16>) -> ReportRequest {
        ReportRequest {
            scope: scope.to_string(),
            cache: PathBuf::from("cache.json"),
            vault: PathBuf::from("vault"),
            last: 5,
            slot,
        }
    }
}
