# PH11 · T01 — Compaction debt meter + throttle proptest

| Field | Value |
|---|---|
| **Phase** | PH11 — Compaction + hot/cold tiering |
| **Stage** | S1 — Aster storage core |
| **Crate** | `calyx-aster` |
| **Files** | `crates/calyx-aster/src/compaction/mod.rs` (≤500) |
| **Depends on** | PH06 T05 (SstLevel, SstShard) |
| **Axioms** | A26 |
| **PRD** | `dbprdplans/04 §6`, `dbprdplans/24 §3` (anti-storm) |

## Goal

Prove that `CompactionDebt::measure` produces monotonically increasing
`score_milli` as `pending_bytes` grows relative to `target_bytes`, that
`CompactionThrottle::max_input_bytes` correctly skips compaction when pending
bytes exceed the throttle cap, and that write-amp is 0 for zero-debt situations.
These are the invariants the scheduler uses to choose cadence and throttle.

## Build (checklist of concrete, code-level steps)

- [x] Add proptest: for any `(pending_bytes, target_bytes > 0)`,
  `CompactionDebt::measure(...).score_milli == pending_bytes * 1000 / target_bytes`
  (integer division, no overflow for sane inputs).
- [x] Add test: `CompactionDebt { pending=0, target=64MiB }.score_milli == 0`.
- [x] Add test: `CompactionDebt { pending=64MiB, target=64MiB }.score_milli == 1000`.
- [x] Add test: `CompactionDebt { pending=128MiB, target=64MiB }.score_milli == 2000`.
- [x] Add test: `compact_shards` with `throttle.max_input_bytes = Some(32 MiB)`
  and `pending_bytes = 64 MiB` → `CompactionResult::Skipped { debt }` where
  `debt.pending_bytes == 64 MiB`.
- [x] Add proptest: for any `shards` list, `CompactionDebt::measure(shards).pending_bytes
  == shards.iter().map(|s| s.bytes).sum()`.
- [x] Add test: `compact_shards` on an empty input list → `Skipped` (no crash,
  no SST output file).

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: 3 known-size shards → debt.pending_bytes is sum of sizes.
- [x] unit: throttle skips when pending > max_input.
- [x] proptest: `score_milli` formula correct for all (pending, target) pairs.
- [x] edge (≥3): (1) target=0 → `target_bytes.max(1)` prevents div-by-zero;
  (2) single shard → compacted to 1 output; (3) 0 shards → Skipped.
- [x] fail-closed: `compact_shards` on a shard with a non-existent path →
  `CALYX_DISK_PRESSURE` (stat fails).

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `cargo test -p calyx-aster compaction::tests` on aiwonder.
- **Readback:** `cargo test -p calyx-aster compaction -- --nocapture 2>&1`
- **Prove:** Proptest shows ≥100 passing cases; unit tests print the expected
  debt score values and assert. Screenshot posted to PH11 GitHub issue.

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH11 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
