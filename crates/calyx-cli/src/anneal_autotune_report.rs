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
    let bandit = request
        .shape_key()
        .map(|shape_key| read_bandit_status(&request.vault, &shape_key))
        .transpose()?;
    let loom_plan = loom_plan_summary(&cache.entries, &request);
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
        "bandit": bandit,
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
            "loom" if self.slot.is_none() => Ok(()),
            "loom" => Err("autotune-report --scope loom does not accept --slot".to_string()),
            "index" if self.slot.is_some() => Ok(()),
            "index" => Err("autotune-report --scope index requires --slot".to_string()),
            other => Err(format!(
                "autotune-report currently supports --scope forge, --scope index, or --scope loom, got {other}"
            )),
        }
    }

    fn shape_key(&self) -> Option<String> {
        match self.scope.as_str() {
            "forge" => None,
            "index" => self.slot.map(|slot| index_slot_label(SlotId::new(slot))),
            "loom" => Some(loom_plan_shape_key().to_string()),
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
    let store = AsterLedgerCfStore::open(vault).map_err(|error| error.to_string())?;
    let mut promotions = Vec::new();
    for row in store.scan().map_err(|error| error.to_string())? {
        let entry = decode(&row.bytes).map_err(|error| error.to_string())?;
        if entry.kind != EntryKind::Anneal {
            continue;
        }
        let anneal =
            decode_anneal_ledger_payload(&entry.payload).map_err(|error| error.to_string())?;
        if promotion_matches(&anneal.artifact_id, request)
            && anneal.action == AnnealLedgerAction::AutotunePromote
        {
            promotions.push(json!({
                "seq": row.seq,
                "entry_hash": hex(&entry.entry_hash),
                "payload_hex": hex(&entry.payload),
                "payload_json": anneal,
            }));
        }
    }
    if request.last < promotions.len() {
        promotions.drain(0..promotions.len() - request.last);
    }
    Ok(promotions)
}

fn promotion_matches(artifact_id: &str, request: &ReportRequest) -> bool {
    match request.scope.as_str() {
        "forge" => artifact_id.starts_with("forge:"),
        "loom" => artifact_id == loom_plan_shape_key(),
        "index" => request
            .slot
            .map(|slot| artifact_id == index_slot_label(SlotId::new(slot)))
            .unwrap_or(false),
        _ => false,
    }
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
