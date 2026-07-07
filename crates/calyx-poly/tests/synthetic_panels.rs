//! Deterministic synthetic panel generators for #207/#208/#209 FSV tests.
//!
//! Every generator is seeded with a fixed `ChaCha8Rng` so the constructed truth (a known redundancy,
//! a known synergy, a known pair-gain) is reproducible run-to-run. This is real generated data with
//! a *known* information structure — not mock data that hard-codes the estimator's answer.

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::f64::consts::PI;

use calyx_core::{Anchor, AnchorKind, AnchorValue};
use calyx_poly::grounding::{ProxyKind, proxy_anchor};
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;

/// A standard-normal sample via Box–Muller from the seeded uniform stream.
pub fn gaussian(rng: &mut ChaCha8Rng) -> f32 {
    let u1 = rng.r#gen::<f64>().max(1e-12);
    let u2 = rng.r#gen::<f64>();
    ((-2.0 * u1.ln()).sqrt() * (2.0 * PI * u2).cos()) as f32
}

/// A resolved-UMA outcome anchor carrying boolean outcome `won` (Trusted grounding).
pub fn resolved_anchor(won: bool, i: usize) -> Anchor {
    Anchor {
        kind: AnchorKind::TestPass,
        value: AnchorValue::Bool(won),
        source: "uma:synthetic".to_string(),
        observed_at: i as u64,
        confidence: 1.0,
    }
}

/// A proxy outcome anchor carrying boolean outcome `up` (Provisional grounding, confidence 0.6).
pub fn proxy_up24h(up: bool, i: usize) -> Anchor {
    proxy_anchor(ProxyKind::Up24h, up, 0.6, i as u64).expect("proxy anchor in (0,1)")
}

/// Columns keyed by slot name, one value per observation.
pub struct SyntheticPanel {
    pub keys: Vec<String>,
    pub columns: Vec<Vec<f32>>,
    pub anchors: Vec<Anchor>,
}

impl SyntheticPanel {
    /// Assembles the panel as `(scalars, anchor)` observations for `PanelMatrix::from_scalar_observations`.
    pub fn observations(&self) -> Vec<(BTreeMap<String, f64>, Anchor)> {
        (0..self.anchors.len())
            .map(|i| {
                let mut scalars = BTreeMap::new();
                for (k, key) in self.keys.iter().enumerate() {
                    scalars.insert(key.clone(), self.columns[k][i] as f64);
                }
                (scalars, self.anchors[i].clone())
            })
            .collect()
    }
}

/// #207 happy path: 4 slots `[a, b, sum, dup]` where `sum = a + b` is a clean additive **synergy**
/// triple (a,b independent, but any two of {a,b,sum} determine the third → interaction information is
/// strongly negative) and `dup ≈ a` is a **redundant** copy. Expected: low `n_eff` (only ~2 free
/// dimensions) and a flagged synergistic triple carrying `sum`. The additive relationship is linear
/// and well-conditioned, so the KSG interaction-information CI lands confidently below zero (unlike a
/// multiplicative `a*b`, whose `b = prod/a` blows up near `a ≈ 0`).
pub fn redundant_and_synergy(seed: u64, n: usize) -> SyntheticPanel {
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    let (mut a, mut b, mut sum, mut dup) = (vec![], vec![], vec![], vec![]);
    let mut anchors = Vec::new();
    for i in 0..n {
        let ai = gaussian(&mut rng);
        let bi = gaussian(&mut rng);
        a.push(ai);
        b.push(bi);
        sum.push(ai + bi);
        dup.push(ai + 0.02 * gaussian(&mut rng));
        anchors.push(resolved_anchor(i % 2 == 0, i));
    }
    SyntheticPanel {
        keys: vec!["a".into(), "b".into(), "sum".into(), "dup".into()],
        columns: vec![a, b, sum, dup],
        anchors,
    }
}

/// #207 edge: `slots` fully-independent standard normals → `n_eff ≈ slots`.
pub fn independent(seed: u64, n: usize, slots: usize) -> SyntheticPanel {
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    let mut columns = vec![Vec::with_capacity(n); slots];
    let mut anchors = Vec::new();
    for i in 0..n {
        for col in columns.iter_mut() {
            col.push(gaussian(&mut rng));
        }
        anchors.push(resolved_anchor(i % 2 == 0, i));
    }
    SyntheticPanel {
        keys: (0..slots).map(|k| format!("indep_{k}")).collect(),
        columns,
        anchors,
    }
}

/// #207 edge: `slots` near-identical copies of one normal → `n_eff ≈ 1`.
pub fn fully_redundant(seed: u64, n: usize, slots: usize) -> SyntheticPanel {
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    let mut columns = vec![Vec::with_capacity(n); slots];
    let mut anchors = Vec::new();
    for i in 0..n {
        let base = gaussian(&mut rng);
        for col in columns.iter_mut() {
            col.push(base + 0.01 * gaussian(&mut rng));
        }
        anchors.push(resolved_anchor(i % 2 == 0, i));
    }
    SyntheticPanel {
        keys: (0..slots).map(|k| format!("copy_{k}")).collect(),
        columns,
        anchors,
    }
}

/// #208: 6 slots with a known pair-gain structure about a boolean outcome `y`:
/// - `y = (sign(xa) == sign(xb))` — the pair `(xa, xb)` is a continuous XOR: individually
///   uninformative, **jointly** determines `y` → large positive pair-gain (eager).
/// - `(noise1, noise2)` independent of `y` and each other → ~0 pair-gain (lazy).
/// - `strong1` predicts `y`; `strong2 ≈ strong1` (redundant copy) → the pair adds nothing beyond the
///   stronger member → non-positive pair-gain (lazy).
///
/// `grounded` selects resolved (Trusted) or proxy (Provisional) anchors.
pub fn pair_gain_structure(seed: u64, n: usize, grounded_resolved: bool) -> SyntheticPanel {
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    let (mut xa, mut xb, mut noise1, mut noise2, mut strong1, mut strong2) =
        (vec![], vec![], vec![], vec![], vec![], vec![]);
    let mut anchors = Vec::new();
    for i in 0..n {
        let a = gaussian(&mut rng);
        let b = gaussian(&mut rng);
        let y = (a > 0.0) == (b > 0.0);
        let signal = if y { 1.0 } else { -1.0 };
        let s1 = signal + 0.4 * gaussian(&mut rng);
        xa.push(a);
        xb.push(b);
        noise1.push(gaussian(&mut rng));
        noise2.push(gaussian(&mut rng));
        strong1.push(s1);
        strong2.push(s1 + 0.05 * gaussian(&mut rng));
        anchors.push(if grounded_resolved {
            resolved_anchor(y, i)
        } else {
            proxy_up24h(y, i)
        });
    }
    SyntheticPanel {
        keys: vec![
            "xa".into(),
            "xb".into(),
            "noise1".into(),
            "noise2".into(),
            "strong1".into(),
            "strong2".into(),
        ],
        columns: vec![xa, xb, noise1, noise2, strong1, strong2],
        anchors,
    }
}
