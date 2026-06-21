use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde::Deserialize;

use crate::assay_anchor_audit::AnchorAudit;

use super::request::AssayBitsRequest;

const MIN_SAMPLES: usize = 50;
const MIN_LENSES: usize = 1;

type LoadedVectors = (Vec<usize>, Vec<String>, BTreeMap<String, Vec<Vec<f32>>>);

/// A loaded, validated labeled multi-lens embedding corpus.
#[derive(Clone, Debug)]
pub(crate) struct AssayCorpus {
    pub(crate) dataset: String,
    pub(crate) embedding_model_id: String,
    pub(crate) lenses: Vec<LensSpec>,
    pub(crate) labels: Vec<usize>,
    pub(crate) anchor_groups: Vec<String>,
    pub(crate) anchor_audit: AnchorAudit,
    /// Per-lens vectors, indexed identically to `lenses`; each inner vec is one
    /// row per sample (same order as `labels`).
    pub(crate) lens_vectors: Vec<Vec<Vec<f32>>>,
}

#[derive(Clone, Debug)]
pub(crate) struct LensSpec {
    pub(crate) name: String,
    pub(crate) redundant: bool,
}

impl AssayCorpus {
    pub(crate) fn load(request: &AssayBitsRequest) -> Result<Self, String> {
        let dir = &request.corpus_dir;
        if !dir.is_dir() {
            return Err(format!(
                "CALYX_FSV_ASSAY_CORPUS_NOT_FOUND: {}",
                dir.display()
            ));
        }
        let manifest_path = dir.join("manifest.json");
        let vectors_path = dir.join("vectors.jsonl");
        if !manifest_path.is_file() {
            return Err(format!(
                "CALYX_FSV_ASSAY_CORPUS_NOT_FOUND: {}",
                manifest_path.display()
            ));
        }
        if !vectors_path.is_file() {
            return Err(format!(
                "CALYX_FSV_ASSAY_CORPUS_NOT_FOUND: {}",
                vectors_path.display()
            ));
        }
        let manifest = read_manifest(&manifest_path)?;
        let lens_names: Vec<String> = manifest.lenses.iter().map(|l| l.name.clone()).collect();
        if lens_names.len() < MIN_LENSES {
            return Err(format!(
                "CALYX_FSV_ASSAY_INVALID_CORPUS: need >={MIN_LENSES} lenses, got {}",
                lens_names.len()
            ));
        }
        let (labels, anchor_groups, raw_lens_vectors) = read_vectors(&vectors_path, &lens_names)?;
        if labels.len() < MIN_SAMPLES {
            return Err(format!(
                "CALYX_FSV_ASSAY_INVALID_CORPUS: need >={MIN_SAMPLES} samples, got {}",
                labels.len()
            ));
        }
        let mut lenses = Vec::with_capacity(lens_names.len());
        let mut lens_vectors = Vec::with_capacity(lens_names.len());
        for spec in &manifest.lenses {
            let rows = raw_lens_vectors
                .get(&spec.name)
                .ok_or_else(|| invalid(format!("lens {} has no vectors", spec.name)))?;
            check_lens_dim(&spec.name, rows)?;
            lenses.push(LensSpec {
                name: spec.name.clone(),
                redundant: spec.redundant,
            });
            lens_vectors.push(rows.clone());
        }
        if !labels.contains(&manifest.target_class) {
            return Err(invalid(format!(
                "target_class {} absent from labels",
                manifest.target_class
            )));
        }
        Ok(Self {
            dataset: manifest.dataset,
            embedding_model_id: manifest.embedding_model_id,
            lenses,
            labels,
            anchor_groups,
            anchor_audit: AnchorAudit::from_parts(
                manifest.anchor_audit,
                manifest.anchor_leaks_into_input,
                manifest.trivial_anchor,
                manifest.grounded_gate_eligible,
            ),
            lens_vectors,
        })
    }

