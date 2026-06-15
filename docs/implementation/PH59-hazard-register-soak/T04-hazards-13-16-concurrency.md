# PH59 · T04 — Hazards 13–16: hot-shard skew, lock contention, cache stampede, slow-lens HOL

| Field | Value |
|---|---|
| **Phase** | PH59 — 25-hazard register FSV + soak |
| **Stage** | S13 — Resource, GC & Reliability Hardening |
| **Crate** | `calyx-hazard-soak` |
| **Files** | `crates/calyx-hazard-soak/src/hazards/operational.rs` (≤500, partial) |
| **Depends on** | PH09 (Constellation CRUD + isolation), PH56 T03 (LRU+TTL cache + single-flight) |
| **Axioms** | A26, A16 |
| **PRD** | `dbprdplans/24 §7` hazards 13–16 |

## Goal

Drive hazards 13 (hot-shard tenant skew), 14 (lock contention / deadlock under concurrency),
15 (cache stampede / thundering herd on kernel/cross-term recompute), and 16 (slow-lens
head-of-line blocking), read the SoT bytes, prove each mitigation. Hazard 15 specifically
requires the single-flight + LRU + TTL jitter from PH56 T03. Hazard 16 requires the
per-lens timeout + circuit breaker from PH20/PH58.

## Build (checklist of concrete, code-level steps)

**Hazard 13 — Hot-shard / tenant skew:**
- [ ] `fn probe_h13_hot_shard(vault: &mut Vault) -> HazardResult`:
  - Create 10 vaults; route 90% of writes to vault 0 (hot shard)
  - Run 60 s; verify vault 0 does not starve vaults 1–9 (read throughput on vaults 1–9 ≥ 50% of baseline)
  - Verify per-vault rate limits fire on vault 0 (`CALYX_RATE_LIMITED` counter > 0)
  - Verify no single-vault collapse (all vaults still respond)

**Hazard 14 — Lock contention:**
- [ ] `fn probe_h14_lock_contention(vault: &mut Vault) -> HazardResult`:
  - Spawn 64 concurrent writer threads all targeting the same vault
  - Run for 30 s with `--cfg loom` integration (if loom available) or ThreadSanitizer
  - Verify zero deadlocks (all threads complete within 30 s + timeout)
  - Verify read throughput during concurrent writes ≥ 80% of single-writer baseline (MVCC lock-free reads)
  - Record `calyx_lock_contention_events_total` (optional advisory metric)

**Hazard 15 — Cache stampede / thundering herd:**
- [ ] `fn probe_h15_cache_stampede(cache: &LruTtlCache) -> HazardResult`:
  - Expire a high-value cache entry (advance mock clock past TTL)
  - Simultaneously fire 100 concurrent `get()` calls for the same key
  - Verify the recompute function is called exactly once (single-flight gate)
  - All 100 callers receive the result; none block indefinitely
  - Record `cache_stampede_single_flight_count` (must be 1, not 100)
  - Verify `LruTtlCache` TTL jitter is configured (inspect `jitter` field > 0)

**Hazard 16 — Slow-lens head-of-line:**
- [ ] `fn probe_h16_slow_lens_hol(registry: &LensRegistry) -> HazardResult`:
  - Register a synthetic "slow lens" that takes 5 s to respond
  - Fire a search that uses both the slow lens and 2 fast lenses
  - Verify: the per-lens timeout fires after `lens_timeout_ms`; the circuit breaker trips after `breaker_threshold` failures
  - Verify: search returns results from the 2 fast lenses (graceful degradation, not a hang)
  - Verify: `lens_timeout_total` and `lens_breaker_trips_total` counters increment
  - Verify: restoring the slow lens closes the breaker after the half-open probe succeeds

- [ ] Aggregate into `target/ph59_hazards_13_16.json`

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] H13: vaults 1–9 throughput ≥ 50% of baseline during hot-shard injection (measured ops/s); `CALYX_RATE_LIMITED >= 1`
- [ ] H14: all 64 threads complete in ≤ 30 s (no deadlock timeout); no ThreadSanitizer data-race reports
- [ ] H15: recompute called exactly once during 100-concurrent-miss scenario (verified by a counter in the `recompute_fn` closure that runs atomically); all 100 callers get the correct value
- [ ] H16: search with slow-lens returns in ≤ `lens_timeout_ms + 100 ms` (not 5 s); results from fast lenses present; `lens_timeout_total >= 1`; `lens_breaker_trips_total >= 1`
- [ ] edge: H15 — TTL jitter = 0 (synchronous) → stampede risk is present but single-flight still protects; verify single-flight holds even without jitter
- [ ] edge: H16 — all lenses slow → search returns `CALYX_LENS_TIMEOUT` with partial results or empty; does not hang for 5× lens_timeout_ms
- [ ] fail-closed: H14 — if a deadlock were to occur (injected via a test double), the `Mutex::try_lock_for(timeout)` pattern returns an error rather than blocking forever

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `target/ph59_hazards_13_16.json`; Prometheus `calyx_rate_limited_total`, `calyx_lens_timeout_total`, `calyx_lens_breaker_trips_total`
- **Readback:**
  ```
  calyx readback --metric rate_limited_total
  calyx readback --metric lens_timeout_total
  calyx readback --metric lens_breaker_trips_total
  cat target/ph59_hazards_13_16.json | python3 -c "import json,sys; d=json.load(sys.stdin); print('passed:', all(h['passed'] for h in d))"
  ```
- **Prove:** all four hazards report `passed: true`; `lens_timeout_total >= 1` and `lens_breaker_trips_total >= 1` (breaker fired); H15 single-flight count == 1 (stamped in JSON evidence). Attach JSON + readback output to PH59 GitHub issue.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH59 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
