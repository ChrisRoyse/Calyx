use std::fs;
use std::io::{BufRead, BufReader};
use std::net::TcpListener;
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
fn batch_ingest_uses_existing_resident_service_and_persists_cfs() {
    let root = temp_root("resident-service-fsv");
    fs::create_dir_all(&root).expect("create resident-service FSV root");
    let template = "resident-service-fsv";
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
        "save resident-service template",
    );
    let create = run_ok(
        Command::new(calyx_exe())
            .env("CALYX_HOME", &root)
            .arg("create-vault")
            .arg("resident-service-vault")
            .arg("--panel-template")
            .arg(template),
        "create resident-service vault",
    );
    let create_json: Value = serde_json::from_slice(&create.stdout).expect("parse create-vault");
    let vault_id: VaultId = create_json["vault_id"]
        .as_str()
        .expect("vault_id")
        .parse()
        .expect("parse vault id");
    let vault_path = root.join("vaults").join(vault_id.to_string());
    println!("resident_service_fsv_root={}", root.display());
    println!("resident_service_fsv_vault={}", vault_path.display());

    let before = cf_state(&vault_path, vault_id, "resident-service-vault");
    println!(
        "resident_service_fsv_before source_of_truth=Aster CF readback state={}",
        before
    );

    let progress = root.join("resident-progress.jsonl");
    let mut service = spawn_resident_service(&root, template, &progress);
    let ready = read_ready(&mut service);
    let addr = ready["bind"].as_str().expect("ready bind").to_string();
    let process_id = ready["process_id"].as_u64().expect("ready process_id");
    println!("resident_service_fsv_ready={ready}");

    let batch1 = write_batch(
        &root,
        "resident-service-1.jsonl",
        &["resident service alpha", "resident service beta"],
    );
    let ingest1 = run_ok(
        Command::new(calyx_exe())
            .env("CALYX_HOME", &root)
            .arg("ingest")
            .arg(&vault_path)
            .arg("--batch")
            .arg(&batch1)
            .arg("--resident-addr")
            .arg(&addr),
        "ingest first resident-service batch",
    );
    println!(
        "resident_service_fsv_ingest1_stdout={}",
        String::from_utf8_lossy(&ingest1.stdout)
    );
    println!(
        "resident_service_fsv_ingest1_stderr={}",
        String::from_utf8_lossy(&ingest1.stderr)
    );
    let after_first = cf_state(&vault_path, vault_id, "resident-service-vault");
    println!(
        "resident_service_fsv_after_first source_of_truth=Aster CF readback state={}",
        after_first
    );

    let batch2 = write_batch(
        &root,
        "resident-service-2.jsonl",
        &["resident service gamma", "resident service delta"],
    );
    let ingest2 = run_ok(
        Command::new(calyx_exe())
            .env("CALYX_HOME", &root)
            .arg("ingest")
            .arg(&vault_path)
            .arg("--batch")
            .arg(&batch2)
            .arg("--resident-addr")
            .arg(&addr),
        "ingest second resident-service batch",
    );
    println!(
        "resident_service_fsv_ingest2_stdout={}",
        String::from_utf8_lossy(&ingest2.stdout)
    );
    println!(
        "resident_service_fsv_ingest2_stderr={}",
        String::from_utf8_lossy(&ingest2.stderr)
    );
    let after_second = cf_state(&vault_path, vault_id, "resident-service-vault");
    println!(
        "resident_service_fsv_after_second source_of_truth=Aster CF readback state={}",
        after_second
    );

    let missing_addr = unused_loopback_addr();
    let failed_batch = write_batch(
        &root,
        "resident-service-unavailable.jsonl",
        &["resident service should not persist"],
    );
    let failed = Command::new(calyx_exe())
        .env("CALYX_HOME", &root)
        .arg("ingest")
        .arg(&vault_path)
        .arg("--batch")
        .arg(&failed_batch)
        .arg("--resident-addr")
        .arg(&missing_addr)
        .output()
        .expect("run unavailable resident-service ingest");
    let after_failed = cf_state(&vault_path, vault_id, "resident-service-vault");
    println!(
        "resident_service_fsv_unavailable_status={:?} stderr={} state_before={} state_after={}",
        failed.status.code(),
        String::from_utf8_lossy(&failed.stderr),
        after_second,
        after_failed
    );
    let failed_text = Command::new(calyx_exe())
        .env("CALYX_HOME", &root)
        .arg("ingest")
        .arg(&vault_path)
        .arg("--text")
        .arg("resident service stopped text should not persist")
        .arg("--resident-addr")
        .arg(&missing_addr)
        .output()
        .expect("run unavailable resident-service text ingest");
    let after_failed_text = cf_state(&vault_path, vault_id, "resident-service-vault");
    println!(
        "resident_service_fsv_unavailable_text_status={:?} stderr={} state_before={} state_after={}",
        failed_text.status.code(),
        String::from_utf8_lossy(&failed_text.stderr),
        after_failed,
        after_failed_text
    );

    let stop = Command::new(calyx_exe())
        .arg("panel")
        .arg("resident")
        .arg("stop")
        .arg("--addr")
        .arg(&addr)
        .output()
        .expect("stop resident service");
    if !stop.status.success() {
        service.kill().ok();
    }
    let service_output = service
        .wait_with_output()
        .expect("wait for resident service");
    println!(
        "resident_service_fsv_service_stderr={}",
        String::from_utf8_lossy(&service_output.stderr)
    );
    let progress_text = fs::read_to_string(&progress).expect("read resident progress log");
    println!("resident_service_fsv_progress_log={progress_text}");

    let first_summary: Value = serde_json::from_slice(&ingest1.stdout).expect("parse ingest1");
    let second_summary: Value = serde_json::from_slice(&ingest2.stdout).expect("parse ingest2");
    assert_eq!(first_summary["row_count"], 2);
    assert_eq!(first_summary["new_count"], 2);
    assert_eq!(first_summary["verified_base_rows"], 2);
    assert_eq!(second_summary["row_count"], 2);
    assert_eq!(second_summary["new_count"], 2);
    assert_eq!(second_summary["verified_base_rows"], 2);
    assert_eq!(before["base_rows"], 0);
    assert_eq!(after_first["base_rows"], 2);
    assert_eq!(after_first["slot_00_rows"], 2);
    assert_eq!(after_first["slot_09_rows"], 2);
    assert_eq!(after_second["base_rows"], 4);
    assert_eq!(after_second["slot_00_rows"], 4);
    assert_eq!(after_second["slot_09_rows"], 4);
    assert_eq!(after_second["ledger_rows"], 2);
    assert!(!failed.status.success());
    assert_eq!(after_failed, after_second);
    assert!(!failed_text.status.success());
    assert_eq!(after_failed_text, after_failed);

    let ingest1_stderr = String::from_utf8_lossy(&ingest1.stderr);
    let ingest2_stderr = String::from_utf8_lossy(&ingest2.stderr);
    let failed_text_stderr = String::from_utf8_lossy(&failed_text.stderr);
    assert!(ingest1_stderr.contains("phase=measure_resident_service_ok"));
    assert!(ingest2_stderr.contains("phase=measure_resident_service_ok"));
    assert!(ingest1_stderr.contains("protocol=binary"));
    assert!(ingest2_stderr.contains("protocol=binary"));
    assert!(ingest1_stderr.contains("request_bytes="));
    assert!(ingest1_stderr.contains("response_bytes="));
    assert!(ingest2_stderr.contains("request_bytes="));
    assert!(ingest2_stderr.contains("response_bytes="));
    assert!(ingest1_stderr.contains(&format!("process_id={process_id}")));
    assert!(ingest2_stderr.contains(&format!("process_id={process_id}")));
    assert!(!ingest1_stderr.contains("phase=measure_lens_worker_resident_spawned"));
    assert!(!ingest2_stderr.contains("phase=measure_lens_worker_resident_spawned"));
    assert!(failed_text_stderr.contains("CALYX_PANEL_RESIDENT_UNAVAILABLE"));
    assert!(failed_text_stderr.contains("phase=measure_resident_service_gate"));
    assert!(!failed_text_stderr.contains("phase=measure_lens_worker_resident_spawned"));
    let service_stderr = String::from_utf8_lossy(&service_output.stderr);
    assert!(service_stderr.contains("phase=measure_batch_binary_request"));
    assert!(service_stderr.contains("phase=measure_batch_binary_response"));
    assert_eq!(
        progress_text
            .matches("\"phase\":\"runtime_prepare_start\"")
            .count(),
        10
    );
    assert_eq!(progress_text.matches("\"phase\":\"prime_ok\"").count(), 10);

    if std::env::var("CALYX_KEEP_RESIDENT_SERVICE_FSV_ROOT").as_deref() == Ok("1") {
        println!("resident_service_fsv_preserved_root={}", root.display());
    } else {
        fs::remove_dir_all(root).ok();
    }
}