    /// Binary one-vs-rest anchor labels: `true` iff sample is `target_class`.
    pub(crate) fn anchor_labels(&self, target_class: usize) -> Vec<bool> {
        self.labels.iter().map(|&l| l == target_class).collect()
    }

    pub(crate) fn n_samples(&self) -> usize {
        self.labels.len()
    }
}

fn check_lens_dim(name: &str, rows: &[Vec<f32>]) -> Result<(), String> {
    let mut dim: Option<usize> = None;
    for (row_idx, row) in rows.iter().enumerate() {
        if row.is_empty() {
            return Err(invalid(format!("lens {name} row {row_idx} is empty")));
        }
        match dim {
            Some(expected) if expected != row.len() => {
                return Err(invalid(format!(
                    "lens {name} row {row_idx} dim {} != {expected}",
                    row.len()
                )));
            }
            None => dim = Some(row.len()),
            _ => {}
        }
        if row.iter().any(|v| !v.is_finite()) {
            return Err(invalid(format!("lens {name} row {row_idx} non-finite")));
        }
    }
    if dim.is_none() {
        return Err(invalid(format!("lens {name} has no rows")));
    }
    Ok(())
}

fn read_manifest(path: &Path) -> Result<ManifestJson, String> {
    let text = fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let manifest: ManifestJson = serde_json::from_str(&text)
        .map_err(|error| invalid(format!("{}: {error}", path.display())))?;
    Ok(manifest)
}

fn read_vectors(path: &Path, lens_names: &[String]) -> Result<LoadedVectors, String> {
    let text = fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let mut labels = Vec::new();
    let mut groups = Vec::new();
    let mut lens_vectors: BTreeMap<String, Vec<Vec<f32>>> = lens_names
        .iter()
        .map(|name| (name.clone(), Vec::new()))
        .collect();
    for (line_idx, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let row: VectorRow = serde_json::from_str(line)
            .map_err(|error| invalid(format!("line {line_idx}: {error}")))?;
        labels.push(row.label);
        groups.push(row.group_id(line_idx));
        for name in lens_names {
            let vector = row
                .lenses
                .get(name)
                .ok_or_else(|| invalid(format!("line {line_idx} missing lens {name}")))?;
            lens_vectors
                .get_mut(name)
                .expect("lens map seeded with all names")
                .push(vector.clone());
        }
    }
    Ok((labels, groups, lens_vectors))
}

fn invalid(detail: impl AsRef<str>) -> String {
    format!("CALYX_FSV_ASSAY_INVALID_CORPUS: {}", detail.as_ref())
}

#[derive(Deserialize)]
struct ManifestJson {
    dataset: String,
    embedding_model_id: String,
    #[allow(dead_code)]
    #[serde(default)]
    n_samples: usize,
    #[allow(dead_code)]
    #[serde(default)]
    label_counts: BTreeMap<String, usize>,
    lenses: Vec<ManifestLens>,
    target_class: usize,
    #[serde(default)]
    anchor_audit: Option<AnchorAudit>,
    #[serde(default)]
    anchor_leaks_into_input: Option<bool>,
    #[serde(default)]
    trivial_anchor: Option<bool>,
    #[serde(default)]
    grounded_gate_eligible: Option<bool>,
}

#[derive(Deserialize)]
struct ManifestLens {
    name: String,
    #[serde(default)]
    redundant: bool,
}

#[derive(Deserialize)]
struct VectorRow {
    #[serde(default)]
    id: String,
    #[allow(dead_code)]
    #[serde(default)]
    split: String,
    #[serde(default, alias = "group", alias = "anchor_group")]
    group_id: Option<String>,
    label: usize,
    lenses: BTreeMap<String, Vec<f32>>,
}

impl VectorRow {
    fn group_id(&self, line_idx: usize) -> String {
        self.group_id
            .as_ref()
            .filter(|value| !value.trim().is_empty())
            .cloned()
            .or_else(|| {
                if self.id.trim().is_empty() {
                    None
                } else {
                    Some(self.id.clone())
                }
            })
            .unwrap_or_else(|| format!("row_{line_idx}"))
    }
}
