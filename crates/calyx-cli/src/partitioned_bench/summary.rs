use serde_json::json;

pub(super) fn percentiles(values: &[u64]) -> serde_json::Value {
    summarize_u64(values)
}

pub(super) fn summarize_u64(values: &[u64]) -> serde_json::Value {
    let mut s = values.to_vec();
    s.sort_unstable();
    let pct = |p: usize| -> u64 {
        if s.is_empty() {
            return 0;
        }
        let rank = ((p as f64 / 1000.0) * s.len() as f64).ceil() as usize;
        s[rank.saturating_sub(1).min(s.len() - 1)]
    };
    json!({ "p50": pct(500), "p99": pct(990), "p999": pct(999), "max": s.last().copied().unwrap_or(0) })
}
