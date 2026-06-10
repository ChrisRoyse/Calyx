use calyx_core::Result;
use serde::{Deserialize, Serialize};

use crate::hit::Hit;

use super::{
    BoostConfig, Clock, TemporalPolicy, TimeWindow, apply_temporal_boost, filter_hits_by_window,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CausalConfidence {
    High,
    Neutral,
    Low,
    #[default]
    Absent,
}

impl CausalConfidence {
    pub const fn is_absent(&self) -> bool {
        matches!(self, Self::Absent)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct CausalGateEvidence {
    pub confidence: CausalConfidence,
    pub multiplier: f32,
}

#[inline]
pub fn causal_gate_mult(confidence: CausalConfidence, cfg: &BoostConfig) -> f32 {
    match confidence {
        CausalConfidence::High => cfg.causal_high_mult,
        CausalConfidence::Low => cfg.causal_low_mult,
        CausalConfidence::Neutral | CausalConfidence::Absent => 1.0,
    }
}

pub fn derive_causal_confidence(hit: &Hit) -> CausalConfidence {
    if !hit.causal_confidence.is_absent() {
        return hit.causal_confidence;
    }
    let Some(guard) = &hit.guard else {
        return CausalConfidence::Absent;
    };
    if guard.verdict.overall_pass && !guard.verdict.provisional {
        CausalConfidence::High
    } else {
        CausalConfidence::Low
    }
}

pub fn apply_causal_gate(mut hits: Vec<Hit>, cfg: &BoostConfig) -> Result<Vec<Hit>> {
    cfg.validate()?;
    for hit in &mut hits {
        let confidence = derive_causal_confidence(hit);
        let multiplier = causal_gate_mult(confidence, cfg);
        hit.score *= multiplier;
        hit.causal_confidence = confidence;
        hit.causal_gate = Some(CausalGateEvidence {
            confidence,
            multiplier,
        });
    }

    hits.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| a.rank.cmp(&b.rank))
            .then_with(|| a.cx_id.to_string().cmp(&b.cx_id.to_string()))
    });
    for (index, hit) in hits.iter_mut().enumerate() {
        hit.rank = index + 1;
    }
    Ok(hits)
}

pub fn temporal_search_pipeline(
    hits: Vec<Hit>,
    window: &TimeWindow,
    policy: &TemporalPolicy,
    tz_offset_secs: i32,
    clock: &dyn Clock,
) -> Result<Vec<Hit>> {
    let filtered = filter_hits_by_window(hits, window);
    let boosted = apply_temporal_boost(filtered, policy, clock.now_secs(), tz_offset_secs)?;
    apply_causal_gate(boosted, &policy.boost)
}

#[cfg(test)]
mod tests {
    use calyx_core::{
        BoostConfig, CALYX_TEMPORAL_INVALID_BOOST_CONFIG, CxId, DecayFunction, FusionWeights,
        LedgerRef,
    };
    use calyx_ward::{GuardVerdict, NoveltyAction, SlotVerdict};
    use proptest::prelude::*;

    use super::*;
    use crate::hit::{FreshnessTag, HitGuardEvidence, HitGuardMode, ProvenanceSource};

    const SCORE_EPSILON: f32 = 1.0e-5;

    #[test]
    fn causal_gate_multiplier_uses_configured_defaults() {
        let cfg = BoostConfig::default();

        assert_eq!(causal_gate_mult(CausalConfidence::High, &cfg), 1.10);
        assert_eq!(causal_gate_mult(CausalConfidence::Low, &cfg), 0.85);
        assert_eq!(causal_gate_mult(CausalConfidence::Neutral, &cfg), 1.0);
        assert_eq!(causal_gate_mult(CausalConfidence::Absent, &cfg), 1.0);
    }

    #[test]
    fn causal_gate_applies_multipliers_and_reranks() {
        let hits = vec![
            hit(1, 0.90, 1, CausalConfidence::High, Some(999_500)),
            hit(2, 0.80, 2, CausalConfidence::Neutral, Some(999_000)),
            hit(3, 0.70, 3, CausalConfidence::Low, Some(998_000)),
        ];

        let gated = apply_causal_gate(hits, &BoostConfig::default()).expect("gate");

        assert_eq!(ids(&gated), vec![1, 2, 3]);
        assert_close(gated[0].score, 0.99);
        assert_close(gated[1].score, 0.80);
        assert_close(gated[2].score, 0.595);
        assert_eq!(gated[0].causal_confidence, CausalConfidence::High);
        assert_eq!(
            gated[0].causal_gate,
            Some(CausalGateEvidence {
                confidence: CausalConfidence::High,
                multiplier: 1.10,
            })
        );
    }

