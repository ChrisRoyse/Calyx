use std::fs;
use std::io::{BufRead, BufReader};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

use calyx_aster::cf::ColumnFamily;
use calyx_aster::vault::encode::decode_constellation_base;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{
    Constellation, LensCost, Modality, Placement, QuantPolicy, SlotId, VaultId, VaultStore,
};
use calyx_registry::{LensForgeManifest, lens_spec_from_manifest_path};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

pub(super) fn write_algorithmic_catalog(root: &Path, count: usize) {
    let manifest_root = root.join("manifests");
    fs::create_dir_all(&manifest_root).expect("create manifest dir");
    let entries = (0..count)
        .map(|idx| {
            let name = format!("search-resident-lens-{idx:02}");
            let manifest_path = manifest_root.join(format!("{name}.json"));
            let manifest = LensForgeManifest {
                name: name.clone(),
                modality: Modality::Text,
                runtime: "algorithmic:byte-features".to_string(),
                dim: 16,
                shape: None,
                dtype: "f32".to_string(),
                weights_sha256: String::new(),
                artifact_set_sha256: None,
                files: Vec::new(),
                pooling: "algorithmic".to_string(),
                norm: "none".to_string(),
                source_hf_id: format!("calyx/{name}"),
                endpoint: None,
                license: Some("apache-2.0".to_string()),
                non_commercial: false,
                quant_default: QuantPolicy::None,
                truncate_dim: None,
                recall_delta: calyx_registry::spec::default_recall_delta(),
                max_batch: Some(4),
                batch_policy: None,
            };
            fs::write(
                &manifest_path,
                serde_json::to_vec_pretty(&manifest).unwrap(),
            )
            .unwrap();
            let spec = lens_spec_from_manifest_path(&manifest_path).unwrap();
            json!({
                "lens_id": spec.lens_id().to_string(),
                "name": name,
                "modality": "text",
                "runtime": "algorithmic",
                "dim": 16,
                "weights_sha256": hex32(&spec.weights_sha256),
                "manifest": manifest_path,
                "cost": LensCost::default(),
                "placement": Placement::Gpu,
            })
        })
        .collect::<Vec<_>>();
    let catalog_path = root.join("lenses").join("registry.json");
    fs::create_dir_all(catalog_path.parent().unwrap()).unwrap();
    fs::write(
        &catalog_path,
        serde_json::to_vec_pretty(&json!({ "lenses": entries })).unwrap(),
    )
    .unwrap();
    let migrate = Command::new(calyx_exe())
        .arg("lens")
        .arg("migrate-catalog")
        .arg("--home")
        .arg(root)
        .output()
        .expect("migrate algorithmic catalog to DB");
    assert!(
        migrate.status.success(),
        "migrate catalog failed: stdout={} stderr={}",
        String::from_utf8_lossy(&migrate.stdout),
        String::from_utf8_lossy(&migrate.stderr)
    );
    fs::remove_file(catalog_path).expect("remove legacy catalog after DB migration");
}

pub(super) fn spawn_template_resident_service(
    root: &Path,
    template: &str,
    progress: &Path,
) -> Child {
    let mut command = resident_command(root, progress);
    command.arg("--template").arg(template);
    command.spawn().expect("spawn template resident service")
}

pub(super) fn spawn_vault_resident_service(root: &Path, vault: &Path, progress: &Path) -> Child {
    let mut command = resident_command(root, progress);
    command.arg("--vault").arg(vault);
    command.spawn().expect("spawn vault resident service")
}

pub(super) fn read_ready(child: &mut Child) -> Value {
    let stdout = child.stdout.take().expect("resident stdout pipe");
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    assert!(reader.read_line(&mut line).unwrap() > 0);
    serde_json::from_str(line.trim()).expect("parse resident ready")
}

pub(super) fn stop_resident_service(addr: &str, mut child: Child) -> Output {
    let stop = Command::new(calyx_exe())
        .arg("panel")
        .arg("resident")
        .arg("stop")
        .arg("--addr")
        .arg(addr)
        .output()
        .expect("stop resident service");
    if !stop.status.success() {
        child.kill().ok();
    }
    child.wait_with_output().expect("wait resident service")
}

pub(super) fn write_batch(root: &Path, name: &str, texts: &[&str]) -> PathBuf {
    let path = root.join(name);
    let mut body = String::new();
    for text in texts {
        body.push_str(
            &serde_json::to_string(&json!({
                "text": *text,
                "metadata": provenance_metadata("search-resident-fsv", text),
            }))
            .unwrap(),
        );
        body.push('\n');
    }
    fs::write(&path, body).unwrap();
    path
}

fn provenance_metadata(dataset: &str, text: &str) -> Value {
    let slug = provenance_slug(text);
    json!({
        "source_dataset": dataset,
        "source_sha256": format!("sha256-{slug}"),
        "source_url": format!("https://example.test/{dataset}/{slug}"),
        "license": "CC-BY-4.0",
        "retrieval_ts": "2026-07-04T00:00:00Z",
    })
}

fn provenance_slug(text: &str) -> String {
    text.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect()
}

