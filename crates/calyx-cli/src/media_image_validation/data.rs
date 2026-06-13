use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde::Deserialize;

#[derive(Clone, Debug)]
pub(crate) struct ClassSample {
    pub(crate) image_features: Vec<f32>,
    pub(crate) class_label: usize,
}

#[derive(Clone, Debug)]
pub(crate) struct CrossModalSample {
    pub(crate) image_features: Vec<f32>,
    pub(crate) caption_features: Vec<f32>,
}

#[derive(Clone, Debug)]
pub(crate) struct ValidationData {
    pub(crate) class_samples: Vec<ClassSample>,
    pub(crate) cross_modal_samples: Vec<CrossModalSample>,
    pub(crate) dataset_counts: BTreeMap<String, usize>,
    pub(crate) source_sha256_count: usize,
    pub(crate) total_rows: usize,
}

impl ValidationData {
    pub(crate) fn load(path: &Path) -> Result<Self, String> {
        let text =
            fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
        let mut class_samples = Vec::new();
        let mut cross_modal_samples = Vec::new();
        let mut dataset_counts = BTreeMap::<String, usize>::new();
        let mut source_sha256_count = 0;
        let mut total_rows = 0;
        for (idx, line) in text.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let row: SampleJson = serde_json::from_str(line)
                .map_err(|error| format!("{}:{}: {error}", path.display(), idx + 1))?;
            row.validate(idx + 1)?;
            total_rows += 1;
            *dataset_counts.entry(row.dataset.clone()).or_default() += 1;
            if row
                .source_sha256
                .as_ref()
                .is_some_and(|value| !value.is_empty())
            {
                source_sha256_count += 1;
            }
            if let Some(class_label) = row.class_label {
                class_samples.push(ClassSample {
                    image_features: row.image_features.clone(),
                    class_label,
                });
            }
            if let Some(caption_features) = row.caption_features {
                cross_modal_samples.push(CrossModalSample {
                    image_features: row.image_features,
                    caption_features,
                });
            }
        }
        if total_rows == 0 {
            return Err("CALYX_FSV_MEDIA_EMPTY_DATASET".to_string());
        }
        Ok(Self {
            class_samples,
            cross_modal_samples,
            dataset_counts,
            source_sha256_count,
            total_rows,
        })
    }
}

#[derive(Deserialize)]
struct SampleJson {
    sample_id: String,
    dataset: String,
    image_features: Vec<f32>,
    #[serde(default)]
    class_label: Option<usize>,
    #[serde(default)]
    caption_features: Option<Vec<f32>>,
    #[serde(default)]
    source_sha256: Option<String>,
}

impl SampleJson {
    fn validate(&self, line: usize) -> Result<(), String> {
        if self.sample_id.trim().is_empty() || self.dataset.trim().is_empty() {
            return Err(format!(
                "CALYX_FSV_MEDIA_INVALID_FEATURE: line {line} missing sample_id or dataset"
            ));
        }
        validate_features(line, "image_features", &self.image_features)?;
        if let Some(features) = &self.caption_features {
            validate_features(line, "caption_features", features)?;
        }
        if self.class_label.is_none() && self.caption_features.is_none() {
            return Err(format!(
                "CALYX_FSV_MEDIA_CAPTION_INTEGRITY_MISMATCH: line {line} has no class or caption anchor"
            ));
        }
        Ok(())
    }
}

fn validate_features(line: usize, name: &str, values: &[f32]) -> Result<(), String> {
    if values.is_empty() {
        return Err(format!(
            "CALYX_FSV_MEDIA_INVALID_FEATURE: line {line} {name} is empty"
        ));
    }
    if values.iter().any(|value| !value.is_finite()) {
        return Err(format!(
            "CALYX_FSV_MEDIA_INVALID_FEATURE: line {line} {name} contains NaN or infinity"
        ));
    }
    Ok(())
}
