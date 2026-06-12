use calyx_aster::vault::AsterVault;
use calyx_core::{Clock as CoreClock, CxId, RecurrenceBoostConfig, Result};
use serde::{Deserialize, Serialize};

use crate::hit::Hit;

use super::recurrence_boost::{RecurrenceBoostEvidence, recurrence_boost_evidence};
use super::{DecayFunction, FusionWeights, PeriodicOptions, TemporalPolicy};

const SECS_PER_HOUR: i64 = 3_600;
const SECS_PER_DAY: i64 = 86_400;
const UNIX_EPOCH_DAY_OF_WEEK_MONDAY_ZERO: i64 = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemporalTimeBucket {
    pub hour: u8,
    pub day_of_week: u8,
    pub tz_offset_secs: i32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TemporalScores {
    pub e2_recency: f32,
    pub e3_periodic: f32,
    pub e4_sequence: f32,
}

impl TemporalScores {
    pub const fn zero() -> Self {
        Self {
            e2_recency: 0.0,
            e3_periodic: 0.0,
            e4_sequence: 0.0,
        }
    }
}

#[inline]
pub fn score_e2_recency(event_time_secs: i64, query_time_secs: i64, decay: &DecayFunction) -> f32 {
    let age_secs = query_time_secs.saturating_sub(event_time_secs).max(0) as f32;
    match *decay {
        DecayFunction::Linear { max_age_secs } => {
            if max_age_secs == 0 {
                return if age_secs == 0.0 { 1.0 } else { 0.0 };
            }
            (1.0 - age_secs / max_age_secs as f32).clamp(0.0, 1.0)
        }
        DecayFunction::Exponential { half_life_secs } => {
            if half_life_secs == 0 {
                return if age_secs == 0.0 { 1.0 } else { 0.0 };
            }
            (-age_secs * 0.693 / half_life_secs as f32)
                .exp()
                .clamp(0.0, 1.0)
        }
        DecayFunction::Step => {
            if age_secs < SECS_PER_HOUR as f32 {
                0.8
            } else if age_secs < SECS_PER_DAY as f32 {
                0.5
            } else {
                0.1
            }
        }
    }
}

#[inline]
pub fn score_e3_periodic(
    event_time_secs: i64,
    query_time_secs: i64,
    opts: &PeriodicOptions,
    tz_offset_secs: i32,
) -> f32 {
    let (query_hour, query_day) = local_hour_and_day(query_time_secs, tz_offset_secs);
    let target_hour = opts.target_hour.or(opts.use_now.then_some(query_hour));
    let target_day = opts
        .target_day_of_week
        .or(opts.use_now.then_some(query_day));
    score_e3_periodic_with_targets(event_time_secs, target_hour, target_day, tz_offset_secs)
}

#[inline]
fn score_e3_periodic_with_targets(
    event_time_secs: i64,
    target_hour: Option<u8>,
    target_day_of_week: Option<u8>,
    tz_offset_secs: i32,
) -> f32 {
    let (local_hour, day_of_week) = local_hour_and_day(event_time_secs, tz_offset_secs);

    let mut score: f32 = 0.0;
    if target_hour == Some(local_hour) {
        score += 0.5;
    }
    if target_day_of_week == Some(day_of_week) {
        score += 0.5;
    }
    score.min(1.0)
}

#[inline]
pub fn temporal_time_bucket(time_secs: i64, tz_offset_secs: i32) -> TemporalTimeBucket {
    let local_secs = time_secs.saturating_add(i64::from(tz_offset_secs));
    let local_hour = (local_secs.rem_euclid(SECS_PER_DAY) / SECS_PER_HOUR) as u8;
    let local_day = local_secs.div_euclid(SECS_PER_DAY);
    let day_of_week = (local_day + UNIX_EPOCH_DAY_OF_WEEK_MONDAY_ZERO).rem_euclid(7) as u8;
    TemporalTimeBucket {
        hour: local_hour,
        day_of_week,
        tz_offset_secs,
    }
}

#[inline]
fn local_hour_and_day(time_secs: i64, tz_offset_secs: i32) -> (u8, u8) {
    let bucket = temporal_time_bucket(time_secs, tz_offset_secs);
    (bucket.hour, bucket.day_of_week)
}

#[inline]
pub fn score_e4_sequence(rank: usize, total: usize) -> f32 {
    if total <= 1 {
        return 1.0;
    }
    if rank >= total {
        return 0.0;
    }
    (1.0 - rank as f32 / total as f32).clamp(0.0, 1.0)
}

#[inline]
pub fn fuse_temporal(scores: &TemporalScores, weights: &FusionWeights) -> f32 {
    (weights.recency * scores.e2_recency
        + weights.sequence * scores.e4_sequence
        + weights.periodic * scores.e3_periodic)
        .clamp(0.0, 1.0)
}

pub fn apply_temporal_boost(
    hits: Vec<Hit>,
    policy: &TemporalPolicy,
    query_time_secs: i64,
    tz_offset_secs: i32,
) -> Result<Vec<Hit>> {
    boost_hits(
        hits,
        policy,
        query_time_secs,
        tz_offset_secs,
        |_cx_id, _query_time_secs, _config| Ok(None),
    )
}

pub fn apply_temporal_boost_with_recurrence<C>(
    hits: Vec<Hit>,
    policy: &TemporalPolicy,
    query_time_secs: i64,
    tz_offset_secs: i32,
    vault: &AsterVault<C>,
) -> Result<Vec<Hit>>
where
    C: CoreClock,
{
    boost_hits(
        hits,
        policy,
        query_time_secs,
        tz_offset_secs,
        |cx_id, query_time_secs, config| {
            recurrence_boost_evidence(cx_id, vault, query_time_secs, config).map(Some)
        },
    )
}

fn boost_hits(
    hits: Vec<Hit>,
    policy: &TemporalPolicy,
    query_time_secs: i64,
    tz_offset_secs: i32,
    mut recurrence: impl FnMut(
        CxId,
        i64,
        &RecurrenceBoostConfig,
    ) -> Result<Option<RecurrenceBoostEvidence>>,
) -> Result<Vec<Hit>> {
    policy.validate()?;
    if !policy.enabled {
        return Ok(hits);
    }
    let alpha = policy.boost.post_retrieval_alpha;
    let total = hits.len();
    let mut boosted = Vec::with_capacity(total);
    for (index, mut hit) in hits.into_iter().enumerate() {
        let recurrence_evidence = match policy.recurrence_boost {
            Some(config) => recurrence(hit.cx_id, query_time_secs, &config)?,
            None => None,
        };
        let scores = if hit.score <= 0.0 {
            TemporalScores::zero()
        } else {
            match hit.event_time_secs {
                Some(event_time_secs) => TemporalScores {
                    e2_recency: score_e2_recency(event_time_secs, query_time_secs, &policy.decay),
                    e3_periodic: score_e3_periodic(
                        event_time_secs,
                        query_time_secs,
                        &policy.periodic,
                        tz_offset_secs,
                    ),
                    e4_sequence: score_e4_sequence(index, total),
                },
                None => TemporalScores {
                    e2_recency: 0.0,
                    e3_periodic: 0.0,
                    e4_sequence: score_e4_sequence(index, total),
                },
            }
        };
        if hit.score > 0.0 {
            let temporal_multiplier = fuse_temporal(&scores, &policy.fusion_weights) * alpha;
            let recurrence_multiplier = recurrence_evidence.as_ref().map_or(0.0, |item| item.total);
            hit.score += hit.score * (temporal_multiplier + recurrence_multiplier);
        }
        hit.temporal_scores = Some(scores);
        if let Some(explain) = hit.explain.as_mut() {
            explain.recurrence_boost = recurrence_evidence;
        }
        boosted.push(hit);
    }

    boosted.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| a.rank.cmp(&b.rank))
            .then_with(|| a.cx_id.to_string().cmp(&b.cx_id.to_string()))
    });
    for (index, hit) in boosted.iter_mut().enumerate() {
        hit.rank = index + 1;
    }
    Ok(boosted)
}