fn write_algorithmic_catalog(root: &Path, count: usize) {
    let manifest_root = root.join("manifests");
    fs::create_dir_all(&manifest_root).expect("create manifest dir");
    let entries = (0..count)
        .map(|idx| {
            let name = format!("resident-service-lens-{idx:02}");
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
                "metadata": provenance_metadata("resident-service-fsv", text),
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

fn cf_state(vault_path: &Path, vault_id: VaultId, name: &str) -> Value {
    let vault = AsterVault::new_durable(
        vault_path,
        vault_id,
        format!("calyx-cli-vault:{vault_id}:{name}").into_bytes(),
        VaultOptions::default(),
    )
    .expect("open durable resident-service FSV vault");
    let snapshot = vault.snapshot();
    json!({
        "snapshot": snapshot,
        "base_rows": vault.scan_cf_at(snapshot, ColumnFamily::Base).expect("scan Base CF").len(),
        "ledger_rows": vault.scan_cf_at(snapshot, ColumnFamily::Ledger).expect("scan Ledger CF").len(),
        "slot_00_rows": vault.scan_cf_at(snapshot, ColumnFamily::slot(SlotId::new(0))).expect("scan slot_00 CF").len(),
        "slot_09_rows": vault.scan_cf_at(snapshot, ColumnFamily::slot(SlotId::new(9))).expect("scan slot_09 CF").len(),
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
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind unused loopback");
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
