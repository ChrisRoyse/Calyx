use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::sync::Arc;

use calyx_core::{SlotId, SystemClock};
use calyx_ward::{
    CalibrationInput, CalibrationMeta, GuardPolicy, GuardProfile, MatchedSlots, NoveltyAction,
    NoveltyHandler, NoveltyRecord, NoveltyStatus, ProducedSlots, SlotKind, VaultSink, WardError,
    guard, novel_regions,
};
use serde::Serialize;

use super::data::ScoreCorpus;
use super::request::WardGuardRequest;

const SLOT: SlotId = SlotId::new(1);
/// Cosine at which the synthetic OOD slot vector sits to its trusted vector.
const NOVELTY_COS: f32 = 0.30;
/// Representative cosine threshold for the novelty-routing demonstration. Novelty
/// routing (OOD -> new-region, not hard-block) is a Ward-mechanism property and is
/// demonstrated independently of the classifier-derived block/FRR tau (which lives
/// in classifier-score space, not cosine space). NOVELTY_COS < NOVELTY_TAU so the
/// synthetic vector is genuinely out-of-region.
const NOVELTY_TAU: f32 = 0.70;
const GUARD_UUID: &str = "018f48a4-9a79-74d2-8a5c-9ad7f6b8c562";

#[derive(Clone, Debug, Serialize)]
pub(crate) struct WardGuardReport {
    pub(crate) tau: f32,
    pub(crate) eval_split: String,
    pub(crate) n_scores: usize,
    pub(crate) calibration: CalibrationReport,
    pub(crate) heldout: HeldoutReport,
    pub(crate) gates: GateReport,
    pub(crate) novelty: NoveltyReport,
    pub(crate) verdicts_path: String,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct CalibrationReport {
    pub(crate) good_count: usize,
    pub(crate) bad_count: usize,
    pub(crate) meta_far: f32,
    pub(crate) meta_frr: f32,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct HeldoutReport {
    pub(crate) injection_total: usize,
    pub(crate) blocked: usize,
    pub(crate) block_rate: f32,
    pub(crate) benign_total: usize,
    pub(crate) benign_rejected: usize,
    pub(crate) benign_frr: f32,
    pub(crate) benign_acc: f32,
    pub(crate) heldout_far: f32,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct GateReport {
    pub(crate) required_block_rate: f32,
    pub(crate) max_frr: f32,
    pub(crate) block_pass: bool,
    pub(crate) frr_pass: bool,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct NoveltyReport {
    pub(crate) routed: bool,
    pub(crate) novel_regions: usize,
}

pub(crate) fn evaluate(
    corpus: &ScoreCorpus,
    request: &WardGuardRequest,
) -> Result<WardGuardReport, String> {
    let clock = SystemClock;

    // 1. Real Ward conformal tau-calibration on the calibration subset.
    let good = ScoreCorpus::benign_scores(&corpus.calibration, 0);
    let bad = ScoreCorpus::benign_scores(&corpus.calibration, 1);
    let good_count = good.len();
    let bad_count = bad.len();
    let input = CalibrationInput {
        slot: SLOT,
        good_scores: good,
        bad_scores: bad,
        slot_kind: SlotKind::Identity,
        target_far: request.target_far,
    };
    let (tau, meta) = calyx_ward::calibrate_slot(&input, request.alpha, &clock)
        .map_err(|error| error.code().to_string())?;

    // 2. Held-out metrics under the guard convention (blocked iff score < tau).
    let injection_total = corpus.heldout.iter().filter(|r| r.label == 1).count();
    let blocked = corpus
        .heldout
        .iter()
        .filter(|r| r.label == 1 && r.benign_score < tau)
        .count();
    let block_rate = blocked as f32 / injection_total as f32;
    let heldout_far = 1.0 - block_rate;

    let benign_total = corpus.heldout.iter().filter(|r| r.label == 0).count();
    let benign_rejected = corpus
        .heldout
        .iter()
        .filter(|r| r.label == 0 && r.benign_score < tau)
        .count();
    let benign_frr = benign_rejected as f32 / benign_total as f32;
    let benign_acc = 1.0 - benign_frr;

    // 5. Persist per-example heldout verdicts as the source of truth. No
    // `ColumnFamily::GuardVerdicts` exists, so this jsonl is the persisted SoT.
    let verdicts_path = persist_verdicts(request, corpus, tau)?;

    // 3. Fail-closed gates (return Err, never silently pass).
    let block_pass = block_rate >= request.required_block_rate;
    let frr_pass = benign_frr <= request.max_frr;
    if !block_pass {
        return Err(format!(
            "CALYX_FSV_WARD_BLOCK_RATE_BELOW_99PCT: rate={block_rate:.6}"
        ));
    }
    if !frr_pass {
        return Err(format!(
            "CALYX_FSV_WARD_FRR_ABOVE_TARGET: frr={benign_frr:.6}"
        ));
    }

    // 4. Novelty routing demonstration: an OOD slot vector must be routed to a
    // new region (AwaitingGrounding), not hard-blocked.
    let novelty = demonstrate_novelty(request, &meta)?;

    Ok(WardGuardReport {
        tau,
        eval_split: corpus.eval_split.clone(),
        n_scores: corpus.n_scores,
        calibration: CalibrationReport {
            good_count,
            bad_count,
            meta_far: meta.far,
            meta_frr: meta.frr,
        },
        heldout: HeldoutReport {
            injection_total,
            blocked,
            block_rate,
            benign_total,
            benign_rejected,
            benign_frr,
            benign_acc,
            heldout_far,
        },
        gates: GateReport {
            required_block_rate: request.required_block_rate,
            max_frr: request.max_frr,
            block_pass,
            frr_pass,
        },
        novelty,
        verdicts_path: verdicts_path.display().to_string(),
    })
}

fn persist_verdicts(
    request: &WardGuardRequest,
    corpus: &ScoreCorpus,
    tau: f32,
) -> Result<PathBuf, String> {
    std::fs::create_dir_all(&request.metrics_dir).map_err(|error| error.to_string())?;
    let path = request.metrics_dir.join("ward_guard_verdicts.jsonl");
    let mut out = String::new();
    for row in &corpus.heldout {
        let blocked = row.benign_score < tau;
        out.push_str(&format!(
            "{{\"row\":{},\"label\":{},\"score\":{:.6},\"tau\":{:.6},\"blocked\":{}}}\n",
            row.row, row.label, row.benign_score, tau, blocked
        ));
    }
    std::fs::write(&path, out).map_err(|error| error.to_string())?;
    Ok(path)
}

/// Builds a calibrated guard profile at `tau`, runs a synthetic OOD slot vector
/// through Ward's real `guard`, asserts it does NOT pass, then routes it through
/// the real `NoveltyHandler` into a file-backed `VaultSink`. Confirms the OOD
/// input was routed to a new region (AwaitingGrounding), not hard-blocked.
fn demonstrate_novelty(
    request: &WardGuardRequest,
    meta: &CalibrationMeta,
) -> Result<NoveltyReport, String> {
    let profile = GuardProfile {
        guard_id: GUARD_UUID.parse().map_err(|error| {
            format!("CALYX_FSV_WARD_INVALID_CONFIG: guard id parse failed: {error}")
        })?,
        panel_version: 70_004,
        domain: "injection_guard".to_string(),
        tau: BTreeMap::from([(SLOT, NOVELTY_TAU)]),
        required_slots: vec![SLOT],
        policy: GuardPolicy::AllRequired,
        calibration: Some(meta.clone()),
        novelty_action: NoveltyAction::NewRegion,
    };

    let trusted = normalize(&[0.2, 0.4, 0.8]).ok_or_else(|| novelty_err("trusted vector"))?;
    let ood = vector_at_cos(&trusted, NOVELTY_COS).ok_or_else(|| novelty_err("ood vector"))?;
    let produced = ProducedSlots::from([(SLOT, ood)]);
    let matched = MatchedSlots::from([(SLOT, trusted)]);

    let verdict = guard(&profile, &produced, &matched, false).map_err(ward_code)?;
    if verdict.overall_pass {
        return Err(
            "CALYX_FSV_WARD_INVALID_CONFIG: synthetic OOD vector unexpectedly passed guard"
                .to_string(),
        );
    }

    let sink = FileVault::new(request.metrics_dir.join("ward_novel_regions.jsonl"));
    let handler = NoveltyHandler::new(Arc::new(sink.clone()), Arc::new(SystemClock));
    let record = handler
        .handle(&profile, &verdict, &produced)
        .map_err(ward_code)?;
    let routed = record.status == NoveltyStatus::AwaitingGrounding;
    let listed = novel_regions(&sink, Some(0)).map_err(ward_code)?;

    Ok(NoveltyReport {
        routed,
        novel_regions: listed.len(),
    })
}

fn ward_code(error: WardError) -> String {
    error.code().to_string()
}

fn novelty_err(detail: &str) -> String {
    format!("CALYX_FSV_WARD_INVALID_CONFIG: {detail}")
}

/// File-backed `VaultSink` mirroring the PH38 FSV `FileVault`: appends one JSON
/// novelty record per line and reads them back for `novel_regions`.
#[derive(Clone)]
struct FileVault {
    path: PathBuf,
}

impl FileVault {
    const fn new(path: PathBuf) -> Self {
        Self { path }
    }
}

impl VaultSink for FileVault {
    fn write_novel(&self, record: &NoveltyRecord) -> Result<(), WardError> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(novelty_sink)?;
        }
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(novelty_sink)?;
        serde_json::to_writer(&mut file, record).map_err(novelty_sink_json)?;
        writeln!(file).map_err(novelty_sink)
    }

    fn novel_records(&self) -> Result<Vec<NoveltyRecord>, WardError> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let file = File::open(&self.path).map_err(novelty_sink)?;
        BufReader::new(file)
            .lines()
            .map(|line| {
                let line = line.map_err(novelty_sink)?;
                serde_json::from_str::<NoveltyRecord>(&line).map_err(novelty_sink_json)
            })
            .collect()
    }
}

fn novelty_sink(error: std::io::Error) -> WardError {
    WardError::NoveltySink {
        reason: error.to_string(),
    }
}

fn novelty_sink_json(error: serde_json::Error) -> WardError {
    WardError::NoveltySink {
        reason: error.to_string(),
    }
}

/// L2-normalizes a vector; `None` when the norm is zero or non-finite.
fn normalize(values: &[f32]) -> Option<Vec<f32>> {
    let norm = values.iter().map(|value| value * value).sum::<f32>().sqrt();
    (norm.is_finite() && norm > 0.0).then(|| values.iter().map(|value| value / norm).collect())
}

/// Builds a unit vector at the requested cosine to `anchor` (copied from the
/// PH38 FSV support helpers; not a test-module dependency).
fn vector_at_cos(anchor: &[f32], target_cos: f32) -> Option<Vec<f32>> {
    let anchor = normalize(anchor)?;
    let pivot = anchor
        .iter()
        .enumerate()
        .min_by(|(_, left), (_, right)| left.abs().total_cmp(&right.abs()))?
        .0;
    let mut orthogonal = vec![0.0; anchor.len()];
    orthogonal[pivot] = 1.0;
    for (value, base) in orthogonal.iter_mut().zip(&anchor) {
        *value -= anchor[pivot] * *base;
    }
    let orthogonal = normalize(&orthogonal)?;
    let side = (1.0 - target_cos * target_cos).sqrt();
    Some(
        anchor
            .iter()
            .zip(orthogonal)
            .map(|(base, side_axis)| target_cos * base + side * side_axis)
            .collect(),
    )
}