#[cfg(test)]
mod tests {
    use calyx_core::{BoostConfig, CALYX_TEMPORAL_AP60_VIOLATION, CxId, LedgerRef};
    use proptest::prelude::*;

    use super::*;
    use crate::hit::{FreshnessTag, ProvenanceSource};

    const TUESDAY_2024_01_02_14H_UTC: i64 = 1_704_204_000;

    #[test]
    fn e2_linear_expires_at_max_age() {
        let score = score_e2_recency(
            1_000,
            4_600,
            &DecayFunction::Linear {
                max_age_secs: 3_600,
            },
        );
        assert_eq!(score, 0.0);
    }

    #[test]
    fn e2_step_scores_sub_hour_as_recent() {
        assert_eq!(score_e2_recency(1_000, 1_900, &DecayFunction::Step), 0.8);
    }

    #[test]
    fn e3_periodic_matches_tuesday_at_fourteen_utc() {
        let opts = PeriodicOptions::new(Some(14), Some(1)).expect("periodic");
        assert_eq!(
            score_e3_periodic(
                TUESDAY_2024_01_02_14H_UTC,
                TUESDAY_2024_01_02_14H_UTC + SECS_PER_DAY,
                &opts,
                0
            ),
            1.0
        );
    }

    #[test]
    fn e3_use_now_targets_query_local_hour_and_day() {
        let opts = PeriodicOptions::from_query_time();
        assert_eq!(
            score_e3_periodic(
                TUESDAY_2024_01_02_14H_UTC,
                TUESDAY_2024_01_02_14H_UTC,
                &opts,
                0
            ),
            1.0
        );
        assert_eq!(
            score_e3_periodic(
                TUESDAY_2024_01_02_14H_UTC,
                TUESDAY_2024_01_02_14H_UTC + SECS_PER_DAY + SECS_PER_HOUR,
                &opts,
                0
            ),
            0.0
        );
    }

