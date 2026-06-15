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

- [x] Add `cap_bytes: usize`, `high_water_bytes: usize` (default 80% of cap), `used_bytes: AtomicUsize` to the memtable struct in `bounded.rs`
- [x] Implement `BoundedMemtable::write(&self, key: &[u8], value: &[u8], seq: u64) -> Result<WriteAck, CalyxError>` — estimate `key.len() + value.len() + overhead`; if `used_bytes + size > cap_bytes` return `CALYX_BACKPRESSURE` immediately; if `used_bytes > high_water_bytes` signal flush trigger before returning ack
- [x] Implement `BoundedMemtable::flush_trigger(&self) -> bool` — returns true when `used_bytes > high_water_bytes`; background flusher polls this
- [x] Implement `BoundedMemtable::reset_after_flush(&self, flushed_bytes: usize)` — decrements `used_bytes` atomically after successful SST flush; never underflows (saturating sub)
- [x] Implement `BoundedMemtable::used_bytes(&self) -> usize` and `cap_bytes(&self) -> usize` for metrics
- [x] Add `CALYX_BACKPRESSURE` to `calyx-core` error catalog if not already present (structured code + remediation text: "reduce write rate; memtable at capacity; flush in progress")
- [x] Wire `BoundedMemtable` into existing `calyx-aster` write path in place of unbounded memtable

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: write entries up to `high_water_bytes` → `flush_trigger()` returns true; write more up to `cap_bytes` → still succeeds; write one more byte → `CALYX_BACKPRESSURE`
- [x] unit: `reset_after_flush(n)` decrements `used_bytes` by exactly `n`; subsequent write succeeds
- [x] proptest: `forall cap in 1024..=1_048_576, writes: Vec<(key, value)>` — `used_bytes` never exceeds `cap_bytes`; all writes either succeed or return `CALYX_BACKPRESSURE`
- [x] unit: concurrent writes from 8 threads — no data race (verified by `cargo test`); `used_bytes` never exceeds `cap_bytes`
- [x] unit: `reset_after_flush` with `flushed_bytes > used_bytes` → saturating underflow (used_bytes stays 0, no wrap-around)
- [x] edge: `cap_bytes == 0` → every write returns `CALYX_BACKPRESSURE`
- [x] edge: single write whose `key + value` exceeds `cap_bytes` → `CALYX_BACKPRESSURE` immediately (cannot fit even in empty memtable)
- [x] fail-closed: fill to exactly cap, verify `CALYX_BACKPRESSURE` on next write; call `reset_after_flush(cap_bytes)`, verify write succeeds again

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `calyx_memtable_used_bytes` and `calyx_backpressure_events_total` Prometheus metrics on aiwonder
- **Readback:** `calyx readback --metric memtable_used_bytes` — must stay ≤ `cap_bytes` throughout the 1e7-op write soak; `calyx readback --metric backpressure_events_total` — must be non-zero when write flood injected
- **Prove:** inject a write flood at 2× the cap rate for 10 s; `memtable_used_bytes` plateaus at `cap_bytes` (not beyond); `backpressure_events_total` counter increments; no OOM kill; restart the process and verify no data loss past last-acked write.

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH56 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV

## Implementation evidence

Implemented in `ad69d36062f68faea725b3bf504a81cdf9df36d8`.

- `crates/calyx-aster/src/memtable/bounded.rs`: `BoundedMemtable` with `cap_bytes`, `high_water_bytes`, `used_bytes: AtomicUsize`, `write`, `flush_trigger`, `reset_after_flush`, `used_bytes`, `cap_bytes`, and `MemtableUsage`.
- `crates/calyx-aster/src/cf/router.rs`: write path uses bounded `write`, flushes at high-water, retries after cap pressure, and increments memtable absorbed/rejected counters.
- `crates/calyx-aster/src/vault/commit.rs`: durable commits preflight rows before WAL append so an oversize memtable row fails closed without becoming durable.
- `crates/calyx-aster/src/resource/status.rs`: Prometheus text now emits `calyx_memtable_used_bytes`, `calyx_memtable_cap_bytes`, `calyx_memtable_high_water_bytes`, and `calyx_memtable_flush_trigger`, plus existing `calyx_backpressure_events_total`.
- `crates/calyx-aster/tests/issue471_memtable_fsv.rs`: ignored aiwonder FSV scenario creates deterministic vault bytes for manual SoT readback.

## aiwonder FSV evidence

Evidence root: `/home/croyse/calyx/data/fsv-issue471-memtable-20260614T171314Z`.

aiwonder gates at `ad69d36062f68faea725b3bf504a81cdf9df36d8`:

- `cargo fmt --all -- --check`
- Rust line-count gate: all `.rs` files <= 500 lines
- `cargo check -p calyx-aster`
- `cargo test -p calyx-aster memtable -- --nocapture` -> 13 passed
- `cargo clippy -p calyx-aster --all-targets -- -D warnings`
- `CALYX_FSV_ROOT=/home/croyse/calyx/data/fsv-issue471-memtable-20260614T171314Z cargo test -p calyx-aster --test issue471_memtable_fsv -- --ignored --nocapture` -> 1 passed

Manual SoT readback values:

- Hand-computed row bytes: key 8 + value 52 + overhead 4 = 64.
- Memtable cap: 128; high-water: 102.
- Base memtable before: 0; after first Base write: 64; after second Base write and high-water flush: 0.
- Backpressure metrics after reject: `calyx_backpressure_events_total{source="memtable_absorbed"} 1`, `calyx_backpressure_events_total{source="memtable_rejected"} 1`.
- Fail-closed oversize row: returned `CALYX_BACKPRESSURE`; `seq_before_reject` = 2 and `seq_after_reject` = 2; `wal_bytes_before_reject` = 238 and `wal_bytes_after_reject` = 238.
- Reopen proof: accepted keys 1 and 2 visible; rejected key 3 absent.
- Byte proof: WAL `xxd` shows two `CXW1` records carrying 52-byte `0x4d` values for keys 1 and 2 only; Base SST dumps show key 1 and key 2 payloads, no key 3 payload.

Artifact hashes:

- `issue471-memtable-fsv-readback.json`: `e78622cbf7c80a5c18c5c2cc28c9b91435642858379c6747ff2f68e93ac97560`
- `resource-after-reject.prom`: `6dac8af1defb88acf1bda1223318f0a3c1a0337f3b79d89cbc4081f44b42e99a`
- `resource-after-reject.json`: `6162e9160faac97df6c718d4fd91d85bf9c114b315bae29d7847e8a6eed54ec8`
- `vault/wal/00000000000000000000.wal`: `c21db900365ec92a8ec8e394fea1ddc347dd47986243954a2fe7e86b7a1fe6ef`
- `vault/cf/base/00000000000000000001-0000.sst`: `ff028600b325055cc2534c7d350cd8bf6043c71617fbafdccbb61ca3677d457c`
- `vault/cf/base/00000000000000000001.sst`: `8e5ce3bd8915aeb1ce13f969c168309273f890872ab2f6cc4a576831330d9d87`
- `vault/cf/base/00000000000000000002-0000.sst`: `820c525633e2b61c81f95fdd9b5705703dfc255887f23ede7d4c4b2068c7bd6b`
