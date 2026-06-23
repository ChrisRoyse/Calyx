use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use calyx_assay::MIN_ASSAY_SAMPLES;
use serde::Deserialize;

use super::request::OracleSufficiencyRequest;

const MIN_LENSES: usize = 10;

type LoadedVectors = (Vec<bool>, BTreeMap<String, Vec<Vec<f32>>>);

/// A loaded, validated labeled multi-lens oracle-sufficiency corpus.
///
/// Each lens is a form-only text-embedding view of a SWE-bench instance's
/// surface text. The binary label is the oracle `test_pass_fail`: `true` iff a
/// model's patch resolved the instance.
#[derive(Clone, Debug)]
pub(crate) struct OracleCorpus {
    pub(crate) oracle_model: String,
    pub(crate) dataset: String,
    pub(crate) anchor: String,
    pub(crate) embedding_model_id: String,
    pub(crate) lenses: Vec<LensSpec>,
    /// One bool per instance (`true` == resolved), same order as the rows.
    pub(crate) labels: Vec<bool>,
    /// Per-lens vectors, indexed identically to `lenses`; each inner vec is one
    /// row per instance (same order as `labels`).
    pub(crate) lens_vectors: Vec<Vec<Vec<f32>>>,
}

#[derive(Clone, Debug)]
pub(crate) struct LensSpec {
    pub(crate) name: String,
}

impl OracleCorpus {
    pub(crate) fn load(request: &OracleSufficiencyRequest) -> Result<Self, String> {
        let dir = &request.corpus_dir;
        if !dir.is_dir() {
            return Err(format!(
                "CALYX_FSV_ORACLE_CORPUS_NOT_FOUND: {}",
                dir.display()
            ));
        }
        let manifest_path = dir.join("manifest.json");
        let vectors_path = dir.join("vectors.jsonl");
        if !manifest_path.is_file() {
            return Err(format!(
                "CALYX_FSV_ORACLE_CORPUS_NOT_FOUND: {}",
                manifest_path.display()
            ));
        }
        if !vectors_path.is_file() {
            return Err(format!(
                "CALYX_FSV_ORACLE_CORPUS_NOT_FOUND: {}",
                vectors_path.display()
            ));
        }
        let manifest = read_manifest(&manifest_path)?;
        let lens_names: Vec<String> = manifest.lenses.iter().map(|l| l.name.clone()).collect();
        if lens_names.len() < MIN_LENSES {
            return Err(format!(
                "CALYX_FSV_ORACLE_INVALID_CORPUS: need >={MIN_LENSES} lens, got {}",
                lens_names.len()
            ));
        }
        let (labels, raw_lens_vectors) = read_vectors(&vectors_path, &lens_names)?;
        if labels.len() < MIN_ASSAY_SAMPLES {
            return Err(format!(
                "CALYX_FSV_ORACLE_INVALID_CORPUS: need >={MIN_ASSAY_SAMPLES} samples, got {}",
                labels.len()
            ));
        }
        let resolved = labels.iter().filter(|&&l| l).count();
        if resolved == 0 || resolved == labels.len() {
            return Err(format!(
                "CALYX_FSV_ORACLE_INVALID_CORPUS: need both labels present, resolved={resolved} total={}",
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
            });
            lens_vectors.push(rows.clone());
        }
        Ok(Self {
            oracle_model: manifest
                .oracle_model
                .unwrap_or_else(|| manifest.dataset.clone()),
            dataset: manifest.dataset,
            anchor: manifest
                .anchor
                .unwrap_or_else(|| "test_pass_fail(resolved)".to_string()),
            embedding_model_id: manifest.embedding_model_id,
            lenses,
            labels,
            lens_vectors,
        })
    }

    pub(crate) fn n_samples(&self) -> usize {
        self.labels.len()
    }

    pub(crate) fn resolved(&self) -> usize {
        self.labels.iter().filter(|&&l| l).count()
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
        labels.push(row.label != 0);
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
    Ok((labels, lens_vectors))
}

fn invalid(detail: impl AsRef<str>) -> String {
    format!("CALYX_FSV_ORACLE_INVALID_CORPUS: {}", detail.as_ref())
}

#[derive(Deserialize)]
struct ManifestJson {
    #[serde(default)]
    oracle_model: Option<String>,
    dataset: String,
    #[serde(default)]
    anchor: Option<String>,
    #[allow(dead_code)]
    #[serde(default)]
    n: usize,
    #[allow(dead_code)]
    #[serde(default)]
    resolved: usize,
    embedding_model_id: String,
    lenses: Vec<ManifestLens>,
}

#[derive(Deserialize)]
struct ManifestLens {
    name: String,
}

#[derive(Deserialize)]
struct VectorRow {
    #[allow(dead_code)]
    #[serde(default)]
    id: String,
    #[allow(dead_code)]
    #[serde(default)]
    split: String,
    label: i64,
    lenses: BTreeMap<String, Vec<f32>>,
}
