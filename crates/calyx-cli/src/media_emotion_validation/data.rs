use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use serde::Deserialize;

#[derive(Clone, Debug)]
pub(crate) struct EmotionSample {
    pub(crate) features: Vec<f32>,
    pub(crate) label: usize,
}

#[derive(Clone, Debug)]
pub(crate) struct ValidationData {
    pub(crate) samples: Vec<EmotionSample>,
    pub(crate) dataset_counts: BTreeMap<String, usize>,
    pub(crate) source_sha256_count: usize,
    pub(crate) total_rows: usize,
}

impl ValidationData {
    pub(crate) fn load(path: &Path) -> Result<Self, String> {
        let text =
            fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
        let mut samples = Vec::new();
        let mut dataset_counts = BTreeMap::<String, usize>::new();
        let mut sample_ids = BTreeSet::<String>::new();
        let mut source_hashes = BTreeSet::<String>::new();
        let mut source_sha256_count = 0;
        let mut total_rows = 0;
        for (idx, line) in text.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let row: SampleJson = serde_json::from_str(line)
                .map_err(|error| format!("{}:{}: {error}", path.display(), idx + 1))?;
            row.validate(idx + 1)?;
            if !sample_ids.insert(row.sample_id.clone()) {
                return Err(format!(
                    "CALYX_FSV_MEDIA_EMOTION_DUPLICATE_SAMPLE_ID: line {} repeats sample_id {:?}",
                    idx + 1,
                    row.sample_id
                ));
            }
            total_rows += 1;
            *dataset_counts.entry(row.dataset).or_default() += 1;
            if let Some(value) = row.source_sha256.as_ref().filter(|value| !value.is_empty()) {
                if !source_hashes.insert(value.clone()) {
                    return Err(format!(
                        "CALYX_FSV_MEDIA_EMOTION_DUPLICATE_SOURCE_SHA256: line {} repeats source_sha256 {}",
                        idx + 1,
                        value
                    ));
                }
                source_sha256_count += 1;
            }
            samples.push(EmotionSample {
                features: row.audio_features,
                label: row.emotion_label,
            });
        }
        if total_rows == 0 {
            return Err("CALYX_FSV_MEDIA_EMOTION_EMPTY_DATASET".to_string());
        }
        Ok(Self {
            samples,
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
    audio_features: Vec<f32>,
    emotion_label: usize,
    #[serde(default)]
    source_sha256: Option<String>,
}

impl SampleJson {
    fn validate(&self, line: usize) -> Result<(), String> {
        if self.sample_id.trim().is_empty() || self.dataset.trim().is_empty() {
            return Err(format!(
                "CALYX_FSV_MEDIA_EMOTION_INVALID_FEATURE: line {line} missing sample_id or dataset"
            ));
        }
        if self.audio_features.is_empty() {
            return Err(format!(
                "CALYX_FSV_MEDIA_EMOTION_INVALID_FEATURE: line {line} audio_features is empty"
            ));
        }
        if self.audio_features.iter().any(|value| !value.is_finite()) {
            return Err(format!(
                "CALYX_FSV_MEDIA_EMOTION_INVALID_FEATURE: line {line} audio_features contains NaN or infinity"
            ));
        }
        Ok(())
    }
}
