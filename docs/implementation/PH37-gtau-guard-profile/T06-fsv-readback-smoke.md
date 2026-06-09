# PH37 · T06 — FSV harness — per-slot verdict readback + anti-flatten smoke test

| Field | Value |
|---|---|
| **Phase** | PH37 — Gτ Guard Math + GuardProfile |
| **Stage** | S8 — Ward Gτ Guard |
| **Crate** | `calyx-ward` |
| **Files** | `crates/calyx-ward/tests/guard_ph37_fsv.rs` (≤500) |
| **Depends on** | T05 (this phase) |
| **Axioms** | A3, A12, A16 |
| **PRD** | `dbprdplans/09 §1`, `09 §2`, `09 §4` |

## Goal

Provide the complete FSV harness for PH37: a single runnable test binary on
aiwonder that covers the phase's full exit gate — per-slot verdict readback,
the average-passing/slot-failing rejection proof, `CALYX_GUARD_OOD` emission,
and the anti-flatten source check. The output of this test run is the evidence
attached to the PH37 GitHub issue.

## Build (checklist of concrete, code-level steps)

- [ ] Write `tests/guard_ph37_fsv.rs` with `#[test] fn fsv_per_slot_verdict_readback`
      that:
      - Constructs a `GuardProfile` with slots `["content", "style"]`,
        τ = `{"content": 0.72, "style": 0.65}`, policy `AllRequired`,
        novelty `NewRegion`; `calibration: None`
      - Provides produced vecs (seeded `f32` arrays, seed=42) and matched vecs
        (seeded, seed=7); both pre-normalized
      - Calls `guard()`; prints `GuardVerdict` as `{:?}` and as JSON
        (`serde_json::to_string_pretty`)
      - Asserts `per_slot.len() == 2`; asserts each `SlotVerdict` has finite
        `cos` in `[-1.0, 1.0]`
- [ ] Write `#[test] fn fsv_average_passing_slot_failing_rejected` with the
      exact scenario from T05: cos=`[0.95, 0.45]`, τ=`[0.70, 0.70]`; assert
      `overall_pass == false` and `average_cosine_would_pass(..) == true`;
      print both values to stdout
- [ ] Write `#[test] fn fsv_ood_code_emitted` — call `guard()` in a failing
      scenario; capture `WardError::Ood { .. }`; print `format!("{}", err)`;
      assert the formatted string contains `"CALYX_GUARD_OOD"`
- [ ] Write `#[test] fn fsv_no_flatten_source_check` — read
      `concat!(env!("CARGO_MANIFEST_DIR"), "/src/guard.rs")` as a string;
      require the `INVARIANT A3` marker and assert no non-comment line contains
      aggregate-vector gate markers (`concat`, `extend_from_slice`, `.append(`,
      `flat_map`); print line count; assert ≤ 500
- [ ] Write `#[test] fn fsv_guard_profile_serde_roundtrip` — construct full
      `GuardProfile` with `CalibrationMeta` populated; round-trip via
      `serde_json`; assert equality; print JSON to stdout
- [ ] All test functions use `seed = 42` / `seed = 7` RNG via `rand::SeedableRng`
      (no `SystemTime`, no live network)

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: `fsv_per_slot_verdict_readback` — prints per-slot cos/tau/pass;
      JSON output parseable; assertion green
- [ ] unit: `fsv_average_passing_slot_failing_rejected` — prints
      `overall_pass=false` and `average_would_pass=true` to stdout
- [ ] unit: `fsv_ood_code_emitted` — formatted error string contains
      `CALYX_GUARD_OOD`
- [ ] unit: `fsv_no_flatten_source_check` — guard.rs ≤ 500 lines; `INVARIANT
      A3` present; aggregate-vector gate markers absent in non-comment source
      lines
- [ ] unit: `fsv_guard_profile_serde_roundtrip` — original == deserialized;
      JSON printed includes all required keys

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** durable aiwonder evidence root
  `/home/croyse/calyx/data/fsv-issue263-ph37-t06-<date>/` containing the
  captured cargo log, per-slot verdict JSON, anti-flatten source-check readback,
  and SHA-256 manifest. Stdout is only one captured artifact, not the verdict.
- **Readback:**
  ```
  root=/home/croyse/calyx/data/fsv-issue263-ph37-t06-<date>
  mkdir -p "$root"
  cargo test -p calyx-ward -- --nocapture 2>&1 | tee "$root/ph37-fsv.log"
  grep -E "CALYX_GUARD_OOD|overall_pass|per_slot|average_would_pass" "$root/ph37-fsv.log"
  xxd -g 1 "$root/per-slot-verdict.json" | head -32
  sha256sum "$root"/* | sort
  wc -l crates/calyx-ward/src/guard.rs
  ```
- **Prove:** grep output contains `CALYX_GUARD_OOD`, `overall_pass: false`,
  `average_would_pass: true`; `wc -l` shows ≤ 500; all tests marked `ok` in
  cargo output; `xxd` shows durable JSON bytes with per-slot `(cos,tau,pass)`;
  attach the root path, hashes, and readback excerpts to the PH37 GitHub issue

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] CPU↔GPU bit-parity ≤ 1e-3 on the golden cosine set (Forge-touching via
      Forge cosine in the guard)
- [ ] FSV evidence (readback output / screenshot) attached to the PH37 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
