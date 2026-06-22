use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use calyx_forge::{
    CompressionReport, CompressionReportInput, CompressionSlotMeasurement,
    KernelCompressionMeasurement, QuantLevel, Quantizer, TurboQuantCodec, compression_report,
    new_seed,
};
use serde_json::{Value, json};

mod support;
use support::fsv_io::write_json;

const ARTIFACT_SOURCE_OF_TRUTH: &str = "PH59 compression report artifact";

#[test]
#[ignore = "requires CALYX_ISSUE613_FSV_ROOT in a manual verification run"]
fn issue613_compression_report_full_doc23_fsv() {
    let root = fsv_root();
    assert!(
        !root.exists(),
        "choose a fresh CALYX_ISSUE613_FSV_ROOT; already exists: {}",
        root.display()
    );
    fs::create_dir_all(&root).expect("create FSV root");
    let vault = root.join("vault");
    let reports = root.join("reports");
    fs::create_dir_all(&vault).expect("create vault root");
    fs::create_dir_all(&reports).expect("create reports root");

    let quantized_before = directory_state(&vault);
    let (input, quantized_slots) = measured_input(&vault);
    let report_path = reports.join("compression-report.json");
    let happy_before = file_state(&report_path);
    let report = compression_report(input.clone()).expect("compression report");
    let artifact = report_artifact(&report, &quantized_slots);
    write_json(&report_path, &artifact);
    let happy_after = file_state(&report_path);
    let cli_readback = output_state(readback(
        &report_path,
        Some("report.totals.weighted_bits_per_channel"),
    ));

    let edges_root = root.join("edges");
    fs::create_dir_all(&edges_root).expect("create edges root");
    let edges = json!({
        "empty_slots": edge_empty_slots(&input, &edges_root),
        "cosine_loss": edge_cosine_loss(&input, &edges_root),
        "guard_far_loss": edge_guard_far_loss(&input, &edges_root),
        "kernel_recall_loss": edge_kernel_recall_loss(&input, &edges_root),
    });

    let evidence = json!({
        "issue": 613,
        "trigger": "Forge compression_report(input) over persisted quantized vault bytes",
        "outcome": "PH59 compression-report artifact read through calyx readback",
        "source_of_truth": {
            "quantized_vault": display(&vault),
            "report_artifact": display(&report_path),
            "source_of_truth_marker": ARTIFACT_SOURCE_OF_TRUTH,
        },
        "known_io": {
            "expected_weighted_bits_per_channel": 3.0,
            "expected_total_original_bytes": report.totals.original_bytes,
            "expected_total_compressed_bytes": report.totals.compressed_bytes,
            "expected_total_bytes_saved": report.totals.bytes_saved,
            "expected_kernel_recall_delta": report.kernel.recall_delta,
        },
        "quantized_vault": {
            "before": quantized_before,
            "after": directory_state(&vault),
            "slots": quantized_slots,
        },
        "happy": {
            "before": happy_before,
            "after": happy_after,
            "cli_readback": cli_readback,
        },
        "edges": edges,
    });

    let evidence_path = root.join("issue613-fsv-readback.json");
    write_json(&evidence_path, &evidence);
    let manifest_path = write_blake3_manifest(&root);

    assert_eq!(
        evidence["happy"]["cli_readback"]["stdout_json"]["value"],
        json!(3.0)
    );
    assert_eq!(evidence["happy"]["after"]["exists"], json!(true));
    assert_eq!(evidence["edges"]["empty_slots"]["success"], json!(false));
    assert_eq!(
        evidence["edges"]["cosine_loss"]["code"],
        json!("CALYX_QUANT_INTELLIGENCE_LOSS")
    );
    assert_eq!(
        evidence["edges"]["guard_far_loss"]["code"],
        json!("CALYX_QUANT_INTELLIGENCE_LOSS")
    );
    assert_eq!(
        evidence["edges"]["kernel_recall_loss"]["code"],
        json!("CALYX_QUANT_INTELLIGENCE_LOSS")
    );

    println!("ISSUE613_FSV_ROOT={}", root.display());
    println!("ISSUE613_REPORT_ARTIFACT={}", report_path.display());
    println!("ISSUE613_EVIDENCE={}", evidence_path.display());
    println!("ISSUE613_BLAKE3={}", manifest_path.display());
    println!("{}", serde_json::to_string_pretty(&evidence).unwrap());
}

