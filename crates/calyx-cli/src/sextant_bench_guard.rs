use calyx_core::CalyxError;

use crate::error::{CliError, CliResult};

pub(crate) const CALYX_FSV_FLAT_BENCH_MATERIALIZES: &str = "CALYX_FSV_FLAT_BENCH_MATERIALIZES";

const BYTES_PER_F32: u128 = std::mem::size_of::<f32>() as u128;
const GIB: u128 = 1024 * 1024 * 1024;
const MAX_FLAT_BENCH_RAW_DENSE_BYTES: u128 = GIB;

pub(crate) fn require_flat_bench_budget(
    command: &'static str,
    n_cx: usize,
    dim: usize,
) -> CliResult {
    let Some(raw_dense_bytes) = raw_dense_bytes(n_cx, dim) else {
        return Err(flat_bench_error(command, n_cx, dim, None));
    };
    if raw_dense_bytes > MAX_FLAT_BENCH_RAW_DENSE_BYTES {
        return Err(flat_bench_error(command, n_cx, dim, Some(raw_dense_bytes)));
    }
    Ok(())
}

fn raw_dense_bytes(n_cx: usize, dim: usize) -> Option<u128> {
    (n_cx as u128)
        .checked_mul(dim as u128)?
        .checked_mul(BYTES_PER_F32)
}

fn flat_bench_error(
    command: &'static str,
    n_cx: usize,
    dim: usize,
    raw_dense_bytes: Option<u128>,
) -> CliError {
    let size = raw_dense_bytes
        .map(format_bytes)
        .unwrap_or_else(|| "overflow".to_string());
    CliError::Calyx(CalyxError {
        code: CALYX_FSV_FLAT_BENCH_MATERIALIZES,
        message: format!(
            "{command} would materialize {n_cx}x{dim} f32 synthetic rows ({size} raw) in the legacy flat bench path; limit is {} raw",
            format_bytes(MAX_FLAT_BENCH_RAW_DENSE_BYTES)
        ),
        remediation: "use build-partitioned-vault --vectors and bench partitioned-search with .fbin sources",
    })
}

fn format_bytes(bytes: u128) -> String {
    if bytes.is_multiple_of(GIB) {
        format!("{} GiB", bytes / GIB)
    } else {
        format!("{bytes} bytes")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_flat_bench_is_allowed() {
        require_flat_bench_budget("calyx build-bench-vault", 64, 8)
            .expect("small synthetic bench remains usable");
    }

    #[test]
    fn exact_raw_dense_byte_cap_is_allowed() {
        let n_cx = (MAX_FLAT_BENCH_RAW_DENSE_BYTES / BYTES_PER_F32) as usize;

        require_flat_bench_budget("calyx bench search", n_cx, 1)
            .expect("exact cap is still within budget");
    }

    #[test]
    fn one_vector_past_cap_fails_closed() {
        let n_cx = (MAX_FLAT_BENCH_RAW_DENSE_BYTES / BYTES_PER_F32) as usize + 1;
        let error = require_flat_bench_budget("calyx bench recall", n_cx, 1)
            .expect_err("legacy flat bench must fail before allocating");

        assert_eq!(error.code(), CALYX_FSV_FLAT_BENCH_MATERIALIZES);
        assert!(error.message().contains("would materialize"), "{error}");
    }

    #[test]
    fn arithmetic_overflow_fails_closed() {
        let error = require_flat_bench_budget("calyx bench search", usize::MAX, usize::MAX)
            .expect_err("overflowed size math must fail closed");

        assert_eq!(error.code(), CALYX_FSV_FLAT_BENCH_MATERIALIZES);
    }
}
