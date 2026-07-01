use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use calyx_forge::{
    CompressionReportInput, CompressionSlotMeasurement, KernelCompressionMeasurement, QuantLevel,
    QuantizedVec, Quantizer, TurboQuantCodec, compression_report, new_seed,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

#[test]
fn issue935_report_recomputes_from_persisted_quantized_records() {
    let root = fsv_case_root("issue935-compression-report-bytes");
    fs::create_dir_all(&root).unwrap();
    let records_dir = root.join("persisted-records");
    let reports_dir = root.join("reports");
    fs::create_dir_all(&records_dir).unwrap();
    fs::create_dir_all(&reports_dir).unwrap();

    let text = quantized_fixture(QuantLevel::Bits3p5, 32, b"issue935-text");
    let image = quantized_fixture(QuantLevel::Bits2p5, 32, b"issue935-image");
    let text_record = write_quantized_record(&records_dir, "slot-text", &text);
    let image_record = write_quantized_record(&records_dir, "slot-image", &image);
    let persisted_text = read_quantized_record(&text_record.json_path, &text_record.bytes_path);
    let persisted_image = read_quantized_record(&image_record.json_path, &image_record.bytes_path);

    let report_path = reports_dir.join("compression-report.json");
    let happy_before = file_state(&report_path);
    let input = CompressionReportInput {
        vault_id: "vault-issue935-persisted-records".to_string(),
        slots: vec![
            measurement("slot-text", persisted_text, 0.420, 0.435),
            measurement("slot-image", persisted_image, 0.550, 0.556),
        ],
        kernel: KernelCompressionMeasurement {
            original_bytes: 2048,
            compressed_bytes: 1024,
            recall_before: 0.980,
            recall_after: 0.981,
            min_recall_delta: -0.001,
        },
    };
    let report = compression_report(input.clone()).unwrap();
    write_json(
        &report_path,
        &json!({
            "schema_version": 1,
            "surface": "compression-report",
            "artifact_kind": "ph59.compression-report.v1",
            "source_of_truth": "PH59 compression report artifact",
            "report": report,
        }),
    );
    let report_readback: Value =
        serde_json::from_slice(&fs::read(&report_path).unwrap()).expect("report json readback");
    let happy_after = file_state(&report_path);

    assert_eq!(happy_before["exists"], json!(false));
    assert_eq!(happy_after["exists"], json!(true));
    assert_eq!(
        report_readback["report"]["slots"][0]["compressed_bytes"],
        json!(text_record.bytes_len)
    );
    assert_eq!(
        report_readback["report"]["slots"][0]["stored_payload_sha256"],
        json!(text_record.bytes_sha256)
    );

    let edges = vec![
        edge_declared_bits8_with_raw_f32(&records_dir),
        edge_truncated_payload(&records_dir),
        edge_mismatched_dim(&records_dir),
        edge_byte_length_mismatch(&records_dir),
    ];
    for edge in &edges {
        assert_eq!(edge["after"]["success"], json!(false));
        assert_eq!(
            edge["after"]["error_code"],
            json!("CALYX_FORGE_QUANT_ERROR")
        );
    }

    let readback = json!({
        "issue": 935,
        "trigger": "compression_report over persisted QuantizedVec JSON plus payload bytes",
        "source_of_truth": {
            "records_dir": display(&records_dir),
            "report_path": display(&report_path),
            "readback_file": display(&root.join("issue935-readback.json")),
        },
        "persisted_records": {
            "slot_text": text_record,
            "slot_image": image_record,
        },
        "happy": {
            "before": happy_before,
            "after": happy_after,
            "report": report_readback,
        },
        "edge_cases": edges,
    });
    let readback_path = root.join("issue935-readback.json");
    write_json(&readback_path, &readback);
    let readback_sha256 = sha256_hex(&fs::read(&readback_path).unwrap());

    println!("ISSUE935_FSV_ROOT={}", root.display());
    println!("ISSUE935_READBACK={}", readback_path.display());
    println!("ISSUE935_READBACK_SHA256={readback_sha256}");
    println!("ISSUE935_REPORT={}", report_path.display());
    println!(
        "ISSUE935_EDGE_COUNT={}",
        readback["edge_cases"].as_array().unwrap().len()
    );

    if keep_fsv_root() {
        return;
    }
    fs::remove_dir_all(root).unwrap();
}

#[derive(serde::Serialize)]
struct PersistedRecord {
    json_path: String,
    bytes_path: String,
    json_len: usize,
    bytes_len: u64,
    json_sha256: String,
    bytes_sha256: String,
}

fn write_quantized_record(root: &Path, slot: &str, qv: &QuantizedVec) -> PersistedRecord {
    let json_path = root.join(format!("{slot}.qv.json"));
    let bytes_path = root.join(format!("{slot}.qv.bin"));
    write_json(&json_path, qv);
    fs::write(&bytes_path, &qv.bytes).unwrap();
    let json_bytes = fs::read(&json_path).unwrap();
    let payload = fs::read(&bytes_path).unwrap();
    PersistedRecord {
        json_path: display(&json_path),
        bytes_path: display(&bytes_path),
        json_len: json_bytes.len(),
        bytes_len: payload.len() as u64,
        json_sha256: sha256_hex(&json_bytes),
        bytes_sha256: sha256_hex(&payload),
    }
}

fn read_quantized_record(json_path: &str, bytes_path: &str) -> QuantizedVec {
    let mut qv: QuantizedVec =
        serde_json::from_slice(&fs::read(json_path).unwrap()).expect("read qv json");
    let payload = fs::read(bytes_path).unwrap();
    assert_eq!(qv.bytes, payload);
    qv.bytes = payload;
    qv
}

fn measurement(
    slot_id: &str,
    quantized: QuantizedVec,
    before: f64,
    after: f64,
) -> CompressionSlotMeasurement {
    CompressionSlotMeasurement {
        slot_id: slot_id.to_string(),
        level: quantized.level,
        channel_count: quantized.dim as u64,
        original_bytes: (quantized.dim * std::mem::size_of::<f32>()) as u64,
        compressed_bytes: quantized.bytes.len() as u64,
        quantized,
        turboquant_floor_cosine_error: 0.000_001,
        achieved_cosine_error: 0.000_002,
        max_cosine_error: 0.010,
        bits_about_before: before,
        bits_about_after: after,
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
    }
}

fn edge_declared_bits8_with_raw_f32(root: &Path) -> Value {
    let bytes = raw_f32_payload(&[0.25, -0.5, 0.75, 1.0]);
    let qv = QuantizedVec {
        level: QuantLevel::Bits8,
        dim: 4,
        bytes,
        scale: 1.0,
        seed_id: [0; 32],
    };
    run_edge(
        root,
        "declared_bits8_with_raw_f32_bytes",
        measurement("edge-bits8-raw", qv, 0.4, 0.4),
    )
}

fn edge_truncated_payload(root: &Path) -> Value {
    let qv = QuantizedVec {
        level: QuantLevel::Bits8,
        dim: 4,
        bytes: vec![1, 2, 3],
        scale: 1.0,
        seed_id: [0; 32],
    };
    run_edge(
        root,
        "truncated_payload",
        measurement("edge-truncated", qv, 0.4, 0.4),
    )
}

fn edge_mismatched_dim(root: &Path) -> Value {
    let qv = QuantizedVec {
        level: QuantLevel::Bits8,
        dim: 5,
        bytes: vec![1, 2, 3, 4, 5],
        scale: 1.0,
        seed_id: [0; 32],
    };
    let mut measurement = measurement("edge-dim", qv, 0.4, 0.4);
    measurement.channel_count = 4;
    run_edge(root, "mismatched_dim", measurement)
}

fn edge_byte_length_mismatch(root: &Path) -> Value {
    let qv = QuantizedVec {
        level: QuantLevel::Bits8,
        dim: 4,
        bytes: vec![1, 2, 3, 4],
        scale: 1.0,
        seed_id: [0; 32],
    };
    let mut measurement = measurement("edge-byte-len", qv, 0.4, 0.4);
    measurement.compressed_bytes = 3;
    run_edge(root, "byte_length_mismatch", measurement)
}

fn run_edge(root: &Path, name: &str, measurement: CompressionSlotMeasurement) -> Value {
    let edge_dir = root.join("edges").join(name);
    fs::create_dir_all(&edge_dir).unwrap();
    let record = write_quantized_record(&edge_dir, name, &measurement.quantized);
    let before = json!({
        "record": record,
        "declared_level": format!("{:?}", measurement.level),
        "declared_channel_count": measurement.channel_count,
        "declared_compressed_bytes": measurement.compressed_bytes,
    });
    let input = CompressionReportInput {
        vault_id: format!("vault-issue935-edge-{name}"),
        slots: vec![measurement],
        kernel: KernelCompressionMeasurement {
            original_bytes: 64,
            compressed_bytes: 32,
            recall_before: 0.980,
            recall_after: 0.981,
            min_recall_delta: -0.001,
        },
    };
    let error = compression_report(input).expect_err("edge must fail closed");
    json!({
        "name": name,
        "before": before,
        "after": {
            "success": false,
            "error_code": error.code(),
            "error": error.to_string(),
        }
    })
}

fn quantized_fixture(level: QuantLevel, dim: usize, seed_tag: &[u8]) -> QuantizedVec {
    let codec = TurboQuantCodec::new(new_seed(dim, seed_tag), level).unwrap();
    codec
        .encode(&unit_vector(dim, seed_tag[0] as f32 / 17.0))
        .unwrap()
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

fn raw_f32_payload(values: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(std::mem::size_of_val(values));
    for value in values {
        bytes.extend_from_slice(&value.to_bits().to_be_bytes());
    }
    bytes
}

fn fsv_case_root(label: &str) -> PathBuf {
    let serial = NEXT_DIR.fetch_add(1, Ordering::SeqCst);
    if let Some(root) = calyx_fsv::fsv_root("CALYX_FSV_ROOT") {
        return root.join(format!("{label}-{}-{serial}", std::process::id()));
    }
    std::env::temp_dir().join(format!("calyx-{label}-{}-{serial}", std::process::id()))
}

fn keep_fsv_root() -> bool {
    calyx_fsv::fsv_root("CALYX_FSV_ROOT").is_some()
}

fn file_state(path: &Path) -> Value {
    if !path.exists() {
        return json!({ "path": display(path), "exists": false });
    }
    let bytes = fs::read(path).unwrap();
    json!({
        "path": display(path),
        "exists": true,
        "len": bytes.len(),
        "sha256": sha256_hex(&bytes),
        "prefix_hex": hex_prefix(&bytes, 64),
    })
}

fn write_json<T: serde::Serialize>(path: &Path, value: &T) {
    fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn hex_prefix(bytes: &[u8], count: usize) -> String {
    bytes
        .iter()
        .take(count)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn display(path: &Path) -> String {
    path.display().to_string()
}