    #[test]
    fn e4_sequence_scores_rank_position() {
        assert_eq!(score_e4_sequence(0, 5), 1.0);
        assert!((score_e4_sequence(4, 5) - 0.2).abs() < f32::EPSILON);
        assert_eq!(score_e4_sequence(0, 1), 1.0);
    }

    #[test]
    fn temporal_boost_keeps_high_content_hit_first() {
        let query_time_secs = 1_000_000;
        let policy = policy_with_step_decay();
        let hits = vec![hit(1, 0.95, 900_000, 1), hit(2, 0.80, 999_500, 2)];

        let boosted = apply_temporal_boost(hits, &policy, query_time_secs, 0).expect("boost");

        assert_eq!(boosted[0].cx_id, CxId::from_bytes([1; 16]));
        assert_eq!(boosted[0].rank, 1);
        assert!(boosted[0].score > 0.95);
        assert_eq!(boosted[1].rank, 2);
    }

    #[test]
    fn zero_content_hit_is_not_temporally_boosted() {
        let boosted = apply_temporal_boost(
            vec![hit(9, 0.0, 999_900, 1)],
            &TemporalPolicy::default(),
            1_000_000,
            0,
        )
        .expect("boost");

        assert_eq!(boosted[0].score, 0.0);
        assert_eq!(boosted[0].temporal_scores, Some(TemporalScores::zero()));
    }

    #[test]
    fn bad_alpha_fails_at_policy_boundary() {
        let error = BoostConfig::new(0.11, 1.10, 0.85).expect_err("alpha capped");
        assert_eq!(error.code, CALYX_TEMPORAL_AP60_VIOLATION);
    }

    #[test]
    fn empty_hit_list_is_valid() {
        let boosted = apply_temporal_boost(Vec::new(), &TemporalPolicy::default(), 1_000_000, 0)
            .expect("empty");
        assert!(boosted.is_empty());
    }

    #[test]
    fn never_dominant_false_fails_closed() {
        let policy = TemporalPolicy {
            never_dominant: false,
            ..TemporalPolicy::default()
        };
        let error =
            apply_temporal_boost(Vec::new(), &policy, 1_000_000, 0).expect_err("AP-60 violation");
        assert_eq!(error.code, CALYX_TEMPORAL_AP60_VIOLATION);
    }

    proptest! {
        #[test]
        fn fused_temporal_scores_stay_in_unit_interval(
            e2 in 0.0_f32..=1.0,
            e3 in 0.0_f32..=1.0,
            e4 in 0.0_f32..=1.0,
        ) {
            let score = fuse_temporal(
                &TemporalScores {
                    e2_recency: e2,
                    e3_periodic: e3,
                    e4_sequence: e4,
                },
                &FusionWeights::default(),
            );
            prop_assert!((0.0..=1.0).contains(&score));
        }
    }

    fn policy_with_step_decay() -> TemporalPolicy {
        TemporalPolicy::new(
            true,
            DecayFunction::Step,
            PeriodicOptions::new(None, None).expect("periodic"),
            Default::default(),
            FusionWeights::default(),
            BoostConfig::default(),
            true,
        )
        .expect("policy")
    }

    fn hit(seed: u8, score: f32, event_time_secs: i64, rank: usize) -> Hit {
        Hit {
            cx_id: CxId::from_bytes([seed; 16]),
            score,
            rank,
            event_time_secs: Some(event_time_secs),
            temporal_scores: None,
            causal_confidence: crate::temporal::CausalConfidence::Absent,
            causal_gate: None,
            per_lens: Vec::new(),
            cross_terms_used: false,
            guard: None,
            provenance: LedgerRef {
                seq: seed as u64,
                hash: [seed; 32],
            },
            provenance_source: ProvenanceSource::Stub,
            freshness: FreshnessTag::fresh(0),
            explain: None,
        }
    }
}
