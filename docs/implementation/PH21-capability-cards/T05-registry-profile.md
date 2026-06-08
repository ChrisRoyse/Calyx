# PH21 · T05 — Registry.profile() + collapsed-lens flag

| Field | Value |
|---|---|
| **Phase** | PH21 — Capability cards / profile |
| **Stage** | S3 — Registry / Lenses |
| **Crate** | `calyx-registry` |
| **Files** | `crates/calyx-registry/src/profile.rs` (≤500) |
| **Depends on** | T02, T03, T04 (this phase) |
| **Axioms** | A6, A17 |
| **PRD** | `dbprdplans/05 §5`, `13_STAGE3_REGISTRY.md §PH21 FSV gate` |

## Goal

Implement `Registry.profile(lens_id, probe_set: Option<ProbeSet>) -> Result<CapabilityCard>`
— the main public method that orchestrates spread, separation, and cost
measurements into a single JSON card, and sets `collapsed = true` when
`participation_ratio < COLLAPSE_THRESHOLD`. This is the PH21 FSV gate.

## Build (checklist of concrete, code-level steps)

- [ ] `pub fn profile(&self, lens_id: LensId, probe_set: Option<ProbeSet>) -> Result<CapabilityCard>`:
  1. Look up spec; if absent → `CALYX_REGISTRY_LENS_NOT_FOUND`.
  2. If `probe_set` is `None` → create a minimal built-in probe set:
     8 short text inputs with 4 label pairs (`#[cfg(test)]` can use the
     built-in; production callers should pass a real probe set).
  3. Embed all probe inputs via `self.measure_batch(lens_id, &probe.inputs)`.
  4. Collect successful `SlotVector::Dense.data` into `Vec<Vec<f32>>`.
  5. `spread = spread_metrics(&embeddings)?`.
  6. `separation = separation_metric(&embeddings, probe.labels.as_deref())?`.
  7. `cost = measure_cost(self, lens_id, probe_set)?`.
  8. `coverage = cost.coverage` (computed in `measure_cost`).
  9. `collapsed = spread.participation_ratio < COLLAPSE_THRESHOLD`.
  10. `signal = None; differentiation = None` (delegated to Assay PH29).
  11. Keep Registry's probe-derived estimates only as explicit
      `proxy_signal`/`proxy_differentiation` fields.
  12. Return `CapabilityCard { lens_id, name, signal, differentiation,
      proxy_signal, proxy_differentiation, spread, separation, cost, coverage,
      collapsed }`.
- [ ] `Registry` gains method `profile`; re-export from `lib.rs`.
- [ ] Produce the card as a JSON string helper:
  `pub fn profile_json(&self, lens_id: LensId, probe_set: Option<ProbeSet>) -> Result<String>`
  wrapping `serde_json::to_string_pretty`.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: `profile` on an `AlgorithmicLens` with default probe set → returns
  `CapabilityCard` with `coverage > 0.0`, `spread.participation_ratio > 0.0`,
  `collapsed == false` (algorithmic lenses are not collapsed by design).
- [ ] unit: `profile` on a mock "collapsed" lens (returns same vector for all
  inputs) → `collapsed == true`, `participation_ratio < 0.05`.
- [ ] unit: `profile_json` produces valid JSON parseable back to
  `CapabilityCard`.
- [ ] integration (`#[ignore]`): `profile` on `TeiHttpLens` at `:8088` with
  32 real text probes → card has real numbers; printed to stdout.
- [ ] edge (≥3): (1) probe_set with 0 inputs → card returned with
  `coverage=0.0`, no panic; (2) all probes fail (wrong modality for lens) →
  `coverage=0.0`, `collapsed=true` by fallback; (3) `CALYX_REGISTRY_LENS_NOT_FOUND`
  for unknown id.
- [ ] fail-closed: unknown lens id → `CALYX_REGISTRY_LENS_NOT_FOUND`.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** JSON output of `profile_json` on aiwonder; collapsed-lens flag for
  the mock degenerate lens
- **Readback:**
  `cargo test -p calyx-registry registry_profile -- --include-ignored --nocapture 2>&1`
- **Prove:** output shows the full JSON card with
  `"collapsed": false` for a healthy GTE lens and `"collapsed": true` for the
  degenerate mock; `"signal": null` in both; screenshot attached to PH21
  GitHub issue

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH21 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
