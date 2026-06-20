use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use calyx_core::{Modality, QuantPolicy};
use calyx_registry::{LensForgeFile, LensForgeManifest};
use sha2::{Digest, Sha256};

use super::super::args::{Args, StreamMode};
use super::super::format::VectorFormat;

pub(super) struct Fixture {
    pub(super) root: PathBuf,
    pub(super) rows: PathBuf,
    pub(super) out: PathBuf,
    pub(super) bits: PathBuf,
    manifests: Vec<PathBuf>,
}

impl Fixture {
    pub(super) fn new(
        name: &str,
        manifest_count: usize,
        admitted_lenses: usize,
        rows: usize,
    ) -> Self {
        let root = temp_root(name);
        let manifest_root = root.join("manifests");
        fs::create_dir_all(&manifest_root).unwrap();
        let manifests = write_manifests(&manifest_root, manifest_count);
        let rows_path = root.join("rows.jsonl");
        write_rows(&rows_path, rows);
        let bits = root.join("assay_abundance.json");
        write_bits(&bits, manifest_count, admitted_lenses);
        Self {
            out: root.join("out"),
            root,
            rows: rows_path,
            bits,
            manifests,
        }
    }

    pub(super) fn new_algorithmic(
        name: &str,
        manifest_count: usize,
        admitted_lenses: usize,
        rows: usize,
    ) -> Self {
        let mut fixture = Self::new(name, manifest_count, admitted_lenses, rows);
        fixture.manifests =
            write_algorithmic_manifests(&fixture.root.join("algorithmic"), manifest_count);
        fixture
    }

    pub(super) fn args(&self, query_count: usize) -> Args {
        Args {
            rows_jsonl: self.rows.clone(),
            out_dir: self.out.clone(),
            dataset: "unit_stream_fbin".to_string(),
            target_class: 1,
            manifests: self.manifests.clone(),
            bits_report: self.bits.clone(),
            query_count,
            limit_per_class: None,
            batch_size: 7,
            cost_override_json: None,
            embedding_model_id: None,
            min_bits: 0.05,
            vector_format: VectorFormat::Fbin,
            mode: StreamMode::Gate,
            worker_report: None,
            worker_slot: None,
        }
    }
}

fn write_rows(path: &Path, rows: usize) {
    let mut lines = String::new();
    for row in 0..rows {
        lines.push_str(
            &serde_json::json!({
                "id": format!("row-{row}"),
                "text": format!("alpha beta unit stream fbin row {row}"),
                "label": row % 2,
                "event_time": 1_704_153_600_i64 + row as i64
            })
            .to_string(),
        );
        lines.push('\n');
    }
    fs::write(path, lines).unwrap();
}

fn write_bits(path: &Path, lenses: usize, admitted: usize) {
    write_bits_with_gate(path, lenses, admitted, 1.25, "passed", 1.0);
}

pub(super) fn write_legacy_bits(path: &Path, lenses: usize, admitted: usize) {
    let lenses = (0..lenses)
        .map(|idx| {
            serde_json::json!({
                "name": format!("lens-{idx}"),
                "bits_about": 0.2,
                "admitted": idx < admitted
            })
        })
        .collect::<Vec<_>>();
    fs::write(
        path,
        serde_json::to_vec_pretty(&serde_json::json!({ "lenses": lenses })).unwrap(),
    )
    .unwrap();
}

pub(super) fn write_bits_with_gate(
    path: &Path,
    lenses: usize,
    admitted: usize,
    sufficiency_basis_bits: f32,
    power_status: &str,
    power_recovery_ratio: f32,
) {
    let admitted_names = (0..admitted)
        .map(|idx| format!("lens-{idx}"))
        .collect::<Vec<_>>();
    write_bits_with_panel_names(
        path,
        lenses,
        admitted,
        admitted_names,
        sufficiency_basis_bits,
        power_status,
        power_recovery_ratio,
    );
}

pub(super) fn write_bits_with_panel_names(
    path: &Path,
    lenses: usize,
    admitted: usize,
    admitted_names: Vec<String>,
    sufficiency_basis_bits: f32,
    power_status: &str,
    power_recovery_ratio: f32,
) {
    let lenses = (0..lenses)
        .map(|idx| {
            serde_json::json!({
                "name": format!("lens-{idx}"),
                "bits_about": 0.2,
                "admitted": idx < admitted
            })
        })
        .collect::<Vec<_>>();
    fs::write(
        path,
        serde_json::to_vec_pretty(&serde_json::json!({
            "anchor_entropy_bits": 1.0,
            "min_informative_target_entropy_bits": 0.30,
            "lenses": lenses,
            "panel": {
                "admitted_lenses": admitted_names,
                "i_panel_anchor": sufficiency_basis_bits + 0.01,
                "ci_95": [sufficiency_basis_bits, sufficiency_basis_bits + 0.02],
                "estimate_bound": "lower_bound",
                "sufficiency_basis_bits": sufficiency_basis_bits,
                "power_calibration_status": power_status,
                "power_recovery_ratio": power_recovery_ratio,
                "power_recovered_bits": power_recovery_ratio,
                "power_planted_bits": 1.0
            }
        }))
        .unwrap(),
    )
    .unwrap();
}

