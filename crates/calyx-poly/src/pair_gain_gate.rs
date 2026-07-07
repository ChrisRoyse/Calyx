//! Loom pair-gain materialization gate (issue #208).
//!
//! The handbook (§7.3 "Materialization policy", §13) and PRD `04_ASSOCIATION_ENGINE_NO_EMBEDDERS.md`
//! §1.1 require that Loom *interaction* cross-terms be materialized eagerly **only when the pair's
//! gain in bits ≥ ~0.05** — an information-theoretic gate at **pair** granularity that keeps the
//! `C(N, 2)` interaction set from exploding on record pairs that add no joint signal. This is
//! distinct from #45 (the 0.05-bit floor at *lens* granularity) and #74 (cost-granularity fan-out
//! bounds).
//!
//! This module composes the real subsystems:
//!
//! - `calyx_loom::plan_cross_terms_checked` owns the eager/lazy policy: Agreement is always eager
//!   (cheap scalar edge), Delta/Concat are always lazy, and **Interaction is eager iff the measured
//!   pair-gain ≥ 0.05 bits**, else lazy.
//! - `calyx_assay::ksg_mi_continuous_discrete` supplies the *measured* pair-gain from grounded
//!   outcomes: `pair_gain = pair_bits − max(left_bits, right_bits)`, where `left`/`right` are each
//!   slot's MI about the outcome and `pair` is the joint `[left, right]` MI about the outcome.
//!
//! Below the MI sample floor a pair is treated as Provisional and never eagerly materialized on
//! unproven signal (its effective gain is forced to 0 → lazy). A pair that is individually strong
//! but jointly redundant has a negative pair-gain → lazy. Every decision is persisted per pair with
//! the measured bits and provenance so the gate is auditable and reproducible. Fail closed on
//! non-finite input or an anchor that carries no boolean outcome (doctrine #1).

use std::collections::BTreeMap;

use calyx_assay::{MIN_ASSAY_SAMPLES, TrustTag, ksg_mi_continuous_discrete};
use calyx_core::{Anchor, AnchorValue, Clock, SlotId};
use calyx_loom::{
    CrossTermKind, MaterializationAction, MaterializationPlan, plan_cross_terms_checked,
};
use serde::{Deserialize, Serialize};

use crate::error::{PolyError, Result};
use crate::grounding::rollup_trust;
use crate::panel_diagnostics::{ESTIMATOR_KSG, PanelMatrix};

/// The interaction-materialization pair-gain floor in bits (handbook §7.3).
pub const PAIR_GAIN_BITS_THRESHOLD: f32 = 0.05;
/// Schema tag persisted with every materialization-plan artifact.
pub const MATERIALIZATION_PLAN_SCHEMA_VERSION: &str = "poly.materialization_plan.v1";
/// Artifact-kind tag persisted with every materialization-plan row.
pub const MATERIALIZATION_PLAN_ARTIFACT_KIND: &str = "poly_materialization_plan";
/// Default KSG neighbor count for pair-gain MI.
pub const DEFAULT_PAIR_GAIN_K: usize = 3;

/// An anchor grounding a pair-gain measurement carried no boolean outcome.
pub const ERR_NON_BOOL_ANCHOR: &str = "CALYX_POLY_PAIRGAIN_NON_BOOL_ANCHOR";
/// The outcome labels were single-class (no contrast) so MI about the outcome is undefined.
pub const ERR_DEGENERATE_OUTCOME: &str = "CALYX_POLY_PAIRGAIN_DEGENERATE_OUTCOME";

