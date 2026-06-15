# PH06 · T01 — Memtable freeze/rotate + backpressure proptest

| Field | Value |
|---|---|
| **Phase** | PH06 — Memtable + LSM SSTable writer/reader |
| **Stage** | S1 — Aster storage core |
| **Crate** | `calyx-aster` |
| **Files** | `crates/calyx-aster/src/memtable.rs` (≤500) |
| **Depends on** | PH05 (WAL contract) |
| **Axioms** | A26 |
| **PRD** | `dbprdplans/04 §2/§8` |

## Goal

Add a `freeze()` / rotate API to the memtable so the write path can atomically
hand off a `FrozenMemtable` for background flush while a new empty `Memtable`
accepts further writes. Harden the byte-cap backpressure with a proptest that
proves the invariant under random insert sequences: `estimated_bytes` never
exceeds `byte_cap` after any sequence of successful `put` calls.

## Build (checklist of concrete, code-level steps)

- [x] Add `FrozenMemtable` as a newtype over the sorted entries (`BTreeMap` or
  `Vec<(Vec<u8>, Vec<u8>)>`) with methods `iter()`, `len()`, `flush_to_sst()`.
- [x] Add `Memtable::freeze(self) -> FrozenMemtable` that consumes the memtable
  and returns an immutable view; no new writes are possible after freeze.
- [x] Add `Memtable::needs_flush(&self) -> bool` returning `estimated_bytes >=
  byte_cap * 9 / 10` (at 90% capacity, trigger pre-flush).
- [x] Keep the existing `put`, `get`, `iter`, `estimated_bytes`, `byte_cap`,
  `len`, `is_empty` API surface unchanged.
- [x] Update `flush_to_sst` on `FrozenMemtable` to use the same `sst::write_sst`
  path.
- [x] Write proptest: for any sequence of `(key: Vec<u8>, value: Vec<u8>)` pairs
  with `key.len() + value.len() <= byte_cap / 4`, after inserting until first
  `Err(CALYX_BACKPRESSURE)`, assert `estimated_bytes <= byte_cap`.
- [x] Write proptest: `freeze()` followed by `iter()` returns the same keys in
  the same order as `Memtable::iter()` before freeze.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: 3 puts that fit; freeze; frozen.len() == 3; frozen.flush_to_sst rounds
  trip byte-exact.
- [x] unit: `needs_flush` triggers at 90% capacity: put until exactly 9/10 of
  cap; assert `needs_flush() == true`; put one byte over → `Err(CALYX_BACKPRESSURE)`.
- [x] proptest: `∀ sequence of (k,v) pairs`: `estimated_bytes` after any sequence
  of successful puts ≤ `byte_cap`.
- [x] edge (≥3): (1) empty freeze → `FrozenMemtable` with 0 entries; (2) same key
  inserted twice, second overwrites — byte estimate updates correctly; (3)
  `byte_cap = 0` → first put returns `CALYX_BACKPRESSURE`.
- [x] fail-closed: overflow → `error.code == "CALYX_BACKPRESSURE"`; existing
  entries unaffected.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** Test output from `cargo test memtable` on aiwonder.
- **Readback:** `cargo test -p calyx-aster memtable -- --nocapture 2>&1 | tail -5`
- **Prove:** All tests pass; proptest output shows `100 tests passed` (or
  configured count); `CALYX_BACKPRESSURE` code appears in the failure output
  for the overflow case. The real byte-level proof comes in T02 (SST flush).

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH06 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
