//! The panel of embedder-free signal lenses that map a [`MarketSnapshot`] to typed slot-vectors.
//!
//! Each [`SignalLens`] owns a deterministic encoder (from [`crate::encode`]) and a field selector.
//! A missing feed field yields [`SlotVector::Absent`], never a fabricated zero. [`default_panel`]
//! assembles the v1 Polymarket panel described in `docs/prd/05_SIGNAL_LENSES_CATALOG.md`.

use std::collections::BTreeMap;
use std::f64::consts::PI;

use calyx_core::{AbsentReason, SlotId, SlotShape, SlotVector, SparseEntry};

use crate::book_shape_lens::BookShapeLens;
use crate::encode::{QuantileEncoder, RffEncoder, one_hot, signed_log};
use crate::model::MarketSnapshot;
use crate::question_bm25_lens::{QUESTION_BM25_KEY, QuestionBm25Lens};
use crate::seed_registry::{
    ARB_RESIDUAL_RFF, DISTANCE_FROM_50_RFF, MOMENTUM_RFF, OFI_VEC_RFF, PRICE_RFF, SPREAD_RFF,
};
use crate::temporal_lens::{
    E2_RECENCY_KEY, E3_PERIODIC_KEY, E4_POSITIONAL_KEY, PolyTemporalLens, TemporalLensKind,
};
use crate::toxicity_lens::ToxicityLens;

/// A frozen, deterministic lens: one measurement axis over a market snapshot.
pub trait SignalLens: Send + Sync {
    /// Stable panel slot id.
    fn slot(&self) -> SlotId;
    /// Human-readable slot key.
    fn key(&self) -> &str;
    /// Physical vector shape emitted.
    fn shape(&self) -> SlotShape;
    /// Deterministically measures the snapshot into a slot-vector.
    fn measure(&self, snapshot: &MarketSnapshot) -> SlotVector;
}

/// Extracts an optional scalar from a snapshot.
type ScalarExtract = fn(&MarketSnapshot) -> Option<f64>;

/// A continuous-scalar lens: extract a field, optionally transform, RFF-encode to a dense vector.
pub struct ScalarRffLens {
    slot: SlotId,
    key: String,
    enc: RffEncoder,
    extract: ScalarExtract,
    transform: fn(f64) -> f64,
}

impl ScalarRffLens {
    /// Builds a scalar RFF lens.
    pub fn new(
        slot: u16,
        key: impl Into<String>,
        enc: RffEncoder,
        extract: ScalarExtract,
        transform: fn(f64) -> f64,
    ) -> Self {
        Self {
            slot: SlotId::new(slot),
            key: key.into(),
            enc,
            extract,
            transform,
        }
    }
}

fn identity(x: f64) -> f64 {
    x
}

impl SignalLens for ScalarRffLens {
    fn slot(&self) -> SlotId {
        self.slot
    }
    fn key(&self) -> &str {
        &self.key
    }
    fn shape(&self) -> SlotShape {
        SlotShape::Dense(self.enc.dim() as u32)
    }
    fn measure(&self, snapshot: &MarketSnapshot) -> SlotVector {
        match (self.extract)(snapshot) {
            Some(x) if x.is_finite() => {
                let data = self.enc.encode((self.transform)(x));
                SlotVector::Dense {
                    dim: data.len() as u32,
                    data,
                }
            }
            _ => SlotVector::Absent {
                reason: AbsentReason::LensUnavailable,
            },
        }
    }
}

/// A continuous-scalar lens using the quantile / piecewise-linear encoder (heavy-tailed features).
pub struct ScalarQuantileLens {
    slot: SlotId,
    key: String,
    enc: QuantileEncoder,
    extract: ScalarExtract,
    transform: fn(f64) -> f64,
}

impl ScalarQuantileLens {
    /// Builds a scalar quantile lens.
    pub fn new(
        slot: u16,
        key: impl Into<String>,
        enc: QuantileEncoder,
        extract: ScalarExtract,
        transform: fn(f64) -> f64,
    ) -> Self {
        Self {
            slot: SlotId::new(slot),
            key: key.into(),
            enc,
            extract,
            transform,
        }
    }
}

impl SignalLens for ScalarQuantileLens {
    fn slot(&self) -> SlotId {
        self.slot
    }
    fn key(&self) -> &str {
        &self.key
    }
    fn shape(&self) -> SlotShape {
        SlotShape::Dense(self.enc.dim() as u32)
    }
    fn measure(&self, snapshot: &MarketSnapshot) -> SlotVector {
        match (self.extract)(snapshot) {
            Some(x) if x.is_finite() => {
                let data = self.enc.encode((self.transform)(x));
                SlotVector::Dense {
                    dim: data.len() as u32,
                    data,
                }
            }
            _ => SlotVector::Absent {
                reason: AbsentReason::LensUnavailable,
            },
        }
    }
}

/// A one-hot categorical lens over a fixed vocabulary.
pub struct OneHotLens {
    slot: SlotId,
    key: String,
    vocab: Vec<String>,
    extract: fn(&MarketSnapshot) -> Option<String>,
}

