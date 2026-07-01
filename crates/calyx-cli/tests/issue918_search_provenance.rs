use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use calyx_aster::cf::{CfRouter, ColumnFamily, base_key, ledger_key};
use calyx_aster::sst::write_sst;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

#[test]
fn cli_search_fails_closed_when_hit_ledger_row_is_missing() {
    let root = reset_root("issue918-cli-search-provenance");
    let vault = "issue918-cli";
    let text = "issue918 alpha provenance target";

    let create = calyx(
        &root,
        ["create-vault", vault, "--panel-template", "text-default"],
    );
    assert_success(&create);
    let vault_id = json(&create)["vault_id"].as_str().unwrap().to_string();
    let vault_path = root.join("vaults").join(&vault_id);

    let add_lens = calyx(
        &root,
        [
            "add-lens",
            vault,
            "--name",
            "issue918-byte",
            "--runtime",
            "algorithmic",
            "--modality",
            "text",
        ],
    );
    assert_success(&add_lens);

    let ingest = calyx(&root, ["ingest", vault, "--text", text]);
    assert_success(&ingest);
    let ingested = json(&ingest);
    let cx_id = ingested["cx_id"].as_str().unwrap().to_string();
    let ledger_seq = ingested["ledger_seq"].as_u64().unwrap();

    let happy = calyx(&root, ["search", vault, "alpha provenance", "--k", "1"]);
    assert_success(&happy);
    let happy_hits = json(&happy);
    assert_eq!(happy_hits[0]["cx_id"], cx_id);
    assert_eq!(happy_hits[0]["provenance"]["ledger_seq"], ledger_seq);
    let before = readback(&vault_path, &cx_id);

    remove_ledger_row(&vault_path, ledger_seq);
    remove_ledger_head_anchor(&vault_path);
    let after = readback(&vault_path, &cx_id);

    let failed = calyx(&root, ["search", vault, "alpha provenance", "--k", "1"]);
    assert!(!failed.status.success());
    let error = stderr_json(&failed);
    assert_eq!(error["code"], "CALYX_SEXTANT_PROVENANCE_MISSING");
    assert_eq!(before["base_exists"], true);
    assert_eq!(after["base_exists"], true);
    assert_eq!(after["ledger_rows"].as_array().unwrap().len(), 0);

    maybe_write_fsv_json(
        "cli-search-provenance-missing-ledger-fail-closed.json",
        &json!({
            "source_of_truth": "calyx CLI search over persisted search index, Aster Base CF, and Aster Ledger CF",
            "trigger": "remove hit Ledger CF row after successful CLI ingest/search, then run calyx search",
            "target": {
                "cx_id": cx_id,
                "ledger_seq": ledger_seq,
            },
            "before": before,
            "after": after,
            "happy_search": happy_hits,
            "error": error,
        }),
    );

    if calyx_fsv::fsv_root("CALYX_FSV_ROOT").is_none() {
        fs::remove_dir_all(root).unwrap();
    }
}

fn remove_ledger_row(vault: &Path, seq: u64) {
    let router = CfRouter::open(vault, 0).expect("open CF router");
    let key = ledger_key(seq);
    let mut rows = router
        .iter_cf(ColumnFamily::Ledger)
        .expect("read ledger rows");
    rows.retain(|row| row.key != key);
    let cf_dir = vault.join("cf").join(ColumnFamily::Ledger.name());
    for entry in fs::read_dir(&cf_dir).expect("read ledger CF directory") {
        let path = entry.expect("read ledger CF entry").path();
        if path.extension().and_then(|value| value.to_str()) == Some("sst") {
            fs::remove_file(path).expect("remove original ledger SST");
        }
    }
    if !rows.is_empty() {
        write_sst(
            cf_dir.join("00000000000000000001.sst"),
            rows.iter()
                .map(|entry| (entry.key.as_slice(), entry.value.as_slice())),
        )
        .expect("write rewritten ledger SST");
    }
    let wal_dir = vault.join("wal");
    if wal_dir.exists() {
        fs::remove_dir_all(wal_dir).expect("remove stale WAL");
    }
}

fn remove_ledger_head_anchor(vault: &Path) {
    let path = vault.join("ledger_head").join("current.json");
    if path.exists() {
        fs::remove_file(path).expect("remove ledger head anchor");
    }
}

fn readback(vault: &Path, cx_id: &str) -> Value {
    let parsed = cx_id.parse().expect("parse cx id");
    json!({
        "base_exists": cf_row_exists(vault, ColumnFamily::Base, &base_key(parsed)),
        "ledger_rows": ledger_rows(vault),
        "ledger_head_anchor_exists": vault.join("ledger_head").join("current.json").is_file(),
        "manifest": serde_json::from_slice::<Value>(
            &fs::read(vault.join("idx/search/manifest.json")).expect("read search manifest")
        ).expect("decode search manifest"),
    })
}

fn cf_row_exists(vault: &Path, cf: ColumnFamily, key: &[u8]) -> bool {
    CfRouter::open(vault, 0)
        .and_then(|router| router.iter_cf(cf))
        .map(|rows| rows.iter().any(|row| row.key == key))
        .unwrap_or(false)
}

fn ledger_rows(vault: &Path) -> Vec<Value> {
    CfRouter::open(vault, 0)
        .and_then(|router| router.iter_cf(ColumnFamily::Ledger))
        .unwrap_or_default()
        .into_iter()
        .map(|row| {
            json!({
                "seq": u64::from_be_bytes(row.key.as_slice().try_into().expect("ledger key")),
                "bytes_len": row.value.len(),
                "bytes_sha256": sha256_hex(&row.value),
            })
        })
        .collect()
}

fn calyx<I, S>(root: &Path, args: I) -> Output
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    Command::new(env!("CARGO_BIN_EXE_calyx"))
        .env("CALYX_HOME", root)
        .args(args)
        .output()
        .expect("run calyx")
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        stdout(output),
        stderr(output)
    );
}

fn json(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "parse stdout JSON: {error}\nstdout: {}\nstderr: {}",
            stdout(output),
            stderr(output)
        )
    })
}

fn stderr_json(output: &Output) -> Value {
    serde_json::from_slice(&output.stderr).unwrap_or_else(|error| {
        panic!(
            "parse stderr JSON: {error}\nstdout: {}\nstderr: {}",
            stdout(output),
            stderr(output)
        )
    })
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn maybe_write_fsv_json(name: &str, value: &Value) {
    let Some(root) = calyx_fsv::fsv_root("CALYX_FSV_ROOT") else {
        return;
    };
    fs::create_dir_all(&root).expect("create FSV root");
    fs::write(
        root.join(name),
        serde_json::to_vec_pretty(value).expect("serialize FSV"),
    )
    .expect("write FSV");
}

fn reset_root(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("calyx-cli-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("create root");
    root
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
