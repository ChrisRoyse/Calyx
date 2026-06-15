# PH56 T07 - 10M-op RSS soak

## Status

Implemented for issue #474.

The soak lives in `crates/calyx-aster/tests/soak_ph56.rs` and the reset benchmark lives in
`crates/calyx-aster/benches/bench_arena_reset.rs`.

## Implementation

- Runs a deterministic 10,000,000-op Linux-only soak against `CfRouter`, which exercises the
  bounded Aster memtable/SST path without retaining MVCC version chains in memory.
- Operation mix is deterministic from seed `0xCA1A_0056`: 50% writes, 30% point reads,
  15% range scans, and 5% cache-miss queries.
- Uses tight PH56 caps:
  - arena cap: 4 MiB
  - memtable cap: 32 MiB
  - cache byte cap: 16 MiB
  - page slab slots: 8
- Samples `/proc/self/status` `VmRSS` every 1,000 ops.
- Injects 100,000 oversized write-admission attempts at the midpoint and requires
  `CALYX_BACKPRESSURE`.
- Writes structured evidence to the workspace `target/ph56_soak_rss.json` and to
  `CALYX_FSV_ROOT/ph56_soak_rss.json`.
- Emits Prometheus-compatible evidence lines in `ph56_soak_metrics.prom`.
- Keeps the full FSV test ignored by default; the smoke test runs in normal `cargo test`.

## Budget Model

The RSS budget is reported in the JSON as:

`initial_rss + configured_cap_sum * 1.20 + process_rss_headroom`

`configured_cap_sum` includes arena, active memtable, flush memtable, cache, flood-admission
buffer, and page slab bytes. `process_rss_headroom` is fixed at 64 MiB for Rust runtime and
allocator slack. The asserted leak signal is the tail-half RSS trend, which excludes expected
bounded warmup while caches and memtables fill.

## aiwonder Gates

Final commit: `b32ba8d0c9e6d655a00101a2358ee8686df6a4e3`.

- `cargo fmt --all -- --check`
- `.rs` line-count gate: clean
- `cargo check -p calyx-aster`
- `cargo clippy -p calyx-aster --all-targets -- -D warnings`
- `cargo test -p calyx-aster --test soak_ph56 -- --nocapture`
- `cargo test -p calyx-aster --test soak_ph56 ph56_soak_smoke_bounds_rss_and_backpressure -- --nocapture --test-threads=1`
- `cargo test -p calyx-aster --test soak_ph56 ph56_soak_smoke_bounds_rss_and_backpressure -- --nocapture --test-threads=4`
- `cargo bench -p calyx-aster --bench bench_arena_reset -- --warm-up-time 1 --measurement-time 1 --sample-size 10`

## Final FSV

Evidence root:

`/home/croyse/calyx/data/fsv-issue474-soak-final-20260614T190558Z`

Workspace target file:

`/home/croyse/calyx/repo/target/ph56_soak_rss.json`

The final root JSON and workspace target JSON had the same SHA-256:

`66cfb95e9edd5fb00df51f5791051e39c10500e40783bdaa9bb2bfe466564466`

Key readback values:

- `op_count`: `10000000`
- `flood_ops`: `100000`
- `rss_initial_bytes`: `8044544`
- `rss_final_bytes`: `168718336`
- `rss_max_bytes`: `168718336`
- `configured_cap_sum_bytes`: `121667584`
- `process_rss_headroom_bytes`: `67108864`
- `rss_budget_bytes`: `221154508`
- `rss_trend_bytes_per_op`: `0.5789049667151683`
- `rss_full_trend_bytes_per_op`: `2.2430509654509962`
- `backpressure_events_total`: `100389`
- `flood_backpressure_errors`: `100000`
- `memtable_rejected_total`: `100000`
- `cache_used_bytes`: `16776555`
- `cache_byte_cap`: `16777216`
- `arena_high_water_bytes`: `1087`
- `slab_max_utilization`: `0.0009765625`
- `page_slab_max_utilization`: `0.125`
- `sst_files`: `390`
- `arena_reset_mean_ns`: `1`

Metrics readback:

```text
calyx_rss_bytes{phase="PH56"} 168718336
calyx_backpressure_events_total{phase="PH56"} 100389
calyx_cache_used_bytes{phase="PH56"} 16776555
calyx_arena_high_water_bytes{phase="PH56"} 1087
```

Criterion readback from `target/criterion/bench_arena_reset/*/new/estimates.json`:

- 4 KiB arena slope: `1.319731140668228 ns`
- 4 MiB arena slope: `1.336404767018311 ns`
- 32 MiB arena slope: `1.3435593902999767 ns`
- 128 MiB arena slope: `1.5226987602046893 ns`

All reset measurements are below the 50 ns target and do not scale with arena capacity.

## Notes

- The issue text used `0xCALYX56` as a mnemonic seed, which is not valid Rust hex syntax.
  The implemented deterministic seed is `0xCA1A_0056`.
- Synapse issue `ChrisRoyse/Synapse#988` tracks the observed 120-second inline tool-call
  timeout cap during long FSV commands.