fn measured_input(vault: &Path) -> (CompressionReportInput, Value) {
    let slots_dir = vault.join("slots");
    fs::create_dir_all(&slots_dir).expect("create slots dir");
    let text = measured_slot(
        &slots_dir,
        "slot-text",
        QuantLevel::Bits3p5,
        b"issue613-text",
        0.420,
        0.440,
    );
    let image = measured_slot(
        &slots_dir,
        "slot-image",
        QuantLevel::Bits2p5,
        b"issue613-image",
        0.550,
        0.552,
    );
    let quantized_slots = json!([text.1, image.1]);
    let input = CompressionReportInput {
        vault_id: "vault-issue613-doc23".to_string(),
        slots: vec![text.0, image.0],
        kernel: KernelCompressionMeasurement {
            original_bytes: 4096,
            compressed_bytes: 1536,
            recall_before: 0.981,
            recall_after: 0.982,
            min_recall_delta: -0.001,
        },
    };
    (input, quantized_slots)
}

fn measured_slot(
    slots_dir: &Path,
    slot_id: &str,
    level: QuantLevel,
    seed_tag: &[u8],
    bits_before: f64,
    bits_after: f64,
) -> (CompressionSlotMeasurement, Value) {
    let dim = 128;
    let codec = TurboQuantCodec::new(new_seed(dim, seed_tag), level).expect("codec");
    let vector = unit_vector(dim, seed_tag[0] as f32 / 17.0);
    let quantized = codec.encode(&vector).expect("encode");
    let decoded = codec.decode(&quantized).expect("decode");
    let achieved_cosine_error = (1.0 - cosine(&vector, &decoded)).abs() as f64;
    let floor = (achieved_cosine_error * 0.5).max(0.000001);
    let quantized_path = slots_dir.join(format!("{slot_id}.qv"));
    fs::write(&quantized_path, &quantized.bytes).expect("write quantized bytes");

    let measurement = CompressionSlotMeasurement {
        slot_id: slot_id.to_string(),
        level,
        channel_count: dim as u64,
        original_bytes: (dim * std::mem::size_of::<f32>()) as u64,
        compressed_bytes: quantized.bytes.len() as u64,
        turboquant_floor_cosine_error: floor,
        achieved_cosine_error,
        max_cosine_error: achieved_cosine_error + 0.010,
        bits_about_before: bits_before,
        bits_about_after: bits_after,
        min_bits_delta: -0.005,
        guard_far_before: 0.010,
        guard_far_after: 0.0105,
        max_guard_far_delta: 0.001,
        guard_frr_before: 0.020,
        guard_frr_after: 0.0204,
        max_guard_frr_delta: 0.001,
        kernel_only_recall_before: 0.970,
        kernel_only_recall_after: 0.971,
        min_kernel_recall_delta: -0.001,
    };
    let state = json!({
        "slot_id": slot_id,
        "level": level.to_string(),
        "path": display(&quantized_path),
        "file_state": file_state(&quantized_path),
        "scale": quantized.scale,
        "seed_id_prefix": first_hex(&quantized.seed_id, 16),
        "achieved_cosine_error": achieved_cosine_error,
        "turboquant_floor_cosine_error": floor,
    });
    (measurement, state)
}

fn report_artifact(report: &CompressionReport, quantized_slots: &Value) -> Value {
    json!({
        "schema_version": 1,
        "surface": "compression-report",
        "artifact_kind": "ph59.compression-report.v1",
        "source_of_truth": ARTIFACT_SOURCE_OF_TRUTH,
        "quantized_slots": quantized_slots,
        "report": report,
    })
}

fn edge_empty_slots(input: &CompressionReportInput, root: &Path) -> Value {
    let mut edge = input.clone();
    edge.slots.clear();
    run_reject_edge("empty-slots", edge, root, "CALYX_FORGE_QUANT_ERROR")
}

fn edge_cosine_loss(input: &CompressionReportInput, root: &Path) -> Value {
    let mut edge = input.clone();
    edge.slots[0].achieved_cosine_error = edge.slots[0].max_cosine_error + 0.001;
    run_reject_edge("cosine-loss", edge, root, "CALYX_QUANT_INTELLIGENCE_LOSS")
}

fn edge_guard_far_loss(input: &CompressionReportInput, root: &Path) -> Value {
    let mut edge = input.clone();
    edge.slots[1].guard_far_after = edge.slots[1].guard_far_before + 0.010;
    run_reject_edge(
        "guard-far-loss",
        edge,
        root,
        "CALYX_QUANT_INTELLIGENCE_LOSS",
    )
}

fn edge_kernel_recall_loss(input: &CompressionReportInput, root: &Path) -> Value {
    let mut edge = input.clone();
    edge.kernel.recall_after = edge.kernel.recall_before - 0.050;
    run_reject_edge(
        "kernel-recall-loss",
        edge,
        root,
        "CALYX_QUANT_INTELLIGENCE_LOSS",
    )
}