impl OneHotLens {
    /// Builds a one-hot lens.
    pub fn new(
        slot: u16,
        key: impl Into<String>,
        vocab: Vec<String>,
        extract: fn(&MarketSnapshot) -> Option<String>,
    ) -> Self {
        Self {
            slot: SlotId::new(slot),
            key: key.into(),
            vocab,
            extract,
        }
    }
}

impl SignalLens for OneHotLens {
    fn slot(&self) -> SlotId {
        self.slot
    }
    fn key(&self) -> &str {
        &self.key
    }
    fn shape(&self) -> SlotShape {
        SlotShape::Dense(self.vocab.len().max(1) as u32)
    }
    fn measure(&self, snapshot: &MarketSnapshot) -> SlotVector {
        match (self.extract)(snapshot) {
            Some(value) => match self.vocab.iter().position(|v| v == &value) {
                Some(idx) => {
                    let data = one_hot(idx, self.vocab.len());
                    SlotVector::Dense {
                        dim: data.len() as u32,
                        data,
                    }
                }
                None => SlotVector::Absent {
                    reason: AbsentReason::NotApplicable,
                },
            },
            None => SlotVector::Absent {
                reason: AbsentReason::LensUnavailable,
            },
        }
    }
}

/// Sparse holder-membership lens: hashes each top-holder wallet to an index and stores its
/// normalized share. Two markets sharing whales get a high cosine here — an entity association edge.
pub struct HoldersMembershipLens {
    slot: SlotId,
    key: String,
    dim: u32,
}

impl HoldersMembershipLens {
    /// Builds a holder-membership lens with ambient dimension `dim`.
    pub fn new(slot: u16, key: impl Into<String>, dim: u32) -> Self {
        Self {
            slot: SlotId::new(slot),
            key: key.into(),
            dim: dim.max(1),
        }
    }

    fn wallet_index(&self, wallet: &str) -> u32 {
        let h = blake3::hash(wallet.as_bytes());
        let bytes = h.as_bytes();
        let raw = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        raw % self.dim
    }
}

impl SignalLens for HoldersMembershipLens {
    fn slot(&self) -> SlotId {
        self.slot
    }
    fn key(&self) -> &str {
        &self.key
    }
    fn shape(&self) -> SlotShape {
        SlotShape::Sparse(self.dim)
    }
    fn measure(&self, snapshot: &MarketSnapshot) -> SlotVector {
        if snapshot.holders.is_empty() {
            return SlotVector::Absent {
                reason: AbsentReason::LensUnavailable,
            };
        }
        let total: f64 = snapshot
            .holders
            .iter()
            .map(|h| h.amount.max(0.0))
            .sum::<f64>()
            .max(1.0e-9);
        let mut acc: BTreeMap<u32, f32> = BTreeMap::new();
        for h in &snapshot.holders {
            if h.amount <= 0.0 || !h.amount.is_finite() {
                continue;
            }
            let idx = self.wallet_index(&h.wallet);
            *acc.entry(idx).or_insert(0.0) += (h.amount / total) as f32;
        }
        if acc.is_empty() {
            return SlotVector::Absent {
                reason: AbsentReason::LensUnavailable,
            };
        }
        let entries = acc
            .into_iter()
            .map(|(idx, val)| SparseEntry { idx, val })
            .collect();
        SlotVector::Sparse {
            dim: self.dim,
            entries,
        }
    }
}

/// Periodic (time-of-day / day-of-week) lens: encodes the snapshot timestamp as circular sin/cos —
/// the E3 temporal axis. Dense(4): `[sin(hour), cos(hour), sin(dow), cos(dow)]`.
pub struct PeriodicLens {
    slot: SlotId,
    key: String,
}

impl PeriodicLens {
    /// Builds the periodic lens.
    pub fn new(slot: u16, key: impl Into<String>) -> Self {
        Self {
            slot: SlotId::new(slot),
            key: key.into(),
        }
    }
}

impl SignalLens for PeriodicLens {
    fn slot(&self) -> SlotId {
        self.slot
    }
    fn key(&self) -> &str {
        &self.key
    }
    fn shape(&self) -> SlotShape {
        SlotShape::Dense(4)
    }
    fn measure(&self, snapshot: &MarketSnapshot) -> SlotVector {
        let secs = snapshot.snapshot_ts as f64;
        let hour = (secs.rem_euclid(86_400.0)) / 86_400.0; // fraction of day
        // Monday=0 convention: unix epoch (1970-01-01) was a Thursday (=3).
        let dow = (((secs / 86_400.0).floor() as i64 + 3).rem_euclid(7)) as f64 / 7.0;
        let data = vec![
            (2.0 * PI * hour).sin() as f32,
            (2.0 * PI * hour).cos() as f32,
            (2.0 * PI * dow).sin() as f32,
            (2.0 * PI * dow).cos() as f32,
        ];
        SlotVector::Dense { dim: 4, data }
    }
}