/// The measured pair-gain for one slot pair and the gate's resulting interaction decision.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PairGainMeasurement {
    /// Left slot index (panel order).
    pub slot_a: u16,
    /// Right slot index (panel order).
    pub slot_b: u16,
    /// Left slot key.
    pub key_a: String,
    /// Right slot key.
    pub key_b: String,
    /// `I(left; outcome)` in bits.
    pub left_bits: f32,
    /// `I(right; outcome)` in bits.
    pub right_bits: f32,
    /// `I([left, right]; outcome)` in bits.
    pub pair_bits: f32,
    /// `pair_bits − max(left_bits, right_bits)` — the measured joint gain.
    pub pair_gain: f32,
    /// Paired samples the measurement used.
    pub n_samples: usize,
    /// True if the pair fell below the MI floor and was treated as lazy on unproven signal.
    pub provisional: bool,
    /// Trust derived from the grounding anchors (proxy or below-floor → Provisional).
    pub trust: TrustTag,
    /// The gate's decision for this pair's Interaction cross-term.
    pub interaction_eager: bool,
}

/// The persisted materialization-plan record — the FSV source of truth on disk.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PairGainMaterializationRecord {
    /// Schema tag.
    pub schema_version: String,
    /// Artifact-kind tag.
    pub artifact_kind: String,
    /// The domain the panel belongs to.
    pub domain: String,
    /// The panel version the observations were measured with.
    pub panel_version: u32,
    /// Estimator identity for the pair-gain bits.
    pub estimator: String,
    /// Ordered slot keys.
    pub slot_keys: Vec<String>,
    /// Paired samples per slot.
    pub n_samples: usize,
    /// The pair-gain floor in bits used by the gate.
    pub threshold_bits: f32,
    /// The Loom materialization plan (one entry per cross-term kind per pair).
    pub plan: MaterializationPlan,
    /// The measured pair-gain and decision for every slot pair.
    pub measurements: Vec<PairGainMeasurement>,
    /// Count of interaction cross-terms materialized eagerly.
    pub interaction_eager_count: usize,
    /// Record-level trust: Trusted only if every grounding anchor is resolved and no pair was
    /// below-floor; a proxy anchor or a below-floor pair → Provisional.
    pub trust: TrustTag,
    /// blake3 of the canonical plan payload, for reproducible provenance.
    pub provenance_hash: String,
    /// Wall-clock (from the injected [`Clock`]) at computation.
    pub computed_at: u64,
}

/// Extracts the boolean outcome label (0/1) from a grounding anchor, failing closed otherwise.
fn outcome_label(anchor: &Anchor) -> Result<usize> {
    match &anchor.value {
        AnchorValue::Bool(v) => Ok(usize::from(*v)),
        other => Err(PolyError::diagnostics(
            ERR_NON_BOOL_ANCHOR,
            format!("pair-gain requires a boolean outcome anchor, got {other:?}"),
        )),
    }
}

/// True if the outcome labels have at least `k + 1` members in each of both classes (the mixed
/// continuous-discrete KSG requirement) and clear the global sample floor.
fn labels_meet_floor(labels: &[usize], k: usize) -> bool {
    if labels.len() < MIN_ASSAY_SAMPLES {
        return false;
    }
    let ones = labels.iter().filter(|&&l| l == 1).count();
    let zeros = labels.len() - ones;
    ones > k && zeros > k
}

fn mi_bits(x: &[Vec<f32>], labels: &[usize], k: usize) -> Result<f32> {
    Ok(ksg_mi_continuous_discrete(x, labels, k)?.bits)
}