fn write_manifests(root: &Path, count: usize) -> Vec<PathBuf> {
    let matrix = static_matrix_bytes();
    let tokenizer = tokenizer_bytes();
    fs::write(root.join("embeddings.cslm"), &matrix).unwrap();
    fs::write(root.join("tokenizer.json"), &tokenizer).unwrap();
    (0..count)
        .map(|idx| learned_manifest(root, idx, &matrix, &tokenizer))
        .collect()
}

fn learned_manifest(root: &Path, idx: usize, matrix: &[u8], tokenizer: &[u8]) -> PathBuf {
    let path = root.join(format!("lens-{idx}.json"));
    let manifest = LensForgeManifest {
        name: format!("lens-{idx}"),
        modality: Modality::Text,
        runtime: "model2vec".to_string(),
        dim: 4,
        shape: None,
        dtype: "int8".to_string(),
        weights_sha256: plain_sha256_hex(matrix),
        artifact_set_sha256: Some(artifact_hash(&[matrix, tokenizer])),
        files: vec![
            file("embeddings", "embeddings.cslm", matrix),
            file("tokenizer", "tokenizer.json", tokenizer),
        ],
        pooling: "mean".to_string(),
        norm: "unit".to_string(),
        source_hf_id: format!("calyx/lens-{idx}"),
        endpoint: None,
        license: Some("apache-2.0".to_string()),
        non_commercial: false,
        quant_default: QuantPolicy::turboquant_default(),
        truncate_dim: None,
        recall_delta: calyx_registry::spec::default_recall_delta(),
        max_batch: None,
    };
    fs::write(&path, serde_json::to_vec_pretty(&manifest).unwrap()).unwrap();
    path
}

fn write_algorithmic_manifests(root: &Path, count: usize) -> Vec<PathBuf> {
    fs::create_dir_all(root).unwrap();
    (0..count)
        .map(|idx| {
            let path = root.join(format!("lens-{idx}.json"));
            let manifest = LensForgeManifest {
                name: format!("lens-{idx}"),
                modality: Modality::Text,
                runtime: "algorithmic:one-hot:4".to_string(),
                dim: 4,
                shape: None,
                dtype: "f32".to_string(),
                weights_sha256: String::new(),
                artifact_set_sha256: None,
                files: Vec::new(),
                pooling: "algorithmic".to_string(),
                norm: "none".to_string(),
                source_hf_id: format!("calyx/lens-{idx}"),
                endpoint: None,
                license: Some("apache-2.0".to_string()),
                non_commercial: false,
                quant_default: QuantPolicy::turboquant_default(),
                truncate_dim: None,
                recall_delta: calyx_registry::spec::default_recall_delta(),
                max_batch: None,
            };
            fs::write(&path, serde_json::to_vec_pretty(&manifest).unwrap()).unwrap();
            path
        })
        .collect()
}

fn static_matrix_bytes() -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"CXLKUP1\0");
    bytes.extend_from_slice(&4_u32.to_le_bytes());
    bytes.extend_from_slice(&4_u32.to_le_bytes());
    bytes.push(1);
    bytes.extend_from_slice(&[0, 0, 0]);
    bytes.extend_from_slice(&1.0_f32.to_le_bytes());
    bytes.extend_from_slice(&[0, 0, 0, 0, 3, 0, 0, 0, 0, 4, 0, 0, 0, 0, 5, 0]);
    bytes
}

fn tokenizer_bytes() -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "version": "1.0", "truncation": null, "padding": null, "added_tokens": [],
        "normalizer": null, "pre_tokenizer": {"type": "Whitespace"},
        "post_processor": null, "decoder": null,
        "model": {
            "type": "WordLevel",
            "vocab": {"[UNK]": 0, "alpha": 1, "beta": 2, "gamma": 3},
            "unk_token": "[UNK]"
        }
    }))
    .unwrap()
}

fn file(role: &str, path: &str, bytes: &[u8]) -> LensForgeFile {
    LensForgeFile {
        role: role.to_string(),
        path: PathBuf::from(path),
        sha256: plain_sha256_hex(bytes),
        bytes: bytes.len() as u64,
    }
}

fn artifact_hash(parts: &[&[u8]]) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update((part.len() as u64).to_be_bytes());
        hasher.update(part);
    }
    hex_from_bytes(&hasher.finalize())
}

fn plain_sha256_hex(bytes: &[u8]) -> String {
    hex_from_bytes(&Sha256::digest(bytes))
}

fn hex_from_bytes(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn temp_root(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("{name}-{}-{nanos}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    root
}

pub(super) fn staging_dir(fixture: &Fixture) -> PathBuf {
    fixture
        .out
        .with_file_name(format!(".out.tmp-{}", std::process::id()))
}