pub(super) fn cf_state(vault_path: &Path, vault_id: VaultId, name: &str) -> Value {
    let vault = AsterVault::new_durable(
        vault_path,
        vault_id,
        format!("calyx-cli-vault:{vault_id}:{name}").into_bytes(),
        VaultOptions::default(),
    )
    .expect("open durable search-resident FSV vault");
    let snapshot = vault.snapshot();
    let base = vault.scan_cf_at(snapshot, ColumnFamily::Base).unwrap();
    let base_docs = base
        .iter()
        .map(|(key, bytes)| {
            let decoded: Constellation = decode_constellation_base(bytes).unwrap();
            json!({
                "key": hex_lower(key),
                "cx_id": decoded.cx_id.to_string(),
                "slot_count": decoded.slots.len(),
                "ledger_seq": decoded.provenance.seq,
            })
        })
        .collect::<Vec<_>>();
    json!({
        "snapshot": snapshot,
        "base_rows": base.len(),
        "base_ids": base.iter().map(|(key, _)| hex_lower(key)).collect::<Vec<_>>(),
        "base_docs": base_docs,
        "ledger_rows": vault.scan_cf_at(snapshot, ColumnFamily::Ledger).unwrap().len(),
        "slot_00_rows": vault.scan_cf_at(snapshot, ColumnFamily::slot(SlotId::new(0))).unwrap().len(),
        "slot_01_rows": vault.scan_cf_at(snapshot, ColumnFamily::slot(SlotId::new(1))).unwrap().len(),
        "slot_09_rows": vault.scan_cf_at(snapshot, ColumnFamily::slot(SlotId::new(9))).unwrap().len(),
    })
}

pub(super) fn search_index_state(vault_path: &Path) -> Value {
    let manifest_path = vault_path.join("idx").join("search").join("manifest.json");
    let manifest_bytes = fs::read(&manifest_path).expect("read search manifest");
    let manifest: Value = serde_json::from_slice(&manifest_bytes).unwrap();
    let mut artifacts = Vec::new();
    if let Some(filter) = manifest["filter"].as_object() {
        artifacts.push(artifact_state(
            vault_path,
            "filter",
            filter["index_rel"].as_str().unwrap(),
        ));
    }
    for entry in manifest["slots"].as_array().unwrap() {
        for key in ["index_rel", "graph_rel", "id_map_rel"] {
            if let Some(rel) = entry[key].as_str() {
                artifacts.push(artifact_state(vault_path, key, rel));
            }
        }
    }
    json!({
        "manifest_path": manifest_path,
        "manifest_sha256": sha256_hex(&manifest_bytes),
        "manifest": manifest,
        "artifacts": artifacts,
    })
}

pub(super) fn assert_index_matches_manifest(index: &Value) {
    assert_eq!(
        index["manifest"]["format"],
        "calyx-search-index-manifest-v1"
    );
    let slots = index["manifest"]["slots"].as_array().unwrap();
    assert_eq!(slots.len(), 10);
    for entry in slots {
        assert_eq!(entry["kind"], "flat_dense");
        assert_eq!(entry["len"], 3);
        let rel = entry["index_rel"].as_str().unwrap();
        let artifact = index["artifacts"]
            .as_array()
            .unwrap()
            .iter()
            .find(|artifact| artifact["rel"] == rel)
            .expect("flat dense artifact listed");
        assert_eq!(artifact["sha256"], entry["sha256"]);
        assert!(artifact["bytes"].as_u64().unwrap() > 16);
    }
}

pub(super) fn run_ok(command: &mut Command, label: &str) -> Output {
    let output = command
        .output()
        .unwrap_or_else(|error| panic!("{label}: {error}"));
    assert!(
        output.status.success(),
        "{label}: status={:?}\nstdout={}\nstderr={}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

pub(super) fn run_fail(command: &mut Command, label: &str) -> Output {
    let output = command
        .output()
        .unwrap_or_else(|error| panic!("{label}: {error}"));
    assert!(
        !output.status.success(),
        "{label}: unexpectedly succeeded\nstdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

pub(super) fn parse_json(bytes: &[u8]) -> Value {
    serde_json::from_slice(bytes).unwrap_or_else(|_| json!(String::from_utf8_lossy(bytes)))
}

pub(super) fn unused_loopback_addr() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    addr.to_string()
}

pub(super) fn calyx_exe() -> PathBuf {
    std::env::var_os("CARGO_BIN_EXE_calyx")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/debug/calyx.exe")
        })
}

pub(super) fn temp_root(name: &str) -> PathBuf {
    let id = NEXT_DIR.fetch_add(1, Ordering::SeqCst);
    std::env::temp_dir().join(format!("calyx-cli-{name}-{}-{id}", std::process::id()))
}

fn resident_command(root: &Path, progress: &Path) -> Command {
    let mut command = Command::new(calyx_exe());
    command
        .env("CALYX_HOME", root)
        .arg("panel")
        .arg("resident")
        .arg("serve")
        .arg("--home")
        .arg(root)
        .arg("--bind")
        .arg("127.0.0.1:0")
        .arg("--progress-out")
        .arg(progress)
        .arg("--max-load-secs")
        .arg("30")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

fn artifact_state(vault_path: &Path, kind: &str, rel: &str) -> Value {
    let path = vault_path.join(rel);
    let bytes = fs::read(&path).unwrap_or_else(|error| panic!("read artifact {rel}: {error}"));
    json!({
        "kind": kind,
        "rel": rel,
        "bytes": bytes.len(),
        "sha256": sha256_hex(&bytes),
    })
}

fn hex32(bytes: &[u8; 32]) -> String {
    hex_lower(bytes)
}

fn hex_lower(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex_lower(&hasher.finalize())
}
