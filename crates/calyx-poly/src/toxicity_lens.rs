//! Embedder-free VPIN-style on-chain flow-toxicity lens (#42).

use calyx_core::{AbsentReason, SlotId, SlotShape, SlotVector};
use serde::{Deserialize, Serialize};

use crate::encode::{l2_normalize, signed_log};
use crate::lenses::SignalLens;
use crate::model::{MarketSnapshot, OnchainFill, OnchainFillSide};

pub const TOXICITY_TARGET_BUCKET_COUNT: usize = 3;
pub const TOXICITY_VECTOR_DIM: u32 = 5;
pub const ERR_TOXICITY_INVALID_FILL: &str = "CALYX_POLY_TOXICITY_INVALID_FILL";
pub const ERR_TOXICITY_LOOKAHEAD: &str = "CALYX_POLY_TOXICITY_LOOKAHEAD";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToxicityBucket {
    pub buy_volume: f64,
    pub sell_volume: f64,
    pub total_volume: f64,
    pub imbalance_abs: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToxicityMetrics {
    pub vpin: f64,
    pub signed_imbalance: f64,
    pub buy_volume: f64,
    pub sell_volume: f64,
    pub total_volume: f64,
    pub largest_fill_share: f64,
    pub bucket_volume: f64,
    pub buckets: Vec<ToxicityBucket>,
}

pub struct ToxicityLens {
    slot: SlotId,
    key: String,
}

impl ToxicityLens {
    pub fn new(slot: u16, key: impl Into<String>) -> Self {
        Self {
            slot: SlotId::new(slot),
            key: key.into(),
        }
    }
}

impl SignalLens for ToxicityLens {
    fn slot(&self) -> SlotId {
        self.slot
    }

    fn key(&self) -> &str {
        &self.key
    }

    fn shape(&self) -> SlotShape {
        SlotShape::Dense(TOXICITY_VECTOR_DIM)
    }

    fn measure(&self, snapshot: &MarketSnapshot) -> SlotVector {
        match compute_toxicity_vector(snapshot) {
            Ok(data) => SlotVector::Dense {
                dim: TOXICITY_VECTOR_DIM,
                data,
            },
            Err(reason) => SlotVector::Absent { reason },
        }
    }
}

pub fn compute_toxicity_vector(
    snapshot: &MarketSnapshot,
) -> std::result::Result<Vec<f32>, AbsentReason> {
    let metrics = compute_toxicity_metrics(snapshot)?;
    let mut out = vec![
        metrics.vpin as f32,
        metrics.signed_imbalance as f32,
        signed_log(metrics.total_volume) as f32,
        metrics.largest_fill_share as f32,
        (metrics.buckets.len() as f32 / TOXICITY_TARGET_BUCKET_COUNT as f32).min(1.0),
    ];
    l2_normalize(&mut out);
    Ok(out)
}

pub fn compute_toxicity_metrics(
    snapshot: &MarketSnapshot,
) -> std::result::Result<ToxicityMetrics, AbsentReason> {
    if snapshot.onchain_fills.is_empty() {
        return Err(AbsentReason::LensUnavailable);
    }
    let fills = sorted_valid_fills(snapshot)?;
    let total_volume: f64 = fills.iter().map(|fill| fill.size).sum();
    if !total_volume.is_finite() || total_volume <= 0.0 {
        return Err(AbsentReason::LensUnavailable);
    }

    let bucket_volume = total_volume / TOXICITY_TARGET_BUCKET_COUNT as f64;
    let largest_fill_share =
        fills.iter().map(|fill| fill.size).fold(0.0_f64, f64::max) / total_volume;
    let mut buckets = Vec::new();
    let mut current = OpenBucket::default();
    let mut buy_volume = 0.0;
    let mut sell_volume = 0.0;

    for fill in &fills {
        let mut remaining = fill.size;
        while remaining > 1.0e-9 {
            let space = (bucket_volume - current.total()).max(0.0);
            let part = remaining.min(space.max(1.0e-9));
            match fill.side {
                OnchainFillSide::Buy => {
                    current.buy += part;
                    buy_volume += part;
                }
                OnchainFillSide::Sell => {
                    current.sell += part;
                    sell_volume += part;
                }
            }
            remaining -= part;
            if current.total() + 1.0e-9 >= bucket_volume {
                buckets.push(current.close());
                current = OpenBucket::default();
            }
        }
    }
    if current.total() > 1.0e-9 {
        buckets.push(current.close());
    }

    let imbalance_sum: f64 = buckets.iter().map(|bucket| bucket.imbalance_abs).sum();
    Ok(ToxicityMetrics {
        vpin: (imbalance_sum / total_volume).clamp(0.0, 1.0),
        signed_imbalance: ((buy_volume - sell_volume) / total_volume).clamp(-1.0, 1.0),
        buy_volume,
        sell_volume,
        total_volume,
        largest_fill_share,
        bucket_volume,
        buckets,
    })
}

fn sorted_valid_fills(
    snapshot: &MarketSnapshot,
) -> std::result::Result<Vec<OnchainFill>, AbsentReason> {
    let mut fills = Vec::with_capacity(snapshot.onchain_fills.len());
    for (idx, fill) in snapshot.onchain_fills.iter().enumerate() {
        if fill.timestamp > snapshot.snapshot_ts {
            return Err(AbsentReason::Error(format!(
                "{ERR_TOXICITY_LOOKAHEAD}: fill {idx} timestamp {} exceeds snapshot_ts {}",
                fill.timestamp, snapshot.snapshot_ts
            )));
        }
        if fill.tx_hash.trim().is_empty()
            || fill.maker.trim().is_empty()
            || fill.taker.trim().is_empty()
            || !fill.price.is_finite()
            || !(0.0..=1.0).contains(&fill.price)
            || !fill.size.is_finite()
            || fill.size <= 0.0
        {
            return Err(AbsentReason::Error(format!(
                "{ERR_TOXICITY_INVALID_FILL}: fill {idx} must have ids, price in [0,1], and positive finite size"
            )));
        }
        fills.push(fill.clone());
    }
    fills.sort_by(|a, b| {
        a.timestamp
            .cmp(&b.timestamp)
            .then_with(|| a.tx_hash.cmp(&b.tx_hash))
            .then_with(|| a.log_index.cmp(&b.log_index))
    });
    Ok(fills)
}

#[derive(Default)]
struct OpenBucket {
    buy: f64,
    sell: f64,
}

impl OpenBucket {
    fn total(&self) -> f64 {
        self.buy + self.sell
    }

    fn close(self) -> ToxicityBucket {
        let total = self.total();
        ToxicityBucket {
            buy_volume: self.buy,
            sell_volume: self.sell,
            total_volume: total,
            imbalance_abs: (self.buy - self.sell).abs(),
        }
    }
}
