//! Embedder-free multi-level public-book shape lens (#41).

use calyx_core::{AbsentReason, SlotId, SlotShape, SlotVector};

use crate::encode::{l2_normalize, signed_log};
use crate::lenses::SignalLens;
use crate::model::{Level, MarketSnapshot};

pub const BOOK_SHAPE_LEVELS: usize = 5;
pub const BOOK_SHAPE_VECTOR_DIM: u32 = 4 + (BOOK_SHAPE_LEVELS as u32 * 2);
pub const ERR_BOOK_SHAPE_INVALID: &str = "CALYX_POLY_BOOK_SHAPE_INVALID";
pub const ERR_BOOK_SHAPE_CROSSED: &str = "CALYX_POLY_BOOK_SHAPE_CROSSED";

pub struct BookShapeLens {
    slot: SlotId,
    key: String,
}

impl BookShapeLens {
    pub fn new(slot: u16, key: impl Into<String>) -> Self {
        Self {
            slot: SlotId::new(slot),
            key: key.into(),
        }
    }
}

impl SignalLens for BookShapeLens {
    fn slot(&self) -> SlotId {
        self.slot
    }

    fn key(&self) -> &str {
        &self.key
    }

    fn shape(&self) -> SlotShape {
        SlotShape::Dense(BOOK_SHAPE_VECTOR_DIM)
    }

    fn measure(&self, snapshot: &MarketSnapshot) -> SlotVector {
        match compute_book_shape_vector(snapshot) {
            Ok(data) => SlotVector::Dense {
                dim: BOOK_SHAPE_VECTOR_DIM,
                data,
            },
            Err(reason) => SlotVector::Absent { reason },
        }
    }
}

pub fn compute_book_shape_vector(
    snapshot: &MarketSnapshot,
) -> std::result::Result<Vec<f32>, AbsentReason> {
    if snapshot.book.bids.is_empty() || snapshot.book.asks.is_empty() {
        return Err(AbsentReason::LensUnavailable);
    }
    let mut bids = sorted_levels(&snapshot.book.bids, true)?;
    let mut asks = sorted_levels(&snapshot.book.asks, false)?;
    let best_bid = bids[0].price;
    let best_ask = asks[0].price;
    if best_bid >= best_ask {
        return Err(AbsentReason::Error(format!(
            "{ERR_BOOK_SHAPE_CROSSED}: best_bid={best_bid:.6} best_ask={best_ask:.6}"
        )));
    }

    bids.truncate(BOOK_SHAPE_LEVELS);
    asks.truncate(BOOK_SHAPE_LEVELS);
    let bid_cumulative = cumulative_sizes(&bids);
    let ask_cumulative = cumulative_sizes(&asks);
    let bid_total = *bid_cumulative.last().unwrap_or(&0.0);
    let ask_total = *ask_cumulative.last().unwrap_or(&0.0);
    let depth_total = bid_total + ask_total;
    let imbalance = if depth_total > 0.0 {
        ((bid_total - ask_total) / depth_total).clamp(-1.0, 1.0)
    } else {
        0.0
    };

    let mut out = vec![
        best_bid as f32,
        best_ask as f32,
        (best_ask - best_bid) as f32,
        imbalance as f32,
    ];
    out.extend(bid_cumulative.into_iter().map(log_size));
    out.extend(ask_cumulative.into_iter().map(log_size));
    l2_normalize(&mut out);
    Ok(out)
}

fn sorted_levels(
    levels: &[Level],
    bid_side: bool,
) -> std::result::Result<Vec<Level>, AbsentReason> {
    let mut out = Vec::with_capacity(levels.len());
    for (idx, level) in levels.iter().enumerate() {
        if !level.price.is_finite()
            || !(0.0..=1.0).contains(&level.price)
            || !level.size.is_finite()
            || level.size <= 0.0
        {
            return Err(AbsentReason::Error(format!(
                "{ERR_BOOK_SHAPE_INVALID}: level {idx} must have finite price in [0,1] and positive finite size"
            )));
        }
        out.push(level.clone());
    }
    if bid_side {
        out.sort_by(|a, b| b.price.total_cmp(&a.price));
    } else {
        out.sort_by(|a, b| a.price.total_cmp(&b.price));
    }
    Ok(out)
}

fn cumulative_sizes(levels: &[Level]) -> Vec<f64> {
    let mut acc = 0.0;
    let mut out = Vec::with_capacity(BOOK_SHAPE_LEVELS);
    for idx in 0..BOOK_SHAPE_LEVELS {
        if let Some(level) = levels.get(idx) {
            acc += level.size;
        }
        out.push(acc);
    }
    out
}

fn log_size(value: f64) -> f32 {
    signed_log(value) as f32
}
