//! Per-slot bits vote: direction weighted by measured bits (issue #84).
//!
//! Each panel slot casts a vote for the query market's outcome: the **direction** of the vote is the
//! side of the class boundary the query falls on (oriented by which way the slot separates resolved
//! YES from NO), and its **weight** is the slot's measured mutual information about the outcome
//! (`calyx_assay::ksg_mi_continuous_discrete` — the same grounded estimator #208 uses). The vote is
//! the bits-weighted fraction of slots pointing YES, so an informative slot dominates a noisy one.
//!
//! Note: `calyx-assay`'s logistic probe reports unsigned bits only — it discards the fitted
//! direction — so the per-slot sign is computed here from the difference of class-conditional means
//! (a well-defined discriminant direction). The magnitude is the engine-measured bit value. A slot
//! classified by Assay as low-signal is retained with zero weight and an explicit diagnostic so an
//! informative peer can still vote. All other estimator errors remain fatal, and the vote fails
//! closed below the MI sample floor, on a single-class outcome, or when no slot carries any bits.

use calyx_assay::{MIN_ASSAY_SAMPLES, ksg_mi_continuous_discrete};
use calyx_core::CalyxErrorCode;
use serde::{Deserialize, Serialize};

use crate::error::{PolyError, Result};

/// A slot's aligned training values and the query market's value for that slot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SlotVoteInput {
    /// Slot key.
    pub key: String,
    /// Training values, aligned with the outcome labels (length = n_train).
    pub train_values: Vec<f32>,
    /// The live query market's value for this slot.
    pub query_value: f32,
}

/// Training rows were below the MI sample floor.
pub const ERR_BV_SAMPLES: &str = "CALYX_POLY_BITS_VOTE_INSUFFICIENT_SAMPLES";
/// The outcome labels were single-class.
pub const ERR_BV_SINGLE_CLASS: &str = "CALYX_POLY_BITS_VOTE_SINGLE_CLASS";
/// A slot's row count or value was invalid.
pub const ERR_BV_SLOT: &str = "CALYX_POLY_BITS_VOTE_INVALID_SLOT";
/// No slot carried any measured bits about the outcome.
pub const ERR_BV_NO_SIGNAL: &str = "CALYX_POLY_BITS_VOTE_NO_SIGNAL";

/// One slot's contribution to the vote.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SlotVote {
    /// Slot key.
    pub key: String,
    /// Measured bits about the outcome (weight).
    pub bits: f32,
    /// +1 if higher values point YES, −1 if they point NO.
    pub direction: i8,
    /// Whether the query fell on the YES side of this slot's class boundary.
    pub votes_yes: bool,
    /// Stable Assay code when measurement classified this slot as zero-weight low signal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub measurement_error_code: Option<String>,
}

/// The bits-weighted vote result.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BitsVote {
    /// Bits-weighted fraction of slots voting YES for the query.
    pub p_yes: f64,
    /// Total measured bits across slots (the panel's information about the outcome).
    pub total_bits: f64,
    /// Per-slot contributions.
    pub slots: Vec<SlotVote>,
    /// Training rows used.
    pub n_train: usize,
    /// Reliability for the blend: panel information relative to the outcome entropy, saturated at 1.
    pub reliability: f64,
}

