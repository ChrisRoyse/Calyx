# PH28 · T06 — `AssayGate` impl + `lens_signal` wire-up + planted-signal FSV

| Field | Value |
|---|---|
| **Phase** | PH28 — KSG MI + partitioned NMI |
| **Stage** | S5 — Loom + Assay (DDA & Bits) |
| **Crate** | `calyx-assay` |
| **Files** | `crates/calyx-assay/src/gate.rs` (≤500), `crates/calyx-assay/src/tests.rs` (≤500) |
| **Depends on** | T01, T02, T03, T05 (all estimators) · PH27 T03 (AssayGate trait stub) |
| **Axioms** | A2, A16 |
| **PRD** | `dbprdplans/07 §1`, `07 §2`, `07 §8` |

## Goal

Implement the real `AssayGate` that replaces PH27's stub and wires `ksg_with_ci`
into the materialization plan. Implement `lens_signal(slot, anchor) -> MiEstimate`
as the public entry point for per-lens signal measurement. Run the planted-signal
FSV: MI on a planted-signal synthetic is within the CI of the known value; n<50
fails closed. This card closes PH28.

Post-sweep #318 status: current `AssayGate::lens_signal` uses the logistic-probe
public estimator with seeded bootstrap CI, and `pair_gain_estimate` carries a
conservative CI derived from the left/right/pair bootstrap-backed estimates.
The Aster Assay CF readback for #318 proves persisted `MiEstimate` rows include
`ci_low`/`ci_high` bytes.

Post-sweep #319 status: `AsterAssayMaterializationGate` reads AsterVault
slot/anchor rows, computes grounded PairGain, and drives Loom materialization
planning. Post-sweep #340 makes missing vectors or anchors observable by
default: `materialization_plan` returns `CALYX_STALE_DERIVED` and records
`last_error()`. Returning `0.0` for lazy interaction storage is available only
through the explicit `materialization_plan_fail_safe_lazy` opt-in, which keeps
Agreement eager and parks Interaction lazy.

## Build (checklist of concrete, code-level steps)

- [x] Implement `AssayGateImpl` (the real Assay adapter used by PH27 planning):
  - `pair_gain(slot_a, slot_b, anchor, vault, forge, clock) -> Result<f32, CalyxError>`:
    - load the slot vectors for `(slot_a, slot_b)` and the anchor labels from Aster
    - if n < 50 or required rows are missing → return a fail-closed error by
      default; lazy `0.0` materialization is an explicit fallback call
    - call `ksg_with_ci` for `I(slot_a, slot_b ; anchor) − max(I(slot_a ; anchor), I(slot_b ; anchor))`
    - return the cross-term gain in bits; if negative (no synergy) → return 0.0
- [x] Implement `lens_signal(slot: SlotId, anchor: AnchorKind, vault, forge, clock) -> Result<MiEstimate, CalyxError>`:
  - load slot vectors + anchor labels; call `ksg_with_ci(slot_vecs, anchor_labels, k=5, bootstrap_config, forge)`
  - tag result `trust: Trusted` iff anchor is grounded (A2); else `Provisional`
  - persist `Slot.bits_about[anchor]` to Aster (the `assay` CF, keyed `(slot_id, anchor_kind, shard_hash, ts)`)
- [x] Implement `pair_redundancy(slot_a, slot_b, vault, clock) -> Result<NmiEstimate, CalyxError>`:
  - calls `pair_redundancy_nmi` from T04 on the Agreement scalar stream for this pair
- [x] Wire `AssayGateImpl` into `MaterializationPlan::plan_cross_terms` — now a live gate, not a stub

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] integration: planted-signal test: 100 constellations, slot_a vectors correlated with a binary anchor (known MI ≈ 0.3 bits, n=100, seed=42) → `lens_signal` returns `MiEstimate` with the known value inside `[ci_low, ci_high]`; tagged `trust: Trusted`
- [x] integration: `pair_gain` > 0.05 bits for a planted synergistic pair → materialization plan marks the pair `EagerStore`; pair_gain ≤ 0.0 for an independent pair → `LazyCache`
- [x] unit: ungrounded anchor (auto-labeled) → `lens_signal` returns `trust: Provisional`; grounded anchor → `Trusted`
- [x] edge: slot with zero non-NaN values → `CALYX_LOOM_ZERO_NORM_VECTOR` propagated; anchor with fewer than 50 grounded labels → `CALYX_ASSAY_INSUFFICIENT_SAMPLES` from inner estimator
- [x] fail-closed: Aster read failure on slot vectors → `CALYX_ASTER_NOT_FOUND` propagated; never returns a fabricated 0.0 MI when data is missing

## FSV (read the bytes on aiwonder — the truth gate)

> **Post-sweep #318 superseding readback:** Run:
> ```
> CALYX_FSV_ROOT=/home/croyse/calyx/data/fsv-issue318-bootstrap-ci-20260608 \
>   cargo test -p calyx-assay bootstrap_ci_aiwonder_fsv -- --ignored --nocapture --test-threads=1
> ```
> Then read the Aster Assay CF raw `value_hex` and decoded rows in
> `bootstrap-ci-readback.json`; each persisted public estimator/gate row must
> carry `ci_low` and `ci_high` bytes.

> **Post-sweep #319 materialization readback:** Run:
> ```
> CALYX_FSV_ROOT=/home/croyse/calyx/data/fsv-issue319-aster-materialization-gate-20260608 \
>   cargo test -p calyx-assay aster_materialization_gate_aiwonder_fsv -- --ignored --nocapture --test-threads=1
> ```
> Then read the source AsterVault CF counts and Loom xterm CF kind counts in
> `aster-materialization-gate-readback.json`.

- **SoT:** `Slot.bits_about` persisted to the assay CF for a planted-signal synthetic vault
- **Readback:**
  ```
  calyx readback --cf assay --slot <id> --anchor grounded_outcome
  ```
  The stored `MiEstimate` bytes must decode to `bits ∈ [ci_low, ci_high]` where the known MI is in that range.
- **Prove:** run the planted-signal integration test on aiwonder; read back the persisted bits via `calyx readback`; confirm the known value is within the CI. Also run the n=30 test case and confirm `CALYX_ASSAY_INSUFFICIENT_SAMPLES` is written as the result (not a fabricated number). Evidence posted to PH28 GitHub issue.

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH28 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
