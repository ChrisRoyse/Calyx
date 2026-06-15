use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use calyx_core::{FixedClock, VaultId};
use calyx_ledger::{ActorId, DirectoryLedgerStore, LedgerAppender, LedgerCfStore, decode};
use serde_json::{Value, json};

use super::{FSV_TS, RSS_BUDGET_KIB};
use crate::model::EvaluatedSlot;

pub fn fsv_root() -> PathBuf {
    std::env::var("CALYX_FSV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            std::env::temp_dir().join(format!("issue792-fsv-{}", std::process::id()))
        })
}

pub fn keep_root() -> bool {
    std::env::var_os("CALYX_FSV_ROOT").is_some()
}

pub fn reset_dir(path: &Path) {
    let _ = fs::remove_dir_all(path);
    fs::create_dir_all(path).unwrap();
}

pub fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}

pub fn write_json(path: &Path, value: &impl serde::Serialize) {
    fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
}

pub fn append_decision_ledger(root: &Path, evals: &[EvaluatedSlot]) -> Vec<Value> {
    let ledger_dir = root.join("capability-ledger");
    let mut appender = LedgerAppender::open(
        DirectoryLedgerStore::open(&ledger_dir).unwrap(),
        FixedClock::new(FSV_TS),
    )
    .unwrap();
    for evaluated in evals {
        calyx_registry::append_capability_gate_ledger(
            &mut appender,
            &evaluated.evaluation,
            ActorId::Service("issue792-stage-exit-fsv".to_string()),
        )
        .unwrap();
    }
    DirectoryLedgerStore::open(&ledger_dir)
        .unwrap()
        .scan()
        .unwrap()
        .into_iter()
        .map(|row| {
            let entry = decode(&row.bytes).unwrap();
            json!({
                "seq": row.seq,
                "entry_hash": hex(&entry.entry_hash),
                "payload": serde_json::from_slice::<Value>(&entry.payload).unwrap()
            })
        })
        .collect()
}

pub fn footprint_readback() -> Value {
    let rss_kib = rss_kib();
    let gpu = nvidia_smi();
    json!({
        "rss": {
            "kib": rss_kib,
            "budget_kib": RSS_BUDGET_KIB,
            "within_budget": rss_kib <= RSS_BUDGET_KIB
        },
        "gpu": gpu
    })
}

pub fn write_physical_files(path: &Path, root: &Path) {
    let mut lines = Vec::new();
    collect_files(root, root, &mut lines);
    lines.sort();
    fs::write(path, lines.join("\n")).unwrap();
}

pub fn write_manifest(root: &Path) {
    let manifest = root.join("BLAKE3SUMS.txt");
    let mut files = Vec::new();
    collect_manifest_files(root, root, &manifest, &mut files);
    files.sort();
    let lines = files
        .into_iter()
        .map(|path| {
            let bytes = fs::read(root.join(&path)).unwrap();
            format!("{}  {}\n", blake3::hash(&bytes).to_hex(), path.display())
        })
        .collect::<String>();
    fs::write(manifest, lines).unwrap();
}

fn rss_kib() -> u64 {
    fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|text| {
            text.lines()
                .find(|line| line.starts_with("VmRSS:"))
                .and_then(|line| line.split_whitespace().nth(1))
                .and_then(|value| value.parse().ok())
        })
        .unwrap_or(0)
}

fn nvidia_smi() -> Value {
    let output = Command::new("nvidia-smi")
        .args([
            "--query-gpu=memory.used,memory.total",
            "--format=csv,noheader,nounits",
        ])
        .output();
    match output {
        Ok(output) if output.status.success() => parse_nvidia_smi(&output.stdout),
        Ok(output) => json!({
            "available": false,
            "error": String::from_utf8_lossy(&output.stderr).trim()
        }),
        Err(error) => json!({"available": false, "error": error.to_string()}),
    }
}

fn parse_nvidia_smi(stdout: &[u8]) -> Value {
    let text = String::from_utf8_lossy(stdout);
    let first = text.lines().next().unwrap_or_default();
    let parts = first.split(',').map(str::trim).collect::<Vec<_>>();
    if parts.len() != 2 {
        return json!({"available": false, "error": "unparsed nvidia-smi output"});
    }
    let used: u64 = parts[0].parse().unwrap_or(0);
    let total: u64 = parts[1].parse().unwrap_or(0);
    json!({
        "available": true,
        "used_mib": used,
        "budget_mib": total,
        "total_mib": total,
        "within_budget": used <= total
    })
}

fn collect_files(root: &Path, dir: &Path, out: &mut Vec<String>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            collect_files(root, &path, out);
        } else {
            out.push(format!(
                "{} bytes {}",
                fs::metadata(&path).unwrap().len(),
                path.strip_prefix(root).unwrap().display()
            ));
        }
    }
}

fn collect_manifest_files(root: &Path, dir: &Path, manifest: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path == manifest {
            continue;
        }
        if path.is_dir() {
            collect_manifest_files(root, &path, manifest, out);
        } else {
            out.push(path.strip_prefix(root).unwrap().to_path_buf());
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
