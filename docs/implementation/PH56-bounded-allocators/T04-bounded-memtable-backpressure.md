# PH56 · T04 — Bounded memtable + backpressure — hard byte cap, `CALYX_BACKPRESSURE`

| Field | Value |
|---|---|
| **Phase** | PH56 — Bounded caches/queues/memtables + arenas/pools |
| **Stage** | S13 — Resource, GC & Reliability Hardening |
| **Crate** | `calyx-aster` |
| **Files** | `crates/calyx-aster/src/memtable/bounded.rs` (≤500) |
| **Depends on** | T03 (LRU+TTL cache pattern established) · PH08 (MVCC memtable exists) |
| **Axioms** | A26, A16 |
| **PRD** | `dbprdplans/24 §1`, `24 §6` |

## Goal

Retrofit the existing `calyx-aster` memtable with a hard byte cap and writer backpressure.
When the memtable approaches its high-water mark, writers receive slow-ack backpressure; at the
cap, writes are rejected with `CALYX_BACKPRESSURE` (fail closed). A parallel flush to SST is
triggered at the high-water threshold. This prevents unbounded heap growth from write bursts
and satisfies A26 for the LSM write path (hazard 2: flush stall; hazard 8: heap OOM).

## Build (checklist of concrete, code-level steps)

- [ ] Add `cap_bytes: usize`, `high_water_bytes: usize` (default 80% of cap), `used_bytes: AtomicUsize` to the memtable struct in `bounded.rs`
- [ ] Implement `BoundedMemtable::write(&self, key: &[u8], value: &[u8], seq: u64) -> Result<WriteAck, CalyxError>` — estimate `key.len() + value.len() + overhead`; if `used_bytes + size > cap_bytes` return `CALYX_BACKPRESSURE` immediately; if `used_bytes > high_water_bytes` signal flush trigger before returning ack
- [ ] Implement `BoundedMemtable::flush_trigger(&self) -> bool` — returns true when `used_bytes > high_water_bytes`; background flusher polls this
- [ ] Implement `BoundedMemtable::reset_after_flush(&self, flushed_bytes: usize)` — decrements `used_bytes` atomically after successful SST flush; never underflows (saturating sub)
- [ ] Implement `BoundedMemtable::used_bytes(&self) -> usize` and `cap_bytes(&self) -> usize` for metrics
- [ ] Add `CALYX_BACKPRESSURE` to `calyx-core` error catalog if not already present (structured code + remediation text: "reduce write rate; memtable at capacity; flush in progress")
- [ ] Wire `BoundedMemtable` into existing `calyx-aster` write path in place of unbounded memtable

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: write entries up to `high_water_bytes` → `flush_trigger()` returns true; write more up to `cap_bytes` → still succeeds; write one more byte → `CALYX_BACKPRESSURE`
- [ ] unit: `reset_after_flush(n)` decrements `used_bytes` by exactly `n`; subsequent write succeeds
- [ ] proptest: `forall cap in 1024..=1_048_576, writes: Vec<(key, value)>` — `used_bytes` never exceeds `cap_bytes`; all writes either succeed or return `CALYX_BACKPRESSURE`
- [ ] unit: concurrent writes from 8 threads — no data race (verified by `cargo test` with ThreadSanitizer or loom); `used_bytes` never exceeds `cap_bytes`
- [ ] unit: `reset_after_flush` with `flushed_bytes > used_bytes` → saturating underflow (used_bytes stays 0, no wrap-around)
- [ ] edge: `cap_bytes == 0` → every write returns `CALYX_BACKPRESSURE`
- [ ] edge: single write whose `key + value` exceeds `cap_bytes` → `CALYX_BACKPRESSURE` immediately (cannot fit even in empty memtable)
- [ ] fail-closed: fill to exactly cap, verify `CALYX_BACKPRESSURE` on next write; call `reset_after_flush(cap_bytes)`, verify write succeeds again

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `calyx_memtable_used_bytes` and `calyx_backpressure_events_total` Prometheus metrics on aiwonder
- **Readback:** `calyx readback --metric memtable_used_bytes` — must stay ≤ `cap_bytes` throughout the 1e7-op write soak; `calyx readback --metric backpressure_events_total` — must be non-zero when write flood injected
- **Prove:** inject a write flood at 2× the cap rate for 10 s; `memtable_used_bytes` plateaus at `cap_bytes` (not beyond); `backpressure_events_total` counter increments; no OOM kill; restart the process and verify no data loss past last-acked write.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH56 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
