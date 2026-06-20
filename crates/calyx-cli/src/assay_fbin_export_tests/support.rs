use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use calyx_core::{Modality, QuantPolicy};
use calyx_registry::{LensForgeFile, LensForgeManifest};
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::Args;

pub(super) struct Fixture {
    pub(super) root: PathBuf,
    pub(super) corpus: PathBuf,
    pub(super) out: PathBuf,
    pub(super) bits: PathBuf,
}

impl Fixture {
    pub(super) fn new(name: &str, admitted_lenses: usize, rows: usize) -> Self {
        let names = (0..10).map(|idx| format!("lens-{idx}")).collect::<Vec<_>>();
        Self::with_names(name, &names, admitted_lenses, rows)
    }

    pub(super) fn new_algorithmic(name: &str, admitted_lenses: usize, rows: usize) -> Self {
        let names = (0..10).map(|idx| format!("lens-{idx}")).collect::<Vec<_>>();
        Self::with_names_and_writer(
            name,
            &names,
            admitted_lenses,
            rows,
            write_algorithmic_manifests,
        )
    }

    pub(super) fn with_names(
        name: &str,
        names: &[impl AsRef<str>],
        admitted_lenses: usize,
        rows: usize,
    ) -> Self {
        Self::with_names_and_writer(name, names, admitted_lenses, rows, write_manifests)
    }

    pub(super) fn with_names_and_writer(
        name: &str,
        names: &[impl AsRef<str>],
        admitted_lenses: usize,
        rows: usize,
        writer: fn(&Path, &[String]) -> Vec<PathBuf>,
    ) -> Self {
        let root = temp_root(name);
        let corpus = root.join("corpus");
        let manifests = root.join("manifests");
        let out = root.join("out");
        fs::create_dir_all(&corpus).unwrap();
        fs::create_dir_all(&manifests).unwrap();
        let names = names
            .iter()
            .map(|name| name.as_ref().to_string())
            .collect::<Vec<_>>();
        let manifest_paths = writer(&manifests, &names);
        write_vectors(&corpus.join("vectors.jsonl"), &names, rows);
        write_build_report(
            &corpus.join("corpus_build_report.json"),
            &names,
            &manifest_paths,
        );
        let bits = root.join("assay_abundance.json");
        write_bits(&bits, &names, admitted_lenses);
        Self {
            root,
            corpus,
            out,
            bits,
        }
    }

    pub(super) fn args(&self, query_count: usize) -> Args {
        Args {
            corpus_dir: self.corpus.clone(),
            out_dir: self.out.clone(),
            bits_report: self.bits.clone(),
            query_count,
            min_bits: 0.05,
        }
    }
}

pub(super) fn write_algorithmic_manifests(root: &Path, names: &[String]) -> Vec<PathBuf> {
    names
        .iter()
        .map(|name| {
            let path = root.join(format!("{name}.json"));
            let manifest = LensForgeManifest {
                name: name.clone(),
                modality: Modality::Text,
                runtime: "algorithmic:one-hot:3".to_string(),
                dim: 3,
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

pub(super) fn mark_bits_as_leaked(path: &Path) {
    let mut bits: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    bits["anchor_audit"] = serde_json::json!({
        "anchor_leaks_into_input": true,
        "trivial_anchor": true,
        "grounded_gate_eligible": false
    });
    fs::write(path, serde_json::to_vec_pretty(&bits).unwrap()).unwrap();
}

pub(super) fn assert_fbin_header(path: &Path, dim: u32, count: u64) {
    let bytes = fs::read(path).unwrap();
    assert_eq!(&bytes[0..8], b"CLXVEC01");
    assert_eq!(u32::from_le_bytes(bytes[8..12].try_into().unwrap()), dim);
    assert_eq!(u64::from_le_bytes(bytes[12..20].try_into().unwrap()), count);
}

fn write_vectors(path: &Path, lenses: &[String], rows: usize) {
    let mut lines = String::new();
    for row in 0..rows {
        let lens_map = lenses
            .iter()
            .enumerate()
            .map(|(idx, name)| {
                (
                    name.clone(),
                    serde_json::json!([row as f32 + 0.1, idx as f32 + 0.2, 1.0]),
                )
            })
            .collect::<serde_json::Map<_, _>>();
        lines.push_str(
            &serde_json::json!({
                "id": format!("row-{row}"),
                "source_event_time_secs": 1_704_153_600_i64 + row as i64,
                "source_event_time_raw": format!("{}", 1_704_153_600_i64 + row as i64),
                "temporal_lane_state": "active",
                "source_sequence": "jsonl_line",
                "source_sequence_index": row,
                "lenses": lens_map
            })
            .to_string(),
        );
        lines.push('\n');
    }
    fs::write(path, lines).unwrap();
}

fn write_build_report(path: &Path, names: &[String], manifests: &[PathBuf]) {
    let lenses = manifests
        .iter()
        .zip(names)
        .map(|(manifest, name)| {
            serde_json::json!({
                "name": name,
                "manifest": manifest
            })
        })
        .collect::<Vec<_>>();
    fs::write(
        path,
        serde_json::to_vec_pretty(&serde_json::json!({ "lenses": lenses })).unwrap(),
    )
    .unwrap();
}

fn write_bits(path: &Path, lenses: &[String], admitted: usize) {
    let lenses = lenses
        .iter()
        .enumerate()
        .map(|(idx, name)| {
            serde_json::json!({
                "name": name,
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

fn write_manifests(root: &Path, names: &[String]) -> Vec<PathBuf> {
    let matrix = static_matrix_bytes();
    let tokenizer = tokenizer_bytes();
    fs::write(root.join("embeddings.cslm"), &matrix).unwrap();
    fs::write(root.join("tokenizer.json"), &tokenizer).unwrap();
    names
        .iter()
        .map(|name| {
            let path = root.join(format!("{name}.json"));
            let manifest = LensForgeManifest {
                name: name.clone(),
                modality: Modality::Text,
                runtime: "model2vec".to_string(),
                dim: 3,
                shape: None,
                dtype: "int8".to_string(),
                weights_sha256: plain_sha256_hex(&matrix),
                artifact_set_sha256: Some(artifact_hash(&[&matrix, &tokenizer])),
                files: vec![
                    file("embeddings", "embeddings.cslm", &matrix),
                    file("tokenizer", "tokenizer.json", &tokenizer),
                ],
                pooling: "mean".to_string(),
                norm: "unit".to_string(),
                source_hf_id: format!("calyx/{name}"),
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
    bytes.extend_from_slice(&3_u32.to_le_bytes());
    bytes.push(1);
    bytes.extend_from_slice(&[0, 0, 0]);
    bytes.extend_from_slice(&1.0_f32.to_le_bytes());
    bytes.extend_from_slice(&[0, 0, 0, 3, 0, 0, 0, 4, 0, 0, 0, 5]);
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
