use std::collections::BTreeMap;

use calyx_assay::{entropy_bits, ksg_mi_continuous_discrete};
use calyx_core::SlotId;
use calyx_ward::{
    CalibrationMeta, GuardId, GuardPolicy, GuardProfile, NoveltyAction, ProducedSlots,
    SlotCalibrationMeta, guard,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::artifact::clip_ref;
use super::data::{EmbeddedClip, FsvError, speaker_groups};
use super::math::cosine;
use super::{
    CALIBRATION_TS, CALYX_WARD_VOXCELEB_EMPTY_DATASET, CALYX_WARD_VOXCELEB_INSUFFICIENT_SAMPLES,
    CALYX_WARD_VOXCELEB_MI_BELOW_THRESHOLD, CALYX_WARD_VOXCELEB_NEEDS_IMPOSTOR,
    CALYX_WARD_VOXCELEB_TAU_OVERLAP, CALYX_WARD_VOXCELEB_VERDICT_MISMATCH, GUARD_UUID, MI_K,
    MIN_SAMPLES, MIN_SPEAKERS, SPEAKER_MI_THRESHOLD_BITS, SPEAKER_SLOT,
};

pub(super) fn evaluate_embeddings(clips: &[EmbeddedClip]) -> Result<Value, FsvError> {
    if clips.is_empty() {
        return Err(FsvError::new(
            CALYX_WARD_VOXCELEB_EMPTY_DATASET,
            "no VoxCeleb clips supplied",
        ));
    }
    let groups = speaker_groups(clips);
    if groups.len() < MIN_SPEAKERS {
        return Err(FsvError::new(
            CALYX_WARD_VOXCELEB_NEEDS_IMPOSTOR,
            "speaker identity FSV needs at least two speaker ids",
        ));
    }
    if clips.len() < MIN_SAMPLES {
        return Err(FsvError::new(
            CALYX_WARD_VOXCELEB_INSUFFICIENT_SAMPLES,
            format!(
                "need at least {MIN_SAMPLES} clips for speaker MI, got {}",
                clips.len()
            ),
        ));
    }
    let pairs = pair_scores(clips, &groups);
    let min_genuine = pairs
        .iter()
        .filter(|pair| pair.expected_genuine)
        .map(|pair| pair.cos)
        .fold(f32::INFINITY, f32::min);
    let max_impostor = pairs
        .iter()
        .filter(|pair| !pair.expected_genuine)
        .map(|pair| pair.cos)
        .fold(f32::NEG_INFINITY, f32::max);
    if min_genuine <= max_impostor {
        return Err(FsvError::new(
            CALYX_WARD_VOXCELEB_TAU_OVERLAP,
            format!("min genuine {min_genuine} <= max impostor {max_impostor}"),
        ));
    }
    let tau = (min_genuine + max_impostor) * 0.5;
    let profile = guard_profile(tau, clips);
    let verdicts = guard_pairs(clips, &pairs, &profile)?;
    let mi = speaker_mi(clips)?;
    if mi["bits"].as_f64().unwrap_or(0.0) < f64::from(SPEAKER_MI_THRESHOLD_BITS) {
        return Err(FsvError::new(
            CALYX_WARD_VOXCELEB_MI_BELOW_THRESHOLD,
            format!("speaker MI below {SPEAKER_MI_THRESHOLD_BITS} bits: {mi}"),
        ));
    }
    let genuine_count = pairs.iter().filter(|pair| pair.expected_genuine).count();
    let impostor_count = pairs.len() - genuine_count;
    Ok(json!({
        "calibration": {
            "slot": SPEAKER_SLOT,
            "tau": tau,
            "min_genuine_cos": min_genuine,
            "max_impostor_cos": max_impostor,
            "genuine_count": genuine_count,
            "impostor_count": impostor_count,
            "target_far": 0.0,
            "achieved_far": 0.0,
            "achieved_frr": 0.0,
            "profile": profile,
        },
        "identity_lock": {
            "pairs": verdicts,
            "genuine_count": genuine_count,
            "impostor_count": impostor_count,
        },
        "speaker_mi": mi,
        "checks": {
            "all_genuine_pass": true,
            "all_impostor_fail": true,
            "speaker_mi_pass": true,
            "tau_separates_genuine_and_impostor": true,
        },
    }))
}

#[derive(Clone)]
struct PairScore {
    pair_id: String,
    enroll_index: usize,
    test_index: usize,
    expected_genuine: bool,
    cos: f32,
}

fn pair_scores(clips: &[EmbeddedClip], groups: &BTreeMap<String, Vec<usize>>) -> Vec<PairScore> {
    let speakers = groups.keys().cloned().collect::<Vec<_>>();
    let mut pairs = Vec::new();
    for (speaker_pos, speaker) in speakers.iter().enumerate() {
        let enroll = groups[speaker][0];
        let impostor_speaker = &speakers[(speaker_pos + 1) % speakers.len()];
        let impostor_enroll = groups[impostor_speaker][0];
        for test in groups[speaker].iter().copied().skip(1) {
            pairs.push(pair_score(clips, enroll, test, true));
            pairs.push(pair_score(clips, impostor_enroll, test, false));
        }
    }
    pairs
}

fn pair_score(
    clips: &[EmbeddedClip],
    enroll_index: usize,
    test_index: usize,
    expected_genuine: bool,
) -> PairScore {
    PairScore {
        pair_id: format!(
            "{}::{}",
            clips[enroll_index].rel_path, clips[test_index].rel_path
        ),
        enroll_index,
        test_index,
        expected_genuine,
        cos: cosine(&clips[enroll_index].embedding, &clips[test_index].embedding),
    }
}

fn guard_pairs(
    clips: &[EmbeddedClip],
    pairs: &[PairScore],
    profile: &GuardProfile,
) -> Result<Vec<Value>, FsvError> {
    pairs
        .iter()
        .map(|pair| {
            let produced = slot_map(&clips[pair.test_index].embedding);
            let matched = slot_map(&clips[pair.enroll_index].embedding);
            let verdict = guard(profile, &produced, &matched, true)
                .map_err(|error| FsvError::new(error.code(), error.to_string()))?;
            if verdict.overall_pass != pair.expected_genuine {
                return Err(FsvError::new(
                    CALYX_WARD_VOXCELEB_VERDICT_MISMATCH,
                    format!(
                        "{} expected_genuine={} got pass={}",
                        pair.pair_id, pair.expected_genuine, verdict.overall_pass
                    ),
                ));
            }
            Ok(json!({
                "pair_id": &pair.pair_id,
                "enroll": clip_ref(&clips[pair.enroll_index]),
                "test": clip_ref(&clips[pair.test_index]),
                "expected_genuine": pair.expected_genuine,
                "cos": pair.cos,
                "verdict": verdict,
            }))
        })
        .collect()
}

fn speaker_mi(clips: &[EmbeddedClip]) -> Result<Value, FsvError> {
    let groups = speaker_groups(clips);
    let label_map = groups
        .keys()
        .enumerate()
        .map(|(index, speaker)| (speaker.clone(), index))
        .collect::<BTreeMap<_, _>>();
    let labels = clips
        .iter()
        .map(|clip| label_map[&clip.speaker_id])
        .collect::<Vec<_>>();
    let vectors = clips
        .iter()
        .map(|clip| clip.embedding.clone())
        .collect::<Vec<_>>();
    let estimate = ksg_mi_continuous_discrete(&vectors, &labels, MI_K)
        .map_err(|error| FsvError::new(error.code, error.to_string()))?;
    let bits = estimate.bits;
    Ok(json!({
        "anchor": "speaker_id",
        "slot": SPEAKER_SLOT,
        "threshold_bits": SPEAKER_MI_THRESHOLD_BITS,
        "anchor_entropy_bits": entropy_bits(&labels),
        "estimate": &estimate,
        "bits": bits,
        "pass": bits >= SPEAKER_MI_THRESHOLD_BITS,
        "label_map": label_map,
    }))
}

fn guard_profile(tau: f32, clips: &[EmbeddedClip]) -> GuardProfile {
    let slot = SlotId::new(SPEAKER_SLOT);
    let corpus_hash = corpus_hash(clips);
    let per_slot = BTreeMap::from([(
        slot,
        SlotCalibrationMeta {
            corpus_hash,
            estimator: "voxceleb-mini-genuine-impostor-separation-v1".to_string(),
            far: 0.0,
            frr: 0.0,
            confidence: 1.0,
            ts: CALIBRATION_TS,
        },
    )]);
    GuardProfile {
        guard_id: GUARD_UUID.parse::<GuardId>().expect("static guard id"),
        panel_version: 70,
        domain: "ph70-voxceleb-speaker-identity".to_string(),
        tau: BTreeMap::from([(slot, tau)]),
        required_slots: vec![slot],
        policy: GuardPolicy::AllRequired,
        calibration: Some(CalibrationMeta {
            corpus_hash,
            estimator: "voxceleb-mini-genuine-impostor-separation-v1".to_string(),
            far: 0.0,
            frr: 0.0,
            confidence: 1.0,
            ts: CALIBRATION_TS,
            per_slot,
        }),
        novelty_action: NoveltyAction::RejectClosed,
    }
}

fn slot_map(embedding: &[f32]) -> ProducedSlots {
    BTreeMap::from([(SlotId::new(SPEAKER_SLOT), embedding.to_vec())])
}

fn corpus_hash(clips: &[EmbeddedClip]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    for clip in clips {
        hasher.update(clip.rel_path.as_bytes());
        hasher.update(clip.speaker_id.as_bytes());
        hasher.update(clip.wav_sha256.as_bytes());
    }
    hasher.finalize().into()
}
