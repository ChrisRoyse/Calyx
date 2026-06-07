# PH56 · T07 — 1e7-op soak — RSS bounded, no leak, backpressure fires before OOM

| Field | Value |
|---|---|
| **Phase** | PH56 — Bounded caches/queues/memtables + arenas/pools |
| **Stage** | S13 — Resource, GC & Reliability Hardening |
| **Crate** | `calyx-aster`, `calyx-core` |
| **Files** | `crates/calyx-aster/tests/soak_ph56.rs` (≤500) |
| **Depends on** | T01, T02, T03, T04, T05, T06 (all bounded infrastructure complete) |
| **Axioms** | A26, A16 |
| **PRD** | `dbprdplans/24 §1`, `24 §6`, `24 §7` hazard 8 |

## Goal

Prove on aiwonder — by reading the actual metric bytes — that after 1e7 mixed operations
(writes, reads, queries, compaction triggers, cache misses, flushes) RSS stays bounded, no
allocation leaks through arena/slab/cache/memtable, and `CALYX_BACKPRESSURE` fires (not OOM)
when a write flood is injected. This is the phase FSV gate: a green test harness does not
count; the byte-level RSS metric series is the verdict.

## Build (checklist of concrete, code-level steps)

- [ ] Write `soak_ph56.rs` integration test that opens a `calyx-aster` instance with a tight config: `arena_cap=4MiB`, `memtable_cap=32MiB`, `cache_byte_cap=16MiB`, `hotpool_high_water=0.85`
- [ ] Implement `op_loop(n: u64, rng: &mut SmallRng)` — issues `n` operations sampled by weight: 50% writes (random key+value 64–4096 bytes), 30% point reads, 15% range scans, 5% cache-miss queries; uses seeded RNG (`SmallRng::seed_from_u64(0xCALYX56)`) for determinism
- [ ] Sample RSS every 1000 ops via `/proc/self/status VmRSS` into a `Vec<u64>` (aiwonder is Linux)
- [ ] At op 5e6, inject a write flood: 1e5 ops at 10× normal rate in a tight loop; verify `CALYX_BACKPRESSURE` is returned at least once during the flood; verify no `std::alloc::handle_alloc_error` / OOM kill
- [ ] After 1e7 ops, compute: `rss_max`, `rss_final`, `rss_trend` (linear regression slope over samples); assert `rss_trend < 1.0 bytes/op` (bounded, not leaking)
- [ ] Serialize the RSS series to a JSON file `target/ph56_soak_rss.json` for evidence attachment to the GitHub issue
- [ ] Add `criterion` benchmark `bench_arena_reset` verifying O(1) reset: 1e6 resets, latency < 50 ns mean

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] soak: 1e7 ops with `seed=0xCALYX56` → `rss_max` ≤ configured cap sum + 20% overhead (documented threshold); `rss_trend < 1.0 bytes/op`
- [ ] soak: at least 1 `CALYX_BACKPRESSURE` error during the write-flood phase (verified by counting error returns)
- [ ] soak: `arena_high_water_bytes` from `AllocStats` is bounded (≤ `arena_cap`; never exceeds)
- [ ] soak: `slab_utilization` stays < 1.0 under normal load (slab not exhausted except during flood)
- [ ] soak: `cache_used_bytes` ≤ `cache_byte_cap` throughout (checked at end of run from dumped stats)
- [ ] edge: run with `--test-threads=1` (single-threaded) first, then `--test-threads=4` (concurrent); both must pass
- [ ] fail-closed: no `panic!` / unwrap failure / OOM kill in the 1e7-op run; every error is a structured `CALYX_*` code

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `target/ph56_soak_rss.json` produced on aiwonder; Prometheus `calyx_rss_bytes` metric series; `calyx_backpressure_events_total` counter
- **Readback:**
  ```
  cargo test --release --test soak_ph56 -- --nocapture 2>&1 | tee /tmp/ph56_soak.log
  cat target/ph56_soak_rss.json | python3 -c "import json,sys; d=json.load(sys.stdin); print(f'max={max(d[\"rss_kib\"])} trend={d[\"trend_bytes_per_op\"]:.4f}')"
  calyx readback --metric backpressure_events_total
  ```
- **Prove:** `trend_bytes_per_op < 1.0` in the JSON (RSS not leaking); `max` RSS ≤ sum of all caps + 20%; `backpressure_events_total > 0` (reject fires before OOM). Attach `ph56_soak_rss.json` + the `calyx readback` output as FSV evidence to the PH56 GitHub issue.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH56 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
