# PH53 · T04 — KV layer (`(ns,key)→val`+TTL) and Time-series layer (`(series,ts)→point`+rollups)

| Field | Value |
|---|---|
| **Phase** | PH53 — Collections-as-any-model (relational/doc/KV/TS/blob) |
| **Stage** | S12 — Universal data layer |
| **Crate** | `calyx-aster` |
| **Files** | `crates/calyx-aster/src/layers/kv.rs` (≤500), `crates/calyx-aster/src/layers/timeseries.rs` (≤500) |
| **Depends on** | T01, T02 (Layer trait) |
| **Axioms** | A15, A16, A19 |
| **PRD** | `dbprdplans/04 §2`, `dbprdplans/20 §2` |

## Goal

Implement two more paradigm layers — KV (O(1) keyed state with TTL) and
time-series (`(series,ts)→point` with continuous rollups and retention policy).
Both sit on the same ordered transactional core as the relational and document
layers, using disjoint key-space discriminants. KV TTL is check-on-read for
PH53 (background janitor in PH58). Time-series rollup accumulators are written
in the same group-commit batch as the point to avoid O(n) scan on read.

## Build (checklist of concrete, code-level steps)

### KV layer

- [ ] Define KV key schema (discriminant `0x03`):
  ```
  key   = 0x03 | collection_id (8B BE) | ns (8B BE) | user_key (var, length-prefixed u16 BE)
  value = expires_at (u64 BE, 0=no expiry) | payload (raw bytes)
  ```
- [ ] Implement `kv_set(col: &Collection, ns: u64, key: &[u8], val: &[u8], ttl: Option<Duration>) -> Result<()>`:
  - Encode key + value (TTL → `expires_at = now + ttl`; None → 0).
  - Write in group-commit WAL batch; Ledger stub entry (A15).
- [ ] Implement `kv_get(col: &Collection, ns: u64, key: &[u8]) -> Result<Option<Vec<u8>>>`:
  - Point-read; check `expires_at` against `Clock::now()` (injected `Clock`
    trait — never `SystemTime::now()` in logic per binding rules).
  - Return `None` if absent or expired; do NOT return expired bytes.
- [ ] Implement `kv_delete(col: &Collection, ns: u64, key: &[u8]) -> Result<()>`:
  - Write tombstone; Ledger stub.

### Time-series layer

- [ ] Define TS key schema (discriminant `0x04`):
  ```
  point_key   = 0x04 | 0x00 | collection_id (8B BE) | series_id (8B BE) | ts (u64 BE nanoseconds)
  rollup_key  = 0x04 | 0x01 | collection_id (8B BE) | series_id (8B BE) | window_tag (u8) | window_start (u64 BE)
  value       = f64 BE (the measurement scalar)
  rollup_val  = count (u64 BE) | sum (f64 BE) | min (f64 BE) | max (f64 BE)
  ```
  Big-endian timestamp ensures range scans return points in time order.
- [ ] Implement `ts_write(col: &Collection, series: u64, ts: u64, val: f64) -> Result<()>`:
  - Write point_key in WAL batch.
  - Update rollup accumulator for each active window (1m, 1h, 1d by default)
    in the **same** WAL batch: read current rollup_val (or zero), add `val`,
    write back. One atomic commit.
  - Ledger stub entry.
- [ ] Implement `ts_range(col: &Collection, series: u64, start_ts: u64, end_ts: u64) -> Result<Vec<(u64,f64)>>`:
  - Range scan `[start, end]` on the point key prefix; return `(ts, val)` pairs
    in ascending order.
- [ ] Implement `ts_rollup(col: &Collection, series: u64, window: RollupWindow) -> Result<Option<RollupValue>>`:
  - Point-read the rollup_key for the requested window; return `None` if no
    data written yet.
- [ ] Define `RollupWindow` enum: `OneMinute | OneHour | OneDay`.
- [ ] Respect `RetentionPolicy::DropAfter(d)` on `ts_range`: skip points older
  than `now - d`. (Physical deletion by PH58 janitor.)

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit KV: `kv_set(ns=1, key=b"foo", val=b"bar", ttl=None)` →
  `kv_get(ns=1, key=b"foo")` == `Some(b"bar")`; after vault restart, same.
- [ ] unit KV TTL: `kv_set(ttl=Some(1ns))` → advance clock by 2ns →
  `kv_get` == `None`.
- [ ] proptest KV: `kv_get(kv_set(ns,k,v,None)) == Some(v)` for arbitrary
  `ns`, `k`, `v`.
- [ ] unit TS: write 3 points `(ts=100,val=1.0)`, `(ts=200,val=2.0)`,
  `(ts=300,val=3.0)` → `ts_range(0, 400)` returns all 3 in order;
  `ts_rollup(OneHour)` returns `count=3, sum=6.0, min=1.0, max=3.0`.
- [ ] proptest TS: rollup `sum` equals sum of all written vals for that window
  and series.
- [ ] edge (≥3): (1) `kv_get` absent key → `None`; (2) `ts_range` with no
  points in range → empty vec; (3) `ts_rollup` before any write → `None`;
  (4) `RetentionPolicy::DropAfter(0)` skips all points.
- [ ] fail-closed: corrupt TS point value bytes → `CALYX_ASTER_CORRUPT_SHARD`.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `cf/kv/` and `cf/timeseries/` SST shards.
- **Readback:**
  ```
  calyx kv set --vault /home/croyse/calyx/test-vault --ns 1 --key foo --val bar
  calyx kv get --vault /home/croyse/calyx/test-vault --ns 1 --key foo
  xxd /home/croyse/calyx/test-vault/cf/kv/000001.sst | head -4

  calyx ts write --vault /home/croyse/calyx/test-vault --series cpu --ts 1700000100 --val 0.42
  calyx ts write --vault /home/croyse/calyx/test-vault --series cpu --ts 1700000200 --val 0.55
  calyx ts range  --vault /home/croyse/calyx/test-vault --series cpu --start 0 --end 9999999999
  calyx ts rollup --vault /home/croyse/calyx/test-vault --series cpu --window 1h
  xxd /home/croyse/calyx/test-vault/cf/timeseries/000001.sst | head -8
  ```
- **Prove:** KV: `0x03` discriminant in `xxd`; `kv_get` returns `"bar"` after
  restart. TS: `0x04 | 0x00` for point keys; `0x04 | 0x01` for rollup keys;
  `ts_rollup(1h).sum == 0.97`. Evidence posted to PH53 issue.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH53 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
