use std::fs;

use serde::Deserialize;

use super::request::WardGuardRequest;
use calyx_ward::MIN_BAD_SCORES;

/// One scored example from the classifier's scores corpus.
///
/// `benign_score = 1 - inj_prob` in `[0, 1]`. Under the Ward guard convention an
/// example PASSES iff `benign_score >= tau` and is BLOCKED iff
/// `benign_score < tau`, so a high benign score means "looks benign".
#[derive(Clone, Debug)]
pub(crate) struct ScoreRow {
    pub(crate) row: i64,
    pub(crate) label: u8,
    pub(crate) benign_score: f32,
}

/// A loaded, validated scores corpus split into honest calibration / heldout
/// subsets. Both subsets are drawn from the eval split the model never trained
/// on, then halved by a stable per-row hash, so neither subset is in-sample.
#[derive(Clone, Debug)]
pub(crate) struct ScoreCorpus {
    pub(crate) eval_split: String,
    pub(crate) n_scores: usize,
    pub(crate) calibration: Vec<ScoreRow>,
    pub(crate) heldout: Vec<ScoreRow>,
}

impl ScoreCorpus {
    pub(crate) fn load(request: &WardGuardRequest) -> Result<Self, String> {
        let path = &request.scores;
        if !path.is_file() {
            return Err(format!(
                "CALYX_FSV_WARD_SCORES_NOT_FOUND: {}",
                path.display()
            ));
        }
        let text = fs::read_to_string(path)
            .map_err(|error| format!("CALYX_FSV_WARD_SCORES_NOT_FOUND: {error}"))?;

        let mut eval_rows: Vec<ScoreRow> = Vec::new();
        for (line_idx, line) in text.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let raw: RawRow = serde_json::from_str(line)
                .map_err(|error| invalid(format!("line {line_idx}: {error}")))?;
            if raw.split != request.eval_split {
                continue;
            }
            if raw.label != 0 && raw.label != 1 {
                return Err(invalid(format!(
                    "line {line_idx}: label must be 0 or 1, got {}",
                    raw.label
                )));
            }
            if !raw.benign_score.is_finite() || !(0.0..=1.0).contains(&raw.benign_score) {
                return Err(invalid(format!(
                    "line {line_idx}: benign_score {} not finite in [0,1]",
                    raw.benign_score
                )));
            }
            eval_rows.push(ScoreRow {
                row: raw.row,
                label: raw.label,
                benign_score: raw.benign_score,
            });
        }

        if eval_rows.is_empty() {
            return Err(invalid(format!(
                "no rows with split == {}",
                request.eval_split
            )));
        }

        // Deterministic, in-sample-free 50/50 split by a stable hash of the row
        // index: even hash byte -> calibration, odd -> heldout.
        let mut calibration = Vec::new();
        let mut heldout = Vec::new();
        for row in eval_rows.iter() {
            if to_calibration(row.row) {
                calibration.push(row.clone());
            } else {
                heldout.push(row.clone());
            }
        }

        let n_scores = eval_rows.len();
        let corpus = Self {
            eval_split: request.eval_split.clone(),
            n_scores,
            calibration,
            heldout,
        };
        corpus.validate()?;
        Ok(corpus)
    }

    fn validate(&self) -> Result<(), String> {
        let cal_injection = self.calibration.iter().filter(|r| r.label == 1).count();
        let cal_benign = self.calibration.iter().filter(|r| r.label == 0).count();
        let heldout_injection = self.heldout.iter().filter(|r| r.label == 1).count();
        let heldout_benign = self.heldout.iter().filter(|r| r.label == 0).count();

        if cal_injection < MIN_BAD_SCORES {
            return Err(invalid(format!(
                "calibration has {cal_injection} injection rows, need >= {MIN_BAD_SCORES}"
            )));
        }
        if cal_benign == 0 {
            return Err(invalid("calibration has no benign rows"));
        }
        if heldout_injection == 0 {
            return Err(invalid("heldout has no injection rows"));
        }
        if heldout_benign == 0 {
            return Err(invalid("heldout has no benign rows"));
        }
        Ok(())
    }

    pub(crate) fn benign_scores(rows: &[ScoreRow], label: u8) -> Vec<f32> {
        rows.iter()
            .filter(|r| r.label == label)
            .map(|r| r.benign_score)
            .collect()
    }
}

/// Stable per-row assignment: blake3(row.to_be_bytes()), first byte even ->
/// calibration. Deterministic and independent of file order.
fn to_calibration(row: i64) -> bool {
    let digest = blake3::hash(&row.to_be_bytes());
    digest.as_bytes()[0].is_multiple_of(2)
}

fn invalid(detail: impl AsRef<str>) -> String {
    format!("CALYX_FSV_WARD_INVALID_SCORES: {}", detail.as_ref())
}

#[derive(Deserialize)]
struct RawRow {
    #[allow(dead_code)]
    #[serde(default)]
    split: String,
    #[serde(default)]
    row: i64,
    label: u8,
    #[allow(dead_code)]
    #[serde(default)]
    inj_prob: f32,
    benign_score: f32,
}
