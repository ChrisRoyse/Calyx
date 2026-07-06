//! #1004 FSV: `calyx ingest` auto-discovers a running resident service via the
//! discovery file `panel resident serve` writes under `<CALYX_HOME>/resident/`,
//! and fails closed (`CALYX_INGEST_GPU_ROUTE_REQUIRED`) for GPU panels when no
//! resident route exists. Source of truth: Aster CF readback + the discovery
//! file on disk + runtime log phases, never process return values alone.

use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

use calyx_aster::cf::ColumnFamily;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{LensCost, Modality, Placement, QuantPolicy, SlotId, VaultId, VaultStore};
use calyx_registry::{LensForgeManifest, lens_spec_from_manifest_path};
use serde_json::{Value, json};

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

#[test]
fn batch_ingest_auto_discovers_resident_service_and_gates_cold_route() {
    let root = temp_root("resident-discovery-fsv");
    fs::create_dir_all(&root).expect("create resident-discovery FSV root");
    let template = "resident-discovery-fsv";
    write_algorithmic_catalog(&root, 10);
    run_ok(
        Command::new(calyx_exe())
            .arg("panel")
            .arg("template")
            .arg("save")
            .arg("--home")
            .arg(&root)
            .arg("--name")
            .arg(template)
            .arg("--all-current")
            .arg("--modality")
            .arg("text"),
        "save resident-discovery template",
    );
    let create = run_ok(
        Command::new(calyx_exe())
            .env("CALYX_HOME", &root)
            .arg("create-vault")
            .arg("resident-discovery-vault")
            .arg("--panel-template")
            .arg(template),
        "create resident-discovery vault",
    );
    let create_json: Value = serde_json::from_slice(&create.stdout).expect("parse create-vault");
    let vault_id: VaultId = create_json["vault_id"]
        .as_str()
        .expect("vault_id")
        .parse()
        .expect("parse vault id");
    let vault_path = root.join("vaults").join(vault_id.to_string());
    let discovery_path = root.join("resident").join("discovery.json");
    println!("resident_discovery_fsv_root={}", root.display());

    // Edge 1 (before any service): GPU panel + no route => fail closed, no writes.
    let before_any = cf_state(&vault_path, vault_id);
    let gated_batch = write_batch(&root, "gated.jsonl", &["discovery gate should refuse"]);
    let gated = ingest_no_flag(&root, &vault_path, &gated_batch);
    let after_gated = cf_state(&vault_path, vault_id);
    let gated_stderr = String::from_utf8_lossy(&gated.stderr);
    println!(
        "resident_discovery_fsv_edge_no_route status={:?} state_before={before_any} state_after={after_gated}",
        gated.status.code()
    );
    assert!(!gated.status.success());
    assert!(gated_stderr.contains("CALYX_INGEST_GPU_ROUTE_REQUIRED"));
    assert!(gated_stderr.contains("no_discovery_file"));
    assert_eq!(before_any, after_gated, "gated ingest must not persist");
    assert!(!discovery_path.exists());

    // Happy path: start service, ingest with NO --resident-addr, discovery routes it.
    let progress = root.join("resident-progress.jsonl");
    let mut service = spawn_resident_service(&root, template, &progress);
    let ready = read_ready(&mut service);
    let process_id = ready["process_id"].as_u64().expect("ready process_id");
    println!("resident_discovery_fsv_ready={ready}");
    let discovery_bytes = fs::read(&discovery_path).expect("discovery file must exist");
    let discovery: Value = serde_json::from_slice(&discovery_bytes).expect("parse discovery");
    println!("resident_discovery_fsv_discovery_file={discovery}");
    assert_eq!(
        discovery["schema"].as_str(),
        Some("calyx-panel-resident-discovery-v1")
    );
    assert_eq!(discovery["process_id"].as_u64(), Some(process_id));
    assert_eq!(discovery["bind"].as_str(), ready["bind"].as_str());
    assert_eq!(discovery["template"].as_str(), Some(template));

    let batch = write_batch(
        &root,
        "discovered.jsonl",
        &["discovery alpha", "discovery beta"],
    );
    let ingest = ingest_no_flag(&root, &vault_path, &batch);
    let ingest_stderr = String::from_utf8_lossy(&ingest.stderr);
    let after_ingest = cf_state(&vault_path, vault_id);
    println!(
        "resident_discovery_fsv_happy stdout={} state_after={after_ingest}",
        String::from_utf8_lossy(&ingest.stdout)
    );
    assert!(
        ingest.status.success(),
        "discovered ingest failed: stderr={ingest_stderr}"
    );
    let summary: Value = serde_json::from_slice(&ingest.stdout).expect("parse ingest summary");
    assert_eq!(summary["row_count"], 2);
    assert_eq!(summary["new_count"], 2);
    assert_eq!(summary["verified_base_rows"], 2);
    assert_eq!(after_ingest["base_rows"], 2);
    assert_eq!(after_ingest["slot_00_rows"], 2);
    assert!(ingest_stderr.contains("phase=gpu_route_discovered"));
    assert!(ingest_stderr.contains("phase=gpu_route source=discovery"));
    assert!(ingest_stderr.contains("phase=measure_resident_service_ok"));
    assert!(ingest_stderr.contains(&format!("process_id={process_id}")));
    assert!(!ingest_stderr.contains("phase=measure_lens_worker_resident_spawned"));

    // Stop the service: discovery file must be removed on graceful shutdown.
    run_ok(
        Command::new(calyx_exe())
            .arg("panel")
            .arg("resident")
            .arg("stop")
            .arg("--addr")
            .arg(ready["bind"].as_str().expect("bind")),
        "stop resident service",
    );
    let service_output = service.wait_with_output().expect("wait resident service");
    let service_stderr = String::from_utf8_lossy(&service_output.stderr);
    assert!(service_stderr.contains("phase=discovery_written"));
    assert!(
        !discovery_path.exists(),
        "graceful shutdown must remove the discovery file"
    );

    // Edge 2: stale discovery file (service gone) => fail closed with stale reason.
    fs::create_dir_all(discovery_path.parent().unwrap()).unwrap();
    let stale_addr = unused_loopback_addr();
    fs::write(
        &discovery_path,
        serde_json::to_vec_pretty(&json!({
            "schema": "calyx-panel-resident-discovery-v1",
            "bind": stale_addr,
            "process_id": 4_294_900_000u32,
            "template": template,
            "written_at_unix_ms": 0,
        }))
        .unwrap(),
    )
    .unwrap();
    let stale_batch = write_batch(&root, "stale.jsonl", &["stale discovery should refuse"]);
    let stale = ingest_no_flag(&root, &vault_path, &stale_batch);
    let stale_stderr = String::from_utf8_lossy(&stale.stderr);
    let after_stale = cf_state(&vault_path, vault_id);
    println!(
        "resident_discovery_fsv_edge_stale status={:?} state_before={after_ingest} state_after={after_stale}",
        stale.status.code()
    );
    assert!(!stale.status.success());
    assert!(stale_stderr.contains("CALYX_INGEST_GPU_ROUTE_REQUIRED"));
    assert!(stale_stderr.contains("stale_unreachable"));
    assert_eq!(after_stale, after_ingest, "stale route must not persist");

    // Edge 3: explicit cold opt-in flag passes the gate even with no service.
    fs::remove_file(&discovery_path).unwrap();
    let optin_batch = write_batch(&root, "optin.jsonl", &["cold opt-in row persists"]);
    let optin = run_ok(
        Command::new(calyx_exe())
            .env("CALYX_HOME", &root)
            .arg("ingest")
            .arg(&vault_path)
            .arg("--batch")
            .arg(&optin_batch)
            .arg("--allow-cold-gpu-workers"),
        "cold opt-in ingest",
    );
    let optin_stderr = String::from_utf8_lossy(&optin.stderr);
    let after_optin = cf_state(&vault_path, vault_id);
    println!("resident_discovery_fsv_edge_optin state_after={after_optin}");
    assert_eq!(after_optin["base_rows"], 3);
    assert!(optin_stderr.contains("allow_cold_gpu_workers=true"));
    assert!(optin_stderr.contains("phase=measure_lens_worker_resident_spawned"));

    if std::env::var("CALYX_KEEP_RESIDENT_DISCOVERY_FSV_ROOT").as_deref() == Ok("1") {
        println!("resident_discovery_fsv_preserved_root={}", root.display());
    } else {
        fs::remove_dir_all(root).ok();
    }
}

