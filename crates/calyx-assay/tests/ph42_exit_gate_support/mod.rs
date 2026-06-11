use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use calyx_aster::cf::{ColumnFamily, base_key};
use calyx_aster::dedup::EpochSecs;
use calyx_aster::recurrence::{
    FREQUENCY_SCALAR, OccurrenceContext, RetentionPolicy, append_occurrence,
};
use calyx_aster::vault::AsterVault;
use calyx_core::{
    Constellation, CxFlags, CxId, InputRef, LedgerRef, Modality, VaultId, VaultStore,
};
use calyx_lodestar::{KernelGraph, KernelGraphParams, NodeScore};
use calyx_paths::AssocGraph;
use serde_json::{Value, json};

pub fn append_times(vault: &AsterVault, cx_id: CxId, times: &[i64]) {
    for time in times {
        append_time(vault, cx_id, *time);
    }
}

pub fn append_time(vault: &AsterVault, cx_id: CxId, time: i64) {
    append_occurrence(
        vault,
        cx_id,
        EpochSecs(time),
        OccurrenceContext::new(format!("t={time}")).expect("context"),
        EpochSecs(time),
        RetentionPolicy::default(),
    )
    .expect("append occurrence");
}

pub fn put_base(vault: &AsterVault, cx_id: CxId, frequency: Option<f64>) {
    let mut cx = base_cx(cx_id);
    if let Some(frequency) = frequency {
        cx.scalars.insert(FREQUENCY_SCALAR.to_string(), frequency);
    }
    vault.put(cx).expect("put base");
}

pub fn kernel_graph(ids: &[CxId], betweenness: &[(CxId, f64)]) -> KernelGraph {
    let scores = betweenness
        .iter()
        .map(|(id, bet)| NodeScore {
            id: *id,
            degree_score: 0.0,
            betweenness_score: *bet,
            groundedness_distance: None,
            groundedness_score: 0.0,
            frequency_bonus: 0.0,
            total_score: *bet,
        })
        .collect::<Vec<_>>();
    KernelGraph {
        graph: assoc_graph(ids),
        selected: ids.to_vec(),
        source_fraction: 1.0,
        lp_fraction: None,
        params: KernelGraphParams::default(),
        scores,
        warnings: Vec::new(),
    }
}

pub fn raw_state(vault: &AsterVault) -> Value {
    json!({
        "snapshot": vault.latest_seq(),
        "base": raw_rows(vault, ColumnFamily::Base),
        "recurrence": raw_rows(vault, ColumnFamily::Recurrence),
        "temporal_xterm": raw_rows(vault, ColumnFamily::TemporalXTerm),
        "ledger": raw_rows(vault, ColumnFamily::Ledger)
    })
}

pub fn base_bytes(vault: &AsterVault, cx_id: CxId) -> Vec<u8> {
    vault
        .read_cf_at(vault.latest_seq(), ColumnFamily::Base, &base_key(cx_id))
        .expect("read base")
        .expect("base exists")
}

pub fn fsv_root() -> (PathBuf, bool) {
    if let Ok(root) = env::var("CALYX_ISSUE393_FSV_ROOT") {
        return (PathBuf::from(root), true);
    }
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time")
        .as_nanos();
    (
        env::temp_dir().join(format!("issue393-fsv-{}-{nonce}", std::process::id())),
        false,
    )
}

pub fn write_json(path: impl AsRef<Path>, value: &Value) {
    fs::write(
        path,
        serde_json::to_vec_pretty(value).expect("serialize json"),
    )
    .expect("write json");
}

pub fn write_blake3_sums(root: &Path) {
    let mut files = Vec::new();
    collect_files(root, root, &mut files);
    files.sort();
    let mut lines = String::new();
    for relative in files {
        if relative == Path::new("BLAKE3SUMS.txt") {
            continue;
        }
        let bytes = fs::read(root.join(&relative)).expect("read checksum file");
        lines.push_str(&format!(
            "{}  {}\n",
            blake3::hash(&bytes).to_hex(),
            relative.to_string_lossy().replace('\\', "/")
        ));
    }
    fs::write(root.join("BLAKE3SUMS.txt"), lines).expect("write sums");
}

pub fn reset_dir(path: &Path) {
    if path.exists() {
        fs::remove_dir_all(path).expect("reset fsv root");
    }
    fs::create_dir_all(path).expect("create fsv root");
}

pub fn contains_subslice(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

pub fn temporal_key_hex(cx_a: CxId, cx_b: CxId) -> String {
    let mut key = Vec::with_capacity(32);
    key.extend_from_slice(cx_a.as_bytes());
    key.extend_from_slice(cx_b.as_bytes());
    hex(&key)
}

pub fn ids(start: u8, count: u8) -> Vec<CxId> {
    (start..start + count).map(cx).collect()
}

pub fn cx(seed: u8) -> CxId {
    CxId::from_bytes([seed; 16])
}

pub fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV"
        .parse()
        .expect("valid vault id")
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn assoc_graph(ids: &[CxId]) -> AssocGraph {
    let mut builder = AssocGraph::builder();
    for id in ids {
        builder.add_node(*id, 1.0).expect("node");
    }
    for pair in ids.windows(2) {
        builder.add_edge(pair[0], pair[1], 1.0).expect("edge");
    }
    builder.build()
}

fn raw_rows(vault: &AsterVault, cf: ColumnFamily) -> Value {
    let rows = vault.scan_cf_at(vault.latest_seq(), cf).expect("scan cf");
    json!({
        "row_count": rows.len(),
        "rows": rows.iter().map(|(key, value)| {
            json!({
                "key_hex": hex(key),
                "value_blake3": blake3::hash(value).to_hex().to_string(),
                "value_len": value.len(),
                "value_hex": hex(value)
            })
        }).collect::<Vec<_>>()
    })
}

fn base_cx(cx_id: CxId) -> Constellation {
    Constellation {
        cx_id,
        vault_id: vault_id(),
        panel_version: 42,
        created_at: 1_786_406_600,
        input_ref: InputRef {
            hash: [cx_id.to_bytes()[0]; 32],
            pointer: None,
            redacted: false,
        },
        modality: Modality::Text,
        slots: BTreeMap::new(),
        scalars: BTreeMap::new(),
        metadata: BTreeMap::new(),
        anchors: Vec::new(),
        provenance: LedgerRef {
            seq: 0,
            hash: [0; 32],
        },
        flags: CxFlags::default(),
    }
}

fn collect_files(root: &Path, dir: &Path, files: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("read dir") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            collect_files(root, &path, files);
        } else {
            files.push(path.strip_prefix(root).expect("relative").to_path_buf());
        }
    }
}