    #[test]
    fn temporal_search_pipeline_filters_boosts_then_gates() {
        let clock = super::super::FixedClock::new(1_000_000);
        let window = TimeWindow::last_hours(2, &clock).expect("window");
        let policy = policy();
        let hits = vec![
            hit(1, 0.90, 1, CausalConfidence::High, Some(999_500)),
            hit(2, 0.80, 2, CausalConfidence::Neutral, Some(999_000)),
            hit(3, 0.70, 3, CausalConfidence::Low, Some(989_000)),
        ];

        let piped = temporal_search_pipeline(hits, &window, &policy, 0, &clock).expect("pipeline");

        assert_eq!(ids(&piped), vec![1, 2]);
        assert_close(piped[0].score, 1.06425);
        assert_close(piped[1].score, 0.846);
        assert_eq!(piped[0].causal_gate.map(|gate| gate.multiplier), Some(1.10));
        assert!(piped.iter().all(|hit| hit.temporal_scores.is_some()));
    }

    #[test]
    fn derives_causal_confidence_from_ward_guard_when_explicit_absent() {
        let mut high = hit(1, 0.90, 1, CausalConfidence::Absent, Some(999_500));
        high.guard = Some(HitGuardEvidence {
            mode: HitGuardMode::InRegionOnly,
            verdict: guard_verdict(true, false),
        });
        let mut low = hit(2, 0.80, 2, CausalConfidence::Absent, Some(999_000));
        low.guard = Some(HitGuardEvidence {
            mode: HitGuardMode::InRegionOnly,
            verdict: guard_verdict(true, true),
        });

        assert_eq!(derive_causal_confidence(&high), CausalConfidence::High);
        assert_eq!(derive_causal_confidence(&low), CausalConfidence::Low);
    }

    #[test]
    fn empty_hit_list_is_valid() {
        let gated = apply_causal_gate(Vec::new(), &BoostConfig::default()).expect("empty");
        assert!(gated.is_empty());
    }

    #[test]
    fn invalid_boost_config_fails_closed() {
        let cfg = BoostConfig {
            post_retrieval_alpha: 0.10,
            causal_high_mult: -0.5,
            causal_low_mult: 0.85,
        };
        let error = apply_causal_gate(Vec::new(), &cfg).expect_err("bad mult rejected");
        assert_eq!(error.code, CALYX_TEMPORAL_INVALID_BOOST_CONFIG);
    }

    proptest! {
        #[test]
        fn causal_gate_preserves_hit_id_multiset(
            scores in proptest::collection::vec(0.0_f32..1.0, 0..24),
            confs in proptest::collection::vec(any::<u8>(), 0..24),
        ) {
            let hits = scores
                .iter()
                .enumerate()
                .map(|(idx, score)| {
                    let conf = match confs.get(idx).copied().unwrap_or_default() % 4 {
                        0 => CausalConfidence::High,
                        1 => CausalConfidence::Neutral,
                        2 => CausalConfidence::Low,
                        _ => CausalConfidence::Absent,
                    };
                    hit(idx as u8, *score, idx + 1, conf, Some(999_000))
                })
                .collect::<Vec<_>>();
            let before = sorted_ids(&hits);
            let after = sorted_ids(&apply_causal_gate(hits, &BoostConfig::default()).expect("gate"));

            prop_assert_eq!(before, after);
        }
    }

    fn policy() -> TemporalPolicy {
        TemporalPolicy::new(
            true,
            DecayFunction::Step,
            super::super::PeriodicOptions::new(None, None).expect("periodic"),
            Default::default(),
            FusionWeights::default(),
            BoostConfig::default(),
            true,
        )
        .expect("policy")
    }

    fn hit(
        seed: u8,
        score: f32,
        rank: usize,
        confidence: CausalConfidence,
        event_time_secs: Option<i64>,
    ) -> Hit {
        Hit {
            cx_id: CxId::from_bytes([seed; 16]),
            score,
            rank,
            event_time_secs,
            temporal_scores: None,
            causal_confidence: confidence,
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

    fn guard_verdict(overall_pass: bool, provisional: bool) -> GuardVerdict {
        GuardVerdict {
            guard_id: "018f48a4-9a79-74d2-8a5c-9ad7f6b8c101"
                .parse()
                .expect("guard id"),
            overall_pass,
            provisional,
            per_slot: vec![SlotVerdict {
                slot: calyx_core::SlotId::new(7),
                cos: if overall_pass { 0.91 } else { 0.40 },
                tau: 0.80,
                pass: overall_pass,
            }],
            action: (!overall_pass).then_some(NoveltyAction::Quarantine),
        }
    }

    fn ids(hits: &[Hit]) -> Vec<u8> {
        hits.iter().map(|hit| hit.cx_id.as_bytes()[0]).collect()
    }

    fn sorted_ids(hits: &[Hit]) -> Vec<u8> {
        let mut ids = ids(hits);
        ids.sort_unstable();
        ids
    }

    fn assert_close(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() <= SCORE_EPSILON,
            "actual {actual} expected {expected}"
        );
    }
}
