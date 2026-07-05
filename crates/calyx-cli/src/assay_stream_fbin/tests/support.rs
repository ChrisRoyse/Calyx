use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use calyx_core::{Modality, QuantPolicy};
use calyx_registry::{LensForgeFile, LensForgeManifest};
use sha2::{Digest, Sha256};

use crate::assay_multi_anchor_card::model::{
    LensEvidence, MultiAnchorReport, TargetLensValue, TargetSummary,
};

use super::super::args::{Args, StreamMode};
use super::super::format::VectorFormat;

pub(super) struct Fixture {
    pub(super) root: PathBuf,
    pub(super) rows: PathBuf,
    pub(super) out: PathBuf,
    pub(super) bits: PathBuf,
    a37: PathBuf,
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
        let a37 = root.join("a37_admission_cf");
        write_a37_admission(&a37, manifest_count, admitted_lenses, None, 0.2);
        Self {
            out: root.join("out"),
            root,
            rows: rows_path,
            bits,
            a37,
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
            bits_report: None,
            a37_admission_cf_root: Some(self.a37.clone()),
            a37_admission_key: "a37_multi_anchor_admission".to_string(),
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
            lens_parallelism: 1,
            worker_gpu_mem_limit_mib: None,
        }
    }

    pub(super) fn json_args(&self, query_count: usize, mode: StreamMode) -> Args {
        let mut args = self.args(query_count);
        args.bits_report = Some(self.bits.clone());
        args.a37_admission_cf_root = None;
        args.mode = mode;
        args
    }

    pub(super) fn rewrite_a37(&self, admitted: usize, names: Option<Vec<String>>, bits: f32) {
        write_a37_admission(&self.a37, self.manifests.len(), admitted, names, bits);
    }
}

fn write_a37_admission(
    root: &Path,
    lenses: usize,
    admitted: usize,
    names: Option<Vec<String>>,
    bits: f32,
) {
    let names = names.unwrap_or_else(|| (0..lenses).map(|idx| format!("lens-{idx}")).collect());
    let gate_passed = admitted == lenses && bits >= super::super::DEFAULT_MIN_BITS;
    let report = MultiAnchorReport {
        schema_version: 1,
        role: "a37_multi_anchor_admission_card".to_string(),
        status: if gate_passed {
            "gate_passed"
        } else {
            "gate_failed"
        }
        .to_string(),
        mode: "gate".to_string(),
        gate_passed,
        report_count: 1,
        lens_count: lenses,
        passing_lens_count: admitted,
        min_lenses: super::super::MIN_A35_LENSES,
        min_marginal_bits: super::super::DEFAULT_MIN_BITS,
        max_redundancy: 0.6,
        family_span_pass: gate_passed,
        redundancy_bound_pass: gate_passed,
        no_collapse_pass: gate_passed,
        association_family_count: 2,
        association_families: BTreeMap::new(),
        min_best_marginal_bits: bits,
        max_best_marginal_bits: bits,
        weakest_lens: names.first().cloned().unwrap_or_default(),
        target_summaries: vec![TargetSummary {
            target_class: 1,
            domain: "unit_stream_fbin".to_string(),
            report_path: "calyx/a37/admission/v1/unit".to_string(),
            status: if gate_passed {
                "gate_passed"
            } else {
                "gate_failed"
            }
            .to_string(),
            no_collapse_pass: gate_passed,
            family_span_pass: gate_passed,
            redundancy_bound_pass: gate_passed,
            n_eff: admitted as f32,
            panel_bits: bits,
            max_marginal_bits: bits,
            keep_count: admitted,
            park_count: lenses.saturating_sub(admitted),
        }],
        lenses: names
            .iter()
            .enumerate()
            .map(|(slot, name)| LensEvidence {
                slot: slot as u16,
                name: name.clone(),
                association_family: if slot % 2 == 0 { "dense" } else { "sparse" }.to_string(),
                passed: slot < admitted,
                best_target_class: 1,
                best_domain: "unit_stream_fbin".to_string(),
                best_marginal_bits: bits,
                best_solo_bits: bits + 0.01,
                target_values: vec![TargetLensValue {
                    target_class: 1,
                    domain: "unit_stream_fbin".to_string(),
                    marginal_bits: bits,
                    solo_bits: bits + 0.01,
                    decision: if slot < admitted { "Keep" } else { "Park" }.to_string(),
                }],
            })
            .collect(),
        source_reports: vec!["calyx/a37/admission/v1/unit".to_string()],
    };
    crate::a37_admission_store::write(root, "a37_multi_anchor_admission", &report).unwrap();
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
            // #1140 fail-closed: gate eligibility must be an explicit
            // affirmative audit; fixtures state it instead of relying on
            // defaults.
            "anchor_audit": {
                "anchor_leaks_into_input": false,
                "trivial_anchor": false,
                "grounded_gate_eligible": true,
                "label_recoverable_from_input": false,
                "audit_kind": "unit_fixture_affirmative",
                "source": "stream-fbin unit fixture",
                "reason": "fixture anchor audited eligible for gate tests"
            },
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
        batch_policy: None,
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
                batch_policy: None,
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