/// Measures the pair-gain for slots `(i, j)` about the outcome and returns `(measurement, effective
/// gain fed to the gate)`. Below the floor the effective gain is 0 (→ lazy) and the measurement is
/// marked Provisional. A degenerate single-class outcome fails closed (MI about a constant is
/// meaningless — not a materialization decision to guess at).
fn measure_pair(
    matrix: &PanelMatrix,
    labels: &[usize],
    i: usize,
    j: usize,
    k: usize,
    anchor_trust: TrustTag,
) -> Result<(PairGainMeasurement, f32)> {
    let cols = matrix.columns();
    let keys = matrix.slot_keys();
    let n = matrix.n_samples();
    let base = PairGainMeasurement {
        slot_a: i as u16,
        slot_b: j as u16,
        key_a: keys[i].clone(),
        key_b: keys[j].clone(),
        left_bits: 0.0,
        right_bits: 0.0,
        pair_bits: 0.0,
        pair_gain: 0.0,
        n_samples: n,
        provisional: true,
        trust: TrustTag::Provisional,
        interaction_eager: false,
    };
    if !labels_meet_floor(labels, k) {
        return Ok((base, 0.0));
    }

    let left_x: Vec<Vec<f32>> = cols[i].iter().map(|&v| vec![v]).collect();
    let right_x: Vec<Vec<f32>> = cols[j].iter().map(|&v| vec![v]).collect();
    let joint_x: Vec<Vec<f32>> = cols[i]
        .iter()
        .zip(&cols[j])
        .map(|(&a, &b)| vec![a, b])
        .collect();

    let left_bits = mi_bits(&left_x, labels, k)?;
    let right_bits = mi_bits(&right_x, labels, k)?;
    let pair_bits = mi_bits(&joint_x, labels, k)?;
    let pair_gain = pair_bits - left_bits.max(right_bits);

    let measurement = PairGainMeasurement {
        left_bits,
        right_bits,
        pair_bits,
        pair_gain,
        provisional: false,
        trust: anchor_trust,
        interaction_eager: pair_gain >= PAIR_GAIN_BITS_THRESHOLD,
        ..base
    };
    // The engine gate re-applies the ≥0.05 rule to the effective gain we return here.
    Ok((measurement, pair_gain))
}

/// Computes the pair-gain materialization plan over a grounded panel matrix. The outcome labels come
/// from each observation's grounding anchor (resolved won/lost or a proxy up/down); a single-class
/// outcome fails closed.
pub fn compute_pair_gain_plan(
    domain: &str,
    panel_version: u32,
    matrix: &PanelMatrix,
    clock: &dyn Clock,
    k: usize,
) -> Result<PairGainMaterializationRecord> {
    let keys = matrix.slot_keys().to_vec();
    let n_samples = matrix.n_samples();
    let anchors = matrix.anchors();
    let labels: Vec<usize> = anchors
        .iter()
        .map(outcome_label)
        .collect::<Result<Vec<_>>>()?;
    let ones = labels.iter().filter(|&&l| l == 1).count();
    if ones == 0 || ones == labels.len() {
        return Err(PolyError::diagnostics(
            ERR_DEGENERATE_OUTCOME,
            format!(
                "outcome is single-class ({ones} of {} positive); MI about a constant outcome is undefined",
                labels.len()
            ),
        ));
    }
    let anchor_trust = rollup_trust(anchors)?;

    // Measure every pair, collecting the measurement and the effective gain fed to the loom gate.
    let n = keys.len();
    let mut measurements = Vec::new();
    let mut gains: BTreeMap<(u16, u16), f32> = BTreeMap::new();
    let mut any_provisional = false;
    for i in 0..n {
        for j in (i + 1)..n {
            let (m, gain) = measure_pair(matrix, &labels, i, j, k, anchor_trust)?;
            any_provisional |= m.provisional;
            gains.insert((i as u16, j as u16), gain);
            measurements.push(m);
        }
    }

    // The loom engine owns the eager/lazy policy; we supply the measured gain per pair.
    let slot_ids: Vec<SlotId> = (0..n as u16).map(SlotId::new).collect();
    let plan = plan_cross_terms_checked(&slot_ids, |a, b| {
        let key = order_pair(a.get(), b.get());
        Ok(*gains.get(&key).expect("gain computed for every pair"))
    })?;

    let interaction_eager_count = plan
        .entries
        .iter()
        .filter(|e| {
            e.kind == CrossTermKind::Interaction && e.action == MaterializationAction::EagerStore
        })
        .count();

    let trust = if any_provisional || anchor_trust != TrustTag::Trusted {
        TrustTag::Provisional
    } else {
        TrustTag::Trusted
    };
    let _ = clock; // reserved for future per-entry timestamps; computed_at below is the record time.
    let provenance_hash = provenance_hash(domain, panel_version, &keys, n_samples, &measurements);

    Ok(PairGainMaterializationRecord {
        schema_version: MATERIALIZATION_PLAN_SCHEMA_VERSION.to_string(),
        artifact_kind: MATERIALIZATION_PLAN_ARTIFACT_KIND.to_string(),
        domain: domain.to_string(),
        panel_version,
        estimator: ESTIMATOR_KSG.to_string(),
        slot_keys: keys,
        n_samples,
        threshold_bits: PAIR_GAIN_BITS_THRESHOLD,
        plan,
        measurements,
        interaction_eager_count,
        trust,
        provenance_hash,
        computed_at: clock.now(),
    })
}

