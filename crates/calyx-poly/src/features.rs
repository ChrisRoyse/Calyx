//! Derived market signals computed from raw feed fields — pure, deterministic functions.
//!
//! These are the "compute a scalar association input" transforms: order-flow imbalance, holder
//! concentration, arbitrage residuals, distance-from-50, realized volatility. They feed both the
//! constellation `scalars` map (verbatim numeric truth) and the numeric lenses (encoded to vectors).

/// Order-flow imbalance over a window: `(buy − sell) / (buy + sell)`, in `[-1, 1]`.
/// Zero total volume is absence, not a measured balanced-flow signal.
pub fn order_flow_imbalance(buy_volume: f64, sell_volume: f64) -> Option<f64> {
    if !buy_volume.is_finite() || !sell_volume.is_finite() || buy_volume < 0.0 || sell_volume < 0.0
    {
        return None;
    }
    let total = buy_volume + sell_volume;
    if total <= 0.0 {
        None
    } else {
        Some(((buy_volume - sell_volume) / total).clamp(-1.0, 1.0))
    }
}

/// Herfindahl–Hirschman concentration of a set of holdings, normalized to `[0, 1]`.
/// `Σ (sᵢ/Σs)²`. One holder → 1.0 (fully concentrated); many equal holders → ~0. Empty → 0.0.
pub fn herfindahl(holdings: &[f64]) -> f64 {
    let total: f64 = holdings.iter().filter(|x| x.is_finite() && **x > 0.0).sum();
    if total <= 0.0 {
        return 0.0;
    }
    holdings
        .iter()
        .filter(|x| x.is_finite() && **x > 0.0)
        .map(|x| {
            let f = x / total;
            f * f
        })
        .sum()
}

/// Largest positive share of a finite size distribution, in `[0, 1]`.
pub fn top_share(amounts: &[f64]) -> f64 {
    let positives = amounts.iter().filter(|x| x.is_finite() && **x > 0.0);
    let total: f64 = positives.clone().sum();
    if total <= 0.0 {
        return 0.0;
    }
    positives
        .map(|x| x / total)
        .max_by(|a, b| a.total_cmp(b))
        .unwrap_or(0.0)
}

/// Fraction of a market's held size on the YES side, in `[0, 1]`.
pub fn top_yes_fraction(yes_size: f64, no_size: f64) -> Option<f64> {
    if !yes_size.is_finite() || !no_size.is_finite() || yes_size < 0.0 || no_size < 0.0 {
        return None;
    }
    let total = yes_size + no_size;
    if total <= 0.0 {
        None
    } else {
        Some((yes_size / total).clamp(0.0, 1.0))
    }
}

/// Distance of a price from 0.5 — the favorite/longshot axis, in `[0, 0.5]`.
pub fn distance_from_50(price: f64) -> Option<f64> {
    if !price.is_finite() {
        return None;
    }
    Some((price - 0.5).abs())
}

/// Binary internal-arbitrage residual: `yes_price + no_price − 1`. Nonzero ⇒ mint/merge arb.
pub fn yes_no_residual(yes_price: f64, no_price: f64) -> f64 {
    yes_price + no_price - 1.0
}

/// negRisk multi-outcome residual: `Σ yes_prices − 1`. Negative ⇒ long-arb (buy every YES < $1).
pub fn negrisk_sum_residual(yes_prices: &[f64]) -> f64 {
    let sum: f64 = yes_prices.iter().filter(|p| p.is_finite()).sum();
    sum - 1.0
}

/// Sample standard deviation of a return series (realized volatility proxy).
pub fn realized_vol(returns: &[f64]) -> Option<f64> {
    if returns.iter().any(|x| !x.is_finite()) {
        return None;
    }
    let n = returns.len();
    if n < 2 {
        return None;
    }
    let mean = returns.iter().sum::<f64>() / n as f64;
    let var = returns.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (n as f64 - 1.0);
    Some(var.sqrt())
}

/// Bid/ask spread from best bid and ask (guards inversion/absence).
pub fn spread(best_bid: f64, best_ask: f64) -> Option<f64> {
    if !best_bid.is_finite() || !best_ask.is_finite() {
        return None;
    }
    Some((best_ask - best_bid).max(0.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ofi_bounds_and_sign() {
        assert_eq!(order_flow_imbalance(100.0, 0.0), Some(1.0));
        assert_eq!(order_flow_imbalance(0.0, 100.0), Some(-1.0));
        assert_eq!(order_flow_imbalance(50.0, 50.0), Some(0.0));
        assert_eq!(order_flow_imbalance(0.0, 0.0), None);
        assert_eq!(order_flow_imbalance(f64::NAN, 1.0), None);
    }

    #[test]
    fn yes_fraction_and_spread_keep_absence_typed() {
        assert_eq!(top_yes_fraction(50.0, 50.0), Some(0.5));
        assert_eq!(top_yes_fraction(0.0, 0.0), None);
        assert!((spread(0.49, 0.51).unwrap() - 0.02).abs() < 1.0e-9);
        assert_eq!(spread(0.50, 0.50), Some(0.0));
        assert_eq!(spread(f64::NAN, f64::NAN), None);
    }

    #[test]
    fn herfindahl_concentration() {
        assert!((herfindahl(&[100.0]) - 1.0).abs() < 1.0e-9);
        let many = herfindahl(&[1.0; 100]);
        assert!(
            many < 0.02,
            "100 equal holders should be near 0, got {many}"
        );
        assert_eq!(herfindahl(&[]), 0.0);
    }

    #[test]
    fn top_share_concentration() {
        assert!((top_share(&[70.0, 10.0, 10.0, 10.0]) - 0.7).abs() < 1.0e-9);
        assert_eq!(top_share(&[f64::NAN, -1.0, 0.0]), 0.0);
        assert!((top_share(&[1.0; 4]) - 0.25).abs() < 1.0e-9);
    }

    #[test]
    fn residuals() {
        assert!((yes_no_residual(0.6, 0.45) - 0.05).abs() < 1.0e-9);
        assert!((negrisk_sum_residual(&[0.3, 0.3, 0.3]) - (-0.1)).abs() < 1.0e-9);
    }

    #[test]
    fn distance_and_vol() {
        assert!((distance_from_50(0.9).unwrap() - 0.4).abs() < 1.0e-9);
        assert_eq!(distance_from_50(f64::NAN), None);
        assert_eq!(realized_vol(&[1.0]), None);
        assert_eq!(realized_vol(&[1.0, f64::NAN]), None);
        assert!(realized_vol(&[1.0, 2.0, 3.0]).unwrap() > 0.0);
    }
}
