# PH22 · T04 — TemporalLensFlags: retrieval_only + excluded_from_dedup

| Field | Value |
|---|---|
| **Phase** | PH22 — Default panels + temporal lenses E2/E3/E4 |
| **Stage** | S3 — Registry / Lenses |
| **Crate** | `calyx-registry` |
| **Files** | `crates/calyx-registry/src/temporal/mod.rs` (≤500), `crates/calyx-registry/src/lens.rs` (≤500) |
| **Depends on** | T01, T02, T03 (this phase) |
| **Axioms** | A27 |
| **PRD** | `dbprdplans/25 §2`, `dbprdplans/25 §5`, `dbprdplans/05 §7` |

## Goal

Add `retrieval_only: bool` and `excluded_from_dedup: bool` fields to
`LensSpec`. Set both to `true` for all three temporal lenses (E2/E3/E4) and
`false` for all other lenses. These flags are the mechanism by which PH40
(temporal fusion) and PH41 (dedup) find the AP-60 boundary. Also add a
`TemporalLensFlags` struct as a convenience grouping and expose it from the
temporal module.

## Build (checklist of concrete, code-level steps)

- [x] Add to `LensSpec`:
  `pub retrieval_only: bool,`  (default `false`)
  `pub excluded_from_dedup: bool,`  (default `false`)
  — non-breaking if `LensSpec` has a builder or `Default`; add `#[serde(default)]`
  on both fields.
- [x] `TemporalLensFlags` struct: `retrieval_only: bool`, `excluded_from_dedup: bool`.
  Constant `PUB TEMPORAL_FLAGS: TemporalLensFlags = TemporalLensFlags { retrieval_only: true, excluded_from_dedup: true }`.
- [x] In the constructors for `E2RecencyLens`, `E3PeriodicLens`,
  `E4PositionalLens`: set `spec.retrieval_only = true` and
  `spec.excluded_from_dedup = true` unconditionally.
- [x] `Registry::measure` adds a debug-mode assertion:
  ```
  debug_assert!(
      !spec.retrieval_only || calling_context == CallerContext::RetrievalBoost,
      "temporal lens called from non-retrieval context; AP-60 violation"
  );
  ```
  `CallerContext` is a new enum with variants `RetrievalBoost` and `Primary`;
  `Registry::measure` takes an optional `ctx: Option<CallerContext>`, default
  `None` (treated as `Primary` for the assert). **No hard error in this phase**
  — the assert fires in debug builds only; PH40 will enforce it properly.
- [x] Update all existing unit tests that construct `LensSpec` to add the new
  default fields (they default to `false`).

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: `E2RecencyLens` spec has `retrieval_only = true` and
  `excluded_from_dedup = true` after construction.
- [x] unit: `E3PeriodicLens` and `E4PositionalLens` same.
- [x] unit: a non-temporal `AlgorithmicLens` spec has both flags `false`.
- [x] unit: `LensSpec` serde round-trip with `retrieval_only = true` produces
  correct JSON `"retrieval_only": true`.
- [x] unit: `LensSpec` with old JSON (no `retrieval_only` key) deserializes
  with `retrieval_only = false` (serde default).
- [x] edge (≥3): (1) `TEMPORAL_FLAGS` constant has both fields true; (2)
  `TemporalLensFlags::default()` has both false; (3) debug assert fires when
  a temporal lens is called with `ctx = Some(CallerContext::Primary)` in
  debug builds.
- [x] fail-closed: N/A — flags are metadata; no runtime errors here.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `LensSpec.retrieval_only` and `LensSpec.excluded_from_dedup` fields
  in the registry's in-memory state and serde JSON
- **Readback:** `cargo test -p calyx-registry temporal_flags -- --nocapture 2>&1`
- **Prove:** output shows `E2 retrieval_only=true excluded_from_dedup=true`;
  same for E3 and E4; a non-temporal lens shows `false, false`; screenshot
  attached to PH22 GitHub issue

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH22 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