fn order_pair(a: u16, b: u16) -> (u16, u16) {
    if a <= b { (a, b) } else { (b, a) }
}

/// Persists a materialization-plan record as JSON under `dir` and returns its path.
pub fn write_pair_gain_plan(
    dir: &std::path::Path,
    record: &PairGainMaterializationRecord,
) -> Result<std::path::PathBuf> {
    let file_name = format!(
        "materialization_plan_{}_v{}.json",
        sanitize(&record.domain),
        record.panel_version
    );
    crate::diagnostics_store::write_json(dir, &file_name, record)
}

/// Reads a persisted materialization-plan record back from disk.
pub fn read_pair_gain_plan(path: &std::path::Path) -> Result<PairGainMaterializationRecord> {
    crate::diagnostics_store::read_json(path)
}

fn sanitize(domain: &str) -> String {
    domain
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

fn provenance_hash(
    domain: &str,
    panel_version: u32,
    slot_keys: &[String],
    n_samples: usize,
    measurements: &[PairGainMeasurement],
) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(domain.as_bytes());
    hasher.update(&panel_version.to_le_bytes());
    hasher.update(&(n_samples as u64).to_le_bytes());
    for key in slot_keys {
        hasher.update(key.as_bytes());
        hasher.update(&[0]);
    }
    for m in measurements {
        hasher.update(&m.slot_a.to_le_bytes());
        hasher.update(&m.slot_b.to_le_bytes());
        hasher.update(&m.pair_gain.to_le_bytes());
        hasher.update(&[u8::from(m.interaction_eager)]);
    }
    hasher.finalize().to_hex().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use calyx_core::{AnchorKind, AnchorValue};

    fn bool_anchor(v: bool) -> Anchor {
        Anchor {
            kind: AnchorKind::TestPass,
            value: AnchorValue::Bool(v),
            source: "uma:test".to_string(),
            observed_at: 1,
            confidence: 1.0,
        }
    }

    #[test]
    fn outcome_label_maps_bool() {
        assert_eq!(outcome_label(&bool_anchor(true)).unwrap(), 1);
        assert_eq!(outcome_label(&bool_anchor(false)).unwrap(), 0);
        let numeric = Anchor {
            value: AnchorValue::Number(1.0),
            ..bool_anchor(true)
        };
        assert_eq!(
            outcome_label(&numeric).unwrap_err().code(),
            ERR_NON_BOOL_ANCHOR
        );
    }

    #[test]
    fn labels_floor_requires_both_classes() {
        let all_ones = vec![1usize; 100];
        assert!(!labels_meet_floor(&all_ones, 3));
        let mixed: Vec<usize> = (0..100).map(|i| usize::from(i % 2 == 0)).collect();
        assert!(labels_meet_floor(&mixed, 3));
        let too_few = vec![0usize, 1, 0, 1];
        assert!(!labels_meet_floor(&too_few, 3));
    }
}
