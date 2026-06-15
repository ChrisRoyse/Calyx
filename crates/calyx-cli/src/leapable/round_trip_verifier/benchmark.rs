use std::cmp::Ordering;
use std::collections::BTreeSet;
use std::path::Path;
use std::time::{Duration, Instant};

use super::calyx::{calyx_benchmark_rows, open_calyx};
use super::source::source_rows;
use super::{
    BenchmarkRow, CALYX_ROUND_TRIP_QUERY_INVALID, QueryVec, RecallBenchmark, RecallQueryResult,
    cli_to_calyx, cosine, error, norm, query_bytes,
};
use crate::migrate::manifest::hex_encode;
use crate::migrate::reader::open_sqlite;

const LATENCY_REGRESSION_DIVISOR: u64 = 20;
const LATENCY_JITTER_BUDGET_US: u64 = 100;

pub(super) fn benchmark_recall(
    sqlite_path: &Path,
    calyx_dir: &Path,
    queries: &[QueryVec],
    top_k: usize,
) -> Result<RecallBenchmark, calyx_core::CalyxError> {
    if top_k == 0 || queries.is_empty() {
        return Err(error(
            CALYX_ROUND_TRIP_QUERY_INVALID,
            "benchmark requires --top-k >= 1 and at least one query",
            "provide a non-empty JSONL query file and --top-k >= 1",
        ));
    }
    let conn = open_sqlite(sqlite_path).map_err(cli_to_calyx)?;
    let source = source_rows(&conn)?
        .into_iter()
        .map(|row| BenchmarkRow {
            chunk_id: row.row.chunk_id,
            embedding: row.row.embedding,
        })
        .collect::<Vec<_>>();
    let (_aster_dir, vault, _manifest) = open_calyx(calyx_dir)?;
    let calyx = calyx_benchmark_rows(&vault)?;
    let dim = source.first().map(|row| row.embedding.len()).unwrap_or(0);
    let mut query_rows = Vec::new();
    let mut sqlite_latencies = Vec::new();
    let mut calyx_latencies = Vec::new();
    for query in queries {
        validate_query(query, dim)?;
        let start = Instant::now();
        let sqlite_top = top_rows(&source, &query.query_vec, top_k);
        let sqlite_us = elapsed_us(start.elapsed());
        let start = Instant::now();
        let calyx_top = top_rows(&calyx, &query.query_vec, top_k);
        let calyx_us = elapsed_us(start.elapsed());
        sqlite_latencies.push(sqlite_us);
        calyx_latencies.push(calyx_us);
        let expected = expected_ids(query, &sqlite_top);
        let sqlite_recall = recall(&sqlite_top, &expected);
        let calyx_recall = recall(&calyx_top, &expected);
        let gate = if calyx_recall + f64::EPSILON >= sqlite_recall {
            "PASS"
        } else {
            "FAIL"
        };
        query_rows.push(RecallQueryResult {
            query_hash: hex_encode(blake3::hash(&query_bytes(&query.query_vec)).as_bytes()),
            sqlite_top: ids(&sqlite_top),
            calyx_top: ids(&calyx_top),
            sqlite_recall,
            calyx_recall,
            latency_sqlite_us: sqlite_us,
            latency_calyx_us: calyx_us,
            gate: gate.to_string(),
        });
    }
    let sqlite_mean_recall = mean(query_rows.iter().map(|row| row.sqlite_recall));
    let calyx_mean_recall = mean(query_rows.iter().map(|row| row.calyx_recall));
    let latency_sqlite_p99_us = p99(&mut sqlite_latencies);
    let latency_calyx_p99_us = p99(&mut calyx_latencies);
    let latency_budget_us = latency_budget(latency_sqlite_p99_us);
    let recall_gate = calyx_mean_recall + f64::EPSILON >= sqlite_mean_recall;
    let latency_gate = latency_calyx_p99_us <= latency_budget_us;
    Ok(RecallBenchmark {
        sqlite_mean_recall,
        calyx_mean_recall,
        latency_sqlite_p99_us,
        latency_calyx_p99_us,
        latency_budget_us,
        queries: query_rows,
        gate: if recall_gate && latency_gate {
            "PASS"
        } else {
            "FAIL"
        }
        .to_string(),
    })
}

fn validate_query(query: &QueryVec, dim: usize) -> Result<(), calyx_core::CalyxError> {
    if query.query_vec.len() != dim
        || query.query_vec.iter().any(|value| !value.is_finite())
        || norm(&query.query_vec) == 0.0
    {
        return Err(error(
            CALYX_ROUND_TRIP_QUERY_INVALID,
            format!(
                "query vector dim {} expected {dim} finite nonzero values",
                query.query_vec.len()
            ),
            "provide finite nonzero query vectors matching the source embedding dimension",
        ));
    }
    Ok(())
}

fn top_rows(rows: &[BenchmarkRow], query: &[f32], top_k: usize) -> Vec<BenchmarkRow> {
    let mut scored = rows
        .iter()
        .map(|row| (cosine(query, &row.embedding), row.clone()))
        .collect::<Vec<_>>();
    scored.sort_by(|(left_score, left), (right_score, right)| {
        right_score
            .partial_cmp(left_score)
            .unwrap_or(Ordering::Equal)
            .then_with(|| left.chunk_id.cmp(&right.chunk_id))
    });
    scored.into_iter().take(top_k).map(|(_, row)| row).collect()
}

fn expected_ids(query: &QueryVec, sqlite_top: &[BenchmarkRow]) -> BTreeSet<String> {
    let ids = if query.expected_chunk_ids.is_empty() {
        ids(sqlite_top)
    } else {
        query.expected_chunk_ids.clone()
    };
    ids.into_iter().collect()
}

fn recall(top: &[BenchmarkRow], expected: &BTreeSet<String>) -> f64 {
    if expected.is_empty() {
        return 1.0;
    }
    let hits = top
        .iter()
        .filter(|row| expected.contains(&row.chunk_id))
        .count();
    hits as f64 / expected.len() as f64
}

fn ids(rows: &[BenchmarkRow]) -> Vec<String> {
    rows.iter().map(|row| row.chunk_id.clone()).collect()
}

fn mean(values: impl Iterator<Item = f64>) -> f64 {
    let mut total = 0.0;
    let mut count = 0usize;
    for value in values {
        total += value;
        count += 1;
    }
    if count == 0 {
        1.0
    } else {
        total / count as f64
    }
}

fn p99(values: &mut [u64]) -> u64 {
    if values.is_empty() {
        return 0;
    }
    values.sort_unstable();
    let idx = ((values.len() * 99).div_ceil(100)).saturating_sub(1);
    values[idx]
}

fn elapsed_us(duration: Duration) -> u64 {
    let nanos = duration.as_nanos();
    u64::try_from(nanos.div_ceil(1000)).unwrap_or(u64::MAX)
}

fn latency_budget(sqlite_p99_us: u64) -> u64 {
    let relative = sqlite_p99_us.div_ceil(LATENCY_REGRESSION_DIVISOR);
    sqlite_p99_us.saturating_add(relative.max(LATENCY_JITTER_BUDGET_US))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latency_budget_keeps_absolute_jitter_floor_for_microbenchmarks() {
        assert_eq!(latency_budget(0), LATENCY_JITTER_BUDGET_US);
        assert_eq!(latency_budget(50), 150);
        assert_eq!(latency_budget(10_000), 10_500);
    }
}