fn ingest_no_flag(root: &Path, vault_path: &Path, batch: &Path) -> std::process::Output {
    Command::new(calyx_exe())
        .env("CALYX_HOME", root)
        .arg("ingest")
        .arg(vault_path)
        .arg("--batch")
        .arg(batch)
        .output()
        .expect("run discovery ingest")
}

fn write_algorithmic_catalog(root: &Path, count: usize) {
    let manifest_root = root.join("manifests");
    fs::create_dir_all(&manifest_root).expect("create manifest dir");
    let entries = (0..count)
        .map(|idx| {
            let name = format!("resident-discovery-lens-{idx:02}");
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
            .expect("write algorithmic manifest");
            let spec = lens_spec_from_manifest_path(&manifest_path).expect("read manifest spec");
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
    let catalog = json!({ "lenses": entries });
    let catalog_path = root.join("lenses").join("registry.json");
    fs::create_dir_all(catalog_path.parent().unwrap()).expect("create catalog dir");
    fs::write(&catalog_path, serde_json::to_vec_pretty(&catalog).unwrap()).expect("write catalog");
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

fn hex32(bytes: &[u8; 32]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn spawn_resident_service(root: &Path, template: &str, progress: &Path) -> Child {
    Command::new(calyx_exe())
        .env("CALYX_HOME", root)
        .arg("panel")
        .arg("resident")
        .arg("serve")
        .arg("--home")
        .arg(root)
        .arg("--template")
        .arg(template)
        .arg("--bind")
        .arg("127.0.0.1:0")
        .arg("--progress-out")
        .arg(progress)
        .arg("--max-load-secs")
        .arg("30")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn resident service")
}

fn read_ready(child: &mut Child) -> Value {
    let stdout = child.stdout.take().expect("resident stdout pipe");
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    let read = reader
        .read_line(&mut line)
        .expect("read resident ready line");
    assert!(read > 0, "resident service exited before readiness");
    serde_json::from_str(line.trim()).expect("parse resident ready JSON")
}

fn write_batch(root: &Path, name: &str, texts: &[&str]) -> PathBuf {
    let path = root.join(name);
    let mut body = String::new();
    for text in texts {
        body.push_str(
            &serde_json::to_string(&json!({
                "text": *text,
                "metadata": provenance_metadata("resident-discovery-fsv", text),
            }))
            .unwrap(),
        );
        body.push('\n');
    }
    fs::write(&path, body).expect("write batch jsonl");
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

fn cf_state(vault_path: &Path, vault_id: VaultId) -> Value {
    let vault = AsterVault::new_durable(
        vault_path,
        vault_id,
        format!("calyx-cli-vault:{vault_id}:resident-discovery-vault").into_bytes(),
        VaultOptions::default(),
    )
    .expect("open durable resident-discovery FSV vault");
    let snapshot = vault.snapshot();
    json!({
        "snapshot": snapshot,
        "base_rows": vault.scan_cf_at(snapshot, ColumnFamily::Base).expect("scan Base CF").len(),
        "ledger_rows": vault.scan_cf_at(snapshot, ColumnFamily::Ledger).expect("scan Ledger CF").len(),
        "slot_00_rows": vault.scan_cf_at(snapshot, ColumnFamily::slot(SlotId::new(0))).expect("scan slot_00 CF").len(),
    })
}

fn run_ok(command: &mut Command, label: &str) -> std::process::Output {
    let output = command.output().unwrap_or_else(|error| {
        panic!("{label}: failed to spawn command: {error}");
    });
    assert!(
        output.status.success(),
        "{label}: status={:?}\nstdout={}\nstderr={}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn unused_loopback_addr() -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind unused loopback");
    let addr = listener.local_addr().expect("local addr");
    drop(listener);
    addr.to_string()
}

fn calyx_exe() -> PathBuf {
    std::env::var_os("CARGO_BIN_EXE_calyx")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/debug/calyx.exe")
        })
}

fn temp_root(name: &str) -> PathBuf {
    let id = NEXT_DIR.fetch_add(1, Ordering::SeqCst);
    std::env::temp_dir().join(format!("calyx-cli-{name}-{}-{id}", std::process::id()))
}