fn run_reject_edge(
    name: &str,
    input: CompressionReportInput,
    root: &Path,
    expected_code: &str,
) -> Value {
    let path = root.join(format!("{name}.json"));
    let before = file_state(&path);
    let result = compression_report(input);
    if let Ok(report) = &result {
        write_json(&path, &report_artifact(report, &json!([])));
    }
    let after = file_state(&path);
    let error = result.expect_err("edge must fail").to_string();
    let code = error
        .lines()
        .next()
        .unwrap_or("")
        .split(' ')
        .next()
        .unwrap_or("");
    assert_eq!(code, expected_code);
    assert_eq!(before["exists"], json!(false));
    assert_eq!(after["exists"], json!(false));
    json!({
        "before": before,
        "after": after,
        "success": false,
        "code": code,
        "error": error,
    })
}

fn unit_vector(dim: usize, phase: f32) -> Vec<f32> {
    let mut vector: Vec<f32> = (0..dim)
        .map(|idx| {
            let x = idx as f32 + 1.0;
            (x * phase).sin() + (x * 0.137).cos() * 0.25
        })
        .collect();
    let norm = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
    for value in &mut vector {
        *value /= norm;
    }
    vector
}

fn cosine(left: &[f32], right: &[f32]) -> f32 {
    let dot = left
        .iter()
        .zip(right.iter())
        .map(|(a, b)| a * b)
        .sum::<f32>();
    let left_norm = left.iter().map(|value| value * value).sum::<f32>().sqrt();
    let right_norm = right.iter().map(|value| value * value).sum::<f32>().sqrt();
    dot / (left_norm * right_norm)
}

fn readback(artifact: &Path, field: Option<&str>) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_calyx"));
    command
        .arg("readback")
        .arg("compression-report")
        .arg("--artifact")
        .arg(artifact);
    if let Some(field) = field {
        command.arg("--field").arg(field);
    }
    command.output().expect("run calyx readback")
}

fn output_state(output: Output) -> Value {
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    json!({
        "status": output.status.code(),
        "success": output.status.success(),
        "stdout": stdout,
        "stdout_json": serde_json::from_str::<Value>(&stdout).unwrap_or(Value::Null),
        "stderr": stderr,
    })
}

fn directory_state(path: &Path) -> Value {
    let mut files = Vec::new();
    if path.exists() {
        collect_files(path, path, &mut files);
    }
    json!({
        "path": display(path),
        "exists": path.exists(),
        "files": files,
    })
}

fn collect_files(root: &Path, current: &Path, files: &mut Vec<Value>) {
    for entry in fs::read_dir(current).expect("read dir") {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        if path.is_dir() {
            collect_files(root, &path, files);
        } else {
            files.push(json!({
                "relative": path.strip_prefix(root).unwrap().display().to_string(),
                "state": file_state(&path),
            }));
        }
    }
}

fn file_state(path: &Path) -> Value {
    if !path.exists() {
        return json!({ "path": display(path), "exists": false });
    }
    let bytes = fs::read(path).expect("read file state");
    json!({
        "path": display(path),
        "exists": true,
        "len": bytes.len(),
        "blake3": blake3::hash(&bytes).to_string(),
        "hex_prefix": first_hex(&bytes, 64),
    })
}

fn first_hex(bytes: &[u8], count: usize) -> String {
    bytes
        .iter()
        .take(count)
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>()
}

fn write_blake3_manifest(root: &Path) -> PathBuf {
    let mut files = Vec::new();
    collect_pathbufs(root, root, &mut files);
    files.sort();
    let mut manifest = String::new();
    for path in files {
        let bytes = fs::read(&path).expect("read manifest input");
        let relative = path
            .strip_prefix(root)
            .expect("relative path")
            .display()
            .to_string()
            .replace('\\', "/");
        manifest.push_str(&format!("{}  {relative}\n", blake3::hash(&bytes)));
    }
    let path = root.join("BLAKE3SUMS.txt");
    fs::write(&path, manifest).expect("write BLAKE3 manifest");
    path
}

fn collect_pathbufs(root: &Path, current: &Path, files: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(current).expect("read dir") {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        if path.is_dir() {
            collect_pathbufs(root, &path, files);
        } else if path != root.join("BLAKE3SUMS.txt") {
            files.push(path);
        }
    }
}

fn fsv_root() -> PathBuf {
    std::env::var_os("CALYX_ISSUE613_FSV_ROOT")
        .map(PathBuf::from)
        .expect("set CALYX_ISSUE613_FSV_ROOT to a fresh manual verification path")
}

fn display(path: &Path) -> String {
    path.display().to_string()
}