/// A versioned panel of signal lenses.
pub struct PolyPanel {
    /// Panel version (part of the `CxId` and every slot's provenance).
    pub version: u32,
    /// The ordered lenses.
    pub lenses: Vec<Box<dyn SignalLens>>,
}

impl PolyPanel {
    /// Measures every lens over a snapshot into the constellation's slot map.
    pub fn measure_all(&self, snapshot: &MarketSnapshot) -> BTreeMap<SlotId, SlotVector> {
        let mut slots = BTreeMap::new();
        for lens in &self.lenses {
            slots.insert(lens.slot(), lens.measure(snapshot));
        }
        slots
    }
}

// ── Field extractors (function pointers so lenses stay Send + Sync) ──────────────────────────────
fn f_price(s: &MarketSnapshot) -> Option<f64> {
    s.price.or(s.mid)
}
fn f_distance_from_50(s: &MarketSnapshot) -> Option<f64> {
    s.price.or(s.mid).map(|p| (p - 0.5).abs())
}
fn f_spread(s: &MarketSnapshot) -> Option<f64> {
    s.spread
}
fn f_volume(s: &MarketSnapshot) -> Option<f64> {
    s.volume_24h
}
fn f_liquidity(s: &MarketSnapshot) -> Option<f64> {
    s.liquidity
}
fn f_ofi(s: &MarketSnapshot) -> Option<f64> {
    s.ofi
}
fn f_momentum(s: &MarketSnapshot) -> Option<f64> {
    s.one_day_change.or(s.one_hour_change)
}
fn f_arb_residual(s: &MarketSnapshot) -> Option<f64> {
    s.yes_no_residual
}
fn e_category(s: &MarketSnapshot) -> Option<String> {
    s.category.clone()
}
fn e_region(s: &MarketSnapshot) -> Option<String> {
    s.region.clone()
}

/// Default category vocabulary (aligned with the domain roster).
pub fn default_category_vocab() -> Vec<String> {
    [
        "crypto",
        "politics",
        "sports",
        "economics",
        "weather",
        "culture",
        "geopolitics",
        "mentions",
        "other",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

/// Builds the v1 Polymarket panel: embedder-free numeric + categorical + entity + temporal lenses.
///
/// `region_vocab` is domain-specific (states/countries/cities) and supplied by the caller.
pub fn default_panel(version: u32, region_vocab: Vec<String>) -> PolyPanel {
    let lenses: Vec<Box<dyn SignalLens>> = vec![
        Box::new(ScalarRffLens::new(
            0,
            "price_rff",
            PRICE_RFF.encoder(),
            f_price,
            identity,
        )),
        Box::new(ScalarRffLens::new(
            1,
            "distance_from_50",
            DISTANCE_FROM_50_RFF.encoder(),
            f_distance_from_50,
            identity,
        )),
        Box::new(ScalarRffLens::new(
            2,
            "spread_rff",
            SPREAD_RFF.encoder(),
            f_spread,
            signed_log,
        )),
        Box::new(ScalarQuantileLens::new(
            3,
            "logvol_ple",
            QuantileEncoder::new(vec![0.0, 2.0, 4.0, 6.0, 8.0, 10.0, 12.0, 14.0]),
            f_volume,
            signed_log,
        )),
        Box::new(ScalarQuantileLens::new(
            4,
            "liquidity_ple",
            QuantileEncoder::new(vec![0.0, 2.0, 4.0, 6.0, 8.0, 10.0, 12.0]),
            f_liquidity,
            signed_log,
        )),
        Box::new(ScalarRffLens::new(
            5,
            "ofi_vec",
            OFI_VEC_RFF.encoder(),
            f_ofi,
            identity,
        )),
        Box::new(ScalarRffLens::new(
            6,
            "momentum_rff",
            MOMENTUM_RFF.encoder(),
            f_momentum,
            signed_log,
        )),
        Box::new(ScalarRffLens::new(
            7,
            "arb_residual",
            ARB_RESIDUAL_RFF.encoder(),
            f_arb_residual,
            identity,
        )),
        Box::new(OneHotLens::new(
            8,
            "category_oh",
            default_category_vocab(),
            e_category,
        )),
        Box::new(OneHotLens::new(9, "region_oh", region_vocab, e_region)),
        Box::new(HoldersMembershipLens::new(
            10,
            "holders_membership",
            1 << 20,
        )),
        Box::new(PolyTemporalLens::new(
            11,
            E3_PERIODIC_KEY,
            TemporalLensKind::E3Periodic,
        )),
        Box::new(BookShapeLens::new(12, "book_shape")),
        Box::new(ToxicityLens::new(13, "toxicity")),
        Box::new(PolyTemporalLens::new(
            14,
            E2_RECENCY_KEY,
            TemporalLensKind::E2Recency,
        )),
        Box::new(PolyTemporalLens::new(
            15,
            E4_POSITIONAL_KEY,
            TemporalLensKind::E4Positional,
        )),
        Box::new(QuestionBm25Lens::new(16, QUESTION_BM25_KEY)),
    ];
    PolyPanel { version, lenses }
}
