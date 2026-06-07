# PH08 · T03 — Freshness / bounded-staleness reads

| Field | Value |
|---|---|
| **Phase** | PH08 — MVCC sequence numbers + snapshot reads |
| **Stage** | S1 — Aster storage core |
| **Crate** | `calyx-aster` |
| **Files** | `crates/calyx-aster/src/mvcc/lease.rs` (≤500), `crates/calyx-aster/src/mvcc/tests.rs` (≤500) |
| **Depends on** | T01 (SeqAllocator, Freshness enum) |
| **Axioms** | A26 |
| **PRD** | `dbprdplans/04 §6` |

## Goal

Prove that `Freshness::FreshDerived` rejects a derived structure whose seq is
behind the pinned base seq, and that `Freshness::StaleOk { max_lag }` accepts a
lagging derived structure up to `max_lag` seqs but rejects beyond. This is the
mechanism derived structures (ANN indexes, xterm cache, kernel) use to advertise
their build-seq and let callers choose to wait for fresh or accept stale.

## Build (checklist of concrete, code-level steps)

- [ ] Add test: `Freshness::FreshDerived.ensure(pinned_seq=10, derived_seq=10)`
  → Ok.
- [ ] Add test: `Freshness::FreshDerived.ensure(pinned_seq=10, derived_seq=9)`
  → `Err(code == "CALYX_STALE_DERIVED")`.
- [ ] Add test: `Freshness::StaleOk { max_lag: 5 }.ensure(10, 5)` → Ok;
  `ensure(10, 4)` → Err.
- [ ] Add test: `Freshness::StaleOk { max_lag: 0 }.ensure(10, 10)` → Ok;
  `ensure(10, 9)` → Err.
- [ ] Add proptest: for any `(pinned, derived, max_lag)` triple,
  `StaleOk { max_lag }.ensure(pinned, derived)` is Ok if and only if
  `derived >= pinned || pinned - derived <= max_lag`.
- [ ] Add test: `VersionedCfStore::pin_snapshot` with `Freshness::StaleOk {
  max_lag: 3 }` stores the freshness on the snapshot; calling
  `snapshot.freshness().ensure(...)` enforces the correct policy.
- [ ] Verify `Freshness` is used correctly in `AsterVault::snapshot_handle`
  (currently uses `FreshDerived`); add a `vault.pin_stale_snapshot(max_lag)`
  convenience that creates a `StaleOk` snapshot for ANN search paths.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: all four `FreshDerived`/`StaleOk` boundary cases (see above).
- [ ] proptest: `StaleOk` iff `pinned - derived <= max_lag`.
- [ ] edge (≥3): (1) `derived_seq > pinned_seq` (derived is newer) → always Ok
  for any freshness; (2) `max_lag = u64::MAX` → always Ok; (3) `pinned = 0` →
  always Ok (no writes yet).
- [ ] fail-closed: `FreshDerived` with derived behind pinned → `CALYX_STALE_DERIVED`.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `cargo test -p calyx-aster mvcc::tests::freshness` on aiwonder.
- **Readback:** `cargo test -p calyx-aster mvcc -- --nocapture 2>&1`
- **Prove:** All freshness boundary tests pass; proptest shows ≥100 cases. The
  `CALYX_STALE_DERIVED` code appears in the printed error for the out-of-tolerance
  case. Screenshot posted to PH08 GitHub issue.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH08 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