/// Casts the per-slot bits vote for a query market against resolved training history.
pub fn bits_vote(
    slots: &[SlotVoteInput],
    labels: &[bool],
    anchor_entropy_bits: f64,
    k_neighbors: usize,
) -> Result<BitsVote> {
    let n = labels.len();
    if n < MIN_ASSAY_SAMPLES {
        return Err(PolyError::diagnostics(
            ERR_BV_SAMPLES,
            format!("bits vote needs >= {MIN_ASSAY_SAMPLES} training rows, got {n}"),
        ));
    }
    let yes = labels.iter().filter(|y| **y).count();
    if yes == 0 || yes == n {
        return Err(PolyError::diagnostics(
            ERR_BV_SINGLE_CLASS,
            format!("bits vote outcome is single-class ({yes} of {n} YES)"),
        ));
    }
    if slots.is_empty() {
        return Err(PolyError::diagnostics(
            ERR_BV_SLOT,
            "bits vote requires at least one slot",
        ));
    }
    let labels_usize: Vec<usize> = labels.iter().map(|y| usize::from(*y)).collect();

    let mut votes = Vec::with_capacity(slots.len());
    let mut weighted_yes = 0.0f64;
    let mut total_bits = 0.0f64;
    for slot in slots {
        if slot.train_values.len() != n {
            return Err(PolyError::diagnostics(
                ERR_BV_SLOT,
                format!(
                    "slot '{}' has {} rows, expected {n}",
                    slot.key,
                    slot.train_values.len()
                ),
            ));
        }
        if slot.train_values.iter().any(|v| !v.is_finite()) || !slot.query_value.is_finite() {
            return Err(PolyError::diagnostics(
                ERR_BV_SLOT,
                format!("slot '{}' contains a non-finite value", slot.key),
            ));
        }
        let x: Vec<Vec<f32>> = slot.train_values.iter().map(|v| vec![*v]).collect();
        let (bits, measurement_error_code) =
            match ksg_mi_continuous_discrete(&x, &labels_usize, k_neighbors) {
                Ok(estimate) => (estimate.bits as f64, None),
                Err(error) if error.code == CalyxErrorCode::AssayLowSignal.code() => {
                    (0.0, Some(error.code.to_string()))
                }
                Err(error) => {
                    return Err(PolyError::diagnostics(
                        ERR_BV_SLOT,
                        format!("slot '{}' MI: {error}", slot.key),
                    ));
                }
            };

        let (mut sum_yes, mut sum_no) = (0.0f64, 0.0f64);
        for (v, y) in slot.train_values.iter().zip(labels) {
            if *y {
                sum_yes += *v as f64;
            } else {
                sum_no += *v as f64;
            }
        }
        let mean_yes = sum_yes / yes as f64;
        let mean_no = sum_no / (n - yes) as f64;
        let direction: i8 = if mean_yes >= mean_no { 1 } else { -1 };
        let midpoint = (mean_yes + mean_no) / 2.0;
        // Query on the YES side of the boundary, oriented by direction.
        let votes_yes = (slot.query_value as f64 - midpoint) * direction as f64 > 0.0;

        if votes_yes {
            weighted_yes += bits;
        }
        total_bits += bits;
        votes.push(SlotVote {
            key: slot.key.clone(),
            bits: bits as f32,
            direction,
            votes_yes,
            measurement_error_code,
        });
    }

    if total_bits <= 0.0 {
        return Err(PolyError::diagnostics(
            ERR_BV_NO_SIGNAL,
            "no slot carried measurable bits about the outcome",
        ));
    }
    let reliability = if anchor_entropy_bits > 0.0 {
        (total_bits / anchor_entropy_bits).clamp(0.0, 1.0)
    } else {
        0.0
    };
    // Shrink the raw bits-weighted vote toward 0.5 by the panel's information content: a vote that
    // rests on little total information about the outcome must not claim near-certainty. This keeps
    // an all-agree vote from producing a degenerate 0/1 probability that would dominate the blend.
    let raw_vote = weighted_yes / total_bits;
    let p_yes = 0.5 + (raw_vote - 0.5) * reliability;

    Ok(BitsVote {
        p_yes,
        total_bits,
        slots: votes,
        n_train: n,
        reliability,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::{Rng, SeedableRng};
    use rand_chacha::ChaCha8Rng;

    fn gaussian(rng: &mut ChaCha8Rng) -> f32 {
        use std::f64::consts::PI;
        let u1 = rng.random::<f64>().max(1e-12);
        let u2 = rng.random::<f64>();
        ((-2.0 * u1.ln()).sqrt() * (2.0 * PI * u2).cos()) as f32
    }

    #[test]
    fn informative_slot_dominates_noise() {
        let mut rng = ChaCha8Rng::seed_from_u64(84);
        let n = 200;
        let mut labels = Vec::new();
        let mut signal = Vec::new();
        let mut noise = Vec::new();
        for i in 0..n {
            let y = i % 2 == 0;
            labels.push(y);
            // signal slot: YES markets centered at +2, NO at -2 (high bits, points YES up)
            signal.push(if y { 2.0 } else { -2.0 } + 0.3 * gaussian(&mut rng));
            // noise slot: no relation to outcome
            noise.push(gaussian(&mut rng));
        }
        let slots = vec![
            SlotVoteInput {
                key: "signal".into(),
                train_values: signal,
                query_value: 2.5,
            },
            SlotVoteInput {
                key: "noise".into(),
                train_values: noise,
                query_value: 0.0,
            },
        ];
        let vote = bits_vote(&slots, &labels, 1.0, 3).unwrap();
        // Query has signal=+2.5 (clearly YES side); the informative slot dominates → p_yes high.
        assert!(
            vote.p_yes > 0.8,
            "informative YES query → high p_yes, got {}",
            vote.p_yes
        );
        let sig = vote.slots.iter().find(|s| s.key == "signal").unwrap();
        let noi = vote.slots.iter().find(|s| s.key == "noise").unwrap();
        assert!(
            sig.bits > noi.bits + 0.1,
            "signal slot must carry more bits"
        );
        assert_eq!(sig.direction, 1, "higher signal points YES");
        assert!(sig.votes_yes);
    }

    fn forecast_regression_rows() -> (Vec<SlotVoteInput>, Vec<bool>) {
        let mut rng = ChaCha8Rng::seed_from_u64(21_085);
        let mut labels = Vec::with_capacity(160);
        let mut signal = Vec::with_capacity(160);
        let mut noise = Vec::with_capacity(160);
        for i in 0..160 {
            let up = i % 2 == 0;
            let flip = (i / 2) % 10 == 0;
            labels.push(if up { !flip } else { flip });
            signal.push(if up { 2.0 } else { -2.0 } + 0.4 * gaussian(&mut rng));
            let _neighbor_sidecar = 0.3 * gaussian(&mut rng);
            noise.push(gaussian(&mut rng));
        }
        (
            vec![
                SlotVoteInput {
                    key: "signal".into(),
                    train_values: signal,
                    query_value: 2.2,
                },
                SlotVoteInput {
                    key: "noise".into(),
                    train_values: noise,
                    query_value: 0.0,
                },
            ],
            labels,
        )
    }

    #[test]
    fn low_signal_noise_is_parked_while_informative_slot_votes() {
        let (slots, labels) = forecast_regression_rows();
        let vote = bits_vote(&slots, &labels, 1.0, 3)
            .expect("one low-signal slot must not erase an informative panel vote");
        assert!(vote.p_yes > 0.65, "informative YES slot must still vote");
        let signal = vote.slots.iter().find(|slot| slot.key == "signal").unwrap();
        let noise = vote.slots.iter().find(|slot| slot.key == "noise").unwrap();
        assert!(signal.bits > 0.0);
        assert_eq!(noise.bits, 0.0);
        let noise_json = serde_json::to_value(noise).unwrap();
        assert_eq!(
            noise_json
                .get("measurement_error_code")
                .and_then(serde_json::Value::as_str),
            Some(CalyxErrorCode::AssayLowSignal.code())
        );
    }

    #[test]
    fn all_low_signal_slots_fail_closed_as_no_signal() {
        let (mut slots, labels) = forecast_regression_rows();
        let noise = slots.remove(1);
        assert_eq!(
            bits_vote(&[noise], &labels, 1.0, 3).unwrap_err().code(),
            ERR_BV_NO_SIGNAL
        );
    }

    #[test]
    fn fails_closed() {
        let short = vec![SlotVoteInput {
            key: "a".into(),
            train_values: vec![0.0; 10],
            query_value: 0.0,
        }];
        assert_eq!(
            bits_vote(&short, &[true; 10], 1.0, 3).unwrap_err().code(),
            ERR_BV_SAMPLES
        );
        let single = vec![SlotVoteInput {
            key: "a".into(),
            train_values: vec![0.0; 60],
            query_value: 0.0,
        }];
        assert_eq!(
            bits_vote(&single, &[true; 60], 1.0, 3).unwrap_err().code(),
            ERR_BV_SINGLE_CLASS
        );
    }
}
